//! Vulkan-only SDL_GPU indexed-palette conformance backend.
//!
//! This crate is deliberately an adapter around [`cmr_gpu_contract`]. SDL and
//! raw FFI values never cross the public contract. The safe `sdl3::gpu` layer
//! owns device resources and command encoding; the small [`raw`] module covers
//! the download and device-information calls missing from sdl3 0.20.0.
//!
//! The source-owned GLSL shaders are compiled to SPIR-V by an external,
//! currently unpinned `glslangValidator` build boundary. This implementation
//! therefore requests SDL's Vulkan driver explicitly; it does not claim
//! Direct3D 12 or Metal shader support.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use cmr_gpu_contract::{
    BackendCapabilities, BackendInfo, ConformanceFrame, FrameContractError, FrameTiming,
    GraphicsApi, IndexedFrameView, IndexedPaletteGpuBackend, ReadbackFormat, normalize_readback,
};
use sdl3::gpu::{
    ColorTargetDescription, ColorTargetInfo, CompareOp, CullMode, DepthStencilState, Device,
    FillMode, FrontFace, GraphicsPipeline, GraphicsPipelineTargetInfo, LoadOp, PrimitiveType,
    RasterizerState, Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode,
    ShaderFormat, ShaderStage, StoreOp, TextureCreateInfo, TextureFormat, TextureRegion,
    TextureSamplerBinding, TextureTransferInfo, TextureType, TextureUsage, TransferBufferUsage,
    VertexAttribute, VertexBufferDescription, VertexElementFormat, VertexInputRate,
    VertexInputState,
};
use sdl3::properties::{Properties, Setter};

const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/indexed.vert.spv"));
const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/indexed.frag.spv"));
const C3D_VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/c3d.vert.spv"));
const C3D_FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/c3d.frag.spv"));
const PALETTE_BYTES: usize = 256 * 4;
const TRANSFER_ALIGNMENT: usize = 512;
const READBACK_ROW_ALIGNMENT: usize = 256;
const SDL_GPU_DRIVER: &str = "vulkan";
static NEXT_C3D_BACKEND_TOKEN: AtomicU64 = AtomicU64::new(1);

/// SDL_GPU Vulkan device selection knobs exposed by its creation properties.
///
/// The graphics API is intentionally not configurable: this implementation's
/// source shaders compile only to SPIR-V and construction always requests
/// SDL's Vulkan driver.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SdlGpuConfig {
    pub prefer_low_power: bool,
    pub debug_mode: bool,
}

pub struct SdlGpuIndexedBackend {
    pipeline: GraphicsPipeline,
    c3d_pipeline: GraphicsPipeline,
    sampler: Sampler,
    device: Device,
    c3d_backend_token: u64,
    info: BackendInfo,
    _video: sdl3::VideoSubsystem,
    _sdl: sdl3::Sdl,
}

impl SdlGpuIndexedBackend {
    // The build script deliberately emits empty marker files when the optional
    // external shader compiler is unavailable; these lengths are not constant
    // across build environments.
    #[allow(clippy::const_is_empty)]
    pub fn new(config: SdlGpuConfig) -> Result<Self, SdlGpuError> {
        if env!("CMR_SDL_GPU_SPIRV_AVAILABLE") != "1"
            || VERTEX_SPIRV.is_empty()
            || FRAGMENT_SPIRV.is_empty()
            || C3D_VERTEX_SPIRV.is_empty()
            || C3D_FRAGMENT_SPIRV.is_empty()
        {
            return Err(SdlGpuError::ShaderCompilerUnavailable);
        }

        let sdl = sdl3::init().map_err(|error| SdlGpuError::sdl("initialization", error))?;
        let video = sdl
            .video()
            .map_err(|error| SdlGpuError::sdl("video initialization", error))?;
        let properties =
            Properties::new().map_err(|error| SdlGpuError::sdl("device properties", error))?;
        properties
            .set("SDL.gpu.device.create.debugmode", config.debug_mode)
            .map_err(|error| SdlGpuError::sdl("debug property", error))?;
        properties
            .set(
                "SDL.gpu.device.create.preferlowpower",
                config.prefer_low_power,
            )
            .map_err(|error| SdlGpuError::sdl("power preference", error))?;
        properties
            .set("SDL.gpu.device.create.shaders.spirv", true)
            .map_err(|error| SdlGpuError::sdl("shader-format property", error))?;
        properties
            .set("SDL.gpu.device.create.name", SDL_GPU_DRIVER)
            .map_err(|error| SdlGpuError::sdl("Vulkan driver property", error))?;
        let device = Device::new_with_properties(properties)
            .map_err(|error| SdlGpuError::sdl("device creation", error))?;
        let info = raw::backend_info(&device)?;
        if info.graphics_api != GraphicsApi::Vulkan {
            return Err(SdlGpuError::UnexpectedGraphicsApi {
                actual: info.graphics_api,
            });
        }

        if !raw::texture_supports(
            &device,
            TextureFormat::R8Uint,
            TextureType::_2D,
            TextureUsage::SAMPLER,
        ) {
            return Err(SdlGpuError::UnsupportedFormat("R8_UINT sampled texture"));
        }
        if !raw::texture_supports(
            &device,
            TextureFormat::R8g8b8a8Uint,
            TextureType::_2D,
            TextureUsage::SAMPLER,
        ) {
            return Err(SdlGpuError::UnsupportedFormat(
                "R8G8B8A8_UINT sampled texture",
            ));
        }
        if !raw::texture_supports(
            &device,
            TextureFormat::R8g8b8a8Unorm,
            TextureType::_2D,
            TextureUsage::COLOR_TARGET,
        ) {
            return Err(SdlGpuError::UnsupportedFormat(
                "R8G8B8A8_UNORM color target",
            ));
        }

        let vertex_shader = device
            .create_shader()
            .with_code(ShaderFormat::SPIRV, VERTEX_SPIRV, ShaderStage::Vertex)
            .with_entrypoint(c"main")
            .build()
            .map_err(|error| SdlGpuError::sdl("vertex shader creation", error))?;
        let fragment_shader = device
            .create_shader()
            .with_code(ShaderFormat::SPIRV, FRAGMENT_SPIRV, ShaderStage::Fragment)
            .with_samplers(2)
            .with_entrypoint(c"main")
            .build()
            .map_err(|error| SdlGpuError::sdl("fragment shader creation", error))?;
        let targets = [ColorTargetDescription::new().with_format(TextureFormat::R8g8b8a8Unorm)];
        let pipeline = device
            .create_graphics_pipeline()
            .with_vertex_shader(&vertex_shader)
            .with_fragment_shader(&fragment_shader)
            .with_primitive_type(PrimitiveType::TriangleList)
            .with_fill_mode(FillMode::Fill)
            .with_target_info(
                GraphicsPipelineTargetInfo::new().with_color_target_descriptions(&targets),
            )
            .build()
            .map_err(|error| SdlGpuError::sdl("pipeline creation", error))?;
        let c3d_vertex_shader = device
            .create_shader()
            .with_code(ShaderFormat::SPIRV, C3D_VERTEX_SPIRV, ShaderStage::Vertex)
            .with_uniform_buffers(1)
            .with_entrypoint(c"main")
            .build()
            .map_err(|error| SdlGpuError::sdl("C3D vertex shader creation", error))?;
        let c3d_fragment_shader = device
            .create_shader()
            .with_code(
                ShaderFormat::SPIRV,
                C3D_FRAGMENT_SPIRV,
                ShaderStage::Fragment,
            )
            .with_samplers(2)
            .with_entrypoint(c"main")
            .build()
            .map_err(|error| SdlGpuError::sdl("C3D fragment shader creation", error))?;
        let c3d_vertex_buffers = [VertexBufferDescription::new()
            .with_slot(0)
            .with_pitch(32)
            .with_input_rate(VertexInputRate::Vertex)
            .with_instance_step_rate(0)];
        let c3d_vertex_attributes = [
            VertexAttribute::new()
                .with_location(0)
                .with_buffer_slot(0)
                .with_format(VertexElementFormat::Int3)
                .with_offset(0),
            VertexAttribute::new()
                .with_location(1)
                .with_buffer_slot(0)
                .with_format(VertexElementFormat::Float2)
                .with_offset(12),
            VertexAttribute::new()
                .with_location(2)
                .with_buffer_slot(0)
                .with_format(VertexElementFormat::Int3)
                .with_offset(20),
        ];
        let c3d_targets = [ColorTargetDescription::new().with_format(TextureFormat::R8g8b8a8Unorm)];
        let c3d_pipeline = device
            .create_graphics_pipeline()
            .with_vertex_shader(&c3d_vertex_shader)
            .with_fragment_shader(&c3d_fragment_shader)
            .with_primitive_type(PrimitiveType::TriangleList)
            .with_vertex_input_state(
                VertexInputState::new()
                    .with_vertex_buffer_descriptions(&c3d_vertex_buffers)
                    .with_vertex_attributes(&c3d_vertex_attributes),
            )
            .with_rasterizer_state(
                RasterizerState::new()
                    .with_fill_mode(FillMode::Fill)
                    .with_cull_mode(CullMode::Back)
                    .with_front_face(FrontFace::Clockwise)
                    .with_enable_depth_clip(true),
            )
            .with_depth_stencil_state(
                DepthStencilState::new()
                    .with_enable_depth_test(true)
                    .with_enable_depth_write(true)
                    .with_compare_op(CompareOp::LessOrEqual),
            )
            .with_target_info(
                GraphicsPipelineTargetInfo::new()
                    .with_color_target_descriptions(&c3d_targets)
                    .with_has_depth_stencil_target(true)
                    .with_depth_stencil_format(TextureFormat::D32Float),
            )
            .build()
            .map_err(|error| SdlGpuError::sdl("C3D pipeline creation", error))?;
        let sampler = device
            .create_sampler(
                SamplerCreateInfo::new()
                    .with_min_filter(sdl3::gpu::Filter::Nearest)
                    .with_mag_filter(sdl3::gpu::Filter::Nearest)
                    .with_mipmap_mode(SamplerMipmapMode::Nearest)
                    .with_address_mode_u(SamplerAddressMode::ClampToEdge)
                    .with_address_mode_v(SamplerAddressMode::ClampToEdge),
            )
            .map_err(|error| SdlGpuError::sdl("sampler creation", error))?;
        Ok(Self {
            pipeline,
            c3d_pipeline,
            sampler,
            device,
            c3d_backend_token: NEXT_C3D_BACKEND_TOKEN.fetch_add(1, Ordering::Relaxed),
            info,
            _video: video,
            _sdl: sdl,
        })
    }
}

mod c3d;
pub use c3d::{SdlGpuPreparedC3dScene, SdlGpuPreparedC3dStats};

impl IndexedPaletteGpuBackend for SdlGpuIndexedBackend {
    type Error = SdlGpuError;

    fn info(&self) -> &BackendInfo {
        &self.info
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            sampled_r8_uint: true,
            timestamp_queries: false,
            headless_readback: true,
            device_recovery: false,
        }
    }

    fn render_readback(
        &mut self,
        frame: IndexedFrameView<'_>,
    ) -> Result<ConformanceFrame, Self::Error> {
        let extent = frame.extent();
        let width = u32::from(extent.width);
        let height = u32::from(extent.height);
        // Shared benchmark boundary: all per-frame sizing, allocation, upload,
        // command encoding, and submission are included in the CPU interval.
        let encode_start = Instant::now();
        let tight_pitch = usize::from(extent.width)
            .checked_mul(4)
            .ok_or(SdlGpuError::SizeOverflow)?;
        let readback_pitch = align_up(tight_pitch, READBACK_ROW_ALIGNMENT)?;
        let readback_len = readback_pitch
            .checked_mul(usize::from(extent.height))
            .ok_or(SdlGpuError::SizeOverflow)?;

        let index_texture = self
            .device
            .create_texture(
                TextureCreateInfo::new()
                    .with_format(TextureFormat::R8Uint)
                    .with_type(TextureType::_2D)
                    .with_width(width)
                    .with_height(height)
                    .with_layer_count_or_depth(1)
                    .with_num_levels(1)
                    .with_usage(TextureUsage::SAMPLER),
            )
            .map_err(|error| SdlGpuError::sdl("index texture creation", error))?;
        let palette_texture = self
            .device
            .create_texture(
                TextureCreateInfo::new()
                    .with_format(TextureFormat::R8g8b8a8Uint)
                    .with_type(TextureType::_2D)
                    .with_width(256)
                    .with_height(1)
                    .with_layer_count_or_depth(1)
                    .with_num_levels(1)
                    .with_usage(TextureUsage::SAMPLER),
            )
            .map_err(|error| SdlGpuError::sdl("palette texture creation", error))?;
        let target = self
            .device
            .create_texture(
                TextureCreateInfo::new()
                    .with_format(TextureFormat::R8g8b8a8Unorm)
                    .with_type(TextureType::_2D)
                    .with_width(width)
                    .with_height(height)
                    .with_layer_count_or_depth(1)
                    .with_num_levels(1)
                    .with_usage(TextureUsage::COLOR_TARGET),
            )
            .map_err(|error| SdlGpuError::sdl("target texture creation", error))?;

        let palette_offset = align_up(frame.indices().len(), TRANSFER_ALIGNMENT)?;
        let upload_len = palette_offset
            .checked_add(PALETTE_BYTES)
            .ok_or(SdlGpuError::SizeOverflow)?;
        let upload = self
            .device
            .create_transfer_buffer()
            .with_size(u32_len(upload_len)?)
            .with_usage(TransferBufferUsage::UPLOAD)
            .build()
            .map_err(|error| SdlGpuError::sdl("upload buffer creation", error))?;
        let palette_bytes = palette_rgba(frame);
        raw::write_upload(
            &self.device,
            &upload,
            upload_len,
            frame.indices(),
            palette_offset,
            &palette_bytes,
        )?;
        let download = self
            .device
            .create_transfer_buffer()
            .with_size(u32_len(readback_len)?)
            .with_usage(TransferBufferUsage::DOWNLOAD)
            .build()
            .map_err(|error| SdlGpuError::sdl("download buffer creation", error))?;

        let command = self
            .device
            .acquire_command_buffer()
            .map_err(|error| SdlGpuError::sdl("command buffer acquisition", error))?;
        let copy = self
            .device
            .begin_copy_pass(&command)
            .map_err(|error| SdlGpuError::sdl("upload pass", error))?;
        copy.upload_to_gpu_texture(
            TextureTransferInfo::new()
                .with_transfer_buffer(&upload)
                .with_pixels_per_row(width)
                .with_rows_per_layer(height),
            TextureRegion::new()
                .with_texture(&index_texture)
                .with_width(width)
                .with_height(height)
                .with_depth(1),
            false,
        );
        copy.upload_to_gpu_texture(
            TextureTransferInfo::new()
                .with_transfer_buffer(&upload)
                .with_offset(u32_len(palette_offset)?)
                .with_pixels_per_row(256)
                .with_rows_per_layer(1),
            TextureRegion::new()
                .with_texture(&palette_texture)
                .with_width(256)
                .with_height(1)
                .with_depth(1),
            false,
        );
        self.device.end_copy_pass(copy);

        let targets = [ColorTargetInfo::default()
            .with_texture(&target)
            .with_load_op(LoadOp::CLEAR)
            .with_store_op(StoreOp::STORE)];
        let render = self
            .device
            .begin_render_pass(&command, &targets, None)
            .map_err(|error| SdlGpuError::sdl("render pass", error))?;
        render.bind_graphics_pipeline(&self.pipeline);
        render.bind_fragment_samplers(
            0,
            &[
                TextureSamplerBinding::new()
                    .with_texture(&index_texture)
                    .with_sampler(&self.sampler),
                TextureSamplerBinding::new()
                    .with_texture(&palette_texture)
                    .with_sampler(&self.sampler),
            ],
        );
        render.draw_primitives(3, 1, 0, 0);
        self.device.end_render_pass(render);

        let copy = self
            .device
            .begin_copy_pass(&command)
            .map_err(|error| SdlGpuError::sdl("download pass", error))?;
        raw::download_texture(
            &copy,
            &target,
            &download,
            width,
            height,
            u32_len(readback_pitch / 4)?,
        );
        self.device.end_copy_pass(copy);
        let fence = command
            .submit_and_acquire_fence(&self.device)
            .map_err(|error| SdlGpuError::sdl("command submission", error))?;
        let cpu_encode_submit_ns = elapsed_ns(encode_start);
        let completion_start = Instant::now();
        self.device
            .wait_fences(true, &[fence])
            .map_err(|error| SdlGpuError::sdl("fence wait", error))?;
        let submit_to_completion_ns = elapsed_ns(completion_start);

        let mapped = raw::read_download(&self.device, &download, readback_len)?;
        let rgba8 = normalize_readback(extent, &mapped, readback_pitch, ReadbackFormat::Rgba8)
            .map_err(SdlGpuError::FrameContract)?;
        Ok(ConformanceFrame {
            rgba8,
            timing: FrameTiming {
                cpu_encode_submit_ns,
                submit_to_completion_ns,
                gpu_timestamp_ns: None,
            },
        })
    }
}

fn palette_rgba(frame: IndexedFrameView<'_>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PALETTE_BYTES);
    for entry in frame.palette() {
        bytes.extend_from_slice(&[entry.red, entry.green, entry.blue, u8::MAX]);
    }
    bytes
}

fn align_up(value: usize, alignment: usize) -> Result<usize, SdlGpuError> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or(SdlGpuError::SizeOverflow)
}

fn u32_len(value: usize) -> Result<u32, SdlGpuError> {
    u32::try_from(value).map_err(|_| SdlGpuError::SizeOverflow)
}

fn elapsed_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Debug)]
pub enum SdlGpuError {
    ShaderCompilerUnavailable,
    UnexpectedGraphicsApi {
        actual: GraphicsApi,
    },
    UnsupportedFormat(&'static str),
    SizeOverflow,
    NullTransferMap,
    PreparedSceneBackendMismatch,
    PreparedScenePoisoned,
    RawC3dContract(cmr_gpu_contract::RawC3dContractError),
    FrameContract(FrameContractError),
    Sdl {
        operation: &'static str,
        detail: String,
    },
}

impl SdlGpuError {
    fn sdl(operation: &'static str, source: impl fmt::Display) -> Self {
        Self::Sdl {
            operation,
            detail: source.to_string(),
        }
    }
}

impl fmt::Display for SdlGpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShaderCompilerUnavailable => write!(
                formatter,
                "SDL_GPU SPIR-V shaders unavailable; install an external glslangValidator or set CMR_GLSLANG_VALIDATOR (the toolchain is currently unpinned)"
            ),
            Self::UnexpectedGraphicsApi { actual } => write!(
                formatter,
                "SDL_GPU returned {actual:?}, but this SPIR-V backend supports Vulkan only"
            ),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "SDL_GPU does not support {format}")
            }
            Self::SizeOverflow => write!(formatter, "SDL_GPU transfer size overflow"),
            Self::NullTransferMap => write!(formatter, "SDL_GPU returned a null transfer map"),
            Self::PreparedSceneBackendMismatch => {
                write!(
                    formatter,
                    "SDL_GPU prepared C3D scene belongs to another backend"
                )
            }
            Self::PreparedScenePoisoned => {
                write!(
                    formatter,
                    "SDL_GPU prepared C3D scene is not reusable after a failed wait"
                )
            }
            Self::RawC3dContract(source) => source.fmt(formatter),
            Self::FrameContract(source) => source.fmt(formatter),
            Self::Sdl { operation, detail } => {
                write!(formatter, "SDL_GPU {operation} failed: {detail}")
            }
        }
    }
}

impl std::error::Error for SdlGpuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FrameContract(source) => Some(source),
            Self::RawC3dContract(source) => Some(source),
            _ => None,
        }
    }
}

mod raw {
    use std::ffi::CStr;
    use std::slice;

    use cmr_gpu_contract::{BackendInfo, BackendKind, DeviceKind, GraphicsApi};
    use sdl3::gpu::{
        CopyPass, Device, Texture, TextureFormat, TextureType, TextureUsage, TransferBuffer,
    };
    use sdl3::properties::Properties;
    use sdl3::sys::gpu::{
        SDL_DownloadFromGPUTexture, SDL_GPUTextureFormat, SDL_GPUTextureRegion,
        SDL_GPUTextureSupportsFormat, SDL_GPUTextureTransferInfo, SDL_GPUTextureType,
        SDL_GetGPUDeviceDriver, SDL_GetGPUDeviceProperties,
    };
    use sdl3::sys::gpu::{SDL_MapGPUTransferBuffer, SDL_UnmapGPUTransferBuffer};

    use super::SdlGpuError;

    pub fn texture_supports(
        device: &Device,
        format: TextureFormat,
        texture_type: TextureType,
        usage: TextureUsage,
    ) -> bool {
        unsafe {
            SDL_GPUTextureSupportsFormat(
                device.raw(),
                SDL_GPUTextureFormat(format as i32),
                SDL_GPUTextureType(texture_type as i32),
                usage.0,
            )
        }
    }

    pub fn backend_info(device: &Device) -> Result<BackendInfo, SdlGpuError> {
        let driver = unsafe { SDL_GetGPUDeviceDriver(device.raw()) };
        let driver = if driver.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(driver) }
                .to_string_lossy()
                .into_owned()
        };
        let properties_id = unsafe { SDL_GetGPUDeviceProperties(device.raw()) };
        if properties_id == 0 {
            return Err(SdlGpuError::sdl("device property query", sdl3::get_error()));
        }
        let properties = Properties::const_from_ll(properties_id);
        let adapter_name = properties
            .get_string("SDL.gpu.device.name", "")
            .map_err(|error| SdlGpuError::sdl("device name query", error))?;
        let driver_name = properties
            .get_string("SDL.gpu.device.driver_name", &driver)
            .map_err(|error| SdlGpuError::sdl("driver name query", error))?;
        let driver_version = properties
            .get_string("SDL.gpu.device.driver_version", "")
            .map_err(|error| SdlGpuError::sdl("driver version query", error))?;
        let driver_info = properties
            .get_string("SDL.gpu.device.driver_info", &driver_version)
            .map_err(|error| SdlGpuError::sdl("driver info query", error))?;
        let graphics_api = match driver.to_ascii_lowercase().as_str() {
            "vulkan" => GraphicsApi::Vulkan,
            "direct3d12" | "d3d12" => GraphicsApi::Direct3d12,
            "metal" => GraphicsApi::Metal,
            _ => GraphicsApi::Unknown,
        };
        Ok(BackendInfo {
            backend: BackendKind::SdlGpu,
            graphics_api,
            vendor_id: None,
            device_id: None,
            device_kind: DeviceKind::Unknown,
            adapter_name,
            driver_name,
            driver_info,
        })
    }

    pub fn write_upload(
        device: &Device,
        transfer: &TransferBuffer,
        len: usize,
        indices: &[u8],
        palette_offset: usize,
        palette: &[u8],
    ) -> Result<(), SdlGpuError> {
        write_transfer(
            device,
            transfer,
            len,
            &[(0, indices), (palette_offset, palette)],
        )
    }

    pub(super) fn write_transfer(
        device: &Device,
        transfer: &TransferBuffer,
        len: usize,
        chunks: &[(usize, &[u8])],
    ) -> Result<(), SdlGpuError> {
        for &(offset, chunk) in chunks {
            offset
                .checked_add(chunk.len())
                .filter(|&end| end <= len)
                .ok_or(SdlGpuError::SizeOverflow)?;
        }
        let pointer = unsafe { SDL_MapGPUTransferBuffer(device.raw(), transfer.raw(), false) };
        if pointer.is_null() {
            return Err(SdlGpuError::NullTransferMap);
        }
        let bytes = unsafe { slice::from_raw_parts_mut(pointer.cast::<u8>(), len) };
        bytes.fill(0);
        for &(offset, chunk) in chunks {
            let end = offset + chunk.len();
            bytes[offset..end].copy_from_slice(chunk);
        }
        unsafe { SDL_UnmapGPUTransferBuffer(device.raw(), transfer.raw()) };
        Ok(())
    }

    pub fn download_texture(
        copy: &CopyPass,
        texture: &Texture<'_>,
        transfer: &TransferBuffer,
        width: u32,
        height: u32,
        pixels_per_row: u32,
    ) {
        let region = SDL_GPUTextureRegion {
            texture: texture.raw(),
            mip_level: 0,
            layer: 0,
            x: 0,
            y: 0,
            z: 0,
            w: width,
            h: height,
            d: 1,
        };
        let destination = SDL_GPUTextureTransferInfo {
            transfer_buffer: transfer.raw(),
            offset: 0,
            pixels_per_row,
            rows_per_layer: height,
        };
        unsafe { SDL_DownloadFromGPUTexture(copy.raw(), &region, &destination) };
    }

    pub fn read_download(
        device: &Device,
        transfer: &TransferBuffer,
        len: usize,
    ) -> Result<Vec<u8>, SdlGpuError> {
        let pointer = unsafe { SDL_MapGPUTransferBuffer(device.raw(), transfer.raw(), false) };
        if pointer.is_null() {
            return Err(SdlGpuError::NullTransferMap);
        }
        let bytes = unsafe { slice::from_raw_parts(pointer.cast::<u8>(), len) }.to_vec();
        unsafe { SDL_UnmapGPUTransferBuffer(device.raw(), transfer.raw()) };
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use cmr_core::display::DisplayExtent;
    use cmr_formats::riff_palette::PaletteEntry;
    use cmr_gpu_contract::{IndexedFrameView, IndexedPaletteGpuBackend};

    use super::*;

    #[test]
    fn transfer_alignment_is_checked_and_exact() {
        assert_eq!(align_up(0, 256).unwrap(), 0);
        assert_eq!(align_up(1, 256).unwrap(), 256);
        assert_eq!(align_up(256, 256).unwrap(), 256);
        assert_eq!(align_up(257, 256).unwrap(), 512);
        assert!(matches!(
            align_up(usize::MAX, 256),
            Err(SdlGpuError::SizeOverflow)
        ));
    }

    #[test]
    fn palette_upload_is_opaque_and_ignores_flags() {
        let palette: Vec<_> = (0..256)
            .map(|index| PaletteEntry {
                red: index as u8,
                green: 2,
                blue: 3,
                flags: 0xfe,
            })
            .collect();
        let view = IndexedFrameView::new(
            DisplayExtent {
                width: 1,
                height: 1,
            },
            &[0],
            &palette,
        )
        .unwrap();
        let bytes = palette_rgba(view);
        assert_eq!(&bytes[0..4], &[0, 2, 3, 255]);
        assert_eq!(&bytes[255 * 4..256 * 4], &[255, 2, 3, 255]);
    }

    #[test]
    fn non_vulkan_driver_is_rejected_explicitly() {
        let error = SdlGpuError::UnexpectedGraphicsApi {
            actual: GraphicsApi::Direct3d12,
        };
        assert_eq!(
            error.to_string(),
            "SDL_GPU returned Direct3d12, but this SPIR-V backend supports Vulkan only"
        );
        assert_eq!(SDL_GPU_DRIVER, "vulkan");
    }

    fn assert_hardware_readback(config: SdlGpuConfig) {
        let mut backend = SdlGpuIndexedBackend::new(config).unwrap();
        let palette: Vec<_> = (0..=u8::MAX)
            .map(|index| PaletteEntry {
                red: index,
                green: u8::MAX - index,
                blue: index.wrapping_mul(73),
                flags: index,
            })
            .collect();
        let indices: Vec<_> = (0..=u8::MAX)
            .chain(std::iter::once(0))
            .chain((0..=u8::MAX).rev())
            .chain(std::iter::once(u8::MAX))
            .collect();
        let frame = IndexedFrameView::new(
            DisplayExtent {
                width: 257,
                height: 2,
            },
            &indices,
            &palette,
        )
        .unwrap();
        let expected = frame.canonical_rgba8().unwrap();
        let actual = backend.render_readback(frame).unwrap();

        assert_eq!(actual.rgba8, expected);
        assert!(backend.capabilities().sampled_r8_uint);
        eprintln!("backend={:?}", backend.info());
    }

    #[test]
    #[ignore = "requires SDL_VIDEO_DRIVER=offscreen and a Vulkan SDL_GPU device"]
    fn default_device_readback_matches_the_cpu_oracle() {
        assert_hardware_readback(SdlGpuConfig::default());
    }

    #[test]
    #[ignore = "requires SDL_VIDEO_DRIVER=offscreen and a low-power Vulkan SDL_GPU device"]
    fn low_power_device_readback_matches_the_cpu_oracle() {
        assert_hardware_readback(SdlGpuConfig {
            prefer_low_power: true,
            ..SdlGpuConfig::default()
        });
    }
}
