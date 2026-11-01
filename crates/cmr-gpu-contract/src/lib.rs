//! API-neutral contracts for GPU conformance and performance evidence.
//!
//! This crate deliberately contains no wgpu, SDL, window, or driver types.
//! Candidate backends receive the same validated indexed frame and normalize
//! readback into the same tightly packed opaque RGBA8 representation. Native
//! API initialization can remain asynchronous in each adapter; the per-frame
//! conformance operation is synchronous so tests can compare complete pixels.

use std::fmt;

use cmr_core::display::DisplayExtent;
use cmr_formats::riff_palette::PaletteEntry;

/// Palette entry count of the recovered 8-bit indexed display.
pub const INDEXED_PALETTE_LEN: usize = 256;
/// Bytes per pixel in canonical readback output.
pub const RGBA8_BYTES_PER_PIXEL: usize = 4;

/// A validated borrowed indexed frame shared by every candidate backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexedFrameView<'a> {
    extent: DisplayExtent,
    indices: &'a [u8],
    palette: &'a [PaletteEntry],
}

impl<'a> IndexedFrameView<'a> {
    /// Validates nonzero extent, exact `width * height` index storage, and the
    /// complete 256-entry palette required by an `u8` index.
    pub fn new(
        extent: DisplayExtent,
        indices: &'a [u8],
        palette: &'a [PaletteEntry],
    ) -> Result<Self, FrameContractError> {
        let pixel_count = checked_pixel_count(extent)?;
        if indices.len() != pixel_count {
            return Err(FrameContractError::InvalidIndexLength {
                expected: pixel_count,
                actual: indices.len(),
            });
        }
        if palette.len() != INDEXED_PALETTE_LEN {
            return Err(FrameContractError::InvalidPaletteLength {
                expected: INDEXED_PALETTE_LEN,
                actual: palette.len(),
            });
        }
        Ok(Self {
            extent,
            indices,
            palette,
        })
    }

    #[must_use]
    pub const fn extent(self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub const fn indices(self) -> &'a [u8] {
        self.indices
    }

    #[must_use]
    pub const fn palette(self) -> &'a [PaletteEntry] {
        self.palette
    }

    #[must_use]
    pub fn pixel_count(self) -> usize {
        self.indices.len()
    }

    /// Expands the frame into the canonical comparison representation.
    ///
    /// Palette flags are control/reserved data, not alpha. Every output pixel
    /// is therefore opaque regardless of its palette entry's flag byte.
    pub fn canonical_rgba8(self) -> Result<CanonicalRgba8Frame, FrameContractError> {
        let output_len = self
            .indices
            .len()
            .checked_mul(RGBA8_BYTES_PER_PIXEL)
            .ok_or(FrameContractError::SizeOverflow(self.extent))?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(output_len)
            .map_err(|_| FrameContractError::AllocationFailed { bytes: output_len })?;
        for &index in self.indices {
            let entry = self.palette[usize::from(index)];
            pixels.extend_from_slice(&[entry.red, entry.green, entry.blue, u8::MAX]);
        }
        Ok(CanonicalRgba8Frame {
            extent: self.extent,
            pixels: pixels.into_boxed_slice(),
        })
    }
}

/// A tightly packed, row-major, opaque RGBA8 conformance result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalRgba8Frame {
    extent: DisplayExtent,
    pixels: Box<[u8]>,
}

impl CanonicalRgba8Frame {
    #[must_use]
    pub const fn extent(&self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub const fn row_pitch(&self) -> usize {
        self.extent.width as usize * RGBA8_BYTES_PER_PIXEL
    }
}

/// Byte ordering of a mapped four-channel GPU readback surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadbackFormat {
    Rgba8,
    Bgra8,
}

/// Removes driver-owned row padding and channel-order differences from mapped
/// readback bytes, yielding the canonical tightly packed RGBA8 comparison form.
pub fn normalize_readback(
    extent: DisplayExtent,
    mapped: &[u8],
    bytes_per_row: usize,
    format: ReadbackFormat,
) -> Result<CanonicalRgba8Frame, FrameContractError> {
    let pixel_count = checked_pixel_count(extent)?;
    let tight_pitch = usize::from(extent.width)
        .checked_mul(RGBA8_BYTES_PER_PIXEL)
        .ok_or(FrameContractError::SizeOverflow(extent))?;
    if bytes_per_row < tight_pitch {
        return Err(FrameContractError::ReadbackPitchTooSmall {
            minimum: tight_pitch,
            actual: bytes_per_row,
        });
    }
    let mapped_len = bytes_per_row
        .checked_mul(usize::from(extent.height))
        .ok_or(FrameContractError::SizeOverflow(extent))?;
    if mapped.len() != mapped_len {
        return Err(FrameContractError::InvalidReadbackLength {
            expected: mapped_len,
            actual: mapped.len(),
        });
    }
    let output_len = pixel_count
        .checked_mul(RGBA8_BYTES_PER_PIXEL)
        .ok_or(FrameContractError::SizeOverflow(extent))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(output_len)
        .map_err(|_| FrameContractError::AllocationFailed { bytes: output_len })?;
    for row in mapped.chunks_exact(bytes_per_row) {
        let (row_pixels, remainder) = row[..tight_pitch].as_chunks::<RGBA8_BYTES_PER_PIXEL>();
        debug_assert!(remainder.is_empty());
        for pixel in row_pixels {
            match format {
                ReadbackFormat::Rgba8 => pixels.extend_from_slice(pixel),
                ReadbackFormat::Bgra8 => {
                    pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]])
                }
            }
        }
    }
    Ok(CanonicalRgba8Frame {
        extent,
        pixels: pixels.into_boxed_slice(),
    })
}

fn checked_pixel_count(extent: DisplayExtent) -> Result<usize, FrameContractError> {
    if extent.width == 0 || extent.height == 0 {
        return Err(FrameContractError::InvalidExtent(extent));
    }
    usize::from(extent.width)
        .checked_mul(usize::from(extent.height))
        .ok_or(FrameContractError::SizeOverflow(extent))
}

/// Validation/allocation failure at the shared GPU conformance boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameContractError {
    InvalidExtent(DisplayExtent),
    SizeOverflow(DisplayExtent),
    InvalidIndexLength { expected: usize, actual: usize },
    InvalidPaletteLength { expected: usize, actual: usize },
    ReadbackPitchTooSmall { minimum: usize, actual: usize },
    InvalidReadbackLength { expected: usize, actual: usize },
    AllocationFailed { bytes: usize },
}

impl fmt::Display for FrameContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GPU frame contract failed: {self:?}")
    }
}

impl std::error::Error for FrameContractError {}

/// Adapter implementation being measured, separate from its selected graphics
/// API (for example, both candidates can select Vulkan).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    Wgpu,
    SdlGpu,
}

/// Native graphics API selected by a backend instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsApi {
    Vulkan,
    Direct3d12,
    Metal,
    OpenGl,
    Software,
    Unknown,
}

/// Hardware class reported by the native API for adapter comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    Integrated,
    Discrete,
    Virtual,
    Cpu,
    Unknown,
}

/// Stable adapter identity included with conformance and benchmark evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendInfo {
    pub backend: BackendKind,
    pub graphics_api: GraphicsApi,
    /// PCI-style vendor ID when the native API exposes one.
    pub vendor_id: Option<u32>,
    /// PCI-style device ID when the native API exposes one.
    pub device_id: Option<u32>,
    pub device_kind: DeviceKind,
    pub adapter_name: String,
    pub driver_name: String,
    pub driver_info: String,
}

impl BackendInfo {
    /// Whether both reports identify the same vendor/device model pair.
    ///
    /// A missing native ID never counts as a match; benchmark orchestration
    /// must report the model comparison as unverified instead of guessing from
    /// names. Equal IDs do not prove that two backends selected the same
    /// physical adapter when a system has multiple identical devices.
    #[must_use]
    pub fn same_vendor_device_pair(&self, other: &Self) -> bool {
        matches!(
            (
                self.vendor_id,
                self.device_id,
                other.vendor_id,
                other.device_id
            ),
            (Some(a_vendor), Some(a_device), Some(b_vendor), Some(b_device))
                if a_vendor == b_vendor && a_device == b_device
        )
    }
}

/// Capabilities that change which comparisons/timings are authoritative.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendCapabilities {
    pub sampled_r8_uint: bool,
    pub timestamp_queries: bool,
    pub headless_readback: bool,
    pub device_recovery: bool,
}

/// Per-frame timing returned beside completed readback.
///
/// The two required wall-clock intervals permit a fair comparison even when a
/// backend cannot expose GPU timestamp queries. `gpu_timestamp_ns` is only
/// supplementary and must not replace submit-to-completion measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameTiming {
    /// CPU wall time from per-frame allocation/upload preparation through
    /// encoding and successful submission.
    pub cpu_encode_submit_ns: u64,
    /// Wall time from successful submission until its completion fence/event.
    pub submit_to_completion_ns: u64,
    /// Native GPU timestamp delta when supported and calibrated.
    pub gpu_timestamp_ns: Option<u64>,
}

/// One complete synchronous conformance observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConformanceFrame {
    pub rgba8: CanonicalRgba8Frame,
    pub timing: FrameTiming,
}

/// Common operation implemented by both native GPU candidates.
///
/// Construction is intentionally outside this trait so an adapter can perform
/// asynchronous instance/device setup without infecting this shared contract.
pub trait IndexedPaletteGpuBackend {
    type Error;

    fn info(&self) -> &BackendInfo;
    fn capabilities(&self) -> BackendCapabilities;
    fn render_readback(
        &mut self,
        frame: IndexedFrameView<'_>,
    ) -> Result<ConformanceFrame, Self::Error>;
}

/// One recovered stage C3D vertex ready for a native GPU upload.
///
/// Position and normal remain the six signed Q16.16 source words. UV is the
/// shared recovered float submission value, stored as IEEE-754 bits so this
/// carrier remains exactly comparable and hashable across backends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dStageVertex {
    position_q16: [i32; 3],
    texture_uv_bits: [u32; 2],
    normal_q16: [i32; 3],
    diffuse_rgba: [u8; 4],
}

impl C3dStageVertex {
    /// Builds the mixed stage-upload carrier from the eight raw C3D Q16.16
    /// components, applying the shared draw-time UV preprocessing contract.
    #[must_use]
    pub fn from_raw_components(components: [i32; 8]) -> Self {
        let uv = cmr_core::c3d_render::stage_texture_uv([components[3], components[4]]);
        Self {
            position_q16: [components[0], components[1], components[2]],
            texture_uv_bits: uv.map(f32::to_bits),
            normal_q16: [components[5], components[6], components[7]],
            diffuse_rgba: [0xff, 0xff, 0xff, 0xff],
        }
    }

    /// Attaches the per-vertex diffuse RGBA color (`FUN_004b1e10`, `D3DTBLEND_MODULATE`).
    #[must_use]
    pub const fn with_diffuse_rgba(mut self, diffuse_rgba: [u8; 4]) -> Self {
        self.diffuse_rgba = diffuse_rgba;
        self
    }

    #[must_use]
    pub const fn diffuse_rgba(self) -> [u8; 4] {
        self.diffuse_rgba
    }

    #[must_use]
    pub const fn position_q16(self) -> [i32; 3] {
        self.position_q16
    }

    #[must_use]
    pub fn texture_uv(self) -> [f32; 2] {
        self.texture_uv_bits.map(f32::from_bits)
    }

    #[must_use]
    pub const fn texture_uv_bits(self) -> [u32; 2] {
        self.texture_uv_bits
    }

    #[must_use]
    pub const fn normal_q16(self) -> [i32; 3] {
        self.normal_q16
    }

    /// The exact 32-byte mixed vertex layout consumed by both native backends:
    /// three Q16 position words, two float UV words, three Q16 normal words.
    #[must_use]
    pub const fn upload_words(self) -> [u32; 8] {
        [
            self.position_q16[0] as u32,
            self.position_q16[1] as u32,
            self.position_q16[2] as u32,
            self.texture_uv_bits[0],
            self.texture_uv_bits[1],
            self.normal_q16[0] as u32,
            self.normal_q16[1] as u32,
            self.normal_q16[2] as u32,
        ]
    }
}

impl From<cmr_formats::c3d::C3dEntry32<'_>> for C3dStageVertex {
    fn from(entry: cmr_formats::c3d::C3dEntry32<'_>) -> Self {
        Self::from_raw_components(entry.components_q16())
    }
}

/// Validated owned raw geometry for one recovered C3D primitive batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C3dMeshResource {
    vertices: Box<[C3dStageVertex]>,
    indices: Box<[u16]>,
}

impl C3dMeshResource {
    /// Admits a non-empty indexed triangle list whose compact indices all
    /// address the supplied raw vertices.
    pub fn new(
        vertices: impl Into<Box<[C3dStageVertex]>>,
        indices: impl Into<Box<[u16]>>,
    ) -> Result<Self, RawC3dContractError> {
        let vertices = vertices.into();
        let indices = indices.into();
        if indices.is_empty() || indices.len() % 3 != 0 {
            return Err(RawC3dContractError::InvalidTriangleIndexCount {
                actual: indices.len(),
            });
        }
        for (position, &index) in indices.iter().enumerate() {
            if usize::from(index) >= vertices.len() {
                return Err(RawC3dContractError::VertexIndexOutOfBounds {
                    position,
                    index,
                    vertex_count: vertices.len(),
                });
            }
        }
        Ok(Self { vertices, indices })
    }

    #[must_use]
    pub fn vertices(&self) -> &[C3dStageVertex] {
        &self.vertices
    }

    #[must_use]
    pub fn indices(&self) -> &[u16] {
        &self.indices
    }
}

/// API-neutral RGB entry for a raw C3D indexed texture palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dRgb8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<cmr_formats::bitmap::BitmapPaletteEntry> for C3dRgb8 {
    fn from(entry: cmr_formats::bitmap::BitmapPaletteEntry) -> Self {
        Self {
            red: entry.red,
            green: entry.green,
            blue: entry.blue,
        }
    }
}

/// Validated owned indexed texture referenced by raw C3D draw packets.
///
/// `color_key_index`, when present, discards that source index before either a
/// color or depth write. Callers choose opacity explicitly by supplying `None`;
/// this lets the main stage remain opaque while keyed OBJ/SHT/car layers share
/// one depth buffer without occluding geometry through transparent texels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C3dIndexedTexture {
    extent: DisplayExtent,
    indices: Box<[u8]>,
    palette: Box<[C3dRgb8]>,
    color_key_index: Option<u8>,
}

impl C3dIndexedTexture {
    /// Admits a non-empty tightly packed texture with 1..=256 palette entries.
    /// Every texel and an optional color-key index must address that palette.
    pub fn new(
        extent: DisplayExtent,
        indices: impl Into<Box<[u8]>>,
        palette: impl Into<Box<[C3dRgb8]>>,
        color_key_index: Option<u8>,
    ) -> Result<Self, RawC3dContractError> {
        let expected = checked_pixel_count(extent)?;
        let indices = indices.into();
        let palette = palette.into();
        if indices.len() != expected {
            return Err(RawC3dContractError::InvalidTextureIndexLength {
                expected,
                actual: indices.len(),
            });
        }
        if palette.is_empty() || palette.len() > INDEXED_PALETTE_LEN {
            return Err(RawC3dContractError::InvalidTexturePaletteLength {
                actual: palette.len(),
            });
        }
        for (position, &index) in indices.iter().enumerate() {
            if usize::from(index) >= palette.len() {
                return Err(RawC3dContractError::TexturePaletteIndexOutOfBounds {
                    position,
                    index,
                    palette_len: palette.len(),
                });
            }
        }
        if let Some(index) = color_key_index
            && usize::from(index) >= palette.len()
        {
            return Err(RawC3dContractError::ColorKeyIndexOutOfBounds {
                index,
                palette_len: palette.len(),
            });
        }
        Ok(Self {
            extent,
            indices,
            palette,
            color_key_index,
        })
    }

    #[must_use]
    pub const fn extent(&self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub fn indices(&self) -> &[u8] {
        &self.indices
    }

    #[must_use]
    pub fn palette(&self) -> &[C3dRgb8] {
        &self.palette
    }

    #[must_use]
    pub const fn color_key_index(&self) -> Option<u8> {
        self.color_key_index
    }
}

/// Maps each source material palette entry to the exact active display-palette
/// index used by the established software stage composer.
///
/// RGB Manhattan distance is minimized across all 256 active entries. Iteration
/// is ascending and equal distances do not replace the current result, so the
/// lowest index wins every tie. Palette flags do not participate.
#[must_use]
pub fn c3d_active_palette_l1_map(
    texture: &C3dIndexedTexture,
    active_palette: &[PaletteEntry; INDEXED_PALETTE_LEN],
) -> Box<[u8]> {
    texture
        .palette
        .iter()
        .map(|source| {
            let mut best_index = 0u8;
            let mut best_distance = u16::MAX;
            for (index, destination) in active_palette.iter().enumerate() {
                let distance = u16::from(source.red.abs_diff(destination.red))
                    + u16::from(source.green.abs_diff(destination.green))
                    + u16::from(source.blue.abs_diff(destination.blue));
                if distance < best_distance {
                    best_distance = distance;
                    best_index = index as u8;
                    if distance == 0 {
                        break;
                    }
                }
            }
            best_index
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

/// The scene-walk bracket in which a node was submitted.
///
/// This remains separate from depth and blend state: later recovery established
/// that the dispatcher deferred translucent nodes without disabling depth
/// writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3dSubmissionBracket {
    Normal,
    DeferredTranslucent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3dDepthCompare {
    LessEqual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3dCullMode {
    /// Legacy `D3DCULL_CCW`: clockwise faces survive.
    CullCounterClockwise,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3dFilter {
    PointClamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C3dBlendMode {
    OpaqueCopy,
}

/// The bounded first native-C3D profile shared with the established software
/// stage composer. `OpaqueCopy` still follows the compatibility color path:
/// source material palette RGB is mapped to the nearest active-display-palette
/// RGB by L1 distance (lowest-index tie), and only then expanded for readback.
///
/// Fields are private and the sole value keeps depth writes enabled. Broader
/// recovered blend/filter profiles require their own contract milestone rather
/// than silently changing this benchmark. Texture-level color-key discard is
/// applied before this opaque-copy state writes color or depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dPipelineState {
    depth_compare: C3dDepthCompare,
    depth_write: bool,
    cull: C3dCullMode,
    filter: C3dFilter,
    blend: C3dBlendMode,
    perspective_correct: bool,
}

impl C3dPipelineState {
    pub const OPAQUE_COPY: Self = Self {
        depth_compare: C3dDepthCompare::LessEqual,
        depth_write: true,
        cull: C3dCullMode::CullCounterClockwise,
        filter: C3dFilter::PointClamp,
        blend: C3dBlendMode::OpaqueCopy,
        perspective_correct: true,
    };

    #[must_use]
    pub const fn depth_compare(self) -> C3dDepthCompare {
        self.depth_compare
    }

    #[must_use]
    pub const fn depth_write(self) -> bool {
        self.depth_write
    }

    #[must_use]
    pub const fn cull(self) -> C3dCullMode {
        self.cull
    }

    #[must_use]
    pub const fn filter(self) -> C3dFilter {
        self.filter
    }

    #[must_use]
    pub const fn blend(self) -> C3dBlendMode {
        self.blend
    }

    #[must_use]
    pub const fn perspective_correct(self) -> bool {
        self.perspective_correct
    }
}

/// One raw draw referencing validated scene resources by stable ordinal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dDraw {
    mesh: u32,
    texture: u32,
    world: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
    submission: C3dSubmissionBracket,
    state: C3dPipelineState,
}

impl C3dDraw {
    pub fn new(
        mesh: u32,
        texture: u32,
        world: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
        submission: C3dSubmissionBracket,
    ) -> Result<Self, RawC3dContractError> {
        validate_matrix("world", world)?;
        Ok(Self {
            mesh,
            texture,
            world,
            submission,
            state: C3dPipelineState::OPAQUE_COPY,
        })
    }

    #[must_use]
    pub const fn mesh(self) -> u32 {
        self.mesh
    }

    #[must_use]
    pub const fn texture(self) -> u32 {
        self.texture
    }

    #[must_use]
    pub const fn world(self) -> cmr_core::root_menu_c3d_transform::C3dMatrixWords {
        self.world
    }

    #[must_use]
    pub const fn submission(self) -> C3dSubmissionBracket {
        self.submission
    }

    #[must_use]
    pub const fn state(self) -> C3dPipelineState {
        self.state
    }
}

/// Owned immutable geometry and texture tables shared across benchmark frames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct C3dResourceSet {
    meshes: Box<[C3dMeshResource]>,
    textures: Box<[C3dIndexedTexture]>,
}

impl C3dResourceSet {
    #[must_use]
    pub fn new(
        meshes: impl Into<Box<[C3dMeshResource]>>,
        textures: impl Into<Box<[C3dIndexedTexture]>>,
    ) -> Self {
        Self {
            meshes: meshes.into(),
            textures: textures.into(),
        }
    }

    #[must_use]
    pub fn meshes(&self) -> &[C3dMeshResource] {
        &self.meshes
    }

    #[must_use]
    pub fn textures(&self) -> &[C3dIndexedTexture] {
        &self.textures
    }

    /// Replace one mesh's vertices in place, keeping its topology: the new
    /// list must have the same length, so every index stays in range. A
    /// persistent backend re-uploads that mesh's vertex buffer to match.
    ///
    /// # Errors
    ///
    /// [`RawC3dContractError::MeshRewriteOutOfBounds`] for an unknown mesh,
    /// [`RawC3dContractError::MeshRewriteVertexCount`] for a length change.
    pub fn replace_mesh_vertices(
        &mut self,
        mesh: u32,
        vertices: &[C3dStageVertex],
    ) -> Result<(), RawC3dContractError> {
        let count = self.meshes.len();
        let slot = self.meshes.get_mut(mesh as usize).ok_or(
            RawC3dContractError::MeshRewriteOutOfBounds {
                resource: mesh,
                resource_count: count,
            },
        )?;
        if slot.vertices.len() != vertices.len() {
            return Err(RawC3dContractError::MeshRewriteVertexCount {
                expected: slot.vertices.len(),
                actual: vertices.len(),
            });
        }
        slot.vertices.copy_from_slice(vertices);
        Ok(())
    }
}

/// Validated per-frame raw C3D draw stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dRawFrame<'a> {
    extent: DisplayExtent,
    view: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
    projection: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
    active_palette: &'a [PaletteEntry; INDEXED_PALETTE_LEN],
    resources: &'a C3dResourceSet,
    draws: &'a [C3dDraw],
}

impl<'a> C3dRawFrame<'a> {
    pub fn new(
        extent: DisplayExtent,
        view: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
        projection: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
        active_palette: &'a [PaletteEntry; INDEXED_PALETTE_LEN],
        resources: &'a C3dResourceSet,
        draws: &'a [C3dDraw],
    ) -> Result<Self, RawC3dContractError> {
        checked_pixel_count(extent)?;
        validate_matrix("view", view)?;
        validate_matrix("projection", projection)?;
        for (draw_index, draw) in draws.iter().copied().enumerate() {
            if usize::try_from(draw.mesh)
                .ok()
                .is_none_or(|index| index >= resources.meshes.len())
            {
                return Err(RawC3dContractError::MeshResourceOutOfBounds {
                    draw: draw_index,
                    resource: draw.mesh,
                    resource_count: resources.meshes.len(),
                });
            }
            if usize::try_from(draw.texture)
                .ok()
                .is_none_or(|index| index >= resources.textures.len())
            {
                return Err(RawC3dContractError::TextureResourceOutOfBounds {
                    draw: draw_index,
                    resource: draw.texture,
                    resource_count: resources.textures.len(),
                });
            }
        }
        Ok(Self {
            extent,
            view,
            projection,
            active_palette,
            resources,
            draws,
        })
    }

    #[must_use]
    pub const fn extent(self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub const fn view(self) -> cmr_core::root_menu_c3d_transform::C3dMatrixWords {
        self.view
    }

    #[must_use]
    pub const fn projection(self) -> cmr_core::root_menu_c3d_transform::C3dMatrixWords {
        self.projection
    }

    /// The exact 256-entry display palette into which source material RGB is
    /// quantized before final RGBA presentation.
    ///
    /// The compatibility result is not direct source-palette RGB: each source
    /// texel first selects the active entry with minimum RGB L1 distance (the
    /// lowest index wins ties), then that active entry supplies the output RGB.
    /// Backends may precompute this material-index to active-index mapping
    /// outside the timed draw interval.
    #[must_use]
    pub const fn active_palette(self) -> &'a [PaletteEntry; INDEXED_PALETTE_LEN] {
        self.active_palette
    }

    #[must_use]
    pub const fn resources(self) -> &'a C3dResourceSet {
        self.resources
    }

    #[must_use]
    pub const fn draws(self) -> &'a [C3dDraw] {
        self.draws
    }
}

/// Immutable shape of one persistently prepared raw-C3D scene.
///
/// The fixed extent lets candidates retain color/depth targets. Resource counts
/// validate every per-frame ordinal without exposing native resource handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dPreparedSceneLayout {
    extent: DisplayExtent,
    mesh_count: u32,
    texture_count: u32,
}

impl C3dPreparedSceneLayout {
    #[must_use]
    pub const fn extent(self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub const fn mesh_count(self) -> u32 {
        self.mesh_count
    }

    #[must_use]
    pub const fn texture_count(self) -> u32 {
        self.texture_count
    }
}

/// Borrowed one-time upload input for a persistent raw-C3D scene.
///
/// Extent and active display palette are immutable preparation inputs. A
/// palette change deliberately requires preparing another scene so palette
/// mapping/upload cannot leak into steady-state frame timings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dSceneUpload<'a> {
    layout: C3dPreparedSceneLayout,
    resources: &'a C3dResourceSet,
    active_palette: &'a [PaletteEntry; INDEXED_PALETTE_LEN],
}

impl<'a> C3dSceneUpload<'a> {
    pub fn new(
        extent: DisplayExtent,
        resources: &'a C3dResourceSet,
        active_palette: &'a [PaletteEntry; INDEXED_PALETTE_LEN],
    ) -> Result<Self, RawC3dContractError> {
        checked_pixel_count(extent)?;
        let mesh_count = u32::try_from(resources.meshes.len()).map_err(|_| {
            RawC3dContractError::PreparedResourceCountOverflow {
                resource: "mesh",
                count: resources.meshes.len(),
            }
        })?;
        let texture_count = u32::try_from(resources.textures.len()).map_err(|_| {
            RawC3dContractError::PreparedResourceCountOverflow {
                resource: "texture",
                count: resources.textures.len(),
            }
        })?;
        Ok(Self {
            layout: C3dPreparedSceneLayout {
                extent,
                mesh_count,
                texture_count,
            },
            resources,
            active_palette,
        })
    }

    #[must_use]
    pub const fn layout(self) -> C3dPreparedSceneLayout {
        self.layout
    }

    #[must_use]
    pub const fn resources(self) -> &'a C3dResourceSet {
        self.resources
    }

    #[must_use]
    pub const fn active_palette(self) -> &'a [PaletteEntry; INDEXED_PALETTE_LEN] {
        self.active_palette
    }
}

/// Per-frame matrices and draw ordinals for a persistent scene.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C3dPreparedFrame<'a> {
    layout: C3dPreparedSceneLayout,
    view: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
    projection: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
    draws: &'a [C3dDraw],
}

impl<'a> C3dPreparedFrame<'a> {
    pub fn new(
        layout: C3dPreparedSceneLayout,
        view: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
        projection: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
        draws: &'a [C3dDraw],
    ) -> Result<Self, RawC3dContractError> {
        validate_matrix("view", view)?;
        validate_matrix("projection", projection)?;
        validate_prepared_draws(layout, draws)?;
        Ok(Self {
            layout,
            view,
            projection,
            draws,
        })
    }

    #[must_use]
    pub const fn layout(self) -> C3dPreparedSceneLayout {
        self.layout
    }

    #[must_use]
    pub const fn view(self) -> cmr_core::root_menu_c3d_transform::C3dMatrixWords {
        self.view
    }

    #[must_use]
    pub const fn projection(self) -> cmr_core::root_menu_c3d_transform::C3dMatrixWords {
        self.projection
    }

    #[must_use]
    pub const fn draws(self) -> &'a [C3dDraw] {
        self.draws
    }

    /// Rejects a frame built for a different persistent target/resource shape.
    /// Candidate backends call this before touching native resources.
    pub fn ensure_layout(
        self,
        prepared: C3dPreparedSceneLayout,
    ) -> Result<(), RawC3dContractError> {
        if self.layout != prepared {
            return Err(RawC3dContractError::PreparedSceneLayoutMismatch {
                prepared,
                frame: self.layout,
            });
        }
        Ok(())
    }
}

fn validate_prepared_draws(
    layout: C3dPreparedSceneLayout,
    draws: &[C3dDraw],
) -> Result<(), RawC3dContractError> {
    for (draw_index, draw) in draws.iter().copied().enumerate() {
        if draw.mesh >= layout.mesh_count {
            return Err(RawC3dContractError::MeshResourceOutOfBounds {
                draw: draw_index,
                resource: draw.mesh,
                resource_count: layout.mesh_count as usize,
            });
        }
        if draw.texture >= layout.texture_count {
            return Err(RawC3dContractError::TextureResourceOutOfBounds {
                draw: draw_index,
                resource: draw.texture,
                resource_count: layout.texture_count as usize,
            });
        }
    }
    Ok(())
}

fn validate_matrix(
    name: &'static str,
    matrix: cmr_core::root_menu_c3d_transform::C3dMatrixWords,
) -> Result<(), RawC3dContractError> {
    for (component, word) in matrix.words().into_iter().enumerate() {
        if !f32::from_bits(word).is_finite() {
            return Err(RawC3dContractError::NonFiniteMatrixComponent { name, component });
        }
    }
    Ok(())
}

/// Validation failure at the raw recovered C3D GPU boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawC3dContractError {
    Frame(FrameContractError),
    InvalidTriangleIndexCount {
        actual: usize,
    },
    VertexIndexOutOfBounds {
        position: usize,
        index: u16,
        vertex_count: usize,
    },
    InvalidTextureIndexLength {
        expected: usize,
        actual: usize,
    },
    InvalidTexturePaletteLength {
        actual: usize,
    },
    TexturePaletteIndexOutOfBounds {
        position: usize,
        index: u8,
        palette_len: usize,
    },
    ColorKeyIndexOutOfBounds {
        index: u8,
        palette_len: usize,
    },
    NonFiniteMatrixComponent {
        name: &'static str,
        component: usize,
    },
    MeshResourceOutOfBounds {
        draw: usize,
        resource: u32,
        resource_count: usize,
    },
    TextureResourceOutOfBounds {
        draw: usize,
        resource: u32,
        resource_count: usize,
    },
    PreparedResourceCountOverflow {
        resource: &'static str,
        count: usize,
    },
    PreparedSceneLayoutMismatch {
        prepared: C3dPreparedSceneLayout,
        frame: C3dPreparedSceneLayout,
    },
    MeshRewriteOutOfBounds {
        resource: u32,
        resource_count: usize,
    },
    MeshRewriteVertexCount {
        expected: usize,
        actual: usize,
    },
}

impl From<FrameContractError> for RawC3dContractError {
    fn from(error: FrameContractError) -> Self {
        Self::Frame(error)
    }
}

impl fmt::Display for RawC3dContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "raw C3D GPU contract failed: {self:?}")
    }
}

impl std::error::Error for RawC3dContractError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Frame(error) => Some(error),
            _ => None,
        }
    }
}

/// Common raw-C3D operation implemented by both native GPU candidates.
pub trait RawC3dGpuBackend {
    type Error;

    fn info(&self) -> &BackendInfo;
    fn capabilities(&self) -> BackendCapabilities;
    fn render_c3d_readback(
        &mut self,
        frame: C3dRawFrame<'_>,
    ) -> Result<ConformanceFrame, Self::Error>;
}

/// Persistent raw-C3D candidate boundary used for steady-state comparison.
///
/// `prepare_c3d_scene` owns all one-time allocation, palette mapping, and
/// resource upload. Correctness readback is intentionally a separate operation
/// from timed rendering: `render_prepared_timed` returns no pixels and must not
/// encode a copy/map/readback. It must still wait for GPU completion so its
/// submit-to-completion interval is comparable and queued work stays bounded.
pub trait PersistentRawC3dGpuBackend {
    type Error;
    type PreparedScene;

    fn info(&self) -> &BackendInfo;
    fn capabilities(&self) -> BackendCapabilities;

    fn prepare_c3d_scene(
        &mut self,
        source: C3dSceneUpload<'_>,
    ) -> Result<Self::PreparedScene, Self::Error>;

    fn prepared_layout(scene: &Self::PreparedScene) -> C3dPreparedSceneLayout;

    fn render_prepared_readback(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<CanonicalRgba8Frame, Self::Error>;

    fn render_prepared_timed(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<FrameTiming, Self::Error>;
}

/// A tightly packed, row-major, 8-bit **active-palette index** frame read back
/// from a GPU target.
///
/// The indices are the active display-palette slots the candidate resolved, not
/// the source material texel values: pairing them with the caller's active
/// palette reconstructs exactly the RGBA image the expanded readback would
/// produce. This is the presentation form the indexed host already consumes, so
/// a GPU-rendered stage frame needs no palette inversion and no lossy
/// RGBA -> index conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalIndexedFrame {
    extent: DisplayExtent,
    indices: Box<[u8]>,
}

impl CanonicalIndexedFrame {
    /// Wraps exactly `width * height` index bytes.
    pub fn new(extent: DisplayExtent, indices: Box<[u8]>) -> Result<Self, FrameContractError> {
        let pixel_count = checked_pixel_count(extent)?;
        if indices.len() != pixel_count {
            return Err(FrameContractError::InvalidIndexLength {
                expected: pixel_count,
                actual: indices.len(),
            });
        }
        Ok(Self { extent, indices })
    }

    #[must_use]
    pub const fn extent(&self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub fn indices(&self) -> &[u8] {
        &self.indices
    }

    #[must_use]
    pub const fn row_pitch(&self) -> usize {
        self.extent.width as usize
    }
}

/// Presentation readback for a persistent raw-C3D scene.
///
/// Separate from [`PersistentRawC3dGpuBackend`] because it is not part of the
/// steady-state ranking contract: it exists so a host can present the GPU's
/// rasterized stage through the recovered indexed display path. A candidate
/// that cannot render into an 8-bit integer target (or cannot read it back)
/// simply does not implement it; correctness comparison keeps using the
/// expanded RGBA readback.
pub trait PersistentRawC3dIndexedBackend: PersistentRawC3dGpuBackend {
    fn render_prepared_indexed_readback(
        &mut self,
        scene: &mut Self::PreparedScene,
        frame: C3dPreparedFrame<'_>,
    ) -> Result<CanonicalIndexedFrame, Self::Error>;
}

/// Deterministic nearest-rank timing summary over non-empty nanosecond samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimingSummary {
    pub samples: usize,
    pub minimum_ns: u64,
    pub median_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub maximum_ns: u64,
}

impl TimingSummary {
    pub fn from_samples(samples: &[u64]) -> Result<Self, TimingSummaryError> {
        if samples.is_empty() {
            return Err(TimingSummaryError::Empty);
        }
        let mut ordered = samples.to_vec();
        ordered.sort_unstable();
        Ok(Self {
            samples: ordered.len(),
            minimum_ns: ordered[0],
            median_ns: nearest_rank(&ordered, 50),
            p95_ns: nearest_rank(&ordered, 95),
            p99_ns: nearest_rank(&ordered, 99),
            maximum_ns: ordered[ordered.len() - 1],
        })
    }
}

fn nearest_rank(ordered: &[u64], percentile: usize) -> u64 {
    debug_assert!(!ordered.is_empty());
    debug_assert!((1..=100).contains(&percentile));
    let rank = ordered.len().saturating_mul(percentile).div_ceil(100);
    ordered[rank.saturating_sub(1)]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimingSummaryError {
    Empty,
}

impl fmt::Display for TimingSummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "cannot summarize an empty timing sample set")
    }
}

impl std::error::Error for TimingSummaryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> [PaletteEntry; INDEXED_PALETTE_LEN] {
        std::array::from_fn(|index| PaletteEntry {
            red: index as u8,
            green: (255 - index) as u8,
            blue: (index as u8).wrapping_mul(3),
            flags: (index as u8).wrapping_mul(17),
        })
    }

    #[test]
    fn canonical_oracle_maps_zero_and_255_and_ignores_flags_as_alpha() {
        let palette = palette();
        let view = IndexedFrameView::new(
            DisplayExtent {
                width: 2,
                height: 1,
            },
            &[0, 255],
            &palette,
        )
        .unwrap();

        let rgba = view.canonical_rgba8().unwrap();
        assert_eq!(
            rgba.pixels(),
            &[
                palette[0].red,
                palette[0].green,
                palette[0].blue,
                255,
                palette[255].red,
                palette[255].green,
                palette[255].blue,
                255,
            ]
        );
    }

    #[test]
    fn palette_only_mutation_changes_only_pixels_using_that_entry() {
        let indices = [1, 2, 1];
        let extent = DisplayExtent {
            width: 3,
            height: 1,
        };
        let before_palette = palette();
        let mut after_palette = before_palette;
        after_palette[1] = PaletteEntry {
            red: 9,
            green: 8,
            blue: 7,
            flags: 0,
        };
        let before = IndexedFrameView::new(extent, &indices, &before_palette)
            .unwrap()
            .canonical_rgba8()
            .unwrap();
        let after = IndexedFrameView::new(extent, &indices, &after_palette)
            .unwrap()
            .canonical_rgba8()
            .unwrap();

        assert_ne!(&before.pixels()[0..4], &after.pixels()[0..4]);
        assert_eq!(&before.pixels()[4..8], &after.pixels()[4..8]);
        assert_ne!(&before.pixels()[8..12], &after.pixels()[8..12]);
        assert_eq!(&after.pixels()[0..4], &[9, 8, 7, 255]);
    }

    #[test]
    fn odd_width_padded_bgra_readback_normalizes_without_padding() {
        let extent = DisplayExtent {
            width: 3,
            height: 2,
        };
        let mapped = [
            3, 2, 1, 255, 6, 5, 4, 254, 9, 8, 7, 253, 0xaa, 0xaa, 0xaa, 0xaa, 12, 11, 10, 252, 15,
            14, 13, 251, 18, 17, 16, 250, 0xbb, 0xbb, 0xbb, 0xbb,
        ];

        let normalized = normalize_readback(extent, &mapped, 16, ReadbackFormat::Bgra8).unwrap();

        assert_eq!(normalized.row_pitch(), 12);
        assert_eq!(
            normalized.pixels(),
            &[
                1, 2, 3, 255, 4, 5, 6, 254, 7, 8, 9, 253, 10, 11, 12, 252, 13, 14, 15, 251, 16, 17,
                18, 250,
            ]
        );
    }

    #[test]
    fn rgba_readback_preserves_channels_and_alpha() {
        let extent = DisplayExtent {
            width: 1,
            height: 1,
        };
        let normalized =
            normalize_readback(extent, &[1, 2, 3, 4], 4, ReadbackFormat::Rgba8).unwrap();
        assert_eq!(normalized.pixels(), &[1, 2, 3, 4]);
    }

    #[test]
    fn canonical_indexed_frame_validates_length_and_matches_expanded_rgba() {
        let palette = palette();
        let extent = DisplayExtent {
            width: 3,
            height: 2,
        };
        let indices = Box::new([0u8, 1, 255, 7, 8, 9]);
        let frame = CanonicalIndexedFrame::new(extent, indices.clone()).unwrap();
        assert_eq!(frame.extent(), extent);
        assert_eq!(frame.indices(), &indices[..]);
        assert_eq!(frame.row_pitch(), 3);

        // Pairing the indices with the caller's palette reconstructs exactly the
        // canonical RGBA expansion a host would present.
        let view = IndexedFrameView::new(extent, frame.indices(), &palette).unwrap();
        let expanded = view.canonical_rgba8().unwrap();
        assert_eq!(expanded.extent(), extent);
        assert_eq!(
            expanded.pixels()[..4],
            [palette[0].red, palette[0].green, palette[0].blue, 255]
        );

        assert!(matches!(
            CanonicalIndexedFrame::new(extent, Box::new([0u8; 5])),
            Err(FrameContractError::InvalidIndexLength {
                expected: 6,
                actual: 5
            })
        ));
        assert_eq!(
            CanonicalIndexedFrame::new(
                DisplayExtent {
                    width: 0,
                    height: 0,
                },
                Box::new([]),
            ),
            Err(FrameContractError::InvalidExtent(DisplayExtent {
                width: 0,
                height: 0,
            }))
        );
    }

    #[test]
    fn frame_view_rejects_zero_extent_and_wrong_index_or_palette_lengths() {
        let palette = palette();
        assert_eq!(
            IndexedFrameView::new(
                DisplayExtent {
                    width: 0,
                    height: 1,
                },
                &[],
                &palette,
            ),
            Err(FrameContractError::InvalidExtent(DisplayExtent {
                width: 0,
                height: 1,
            }))
        );
        assert!(matches!(
            IndexedFrameView::new(
                DisplayExtent {
                    width: 2,
                    height: 2,
                },
                &[0; 3],
                &palette,
            ),
            Err(FrameContractError::InvalidIndexLength {
                expected: 4,
                actual: 3
            })
        ));
        assert!(matches!(
            IndexedFrameView::new(
                DisplayExtent {
                    width: 1,
                    height: 1,
                },
                &[0],
                &palette[..255],
            ),
            Err(FrameContractError::InvalidPaletteLength {
                expected: 256,
                actual: 255
            })
        ));
    }

    #[test]
    fn readback_rejects_narrow_pitch_and_nonexact_storage() {
        let extent = DisplayExtent {
            width: 3,
            height: 2,
        };
        assert_eq!(
            normalize_readback(extent, &[0; 24], 11, ReadbackFormat::Rgba8),
            Err(FrameContractError::ReadbackPitchTooSmall {
                minimum: 12,
                actual: 11,
            })
        );
        assert_eq!(
            normalize_readback(extent, &[0; 31], 16, ReadbackFormat::Rgba8),
            Err(FrameContractError::InvalidReadbackLength {
                expected: 32,
                actual: 31,
            })
        );
        assert_eq!(
            normalize_readback(extent, &[0; 33], 16, ReadbackFormat::Rgba8),
            Err(FrameContractError::InvalidReadbackLength {
                expected: 32,
                actual: 33,
            })
        );
    }

    #[test]
    fn timing_summary_uses_deterministic_nearest_rank_percentiles() {
        let samples: Vec<u64> = (1..=100).rev().collect();
        let summary = TimingSummary::from_samples(&samples).unwrap();
        assert_eq!(
            summary,
            TimingSummary {
                samples: 100,
                minimum_ns: 1,
                median_ns: 50,
                p95_ns: 95,
                p99_ns: 99,
                maximum_ns: 100,
            }
        );
        assert_eq!(
            TimingSummary::from_samples(&[]),
            Err(TimingSummaryError::Empty)
        );
    }

    #[test]
    fn single_timing_sample_populates_every_rank() {
        let summary = TimingSummary::from_samples(&[42]).unwrap();
        assert_eq!(summary.minimum_ns, 42);
        assert_eq!(summary.median_ns, 42);
        assert_eq!(summary.p95_ns, 42);
        assert_eq!(summary.p99_ns, 42);
        assert_eq!(summary.maximum_ns, 42);
    }

    fn backend_info(vendor_id: Option<u32>, device_id: Option<u32>) -> BackendInfo {
        BackendInfo {
            backend: BackendKind::Wgpu,
            graphics_api: GraphicsApi::Vulkan,
            vendor_id,
            device_id,
            device_kind: DeviceKind::Discrete,
            adapter_name: "synthetic adapter".into(),
            driver_name: "synthetic driver".into(),
            driver_info: "test-only".into(),
        }
    }

    #[test]
    fn vendor_device_pair_match_requires_equal_present_ids() {
        let device = backend_info(Some(0x1002), Some(0x73bf));
        let same_via_other_backend = BackendInfo {
            backend: BackendKind::SdlGpu,
            ..device.clone()
        };
        assert!(device.same_vendor_device_pair(&same_via_other_backend));
        assert!(!device.same_vendor_device_pair(&backend_info(Some(0x10de), Some(0x73bf))));
        assert!(!device.same_vendor_device_pair(&backend_info(Some(0x1002), Some(0x1234))));
        assert!(!device.same_vendor_device_pair(&backend_info(None, Some(0x73bf))));
        assert!(!device.same_vendor_device_pair(&backend_info(Some(0x1002), None)));
    }

    #[test]
    fn timing_keeps_required_wall_intervals_separate_from_optional_gpu_timestamp() {
        let timing = FrameTiming {
            cpu_encode_submit_ns: 11,
            submit_to_completion_ns: 22,
            gpu_timestamp_ns: Some(17),
        };
        assert_eq!(timing.cpu_encode_submit_ns, 11);
        assert_eq!(timing.submit_to_completion_ns, 22);
        assert_eq!(timing.gpu_timestamp_ns, Some(17));
    }

    fn raw_vertex(seed: i32) -> C3dStageVertex {
        C3dStageVertex::from_raw_components(std::array::from_fn(|index| seed + index as i32))
    }

    fn raw_resources() -> C3dResourceSet {
        let mesh = C3dMeshResource::new(
            vec![raw_vertex(10), raw_vertex(20), raw_vertex(30)],
            vec![0, 1, 2],
        )
        .unwrap();
        let texture = C3dIndexedTexture::new(
            DisplayExtent {
                width: 2,
                height: 1,
            },
            vec![0, 1],
            vec![
                C3dRgb8 {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
                C3dRgb8 {
                    red: 4,
                    green: 5,
                    blue: 6,
                },
            ],
            Some(0),
        )
        .unwrap();
        C3dResourceSet::new(vec![mesh], vec![texture])
    }

    #[test]
    fn a_mesh_rewrite_keeps_its_topology_and_rejects_length_changes() {
        let vertices = [raw_vertex(1), raw_vertex(2), raw_vertex(3)];
        let mesh = C3dMeshResource::new(vertices.to_vec(), vec![0, 1, 2]).unwrap();
        let mut set = C3dResourceSet::new(vec![mesh], Vec::new());
        let moved = [raw_vertex(4), raw_vertex(5), raw_vertex(6)];
        set.replace_mesh_vertices(0, &moved).unwrap();
        assert_eq!(set.meshes()[0].vertices(), &moved);
        assert_eq!(set.meshes()[0].indices(), &[0, 1, 2]);
        assert_eq!(
            set.replace_mesh_vertices(0, &moved[..2]),
            Err(RawC3dContractError::MeshRewriteVertexCount {
                expected: 3,
                actual: 2
            })
        );
        assert_eq!(
            set.replace_mesh_vertices(1, &moved),
            Err(RawC3dContractError::MeshRewriteOutOfBounds {
                resource: 1,
                resource_count: 1
            })
        );
        assert_eq!(
            set.meshes()[0].vertices(),
            &moved,
            "a refusal changes nothing"
        );
    }

    #[test]
    fn raw_mesh_preserves_stage_vertex_carriers_and_validates_triangle_indices() {
        let vertices = [
            raw_vertex(i32::MIN),
            raw_vertex(7),
            raw_vertex(i32::MAX - 7),
        ];
        let mesh = C3dMeshResource::new(vertices.to_vec(), vec![2, 0, 1]).unwrap();
        assert_eq!(mesh.vertices(), &vertices);
        assert_eq!(mesh.indices(), &[2, 0, 1]);

        assert_eq!(
            C3dMeshResource::new(vertices.to_vec(), Vec::<u16>::new()),
            Err(RawC3dContractError::InvalidTriangleIndexCount { actual: 0 })
        );
        assert_eq!(
            C3dMeshResource::new(vertices.to_vec(), vec![0, 1]),
            Err(RawC3dContractError::InvalidTriangleIndexCount { actual: 2 })
        );
        assert_eq!(
            C3dMeshResource::new(vertices.to_vec(), vec![0, 3, 1]),
            Err(RawC3dContractError::VertexIndexOutOfBounds {
                position: 1,
                index: 3,
                vertex_count: 3,
            })
        );
    }

    #[test]
    fn stage_vertex_carrier_uses_the_shared_recovered_uv_contract() {
        let raw = [11, 12, 13, 0x1_0001, -0x8000, 15, 16, 17];
        let vertex = C3dStageVertex::from_raw_components(raw);
        assert_eq!(vertex.position_q16(), [11, 12, 13]);
        assert_eq!(vertex.normal_q16(), [15, 16, 17]);
        assert_eq!(
            vertex.texture_uv(),
            cmr_core::c3d_render::stage_texture_uv([raw[3], raw[4]])
        );
        assert_eq!(
            &vertex.upload_words()[3..5],
            &vertex.texture_uv().map(f32::to_bits)
        );
    }

    #[test]
    fn raw_texture_validates_shape_palette_and_every_index() {
        let extent = DisplayExtent {
            width: 2,
            height: 1,
        };
        let rgb = C3dRgb8 {
            red: 1,
            green: 2,
            blue: 3,
        };
        assert!(C3dIndexedTexture::new(extent, vec![0, 0], vec![rgb], Some(0)).is_ok());
        assert_eq!(
            C3dIndexedTexture::new(extent, vec![0], vec![rgb], None),
            Err(RawC3dContractError::InvalidTextureIndexLength {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            C3dIndexedTexture::new(extent, vec![0, 0], Vec::<C3dRgb8>::new(), None),
            Err(RawC3dContractError::InvalidTexturePaletteLength { actual: 0 })
        );
        assert_eq!(
            C3dIndexedTexture::new(extent, vec![0, 1], vec![rgb], None),
            Err(RawC3dContractError::TexturePaletteIndexOutOfBounds {
                position: 1,
                index: 1,
                palette_len: 1,
            })
        );
        assert_eq!(
            C3dIndexedTexture::new(extent, vec![0, 0], vec![rgb], Some(1)),
            Err(RawC3dContractError::ColorKeyIndexOutOfBounds {
                index: 1,
                palette_len: 1,
            })
        );
        assert_eq!(
            C3dIndexedTexture::new(
                DisplayExtent {
                    width: 0,
                    height: 1,
                },
                Vec::<u8>::new(),
                vec![rgb],
                None,
            ),
            Err(RawC3dContractError::Frame(
                FrameContractError::InvalidExtent(DisplayExtent {
                    width: 0,
                    height: 1,
                })
            ))
        );
    }

    #[test]
    fn raw_texture_maps_into_the_active_palette_by_l1_with_lowest_tie() {
        let texture = C3dIndexedTexture::new(
            DisplayExtent {
                width: 2,
                height: 1,
            },
            vec![0, 1],
            vec![
                C3dRgb8 {
                    red: 10,
                    green: 10,
                    blue: 10,
                },
                C3dRgb8 {
                    red: 20,
                    green: 20,
                    blue: 20,
                },
            ],
            None,
        )
        .unwrap();
        let mut active = [PaletteEntry {
            red: 255,
            green: 255,
            blue: 255,
            flags: 0,
        }; INDEXED_PALETTE_LEN];
        active[3] = PaletteEntry {
            red: 9,
            green: 10,
            blue: 10,
            flags: 255,
        };
        active[4] = PaletteEntry {
            red: 11,
            green: 10,
            blue: 10,
            flags: 0,
        };
        active[7] = PaletteEntry {
            red: 20,
            green: 20,
            blue: 20,
            flags: 99,
        };

        assert_eq!(&*c3d_active_palette_l1_map(&texture, &active), &[3, 7]);
    }

    #[test]
    fn opaque_copy_profile_keeps_submission_bracket_separate_and_depth_writable() {
        let resources = raw_resources();
        let world = cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_VIEW;
        let draw = C3dDraw::new(0, 0, world, C3dSubmissionBracket::DeferredTranslucent).unwrap();
        assert_eq!(draw.submission(), C3dSubmissionBracket::DeferredTranslucent);
        assert_eq!(draw.state().blend(), C3dBlendMode::OpaqueCopy);
        assert_eq!(draw.state().depth_compare(), C3dDepthCompare::LessEqual);
        assert!(draw.state().depth_write());
        assert_eq!(draw.state().cull(), C3dCullMode::CullCounterClockwise);
        assert_eq!(draw.state().filter(), C3dFilter::PointClamp);
        assert!(draw.state().perspective_correct());

        let active_palette = palette();
        let frame = C3dRawFrame::new(
            DisplayExtent {
                width: 4,
                height: 3,
            },
            world,
            cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_PROJECTION,
            &active_palette,
            &resources,
            std::slice::from_ref(&draw),
        )
        .unwrap();
        assert_eq!(frame.draws(), &[draw]);
        assert_eq!(frame.resources(), &resources);
        assert_eq!(frame.active_palette(), &active_palette);
    }

    #[test]
    fn raw_frame_rejects_bad_resource_ids_and_nonfinite_matrices() {
        let resources = raw_resources();
        let active_palette = palette();
        let identity = cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_VIEW;
        let bad_mesh = C3dDraw::new(1, 0, identity, C3dSubmissionBracket::Normal).unwrap();
        assert!(matches!(
            C3dRawFrame::new(
                DisplayExtent {
                    width: 1,
                    height: 1,
                },
                identity,
                cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_PROJECTION,
                &active_palette,
                &resources,
                std::slice::from_ref(&bad_mesh),
            ),
            Err(RawC3dContractError::MeshResourceOutOfBounds {
                draw: 0,
                resource: 1,
                resource_count: 1,
            })
        ));

        let bad_texture = C3dDraw::new(0, 1, identity, C3dSubmissionBracket::Normal).unwrap();
        assert!(matches!(
            C3dRawFrame::new(
                DisplayExtent {
                    width: 1,
                    height: 1,
                },
                identity,
                cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_PROJECTION,
                &active_palette,
                &resources,
                std::slice::from_ref(&bad_texture),
            ),
            Err(RawC3dContractError::TextureResourceOutOfBounds {
                draw: 0,
                resource: 1,
                resource_count: 1,
            })
        ));

        let nonfinite = cmr_core::c3d_camera::projection_matrix_words(f32::NAN, 2.4, 1.0, 250.0);
        assert_eq!(
            C3dRawFrame::new(
                DisplayExtent {
                    width: 1,
                    height: 1,
                },
                nonfinite,
                cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_PROJECTION,
                &active_palette,
                &resources,
                &[],
            ),
            Err(RawC3dContractError::NonFiniteMatrixComponent {
                name: "view",
                component: 0,
            })
        );
    }

    #[test]
    fn persistent_upload_locks_extent_resource_counts_and_active_palette() {
        let resources = raw_resources();
        let active_palette = palette();
        let extent = DisplayExtent {
            width: 320,
            height: 200,
        };
        let upload = C3dSceneUpload::new(extent, &resources, &active_palette).unwrap();

        assert_eq!(upload.layout().extent(), extent);
        assert_eq!(upload.layout().mesh_count(), 1);
        assert_eq!(upload.layout().texture_count(), 1);
        assert_eq!(upload.resources(), &resources);
        assert_eq!(upload.active_palette(), &active_palette);
        assert!(matches!(
            C3dSceneUpload::new(
                DisplayExtent {
                    width: 0,
                    height: 200,
                },
                &resources,
                &active_palette,
            ),
            Err(RawC3dContractError::Frame(
                FrameContractError::InvalidExtent(_)
            ))
        ));
    }

    #[test]
    fn persistent_frame_validates_matrices_draw_ids_and_uploaded_layout() {
        let resources = raw_resources();
        let active_palette = palette();
        let extent = DisplayExtent {
            width: 320,
            height: 200,
        };
        let upload = C3dSceneUpload::new(extent, &resources, &active_palette).unwrap();
        let identity = cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_VIEW;
        let projection = cmr_core::root_menu_c3d_transform::ROOT_MENU_C3D_PROJECTION;
        let draw = C3dDraw::new(0, 0, identity, C3dSubmissionBracket::Normal).unwrap();
        let frame = C3dPreparedFrame::new(
            upload.layout(),
            identity,
            projection,
            std::slice::from_ref(&draw),
        )
        .unwrap();

        assert_eq!(frame.layout(), upload.layout());
        assert_eq!(frame.view(), identity);
        assert_eq!(frame.projection(), projection);
        assert_eq!(frame.draws(), &[draw]);
        frame.ensure_layout(upload.layout()).unwrap();

        let bad_mesh = C3dDraw::new(1, 0, identity, C3dSubmissionBracket::Normal).unwrap();
        assert!(matches!(
            C3dPreparedFrame::new(
                upload.layout(),
                identity,
                projection,
                std::slice::from_ref(&bad_mesh),
            ),
            Err(RawC3dContractError::MeshResourceOutOfBounds {
                draw: 0,
                resource: 1,
                resource_count: 1,
            })
        ));

        let bad_texture = C3dDraw::new(0, 1, identity, C3dSubmissionBracket::Normal).unwrap();
        assert!(matches!(
            C3dPreparedFrame::new(
                upload.layout(),
                identity,
                projection,
                std::slice::from_ref(&bad_texture),
            ),
            Err(RawC3dContractError::TextureResourceOutOfBounds {
                draw: 0,
                resource: 1,
                resource_count: 1,
            })
        ));

        let nonfinite =
            cmr_core::c3d_camera::projection_matrix_words(f32::INFINITY, 2.4, 1.0, 250.0);
        assert_eq!(
            C3dPreparedFrame::new(upload.layout(), identity, nonfinite, &[]),
            Err(RawC3dContractError::NonFiniteMatrixComponent {
                name: "projection",
                component: 0,
            })
        );

        let other_upload = C3dSceneUpload::new(
            DisplayExtent {
                width: 640,
                height: 480,
            },
            &resources,
            &active_palette,
        )
        .unwrap();
        assert_eq!(
            frame.ensure_layout(other_upload.layout()),
            Err(RawC3dContractError::PreparedSceneLayoutMismatch {
                prepared: other_upload.layout(),
                frame: upload.layout(),
            })
        );
    }
}
