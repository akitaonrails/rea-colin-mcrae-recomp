//! wgpu implementation of the indexed-palette GPU conformance contract.
//!
//! This first slice is deliberately offscreen. It uploads an exact `R8Uint`
//! index plane and a 256-entry `Rgba8Uint` palette, resolves them with integer
//! `textureLoad` operations, and reads an `Rgba8Unorm` target back through a
//! driver-aligned buffer. No wgpu type crosses the shared contract boundary.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cmr_gpu_contract::{
    BackendCapabilities, BackendInfo, BackendKind, C3dCullMode, C3dPreparedFrame,
    C3dPreparedSceneLayout, C3dRawFrame, C3dSceneUpload, CanonicalIndexedFrame, ConformanceFrame,
    DeviceKind, FrameContractError, FrameTiming, GraphicsApi, IndexedFrameView,
    IndexedPaletteGpuBackend, PersistentRawC3dGpuBackend, PersistentRawC3dIndexedBackend,
    RawC3dContractError, RawC3dGpuBackend, ReadbackFormat, c3d_active_palette_l1_map,
    normalize_readback,
};

const PALETTE_LEN: u32 = 256;
const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

const SHADER: &str = r#"
@group(0) @binding(0) var indices: texture_2d<u32>;
@group(0) @binding(1) var palette: texture_2d<u32>;

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4f {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f( 3.0, -1.0),
        vec2f(-1.0,  3.0),
    );
    return vec4f(positions[vertex_index], 0.0, 1.0);
}

@fragment
fn fragment_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let coordinate = vec2i(position.xy);
    let palette_index = textureLoad(indices, coordinate, 0).r;
    let color = textureLoad(palette, vec2i(i32(palette_index), 0), 0);
    return vec4f(vec3f(color.rgb) / 255.0, 1.0);
}
"#;

const C3D_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The presentation readback target: one active-palette index per pixel.
const C3D_INDEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Uint;
const C3D_MATERIAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Uint;

const C3D_SHADER: &str = r#"
struct Transform {
    world: mat4x4f,
    view: mat4x4f,
    projection: mat4x4f,
    clip_parameters: vec4f,
};

@group(0) @binding(0) var<uniform> transform: Transform;
@group(1) @binding(0) var material_indices: texture_2d<u32>;
@group(1) @binding(1) var active_palette: texture_2d<u32>;
@group(1) @binding(2) var material_rgba: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) diffuse: vec4f,
};

@vertex
fn vertex_main(
    @location(0) position_q16: vec3i,
    @location(1) texture_uv: vec2f,
    @location(2) normal_q16: vec3i,
    @location(3) diffuse: vec4f,
) -> VertexOutput {
    let scale = 1.0 / 65536.0;
    let model = vec4f(vec3f(position_q16) * scale, 1.0);
    var clip = transform.projection * transform.view * transform.world * model;
    // The recovered D3DVIEWPORT2 clip volume is
    // {-1, 2, width/height, 2*width/height, 0, 1}. X and Z are already in
    // the standard GPU clip volume; Y must be normalized by the aspect ratio.
    clip.y = clip.y / transform.clip_parameters.x;
    var result: VertexOutput;
    result.position = clip;
    result.uv = texture_uv;
    result.diffuse = diffuse;
    return result;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
    let palette_index = resolve_palette_index(input);
    let color = textureLoad(active_palette, vec2i(i32(palette_index), 0), 0);
    return vec4f(vec3f(color.rgb) / 255.0, 1.0);
}

// True-color stage presentation (`FUN_004a2540` `D3DTBLEND_MODULATE` +
// `FUN_004b1e10` per-vertex diffuse + per-texture 256-entry RGB palettes):
// key-tests against the center texel and bilinearly filters non-keyed
// neighbors before multiplying by the interpolated vertex diffuse color.
@fragment
fn fragment_true_color(input: VertexOutput) -> @location(0) vec4f {
    let dimensions = vec2f(textureDimensions(material_rgba));
    let max_coord = vec2i(dimensions) - vec2i(1);
    let bounded = clamp(input.uv, vec2f(0.0), vec2f(1.0));
    let nearest_coord = min(vec2i(floor(bounded * dimensions)), max_coord);
    let center = textureLoad(material_rgba, nearest_coord, 0);
    if center.a < 0.5 {
        discard;
    }
    let texel_pos = bounded * dimensions - vec2f(0.5);
    let base = floor(texel_pos);
    let frac = texel_pos - base;
    let c00 = clamp(vec2i(base), vec2i(0), max_coord);
    let c10 = clamp(vec2i(base) + vec2i(1, 0), vec2i(0), max_coord);
    let c01 = clamp(vec2i(base) + vec2i(0, 1), vec2i(0), max_coord);
    let c11 = clamp(vec2i(base) + vec2i(1, 1), vec2i(0), max_coord);
    let t00 = textureLoad(material_rgba, c00, 0);
    let t10 = textureLoad(material_rgba, c10, 0);
    let t01 = textureLoad(material_rgba, c01, 0);
    let t11 = textureLoad(material_rgba, c11, 0);
    let w00 = (1.0 - frac.x) * (1.0 - frac.y) * t00.a;
    let w10 = frac.x * (1.0 - frac.y) * t10.a;
    let w01 = (1.0 - frac.x) * frac.y * t01.a;
    let w11 = frac.x * frac.y * t11.a;
    let w_sum = w00 + w10 + w01 + w11;
    var sampled_rgb = center.rgb;
    if w_sum > 0.001 {
        sampled_rgb = (t00.rgb * w00 + t10.rgb * w10 + t01.rgb * w01 + t11.rgb * w11) / w_sum;
    }
    return vec4f(sampled_rgb * input.diffuse.rgb, input.diffuse.a);
}

// Presentation readback: emit the resolved active-palette slot instead of the
// palette colour, so the host presents the GPU frame through the recovered
// indexed display path without an RGBA -> index inversion.
@fragment
fn fragment_index(input: VertexOutput) -> @location(0) u32 {
    return resolve_palette_index(input);
}

fn resolve_palette_index(input: VertexOutput) -> u32 {
    let dimensions = vec2f(textureDimensions(material_indices));
    let bounded = clamp(input.uv, vec2f(0.0), vec2f(1.0));
    let coordinate = min(vec2i(floor(bounded * dimensions)), vec2i(dimensions) - vec2i(1));
    let material = textureLoad(material_indices, coordinate, 0);
    // R stores the active-palette slot and G is a precomputed source-key mask.
    // Discard occurs before the fragment can update either color or depth.
    if material.g != 0u {
        discard;
    }
    return material.r;
}
"#;

/// Native graphics API selected for the wgpu instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WgpuApi {
    Vulkan,
    Direct3d12,
    OpenGl,
}

impl WgpuApi {
    const fn backends(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Direct3d12 => wgpu::Backends::DX12,
            Self::OpenGl => wgpu::Backends::GL,
        }
    }
}

/// Adapter class preference used when no exact name filter is supplied.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WgpuPowerPreference {
    LowPower,
    #[default]
    HighPerformance,
}

/// Explicit adapter selection for a headless wgpu backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WgpuBackendConfig {
    pub api: WgpuApi,
    pub adapter_name_contains: Option<String>,
    pub power_preference: WgpuPowerPreference,
}

impl Default for WgpuBackendConfig {
    fn default() -> Self {
        Self {
            api: WgpuApi::Vulkan,
            adapter_name_contains: None,
            power_preference: WgpuPowerPreference::HighPerformance,
        }
    }
}

/// Offscreen indexed-palette renderer backed by one selected wgpu adapter.
pub struct WgpuIndexedPaletteBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    c3d_transform_layout: wgpu::BindGroupLayout,
    c3d_texture_layout: wgpu::BindGroupLayout,
    c3d_culled_pipeline: wgpu::RenderPipeline,
    c3d_two_sided_pipeline: wgpu::RenderPipeline,
    c3d_true_color_culled_pipeline: wgpu::RenderPipeline,
    c3d_true_color_two_sided_pipeline: wgpu::RenderPipeline,
    c3d_index_culled_pipeline: wgpu::RenderPipeline,
    c3d_index_two_sided_pipeline: wgpu::RenderPipeline,
    info: BackendInfo,
    capabilities: BackendCapabilities,
}

struct WgpuPreparedMesh {
    vertex: wgpu::Buffer,
    vertex_count: usize,
    index: wgpu::Buffer,
    index_count: u32,
}

struct WgpuPreparedTexture {
    _texture: wgpu::Texture,
    _rgba_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// Opaque persistent wgpu realization of one validated raw-C3D resource set.
pub struct WgpuPreparedC3dScene {
    layout: C3dPreparedSceneLayout,
    active_palette: Box<[cmr_formats::riff_palette::PaletteEntry; 256]>,
    meshes: Vec<WgpuPreparedMesh>,
    textures: Vec<WgpuPreparedTexture>,
    _active_palette_texture: wgpu::Texture,
    _output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,
    _index_texture: wgpu::Texture,
    index_view: wgpu::TextureView,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    readback_pitch: u32,
    index_readback: wgpu::Buffer,
    index_readback_pitch: u32,
    /// The depth after the frame, for passes the host draws over it.
    depth_readback: wgpu::Buffer,
    transform_buffer: wgpu::Buffer,
    transform_bind_group: wgpu::BindGroup,
    transform_stride: u32,
    transform_capacity: usize,
    static_buffer_uploads: usize,
    static_texture_uploads: usize,
    timed_resource_allocations: usize,
    timed_frames: usize,
}

impl WgpuPreparedC3dScene {
    /// Test/diagnostic counters. Timed frames never change either upload count.
    #[must_use]
    pub const fn static_upload_counts(&self) -> [usize; 2] {
        [self.static_buffer_uploads, self.static_texture_uploads]
    }

    #[must_use]
    pub const fn timed_frame_count(&self) -> usize {
        self.timed_frames
    }

    #[must_use]
    pub const fn timed_resource_allocations(&self) -> usize {
        self.timed_resource_allocations
    }
}

struct WgpuFrameDraw {
    mesh: usize,
    texture: usize,
    cull: C3dCullMode,
    transform_offset: u32,
}

/// Which prepared-scene color target a frame is encoded into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum C3dEncodeTarget {
    /// Expanded RGBA8: the correctness/benchmark readback.
    Rgba,
    /// True-color RGBA8 with per-texture palettes, bilinear filtering, and Gouraud lighting.
    TrueColor,
    /// One active-palette index per pixel: the presentation readback.
    PaletteIndex,
}

impl WgpuIndexedPaletteBackend {
    /// Enumerates only the requested API and selects an adapter deterministically.
    pub fn new(config: &WgpuBackendConfig) -> Result<Self, WgpuBackendError> {
        pollster::block_on(Self::new_async(config))
    }

    async fn new_async(config: &WgpuBackendConfig) -> Result<Self, WgpuBackendError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = config.api.backends();
        let instance = wgpu::Instance::new(descriptor);
        let mut adapters = instance.enumerate_adapters(config.api.backends()).await;
        if let Some(filter) = config.adapter_name_contains.as_deref() {
            let filter = filter.to_lowercase();
            adapters.retain(|adapter| adapter.get_info().name.to_lowercase().contains(&filter));
        }
        adapters.sort_by_key(|adapter| {
            adapter_rank(adapter.get_info().device_type, config.power_preference)
        });
        let adapter = adapters
            .into_iter()
            .next()
            .ok_or_else(|| WgpuBackendError::NoAdapter {
                api: config.api,
                name_filter: config.adapter_name_contains.clone(),
            })?;
        let native_info = adapter.get_info();
        let adapter_features = adapter.features();
        let sampled_r8_uint = supports_usage(
            &adapter,
            wgpu::TextureFormat::R8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        if !sampled_r8_uint {
            return Err(WgpuBackendError::MissingRequiredTextureRole(
                WgpuTextureRole::IndexedPlane,
            ));
        }
        if !supports_usage(
            &adapter,
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ) {
            return Err(WgpuBackendError::MissingRequiredTextureRole(
                WgpuTextureRole::Palette,
            ));
        }
        if !supports_usage(
            &adapter,
            OUTPUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        ) {
            return Err(WgpuBackendError::MissingRequiredTextureRole(
                WgpuTextureRole::ReadbackTarget,
            ));
        }
        if !supports_usage(
            &adapter,
            C3D_INDEX_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        ) {
            return Err(WgpuBackendError::MissingRequiredTextureRole(
                WgpuTextureRole::ReadbackTarget,
            ));
        }
        let required_features = adapter_features & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cmr indexed-palette conformance device"),
                required_features,
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| WgpuBackendError::RequestDevice(error.to_string()))?;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cmr indexed-palette bind group layout"),
            entries: &[texture_layout_entry(0), texture_layout_entry(1)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cmr indexed-palette pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cmr indexed-palette shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cmr indexed-palette pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let c3d_transform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cmr raw C3D transform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let c3d_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cmr raw C3D texture layout"),
                entries: &[
                    texture_layout_entry(0),
                    texture_layout_entry(1),
                    float_texture_layout_entry(2),
                ],
            });
        let c3d_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cmr raw C3D pipeline layout"),
            bind_group_layouts: &[Some(&c3d_transform_layout), Some(&c3d_texture_layout)],
            immediate_size: 0,
        });
        let c3d_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cmr raw C3D shader"),
            source: wgpu::ShaderSource::Wgsl(C3D_SHADER.into()),
        });
        let c3d_culled_pipeline = create_c3d_pipeline(
            &device,
            &c3d_pipeline_layout,
            &c3d_shader,
            Some(wgpu::Face::Back),
        );
        let c3d_two_sided_pipeline =
            create_c3d_pipeline(&device, &c3d_pipeline_layout, &c3d_shader, None);
        let c3d_true_color_culled_pipeline = create_c3d_true_color_pipeline(
            &device,
            &c3d_pipeline_layout,
            &c3d_shader,
            Some(wgpu::Face::Back),
        );
        let c3d_true_color_two_sided_pipeline =
            create_c3d_true_color_pipeline(&device, &c3d_pipeline_layout, &c3d_shader, None);
        let c3d_index_culled_pipeline = create_c3d_index_pipeline(
            &device,
            &c3d_pipeline_layout,
            &c3d_shader,
            Some(wgpu::Face::Back),
        );
        let c3d_index_two_sided_pipeline =
            create_c3d_index_pipeline(&device, &c3d_pipeline_layout, &c3d_shader, None);
        let info = BackendInfo {
            backend: BackendKind::Wgpu,
            graphics_api: graphics_api(native_info.backend),
            vendor_id: (native_info.vendor != 0).then_some(native_info.vendor),
            device_id: (native_info.device != 0).then_some(native_info.device),
            device_kind: device_kind(native_info.device_type),
            adapter_name: native_info.name,
            driver_name: native_info.driver,
            driver_info: native_info.driver_info,
        };
        let capabilities = BackendCapabilities {
            sampled_r8_uint,
            timestamp_queries: adapter_features.contains(wgpu::Features::TIMESTAMP_QUERY),
            headless_readback: true,
            device_recovery: false,
        };
        Ok(Self {
            device,
            queue,
            bind_group_layout,
            pipeline,
            c3d_transform_layout,
            c3d_texture_layout,
            c3d_culled_pipeline,
            c3d_two_sided_pipeline,
            c3d_true_color_culled_pipeline,
            c3d_true_color_two_sided_pipeline,
            c3d_index_culled_pipeline,
            c3d_index_two_sided_pipeline,
            info,
            capabilities,
        })
    }
}

impl IndexedPaletteGpuBackend for WgpuIndexedPaletteBackend {
    type Error = WgpuBackendError;

    fn info(&self) -> &BackendInfo {
        &self.info
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    fn render_readback(
        &mut self,
        frame: IndexedFrameView<'_>,
    ) -> Result<ConformanceFrame, Self::Error> {
        let extent = frame.extent();
        let size = wgpu::Extent3d {
            width: u32::from(extent.width),
            height: u32::from(extent.height),
            depth_or_array_layers: 1,
        };
        let encode_started = Instant::now();
        let index_texture = create_texture(
            &self.device,
            "cmr indexed plane",
            size,
            wgpu::TextureFormat::R8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture_copy(&index_texture),
            frame.indices(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(u32::from(extent.width)),
                rows_per_image: Some(u32::from(extent.height)),
            },
            size,
        );
        let palette_texture = create_texture(
            &self.device,
            "cmr palette",
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let mut palette_bytes = [0u8; PALETTE_LEN as usize * 4];
        let (palette_pixels, remainder) = palette_bytes.as_chunks_mut::<4>();
        debug_assert!(remainder.is_empty());
        for (bytes, entry) in palette_pixels.iter_mut().zip(frame.palette()) {
            bytes.copy_from_slice(&[entry.red, entry.green, entry.blue, u8::MAX]);
        }
        self.queue.write_texture(
            texture_copy(&palette_texture),
            &palette_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PALETTE_LEN * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let output_texture = create_texture(
            &self.device,
            "cmr conformance output",
            size,
            OUTPUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let index_view = index_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let palette_view = palette_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cmr indexed-palette bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&index_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
            ],
        });
        let tight_pitch = u32::from(extent.width) * 4;
        let readback_pitch = tight_pitch.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let readback_size = u64::from(readback_pitch) * u64::from(extent.height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr conformance readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr indexed-palette commands"),
            });
        {
            let color_attachment = Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cmr indexed-palette pass"),
                color_attachments: &[color_attachment],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            texture_copy(&output_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            size,
        );
        let submission = self.queue.submit([encoder.finish()]);
        let cpu_encode_submit_ns = elapsed_ns(encode_started.elapsed());
        let completion_started = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| WgpuBackendError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let submit_to_completion_ns = elapsed_ns(completion_started.elapsed());
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let rgba8 = normalize_readback(
            extent,
            &mapped,
            readback_pitch as usize,
            ReadbackFormat::Rgba8,
        )
        .map_err(WgpuBackendError::Contract)?;
        drop(mapped);
        readback.unmap();
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

impl RawC3dGpuBackend for WgpuIndexedPaletteBackend {
    type Error = WgpuBackendError;

    fn info(&self) -> &BackendInfo {
        &self.info
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    fn render_c3d_readback(
        &mut self,
        frame: C3dRawFrame<'_>,
    ) -> Result<ConformanceFrame, Self::Error> {
        let extent = frame.extent();
        let size = wgpu::Extent3d {
            width: u32::from(extent.width),
            height: u32::from(extent.height),
            depth_or_array_layers: 1,
        };
        let encode_started = Instant::now();
        let output_texture = create_texture(
            &self.device,
            "cmr raw C3D output",
            size,
            OUTPUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth_texture = create_texture(
            &self.device,
            "cmr raw C3D depth",
            size,
            C3D_DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let active_palette_texture = create_texture(
            &self.device,
            "cmr raw C3D active palette",
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let active_palette_bytes = active_palette_rgba8(frame.active_palette());
        self.queue.write_texture(
            texture_copy(&active_palette_texture),
            &active_palette_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PALETTE_LEN * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let active_palette_view =
            active_palette_texture.create_view(&wgpu::TextureViewDescriptor::default());

        struct DrawResources {
            vertex: wgpu::Buffer,
            index: wgpu::Buffer,
            transform: wgpu::Buffer,
            transform_bind_group: wgpu::BindGroup,
            material_texture: wgpu::Texture,
            texture_bind_group: wgpu::BindGroup,
            index_count: u32,
            cull: C3dCullMode,
        }

        let mut draws = Vec::with_capacity(frame.draws().len());
        for draw in frame.draws() {
            let mesh = &frame.resources().meshes()[draw.mesh() as usize];
            let texture = &frame.resources().textures()[draw.texture() as usize];
            let vertex_bytes = c3d_vertex_bytes(mesh.vertices());
            let index_bytes = c3d_index_bytes(mesh.indices());
            let transform_bytes = c3d_transform_bytes(
                draw.world().words(),
                frame.view().words(),
                frame.projection().words(),
                f32::from(extent.width) / f32::from(extent.height),
            );
            let vertex = create_initialized_buffer(
                &self.device,
                &self.queue,
                "cmr raw C3D vertices",
                &vertex_bytes,
                wgpu::BufferUsages::VERTEX,
            );
            let index = create_initialized_buffer(
                &self.device,
                &self.queue,
                "cmr raw C3D indices",
                &index_bytes,
                wgpu::BufferUsages::INDEX,
            );
            let transform = create_initialized_buffer(
                &self.device,
                &self.queue,
                "cmr raw C3D transform",
                &transform_bytes,
                wgpu::BufferUsages::UNIFORM,
            );
            let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cmr raw C3D transform bind group"),
                layout: &self.c3d_transform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: transform.as_entire_binding(),
                }],
            });
            let material_extent = texture.extent();
            let material_size = wgpu::Extent3d {
                width: u32::from(material_extent.width),
                height: u32::from(material_extent.height),
                depth_or_array_layers: 1,
            };
            let material_texture = create_texture(
                &self.device,
                "cmr raw C3D mapped material",
                material_size,
                C3D_MATERIAL_FORMAT,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            let mapped_indices = c3d_mapped_material_texels(texture, frame.active_palette());
            self.queue.write_texture(
                texture_copy(&material_texture),
                &mapped_indices,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(u32::from(material_extent.width) * 2),
                    rows_per_image: Some(u32::from(material_extent.height)),
                },
                material_size,
            );
            let material_view =
                material_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let material_rgba_texture = create_texture(
                &self.device,
                "cmr raw C3D true-color material",
                material_size,
                OUTPUT_FORMAT,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            let true_color_texels = c3d_true_color_material_texels(texture);
            self.queue.write_texture(
                texture_copy(&material_rgba_texture),
                &true_color_texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(u32::from(material_extent.width) * 4),
                    rows_per_image: Some(u32::from(material_extent.height)),
                },
                material_size,
            );
            let material_rgba_view =
                material_rgba_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cmr raw C3D texture bind group"),
                layout: &self.c3d_texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&material_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&active_palette_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&material_rgba_view),
                    },
                ],
            });
            draws.push(DrawResources {
                vertex,
                index,
                transform,
                transform_bind_group,
                material_texture,
                texture_bind_group,
                index_count: mesh.indices().len() as u32,
                cull: draw.state().cull(),
            });
        }

        let tight_pitch = u32::from(extent.width) * 4;
        let readback_pitch = tight_pitch.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let readback_size = u64::from(readback_pitch) * u64::from(extent.height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr raw C3D readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr raw C3D commands"),
            });
        let clear = frame.active_palette()[0];
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cmr raw C3D pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear.red) / 255.0,
                            g: f64::from(clear.green) / 255.0,
                            b: f64::from(clear.blue) / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for draw in &draws {
                let pipeline = match draw.cull {
                    C3dCullMode::CullCounterClockwise => &self.c3d_culled_pipeline,
                    C3dCullMode::None => &self.c3d_two_sided_pipeline,
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &draw.transform_bind_group, &[0]);
                pass.set_bind_group(1, &draw.texture_bind_group, &[]);
                pass.set_vertex_buffer(0, draw.vertex.slice(..));
                pass.set_index_buffer(draw.index.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..draw.index_count, 0, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            texture_copy(&output_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            size,
        );
        let submission = self.queue.submit([encoder.finish()]);
        let cpu_encode_submit_ns = elapsed_ns(encode_started.elapsed());
        let completion_started = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| WgpuBackendError::Poll(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let submit_to_completion_ns = elapsed_ns(completion_started.elapsed());
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let rgba8 = normalize_readback(
            extent,
            &mapped,
            readback_pitch as usize,
            ReadbackFormat::Rgba8,
        )
        .map_err(WgpuBackendError::Contract)?;
        drop(mapped);
        readback.unmap();
        // Keep resources observably alive through submission completion.
        for draw in &draws {
            let _ = (&draw.transform, &draw.material_texture);
        }
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

impl WgpuIndexedPaletteBackend {
    /// Rewrite one prepared mesh's vertex buffer in place, keeping its
    /// vertex count (the car's crash damage moves vertices, never adds or
    /// drops them). Mirrors
    /// [`cmr_gpu_contract::C3dResourceSet::replace_mesh_vertices`].
    ///
    /// # Errors
    ///
    /// [`WgpuBackendError::RawContract`] for an unknown mesh or a changed
    /// vertex count; the buffer is left as it was.
    pub fn rewrite_prepared_mesh_vertices(
        &self,
        scene: &mut WgpuPreparedC3dScene,
        mesh: u32,
        vertices: &[cmr_gpu_contract::C3dStageVertex],
    ) -> Result<(), WgpuBackendError> {
        let resource_count = scene.meshes.len();
        let slot = scene
            .meshes
            .get(mesh as usize)
            .ok_or(WgpuBackendError::RawContract(
                RawC3dContractError::MeshRewriteOutOfBounds {
                    resource: mesh,
                    resource_count,
                },
            ))?;
        if slot.vertex_count != vertices.len() {
            return Err(WgpuBackendError::RawContract(
                RawC3dContractError::MeshRewriteVertexCount {
                    expected: slot.vertex_count,
                    actual: vertices.len(),
                },
            ));
        }
        self.queue
            .write_buffer(&slot.vertex, 0, &c3d_vertex_bytes(vertices));
        Ok(())
    }
}

impl WgpuIndexedPaletteBackend {
    /// One prepared scene's palette-index frame and its depth, for passes
    /// the host draws over it (the stage without a horizon layer).
    ///
    /// # Errors
    ///
    /// As [`PersistentRawC3dIndexedBackend::render_prepared_indexed_readback`].
    pub fn render_prepared_indexed_readback_with_depth(
        &mut self,
        scene: &mut WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<(CanonicalIndexedFrame, Vec<f32>), WgpuBackendError> {
        self.render_single_indexed(scene, frame, true)
            .map(|(frame, depth)| (frame, depth.unwrap_or_default()))
    }

    fn render_single_indexed(
        &mut self,
        scene: &mut WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
        with_depth: bool,
    ) -> Result<(CanonicalIndexedFrame, Option<Vec<f32>>), WgpuBackendError> {
        frame
            .ensure_layout(scene.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr prepared C3D indexed presentation commands"),
            });
        let frame_draws = self.encode_prepared_c3d_target(
            scene,
            frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
        )?;
        let readback = self.submit_indexed_readback(scene, encoder, with_depth);
        drop(frame_draws);
        readback
    }
}

impl PersistentRawC3dGpuBackend for WgpuIndexedPaletteBackend {
    type Error = WgpuBackendError;
    type PreparedScene = WgpuPreparedC3dScene;

    fn info(&self) -> &BackendInfo {
        &self.info
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    fn prepare_c3d_scene(
        &mut self,
        source: C3dSceneUpload<'_>,
    ) -> Result<Self::PreparedScene, Self::Error> {
        let layout = source.layout();
        let extent = layout.extent();
        let size = wgpu::Extent3d {
            width: u32::from(extent.width),
            height: u32::from(extent.height),
            depth_or_array_layers: 1,
        };
        let active_palette_texture = create_texture(
            &self.device,
            "cmr prepared C3D active palette",
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Rgba8Uint,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            texture_copy(&active_palette_texture),
            &active_palette_rgba8(source.active_palette()),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PALETTE_LEN * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: PALETTE_LEN,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let active_palette_view =
            active_palette_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let meshes = source
            .resources()
            .meshes()
            .iter()
            .map(|mesh| WgpuPreparedMesh {
                vertex: create_initialized_buffer(
                    &self.device,
                    &self.queue,
                    "cmr prepared C3D vertices",
                    &c3d_vertex_bytes(mesh.vertices()),
                    wgpu::BufferUsages::VERTEX,
                ),
                vertex_count: mesh.vertices().len(),
                index: create_initialized_buffer(
                    &self.device,
                    &self.queue,
                    "cmr prepared C3D indices",
                    &c3d_index_bytes(mesh.indices()),
                    wgpu::BufferUsages::INDEX,
                ),
                index_count: mesh.indices().len() as u32,
            })
            .collect::<Vec<_>>();
        let textures = source
            .resources()
            .textures()
            .iter()
            .map(|texture| {
                let extent = texture.extent();
                let size = wgpu::Extent3d {
                    width: u32::from(extent.width),
                    height: u32::from(extent.height),
                    depth_or_array_layers: 1,
                };
                let native = create_texture(
                    &self.device,
                    "cmr prepared C3D mapped material",
                    size,
                    C3D_MATERIAL_FORMAT,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                let mapped = c3d_mapped_material_texels(texture, source.active_palette());
                self.queue.write_texture(
                    texture_copy(&native),
                    &mapped,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(u32::from(extent.width) * 2),
                        rows_per_image: Some(u32::from(extent.height)),
                    },
                    size,
                );
                let view = native.create_view(&wgpu::TextureViewDescriptor::default());
                let rgba_native = create_texture(
                    &self.device,
                    "cmr prepared C3D true-color material",
                    size,
                    OUTPUT_FORMAT,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                );
                let rgba_texels = c3d_true_color_material_texels(texture);
                self.queue.write_texture(
                    texture_copy(&rgba_native),
                    &rgba_texels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(u32::from(extent.width) * 4),
                        rows_per_image: Some(u32::from(extent.height)),
                    },
                    size,
                );
                let rgba_view = rgba_native.create_view(&wgpu::TextureViewDescriptor::default());
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cmr prepared C3D texture bind group"),
                    layout: &self.c3d_texture_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&active_palette_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&rgba_view),
                        },
                    ],
                });
                WgpuPreparedTexture {
                    _texture: native,
                    _rgba_texture: rgba_native,
                    bind_group,
                }
            })
            .collect::<Vec<_>>();

        let output_texture = create_texture(
            &self.device,
            "cmr prepared C3D output",
            size,
            OUTPUT_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let index_texture = create_texture(
            &self.device,
            "cmr prepared C3D palette-index output",
            size,
            C3D_INDEX_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let index_view = index_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_texture = create_texture(
            &self.device,
            "cmr prepared C3D depth",
            size,
            C3D_DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let tight_pitch = u32::from(extent.width) * 4;
        let readback_pitch = tight_pitch.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let depth_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr prepared C3D depth readback"),
            size: u64::from(readback_pitch) * u64::from(extent.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr prepared C3D readback"),
            size: u64::from(readback_pitch) * u64::from(extent.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let index_tight_pitch = u32::from(extent.width);
        let index_readback_pitch = index_tight_pitch.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
        let index_readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr prepared C3D index readback"),
            size: u64::from(index_readback_pitch) * u64::from(extent.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let transform_size = 208u32;
        let alignment = self.device.limits().min_uniform_buffer_offset_alignment;
        let transform_stride = transform_size.div_ceil(alignment) * alignment;
        let transform_capacity = meshes.len().max(1);
        let transform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cmr prepared C3D transform ring"),
            size: u64::from(transform_stride) * transform_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cmr prepared C3D transform ring bind group"),
            layout: &self.c3d_transform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &transform_buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(u64::from(transform_size)),
                }),
            }],
        });
        Ok(WgpuPreparedC3dScene {
            layout,
            active_palette: Box::new(*source.active_palette()),
            static_buffer_uploads: meshes.len() * 2,
            static_texture_uploads: textures.len() + 1,
            meshes,
            textures,
            _active_palette_texture: active_palette_texture,
            _output_texture: output_texture,
            output_view,
            _index_texture: index_texture,
            index_view,
            _depth_texture: depth_texture,
            depth_view,
            readback,
            readback_pitch,
            index_readback,
            index_readback_pitch,
            depth_readback,
            transform_buffer,
            transform_bind_group,
            transform_stride,
            transform_capacity,
            timed_frames: 0,
            timed_resource_allocations: 0,
        })
    }

    fn prepared_layout(scene: &Self::PreparedScene) -> C3dPreparedSceneLayout {
        scene.layout
    }

    fn render_prepared_readback(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<cmr_gpu_contract::CanonicalRgba8Frame, Self::Error> {
        frame
            .ensure_layout(scene.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr prepared C3D correctness commands"),
            });
        let frame_draws = self.encode_prepared_c3d(scene, frame, &mut encoder)?;
        let extent = scene.layout.extent();
        encoder.copy_texture_to_buffer(
            texture_copy(&scene._output_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &scene.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(scene.readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            wgpu::Extent3d {
                width: u32::from(extent.width),
                height: u32::from(extent.height),
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::sync_channel(1);
        scene
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.wait_for_submission(submission)?;
        receiver
            .recv()
            .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let mapped = scene
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let rgba8 = normalize_readback(
            extent,
            &mapped,
            scene.readback_pitch as usize,
            ReadbackFormat::Rgba8,
        )
        .map_err(WgpuBackendError::Contract)?;
        drop(mapped);
        scene.readback.unmap();
        drop(frame_draws);
        Ok(rgba8)
    }

    fn render_prepared_timed(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<FrameTiming, Self::Error> {
        frame
            .ensure_layout(scene.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let encode_started = Instant::now();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr prepared C3D timed commands"),
            });
        let frame_draws = self.encode_prepared_c3d(scene, frame, &mut encoder)?;
        let submission = self.queue.submit([encoder.finish()]);
        let cpu_encode_submit_ns = elapsed_ns(encode_started.elapsed());
        let completion_started = Instant::now();
        self.wait_for_submission(submission)?;
        let submit_to_completion_ns = elapsed_ns(completion_started.elapsed());
        drop(frame_draws);
        scene.timed_frames += 1;
        Ok(FrameTiming {
            cpu_encode_submit_ns,
            submit_to_completion_ns,
            gpu_timestamp_ns: None,
        })
    }
}

impl PersistentRawC3dIndexedBackend for WgpuIndexedPaletteBackend {
    /// Rasterizes the prepared frame into the 8-bit palette-index target and
    /// reads it back tightly packed, so the host can present it through the
    /// recovered indexed display without an RGBA -> index inversion.
    fn render_prepared_indexed_readback(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<CanonicalIndexedFrame, Self::Error> {
        self.render_single_indexed(scene, frame, false)
            .map(|(frame, _)| frame)
    }
}

impl WgpuIndexedPaletteBackend {
    /// Renders an independently prepared background and foreground into one
    /// indexed target. The background clears color/depth; the foreground loads
    /// color but clears depth, matching the recovered horizon composition.
    pub fn render_prepared_layered_indexed_readback(
        &mut self,
        background: &mut WgpuPreparedC3dScene,
        background_frame: C3dPreparedFrame<'_>,
        foreground: &mut WgpuPreparedC3dScene,
        foreground_frame: C3dPreparedFrame<'_>,
    ) -> Result<CanonicalIndexedFrame, WgpuBackendError> {
        self.render_layered(
            background,
            background_frame,
            foreground,
            foreground_frame,
            false,
        )
        .map(|(frame, _)| frame)
    }

    /// [`Self::render_prepared_layered_indexed_readback`] that also reads back
    /// the frame's depth (`[0, 1]`, row-major), for the host's passes drawn
    /// over it with a depth test (the car shadows).
    pub fn render_prepared_layered_indexed_readback_with_depth(
        &mut self,
        background: &mut WgpuPreparedC3dScene,
        background_frame: C3dPreparedFrame<'_>,
        foreground: &mut WgpuPreparedC3dScene,
        foreground_frame: C3dPreparedFrame<'_>,
    ) -> Result<(CanonicalIndexedFrame, Vec<f32>), WgpuBackendError> {
        self.render_layered(
            background,
            background_frame,
            foreground,
            foreground_frame,
            true,
        )
        .map(|(frame, depth)| (frame, depth.unwrap_or_default()))
    }

    /// Renders a single prepared scene into both the 8-bit palette-index target
    /// and the true-color RGBA8 target (per-texture palettes, bilinear filtering,
    /// and per-vertex Gouraud `D3DTBLEND_MODULATE`), optionally reading back depth.
    pub fn render_prepared_true_color_and_indexed_readback(
        &mut self,
        scene: &mut WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
        with_depth: bool,
    ) -> Result<
        (
            CanonicalIndexedFrame,
            cmr_gpu_contract::CanonicalRgba8Frame,
            Option<Vec<f32>>,
        ),
        WgpuBackendError,
    > {
        frame
            .ensure_layout(scene.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr prepared C3D true-color + indexed presentation commands"),
            });
        let index_draws = self.encode_prepared_c3d_target(
            scene,
            frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
        )?;
        let rgba_draws = self.encode_prepared_c3d_target(
            scene,
            frame,
            &mut encoder,
            C3dEncodeTarget::TrueColor,
        )?;
        let extent = scene.layout.extent();
        encoder.copy_texture_to_buffer(
            texture_copy(&scene._output_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &scene.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(scene.readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            wgpu::Extent3d {
                width: u32::from(extent.width),
                height: u32::from(extent.height),
                depth_or_array_layers: 1,
            },
        );
        let (indexed, depth) = self.submit_indexed_readback(scene, encoder, with_depth)?;
        let rgba = self.map_rgba_readback(scene)?;
        drop((index_draws, rgba_draws));
        Ok((indexed, rgba, depth))
    }

    /// Renders a layered background + foreground into both the 8-bit palette-index
    /// target and the true-color RGBA8 target, optionally reading back depth.
    pub fn render_prepared_layered_true_color_and_indexed_readback(
        &mut self,
        background: &mut WgpuPreparedC3dScene,
        background_frame: C3dPreparedFrame<'_>,
        foreground: &mut WgpuPreparedC3dScene,
        foreground_frame: C3dPreparedFrame<'_>,
        with_depth: bool,
    ) -> Result<
        (
            CanonicalIndexedFrame,
            cmr_gpu_contract::CanonicalRgba8Frame,
            Option<Vec<f32>>,
        ),
        WgpuBackendError,
    > {
        background_frame
            .ensure_layout(background.layout)
            .map_err(WgpuBackendError::RawContract)?;
        foreground_frame
            .ensure_layout(foreground.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let extent = background.layout.extent();
        if foreground.layout.extent() != extent {
            return Err(WgpuBackendError::LayeredExtentMismatch {
                background: [extent.width, extent.height],
                foreground: [
                    foreground.layout.extent().width,
                    foreground.layout.extent().height,
                ],
            });
        }
        if foreground.active_palette != background.active_palette {
            return Err(WgpuBackendError::LayeredPaletteMismatch);
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr layered C3D true-color + indexed presentation commands"),
            });
        let bg_idx = self.encode_prepared_c3d_target_into(
            background,
            background,
            background_frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
            false,
        )?;
        let fg_idx = self.encode_prepared_c3d_target_into(
            foreground,
            background,
            foreground_frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
            true,
        )?;
        let bg_rgba = self.encode_prepared_c3d_target_into(
            background,
            background,
            background_frame,
            &mut encoder,
            C3dEncodeTarget::TrueColor,
            false,
        )?;
        let fg_rgba = self.encode_prepared_c3d_target_into(
            foreground,
            background,
            foreground_frame,
            &mut encoder,
            C3dEncodeTarget::TrueColor,
            true,
        )?;
        encoder.copy_texture_to_buffer(
            texture_copy(&background._output_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &background.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(background.readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            wgpu::Extent3d {
                width: u32::from(extent.width),
                height: u32::from(extent.height),
                depth_or_array_layers: 1,
            },
        );
        let (indexed, depth) = self.submit_indexed_readback(background, encoder, with_depth)?;
        let rgba = self.map_rgba_readback(background)?;
        drop((bg_idx, fg_idx, bg_rgba, fg_rgba));
        Ok((indexed, rgba, depth))
    }

    fn map_rgba_readback(
        &self,
        scene: &WgpuPreparedC3dScene,
    ) -> Result<cmr_gpu_contract::CanonicalRgba8Frame, WgpuBackendError> {
        let extent = scene.layout.extent();
        let (sender, receiver) = mpsc::sync_channel(1);
        scene
            .readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        receiver
            .recv()
            .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let mapped = scene
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let rgba8 = normalize_readback(
            extent,
            &mapped,
            scene.readback_pitch as usize,
            ReadbackFormat::Rgba8,
        )
        .map_err(WgpuBackendError::Contract)?;
        drop(mapped);
        scene.readback.unmap();
        Ok(rgba8)
    }

    fn render_layered(
        &mut self,
        background: &mut WgpuPreparedC3dScene,
        background_frame: C3dPreparedFrame<'_>,
        foreground: &mut WgpuPreparedC3dScene,
        foreground_frame: C3dPreparedFrame<'_>,
        with_depth: bool,
    ) -> Result<(CanonicalIndexedFrame, Option<Vec<f32>>), WgpuBackendError> {
        background_frame
            .ensure_layout(background.layout)
            .map_err(WgpuBackendError::RawContract)?;
        foreground_frame
            .ensure_layout(foreground.layout)
            .map_err(WgpuBackendError::RawContract)?;
        let extent = background.layout.extent();
        if foreground.layout.extent() != extent {
            return Err(WgpuBackendError::LayeredExtentMismatch {
                background: [extent.width, extent.height],
                foreground: [
                    foreground.layout.extent().width,
                    foreground.layout.extent().height,
                ],
            });
        }
        if foreground.active_palette != background.active_palette {
            return Err(WgpuBackendError::LayeredPaletteMismatch);
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cmr layered C3D indexed presentation commands"),
            });
        let background_draws = self.encode_prepared_c3d_target_into(
            background,
            background,
            background_frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
            false,
        )?;
        let foreground_draws = self.encode_prepared_c3d_target_into(
            foreground,
            background,
            foreground_frame,
            &mut encoder,
            C3dEncodeTarget::PaletteIndex,
            true,
        )?;
        let readback = self.submit_indexed_readback(background, encoder, with_depth);
        drop((background_draws, foreground_draws));
        readback
    }

    /// Copy the palette-index target (and with `with_depth` the depth) of
    /// `scene` after the encoded draws, submit, wait and map them back.
    fn submit_indexed_readback(
        &mut self,
        scene: &WgpuPreparedC3dScene,
        mut encoder: wgpu::CommandEncoder,
        with_depth: bool,
    ) -> Result<(CanonicalIndexedFrame, Option<Vec<f32>>), WgpuBackendError> {
        let extent = scene.layout.extent();
        encoder.copy_texture_to_buffer(
            texture_copy(&scene._index_texture),
            wgpu::TexelCopyBufferInfo {
                buffer: &scene.index_readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(scene.index_readback_pitch),
                    rows_per_image: Some(u32::from(extent.height)),
                },
            },
            wgpu::Extent3d {
                width: u32::from(extent.width),
                height: u32::from(extent.height),
                depth_or_array_layers: 1,
            },
        );
        if with_depth {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &scene._depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &scene.depth_readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(scene.readback_pitch),
                        rows_per_image: Some(u32::from(extent.height)),
                    },
                },
                wgpu::Extent3d {
                    width: u32::from(extent.width),
                    height: u32::from(extent.height),
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = self.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::sync_channel(1);
        scene
            .index_readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.wait_for_submission(submission)?;
        receiver
            .recv()
            .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let mapped = scene
            .index_readback
            .slice(..)
            .get_mapped_range()
            .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
        let width = usize::from(extent.width);
        let height = usize::from(extent.height);
        let pitch = scene.index_readback_pitch as usize;
        let mapped_len = pitch * height;
        if mapped.len() != mapped_len {
            let actual = mapped.len();
            drop(mapped);
            scene.index_readback.unmap();
            return Err(WgpuBackendError::Contract(
                FrameContractError::InvalidReadbackLength {
                    expected: mapped_len,
                    actual,
                },
            ));
        }
        let mut indices = Vec::with_capacity(width * height);
        for row in 0..height {
            indices.extend_from_slice(&mapped[row * pitch..row * pitch + width]);
        }
        drop(mapped);
        scene.index_readback.unmap();
        let depth = if with_depth {
            let (sender, receiver) = mpsc::sync_channel(1);
            scene
                .depth_readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
            receiver
                .recv()
                .map_err(|error| WgpuBackendError::MapChannel(error.to_string()))?
                .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
            let mapped = scene
                .depth_readback
                .slice(..)
                .get_mapped_range()
                .map_err(|error| WgpuBackendError::Map(error.to_string()))?;
            let pitch = scene.readback_pitch as usize;
            let mut values = Vec::with_capacity(width * height);
            for row in 0..height {
                let line = &mapped[row * pitch..row * pitch + width * 4];
                values.extend(
                    line.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|bytes| f32::from_le_bytes(*bytes)),
                );
            }
            drop(mapped);
            scene.depth_readback.unmap();
            Some(values)
        } else {
            None
        };
        CanonicalIndexedFrame::new(extent, indices.into_boxed_slice())
            .map(|frame| (frame, depth))
            .map_err(WgpuBackendError::Contract)
    }

    fn encode_prepared_c3d(
        &self,
        scene: &WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Vec<WgpuFrameDraw>, WgpuBackendError> {
        self.encode_prepared_c3d_target(scene, frame, encoder, C3dEncodeTarget::Rgba)
    }

    fn encode_prepared_c3d_target(
        &self,
        scene: &WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
        encoder: &mut wgpu::CommandEncoder,
        target: C3dEncodeTarget,
    ) -> Result<Vec<WgpuFrameDraw>, WgpuBackendError> {
        self.encode_prepared_c3d_target_into(scene, scene, frame, encoder, target, false)
    }

    fn encode_prepared_c3d_target_into(
        &self,
        scene: &WgpuPreparedC3dScene,
        target_scene: &WgpuPreparedC3dScene,
        frame: C3dPreparedFrame<'_>,
        encoder: &mut wgpu::CommandEncoder,
        target: C3dEncodeTarget,
        load_color: bool,
    ) -> Result<Vec<WgpuFrameDraw>, WgpuBackendError> {
        let extent = target_scene.layout.extent();
        if frame.draws().len() > scene.transform_capacity {
            return Err(WgpuBackendError::TransformCapacity {
                required: frame.draws().len(),
                capacity: scene.transform_capacity,
            });
        }
        let mut transform_upload = vec![0u8; scene.transform_stride as usize * frame.draws().len()];
        let frame_draws = frame
            .draws()
            .iter()
            .enumerate()
            .map(|(index, draw)| {
                let transform = c3d_transform_bytes(
                    draw.world().words(),
                    frame.view().words(),
                    frame.projection().words(),
                    f32::from(extent.width) / f32::from(extent.height),
                );
                let offset = index * scene.transform_stride as usize;
                transform_upload[offset..offset + transform.len()].copy_from_slice(&transform);
                WgpuFrameDraw {
                    mesh: draw.mesh() as usize,
                    texture: draw.texture() as usize,
                    cull: draw.state().cull(),
                    transform_offset: offset as u32,
                }
            })
            .collect::<Vec<_>>();
        if !transform_upload.is_empty() {
            self.queue
                .write_buffer(&scene.transform_buffer, 0, &transform_upload);
        }
        let clear = target_scene.active_palette[0];
        let (target_view, clear_color) = match target {
            C3dEncodeTarget::Rgba | C3dEncodeTarget::TrueColor => (
                &target_scene.output_view,
                wgpu::Color {
                    r: f64::from(clear.red) / 255.0,
                    g: f64::from(clear.green) / 255.0,
                    b: f64::from(clear.blue) / 255.0,
                    a: 1.0,
                },
            ),
            // The software compositor clears its indexed framebuffer to index
            // zero; the presentation target must start from the same index.
            C3dEncodeTarget::PaletteIndex => (
                &target_scene.index_view,
                wgpu::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
            ),
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cmr prepared C3D pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if load_color {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(clear_color)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target_scene.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for draw in &frame_draws {
                let mesh = &scene.meshes[draw.mesh];
                let texture = &scene.textures[draw.texture];
                pass.set_pipeline(match (target, draw.cull) {
                    (C3dEncodeTarget::Rgba, C3dCullMode::CullCounterClockwise) => {
                        &self.c3d_culled_pipeline
                    }
                    (C3dEncodeTarget::Rgba, C3dCullMode::None) => &self.c3d_two_sided_pipeline,
                    (C3dEncodeTarget::TrueColor, C3dCullMode::CullCounterClockwise) => {
                        &self.c3d_true_color_culled_pipeline
                    }
                    (C3dEncodeTarget::TrueColor, C3dCullMode::None) => {
                        &self.c3d_true_color_two_sided_pipeline
                    }
                    (C3dEncodeTarget::PaletteIndex, C3dCullMode::CullCounterClockwise) => {
                        &self.c3d_index_culled_pipeline
                    }
                    (C3dEncodeTarget::PaletteIndex, C3dCullMode::None) => {
                        &self.c3d_index_two_sided_pipeline
                    }
                });
                pass.set_bind_group(0, &scene.transform_bind_group, &[draw.transform_offset]);
                pass.set_bind_group(1, &texture.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
        Ok(frame_draws)
    }

    fn wait_for_submission(
        &self,
        submission: wgpu::SubmissionIndex,
    ) -> Result<(), WgpuBackendError> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| WgpuBackendError::Poll(error.to_string()))?;
        Ok(())
    }
}

fn create_c3d_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    cull_mode: Option<wgpu::Face>,
) -> wgpu::RenderPipeline {
    create_c3d_pipeline_variant(
        device,
        layout,
        shader,
        cull_mode,
        "cmr raw C3D pipeline",
        "fragment_main",
        OUTPUT_FORMAT,
    )
}

fn create_c3d_true_color_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    cull_mode: Option<wgpu::Face>,
) -> wgpu::RenderPipeline {
    create_c3d_pipeline_variant(
        device,
        layout,
        shader,
        cull_mode,
        "cmr raw C3D true-color pipeline",
        "fragment_true_color",
        OUTPUT_FORMAT,
    )
}

fn create_c3d_index_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    cull_mode: Option<wgpu::Face>,
) -> wgpu::RenderPipeline {
    create_c3d_pipeline_variant(
        device,
        layout,
        shader,
        cull_mode,
        "cmr raw C3D palette-index pipeline",
        "fragment_index",
        C3D_INDEX_FORMAT,
    )
}

fn create_c3d_pipeline_variant(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    cull_mode: Option<wgpu::Face>,
    label: &str,
    fragment_entry: &str,
    color_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Sint32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Sint32x3,
            offset: 20,
            shader_location: 2,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Unorm8x4,
            offset: 32,
            shader_location: 3,
        },
    ];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 36,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ATTRIBUTES,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Cw,
            cull_mode,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: C3D_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Packs one material texel as `[active_palette_index, keyed]` for the WGSL
/// integer texture. Keeping the key bit separate avoids collisions when a
/// non-keyed source color quantizes to the same active-palette slot as the key.
fn c3d_mapped_material_texels(
    texture: &cmr_gpu_contract::C3dIndexedTexture,
    active_palette: &[cmr_formats::riff_palette::PaletteEntry; 256],
) -> Vec<u8> {
    let palette_map = c3d_active_palette_l1_map(texture, active_palette);
    texture
        .indices()
        .iter()
        .flat_map(|&source_index| {
            [
                palette_map[usize::from(source_index)],
                u8::from(texture.color_key_index() == Some(source_index)),
            ]
        })
        .collect()
}

fn active_palette_rgba8(palette: &[cmr_formats::riff_palette::PaletteEntry; 256]) -> [u8; 1024] {
    let mut bytes = [0; 1024];
    for (chunk, entry) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(palette) {
        chunk.copy_from_slice(&[entry.red, entry.green, entry.blue, u8::MAX]);
    }
    bytes
}

/// Expands one material's indexed texels through its own 256-entry RGB palette
/// into RGBA8 (`alpha = 0` for `color_key_index`, `255` otherwise).
fn c3d_true_color_material_texels(texture: &cmr_gpu_contract::C3dIndexedTexture) -> Vec<u8> {
    let palette = texture.palette();
    let key = texture.color_key_index();
    texture
        .indices()
        .iter()
        .flat_map(|&source_index| {
            let rgb = palette.get(usize::from(source_index)).copied().unwrap_or(
                cmr_gpu_contract::C3dRgb8 {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
            );
            let alpha = if key == Some(source_index) {
                0
            } else {
                u8::MAX
            };
            [rgb.red, rgb.green, rgb.blue, alpha]
        })
        .collect()
}

fn c3d_vertex_bytes(vertices: &[cmr_gpu_contract::C3dStageVertex]) -> Vec<u8> {
    vertices
        .iter()
        .flat_map(|vertex| {
            vertex
                .upload_words()
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .chain(vertex.diffuse_rgba())
        })
        .collect()
}

fn c3d_index_bytes(indices: &[u16]) -> Vec<u8> {
    indices.iter().copied().flat_map(u16::to_le_bytes).collect()
}

fn c3d_transform_bytes(
    world: [u32; 16],
    view: [u32; 16],
    projection: [u32; 16],
    aspect: f32,
) -> Vec<u8> {
    world
        .into_iter()
        .chain(view)
        .chain(projection)
        .chain([aspect.to_bits(), 0, 0, 0])
        .flat_map(u32::to_le_bytes)
        .collect()
}

fn create_initialized_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let padded_len = bytes.len().div_ceil(wgpu::COPY_BUFFER_ALIGNMENT as usize)
        * wgpu::COPY_BUFFER_ALIGNMENT as usize;
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(bytes);
    padded.resize(padded_len, 0);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: padded.len() as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &padded);
    buffer
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn float_texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn supports_usage(
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
    required: wgpu::TextureUsages,
) -> bool {
    adapter
        .get_texture_format_features(format)
        .allowed_usages
        .contains(required)
}

fn create_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn texture_copy(texture: &wgpu::Texture) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    }
}

fn adapter_rank(device_type: wgpu::DeviceType, preference: WgpuPowerPreference) -> u8 {
    match (preference, device_type) {
        (WgpuPowerPreference::HighPerformance, wgpu::DeviceType::DiscreteGpu)
        | (WgpuPowerPreference::LowPower, wgpu::DeviceType::IntegratedGpu) => 0,
        (_, wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::DiscreteGpu) => 1,
        (_, wgpu::DeviceType::VirtualGpu) => 2,
        (_, wgpu::DeviceType::Cpu) => 3,
        (_, wgpu::DeviceType::Other) => 4,
    }
}

fn graphics_api(backend: wgpu::Backend) -> GraphicsApi {
    match backend {
        wgpu::Backend::Vulkan => GraphicsApi::Vulkan,
        wgpu::Backend::Dx12 => GraphicsApi::Direct3d12,
        wgpu::Backend::Metal => GraphicsApi::Metal,
        wgpu::Backend::Gl => GraphicsApi::OpenGl,
        _ => GraphicsApi::Unknown,
    }
}

fn device_kind(kind: wgpu::DeviceType) -> DeviceKind {
    match kind {
        wgpu::DeviceType::IntegratedGpu => DeviceKind::Integrated,
        wgpu::DeviceType::DiscreteGpu => DeviceKind::Discrete,
        wgpu::DeviceType::VirtualGpu => DeviceKind::Virtual,
        wgpu::DeviceType::Cpu => DeviceKind::Cpu,
        wgpu::DeviceType::Other => DeviceKind::Unknown,
    }
}

fn elapsed_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WgpuTextureRole {
    IndexedPlane,
    Palette,
    ReadbackTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WgpuBackendError {
    NoAdapter {
        api: WgpuApi,
        name_filter: Option<String>,
    },
    MissingRequiredTextureRole(WgpuTextureRole),
    RequestDevice(String),
    Poll(String),
    MapChannel(String),
    Map(String),
    Contract(FrameContractError),
    RawContract(RawC3dContractError),
    TransformCapacity {
        required: usize,
        capacity: usize,
    },
    LayeredExtentMismatch {
        background: [u16; 2],
        foreground: [u16; 2],
    },
    LayeredPaletteMismatch,
}

impl fmt::Display for WgpuBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "wgpu indexed-palette backend failed: {self:?}")
    }
}

impl std::error::Error for WgpuBackendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Contract(source) => Some(source),
            Self::RawContract(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmr_core::c3d_camera::{
        MATRIX_IDENTITY_Q16, projection_matrix_words, view_matrix_words, world_matrix_words,
    };
    use cmr_core::display::DisplayExtent;
    use cmr_formats::riff_palette::PaletteEntry;
    use cmr_gpu_contract::{
        C3dDraw, C3dIndexedTexture, C3dMeshResource, C3dPreparedFrame, C3dRawFrame, C3dResourceSet,
        C3dRgb8, C3dSceneUpload, C3dStageVertex, C3dSubmissionBracket, IndexedPaletteGpuBackend,
        PersistentRawC3dGpuBackend, RawC3dGpuBackend,
    };

    #[test]
    fn adapter_rank_honors_power_preference_before_fallbacks() {
        assert!(
            adapter_rank(
                wgpu::DeviceType::DiscreteGpu,
                WgpuPowerPreference::HighPerformance
            ) < adapter_rank(
                wgpu::DeviceType::IntegratedGpu,
                WgpuPowerPreference::HighPerformance
            )
        );
        assert!(
            adapter_rank(
                wgpu::DeviceType::IntegratedGpu,
                WgpuPowerPreference::LowPower
            ) < adapter_rank(wgpu::DeviceType::DiscreteGpu, WgpuPowerPreference::LowPower)
        );
    }

    #[test]
    fn elapsed_nanoseconds_saturate_instead_of_wrapping() {
        assert_eq!(elapsed_ns(Duration::from_nanos(17)), 17);
        assert_eq!(elapsed_ns(Duration::new(u64::MAX, 999_999_999)), u64::MAX);
    }

    #[test]
    fn raw_c3d_binary_encoders_preserve_exact_words_and_alignment() {
        let vertices = [C3dStageVertex::from_raw_components([
            1, -2, 3, -4, 5, -6, 7, -8,
        ])];
        let bytes = c3d_vertex_bytes(&vertices);
        assert_eq!(bytes.len(), 36);
        assert_eq!(&bytes[0..4], &1i32.to_le_bytes());
        assert_eq!(&bytes[28..32], &(-8i32).to_le_bytes());
        assert_eq!(&bytes[32..36], &[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(&bytes[12..16], &vertices[0].texture_uv()[0].to_le_bytes());
        assert_eq!(&bytes[16..20], &vertices[0].texture_uv()[1].to_le_bytes());
        assert_eq!(c3d_index_bytes(&[0x1234, 0xabcd]), [0x34, 0x12, 0xcd, 0xab]);

        let identity = world_matrix_words(&MATRIX_IDENTITY_Q16).words();
        let transform = c3d_transform_bytes(identity, identity, identity, 4.0 / 3.0);
        assert_eq!(transform.len(), 208);
        assert_eq!(&transform[192..196], &(4.0f32 / 3.0).to_le_bytes());
        assert!(transform[196..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn mapped_material_texels_keep_key_identity_separate_from_palette_mapping() {
        let texture = C3dIndexedTexture::new(
            DisplayExtent {
                width: 2,
                height: 1,
            },
            [0, 1],
            [
                C3dRgb8 {
                    red: 10,
                    green: 20,
                    blue: 30,
                },
                C3dRgb8 {
                    red: 10,
                    green: 20,
                    blue: 30,
                },
            ],
            Some(0),
        )
        .unwrap();
        let palette = std::array::from_fn(|index| PaletteEntry {
            red: if index == 7 { 10 } else { 0 },
            green: if index == 7 { 20 } else { 0 },
            blue: if index == 7 { 30 } else { 0 },
            flags: 0,
        });

        assert_eq!(c3d_mapped_material_texels(&texture, &palette), [7, 1, 7, 0]);
    }

    #[test]
    #[ignore = "requires a Vulkan adapter; run explicitly for raw C3D GPU conformance"]
    fn raw_c3d_triangle_uses_gpu_transform_texture_and_depth_path() {
        const ONE: i32 = 0x1_0000;
        let vertex = |x: i32, y: i32, z: i32, u: i32, v: i32| {
            C3dStageVertex::from_raw_components([x, y, z, u, v, 0, 0, ONE])
        };
        let triangle = [
            vertex(-ONE / 2, -ONE / 2, 2 * ONE, 0, ONE),
            vertex(0, ONE / 2, 2 * ONE, ONE / 2, 0),
            vertex(ONE / 2, -ONE / 2, 2 * ONE, ONE, ONE),
        ];
        let mesh = C3dMeshResource::new(triangle, [0, 1, 2]).unwrap();
        let texture = C3dIndexedTexture::new(
            DisplayExtent {
                width: 1,
                height: 1,
            },
            [0],
            [C3dRgb8 {
                red: 250,
                green: 4,
                blue: 3,
            }],
            None,
        )
        .unwrap();
        let resources = C3dResourceSet::new([mesh], [texture]);
        let identity = world_matrix_words(&MATRIX_IDENTITY_Q16);
        let draw = C3dDraw::new(0, 0, identity, C3dSubmissionBracket::Normal).unwrap();
        let active_palette: [PaletteEntry; 256] = std::array::from_fn(|index| {
            if index == 7 {
                PaletteEntry {
                    red: 250,
                    green: 4,
                    blue: 3,
                    flags: 0xaa,
                }
            } else {
                PaletteEntry {
                    red: 0,
                    green: 0,
                    blue: 0,
                    flags: index as u8,
                }
            }
        });
        let extent = DisplayExtent {
            width: 64,
            height: 64,
        };
        let draws = [draw];
        let frame = C3dRawFrame::new(
            extent,
            view_matrix_words(&MATRIX_IDENTITY_Q16),
            projection_matrix_words(2.0, 2.4, 1.0, 250.0),
            &active_palette,
            &resources,
            &draws,
        )
        .unwrap();
        let config = WgpuBackendConfig {
            adapter_name_contains: std::env::var("CMR_WGPU_TEST_ADAPTER").ok(),
            ..WgpuBackendConfig::default()
        };
        let mut backend = WgpuIndexedPaletteBackend::new(&config).unwrap();
        let rendered = backend.render_c3d_readback(frame).unwrap();
        let red = [250, 4, 3, 255];
        let red_pixels = rendered
            .rgba8
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| **pixel == red)
            .count();
        assert!(
            red_pixels > 0,
            "clockwise raw triangle must survive culling"
        );
        assert!(
            red_pixels < 64 * 64,
            "triangle must not cover the clear target"
        );

        let upload = C3dSceneUpload::new(extent, &resources, &active_palette).unwrap();
        let mut prepared = backend.prepare_c3d_scene(upload).unwrap();
        let prepared_frame = C3dPreparedFrame::new(
            WgpuIndexedPaletteBackend::prepared_layout(&prepared),
            view_matrix_words(&MATRIX_IDENTITY_Q16),
            projection_matrix_words(2.0, 2.4, 1.0, 250.0),
            &draws,
        )
        .unwrap();
        let prepared_pixels = backend
            .render_prepared_readback(&mut prepared, prepared_frame)
            .unwrap();
        assert_eq!(prepared_pixels, rendered.rgba8);

        // The presentation readback emits the active-palette slot per pixel;
        // pairing it with the same palette must reconstruct the expanded
        // readback byte for byte, which proves the indexed host path is the
        // same rendering result (not a lossy inversion).
        let index_readback = backend
            .render_prepared_indexed_readback(&mut prepared, prepared_frame)
            .unwrap();
        assert_eq!(index_readback.extent(), extent);
        let indexed_presentation =
            IndexedFrameView::new(extent, index_readback.indices(), &active_palette).unwrap();
        assert_eq!(
            indexed_presentation.canonical_rgba8().unwrap(),
            prepared_pixels
        );
        // The one mapped material resolves to palette slot 7, and the clear
        // target to index 0.
        assert!(
            index_readback.indices().contains(&7),
            "the mapped material slot must appear in the index readback"
        );
        assert_eq!(index_readback.indices()[0], 0);
        // The depth variant reads the same image and one depth per pixel,
        // nearer than the clear where the triangle drew.
        let (with_depth, depth) = backend
            .render_prepared_indexed_readback_with_depth(&mut prepared, prepared_frame)
            .unwrap();
        assert_eq!(with_depth, index_readback);
        assert_eq!(depth.len(), 64 * 64);
        let drawn = index_readback
            .indices()
            .iter()
            .position(|&i| i == 7)
            .unwrap();
        assert!(depth[drawn] < depth[0]);
        let uploads = prepared.static_upload_counts();
        let first_timing = backend
            .render_prepared_timed(&mut prepared, prepared_frame)
            .unwrap();
        let second_timing = backend
            .render_prepared_timed(&mut prepared, prepared_frame)
            .unwrap();
        assert_eq!(prepared.static_upload_counts(), uploads);
        assert_eq!(prepared.timed_frame_count(), 2);
        assert_eq!(prepared.timed_resource_allocations(), 0);
        assert!(first_timing.cpu_encode_submit_ns > 0);
        assert!(first_timing.submit_to_completion_ns > 0);
        assert!(second_timing.cpu_encode_submit_ns > 0);
        assert!(second_timing.submit_to_completion_ns > 0);

        // A rewritten vertex buffer renders as a freshly prepared one with
        // the same vertices; a changed count is refused.
        let shifted = [
            vertex(-ONE / 4, -ONE / 2, 2 * ONE, 0, ONE),
            vertex(ONE / 4, ONE / 2, 2 * ONE, ONE / 2, 0),
            vertex(3 * ONE / 4, -ONE / 2, 2 * ONE, ONE, ONE),
        ];
        backend
            .rewrite_prepared_mesh_vertices(&mut prepared, 0, &shifted)
            .unwrap();
        let rewritten = backend
            .render_prepared_readback(&mut prepared, prepared_frame)
            .unwrap();
        let fresh_resources = C3dResourceSet::new(
            [C3dMeshResource::new(shifted, [0, 1, 2]).unwrap()],
            resources.textures().to_vec(),
        );
        let fresh_upload = C3dSceneUpload::new(extent, &fresh_resources, &active_palette).unwrap();
        let mut fresh = backend.prepare_c3d_scene(fresh_upload).unwrap();
        let fresh_pixels = backend
            .render_prepared_readback(&mut fresh, prepared_frame)
            .unwrap();
        assert_eq!(rewritten, fresh_pixels);
        assert_ne!(rewritten, prepared_pixels);
        assert!(
            backend
                .rewrite_prepared_mesh_vertices(&mut prepared, 0, &shifted[..2])
                .is_err()
        );
    }

    #[test]
    #[ignore = "requires a Vulkan adapter; run explicitly for keyed C3D GPU conformance"]
    fn keyed_fragment_preserves_prior_color_and_depth() {
        const ONE: i32 = 0x1_0000;
        let vertex = |x: i32, y: i32, z: i32| {
            C3dStageVertex::from_raw_components([x, y, z, 0, 0, 0, 0, ONE])
        };
        let triangle_at = |z| {
            C3dMeshResource::new(
                [
                    vertex(-ONE / 2, -ONE / 2, z),
                    vertex(0, ONE / 2, z),
                    vertex(ONE / 2, -ONE / 2, z),
                ],
                [0, 1, 2],
            )
            .unwrap()
        };
        let texture = |rgb, color_key_index| {
            C3dIndexedTexture::new(
                DisplayExtent {
                    width: 1,
                    height: 1,
                },
                [0],
                [rgb],
                color_key_index,
            )
            .unwrap()
        };
        let red = C3dRgb8 {
            red: 250,
            green: 4,
            blue: 3,
        };
        let keyed = C3dRgb8 {
            red: 220,
            green: 220,
            blue: 220,
        };
        let green = C3dRgb8 {
            red: 5,
            green: 240,
            blue: 7,
        };
        let resources = C3dResourceSet::new(
            [
                triangle_at(3 * ONE),
                triangle_at(2 * ONE),
                triangle_at(5 * ONE / 2),
            ],
            [
                texture(red, None),
                texture(keyed, Some(0)),
                texture(green, None),
            ],
        );
        let identity = world_matrix_words(&MATRIX_IDENTITY_Q16);
        let draws = [
            C3dDraw::new(0, 0, identity, C3dSubmissionBracket::Normal).unwrap(),
            C3dDraw::new(1, 1, identity, C3dSubmissionBracket::Normal).unwrap(),
            C3dDraw::new(2, 2, identity, C3dSubmissionBracket::Normal).unwrap(),
        ];
        let active_palette: [PaletteEntry; 256] = std::array::from_fn(|index| {
            let (red, green, blue) = match index {
                7 => (red.red, red.green, red.blue),
                8 => (keyed.red, keyed.green, keyed.blue),
                9 => (green.red, green.green, green.blue),
                _ => (0, 0, 0),
            };
            PaletteEntry {
                red,
                green,
                blue,
                flags: 0,
            }
        });
        let extent = DisplayExtent {
            width: 64,
            height: 64,
        };
        let config = WgpuBackendConfig {
            adapter_name_contains: std::env::var("CMR_WGPU_TEST_ADAPTER").ok(),
            ..WgpuBackendConfig::default()
        };
        let mut backend = WgpuIndexedPaletteBackend::new(&config).unwrap();
        let upload = C3dSceneUpload::new(extent, &resources, &active_palette).unwrap();
        let mut prepared = backend.prepare_c3d_scene(upload).unwrap();
        let frame = C3dPreparedFrame::new(
            WgpuIndexedPaletteBackend::prepared_layout(&prepared),
            view_matrix_words(&MATRIX_IDENTITY_Q16),
            projection_matrix_words(2.0, 2.4, 1.0, 250.0),
            &draws,
        )
        .unwrap();

        let indices = backend
            .render_prepared_indexed_readback(&mut prepared, frame)
            .unwrap();
        let center = usize::from(extent.height / 2) * usize::from(extent.width)
            + usize::from(extent.width / 2);
        assert_eq!(indices.indices()[center], 9);
        assert!(indices.indices().contains(&9));
        assert!(!indices.indices().contains(&7));
        assert!(!indices.indices().contains(&8));
    }

    #[test]
    #[ignore = "requires a Vulkan adapter; run explicitly for GPU conformance"]
    fn vulkan_readback_matches_cpu_oracle_across_palette_mutation() {
        let config = WgpuBackendConfig {
            adapter_name_contains: std::env::var("CMR_WGPU_TEST_ADAPTER").ok(),
            ..WgpuBackendConfig::default()
        };
        let mut backend = WgpuIndexedPaletteBackend::new(&config).unwrap();
        let extent = DisplayExtent {
            width: 3,
            height: 2,
        };
        let indices = [0, 1, 255, 2, 1, 0];
        let mut palette: [PaletteEntry; 256] = std::array::from_fn(|index| PaletteEntry {
            red: index as u8,
            green: (255 - index) as u8,
            blue: (index as u8).wrapping_mul(7),
            flags: (index as u8).wrapping_mul(13),
        });

        for changed in [false, true] {
            if changed {
                palette[1] = PaletteEntry {
                    red: 9,
                    green: 8,
                    blue: 7,
                    flags: 0xfe,
                };
            }
            let frame = IndexedFrameView::new(extent, &indices, &palette).unwrap();
            let expected = frame.canonical_rgba8().unwrap();
            let actual = backend.render_readback(frame).unwrap();
            assert_eq!(actual.rgba8, expected);
        }

        assert_eq!(
            IndexedPaletteGpuBackend::info(&backend).graphics_api,
            GraphicsApi::Vulkan
        );
        assert!(IndexedPaletteGpuBackend::capabilities(&backend).sampled_r8_uint);
        assert!(IndexedPaletteGpuBackend::capabilities(&backend).headless_readback);
        eprintln!(
            "wgpu adapter: {:?}",
            IndexedPaletteGpuBackend::info(&backend)
        );
    }
}
