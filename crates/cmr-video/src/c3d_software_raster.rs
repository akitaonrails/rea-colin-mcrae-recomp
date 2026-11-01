//! Deterministic transform and clipping preflight for the software C3D path.
//!
//! The shipped renderer establishes row-vector world/view/projection transforms,
//! the `D3DVIEWPORT2` clip volume, and ordinary homogeneous clipping. It does not
//! establish one driver-exact floating-point evaluation order or equality rule.
//! This source-owned oracle therefore uses sequential `f32` arithmetic and closed
//! clip planes. Those two choices are reconstruction policy, not claims about a
//! particular 1990s Direct3D driver.

use std::fmt;

use cmr_core::{
    c3d_batch::C3dDrawCommand,
    c3d_type2_fog::C3dType2PreDrawVertex,
    root_menu_c3d_frame_plan::RootMenuC3dDrawCommand,
    root_menu_c3d_resources::C3dVertexMutationBranch,
    root_menu_c3d_runtime::{
        C3dMode2ResourceScaledVertex, C3dType1ResourceScaledVertex, C3dType2ResourceScaledVertex,
    },
    root_menu_c3d_transform::{C3dMatrixWords, C3dViewportWords, RootMenuC3dViewportError},
};
use cmr_formats::{bitmap::BitmapPaletteEntry, c3d::C3dEntry32, riff_palette::PaletteEntry};

use crate::{
    c3d_software::SoftwareC3dSampledTexture, packed_rgb_target::LogicalMaskedRgbTarget,
    selected_texture_conversion_oracle::SourceOwnedLogicalSelectedTexture,
};

const CLIP_PLANE_COUNT: usize = 6;

/// A lit C3D vertex after world/view/projection and clip-volume normalization.
///
/// Diffuse and specular channels are stored in `[alpha, red, green, blue]`
/// order, normalized to `0.0..=1.0`. The reserved D3DLVERTEX word is
/// deliberately ignored; the recovered legacy ABI identifies it as reserved,
/// not RHW.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareC3dClipVertex {
    clip_position: [f32; 4],
    diffuse_argb: [f32; 4],
    specular_argb: [f32; 4],
    uv: [f32; 2],
}

impl SoftwareC3dClipVertex {
    #[must_use]
    pub const fn clip_position(self) -> [f32; 4] {
        self.clip_position
    }

    #[must_use]
    pub const fn diffuse_argb(self) -> [f32; 4] {
        self.diffuse_argb
    }

    #[must_use]
    pub const fn specular_argb(self) -> [f32; 4] {
        self.specular_argb
    }

    #[must_use]
    pub const fn uv(self) -> [f32; 2] {
        self.uv
    }

    fn is_finite(self) -> bool {
        self.clip_position
            .into_iter()
            .chain(self.diffuse_argb)
            .chain(self.specular_argb)
            .chain(self.uv)
            .all(f32::is_finite)
    }

    fn interpolate(self, other: Self, amount: f32) -> Self {
        Self {
            clip_position: interpolate_array(self.clip_position, other.clip_position, amount),
            diffuse_argb: interpolate_array(self.diffuse_argb, other.diffuse_argb, amount),
            specular_argb: interpolate_array(self.specular_argb, other.specular_argb, amount),
            uv: interpolate_array(self.uv, other.uv, amount),
        }
    }
}

/// A clipped vertex projected into the target's pixel/depth coordinates.
///
/// `uv_over_w` and `reciprocal_w` are retained separately so the later
/// rasterizer can perform the recovered perspective-correct UV interpolation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareC3dScreenVertex {
    position: [f32; 3],
    reciprocal_w: f32,
    uv_over_w: [f32; 2],
    diffuse_argb: [f32; 4],
    specular_argb: [f32; 4],
}

impl SoftwareC3dScreenVertex {
    #[must_use]
    pub const fn position(self) -> [f32; 3] {
        self.position
    }

    #[must_use]
    pub const fn reciprocal_w(self) -> f32 {
        self.reciprocal_w
    }

    #[must_use]
    pub const fn uv_over_w(self) -> [f32; 2] {
        self.uv_over_w
    }

    #[must_use]
    pub const fn diffuse_argb(self) -> [f32; 4] {
        self.diffuse_argb
    }

    #[must_use]
    pub const fn specular_argb(self) -> [f32; 4] {
        self.specular_argb
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareC3dRasterError {
    UnsupportedChampionshipCarProfile(SoftwareChampionshipCarRasterProfile),
    UnsupportedVertexLayout {
        vertex_type: u32,
    },
    IncompleteIndexTriplet {
        index_count: usize,
    },
    IndexOutOfRange {
        triangle: usize,
        corner: usize,
        index: u16,
        vertex_count: usize,
    },
    NonFiniteInput,
    InvalidViewport,
    NonFiniteResult,
    NonPositiveHomogeneousW,
    InvalidDepthExtent([u16; 2]),
    InvalidTextureExtent([u16; 2]),
    InvalidTextureIndexLength {
        expected: usize,
        actual: usize,
    },
    TexturePaletteIndexOutOfRange {
        index: u8,
        palette_len: usize,
    },
    InvalidPreparedTriangle {
        triangle: usize,
    },
    PreparedVertexCountMismatch {
        command: usize,
        prepared: usize,
    },
    NonBlackAlternateDefaultDiffuse {
        triangle: usize,
        vertex: usize,
    },
}

/// Orthogonal runtime state controlling championship-car raster behavior.
///
/// The caller must populate every field from recovered runtime state; frontend
/// entry alone does not prove that fog mutation or alpha blending is disabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftwareChampionshipCarRasterProfile {
    pub unified_indexed: bool,
    pub texture_copy: bool,
    pub point_filter: bool,
    pub alpha_blend: bool,
    pub fog_mutation: bool,
    pub actor_rgb_override: bool,
}

impl SoftwareChampionshipCarRasterProfile {
    /// The default mutation-disabled deterministic software subset.
    pub const MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND: Self = Self {
        unified_indexed: true,
        texture_copy: true,
        point_filter: true,
        alpha_blend: false,
        fog_mutation: false,
        actor_rgb_override: false,
    };

    /// The same deterministic COPY subset with the recovered CPU-side Type-2
    /// word-5 mutation enabled. COPY intentionally does not consume word 5.
    pub const MUTATION_ENABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND: Self = Self {
        fog_mutation: true,
        ..Self::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND
    };

    /// The same deterministic COPY subset with actor-RGB word-4 mutation.
    pub const MUTATION_DISABLED_ACTOR_RGB_ENABLED_UNIFIED_INDEXED_COPY_NO_BLEND: Self = Self {
        actor_rgb_override: true,
        ..Self::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND
    };

    /// The deterministic COPY subset with both independent CPU-side mutations.
    pub const MUTATION_ENABLED_ACTOR_RGB_ENABLED_UNIFIED_INDEXED_COPY_NO_BLEND: Self = Self {
        fog_mutation: true,
        actor_rgb_override: true,
        ..Self::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND
    };
}

/// Nonzero renderer `+0x28` value selecting an alternate render-depth branch.
/// Exact native pixel masks and packing remain device-selected and unresolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SoftwareC3dSplitRenderDepth {
    Bits16 = 0x10,
    Bits24 = 0x18,
    Bits32 = 0x20,
}

/// Renderer branch selected from legacy device-record field `+0xa0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareC3dRendererBranchSelection {
    Indexed,
    Split(SoftwareC3dSplitRenderDepth),
}

/// Decodes the observed priority order `0x400`, `0x200`, `0x100`, then zero.
#[must_use]
pub const fn decode_software_c3d_renderer_branch(
    device_record_a0: u32,
) -> SoftwareC3dRendererBranchSelection {
    if device_record_a0 & 0x400 != 0 {
        SoftwareC3dRendererBranchSelection::Split(SoftwareC3dSplitRenderDepth::Bits16)
    } else if device_record_a0 & 0x200 != 0 {
        SoftwareC3dRendererBranchSelection::Split(SoftwareC3dSplitRenderDepth::Bits24)
    } else if device_record_a0 & 0x100 != 0 {
        SoftwareC3dRendererBranchSelection::Split(SoftwareC3dSplitRenderDepth::Bits32)
    } else {
        SoftwareC3dRendererBranchSelection::Indexed
    }
}

/// Numeric filter request retained by the alternate device branch.
///
/// Value 2 is observed as the request. Whether it becomes the effective device
/// filter depends on cache and capability state and is not implied here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SoftwareC3dAlternateFilterRequest {
    Value2 = 2,
}

/// Source-owned filtering policy used by the canonical RGB oracle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareC3dOracleFilter {
    BilinearClampedTexelCenter,
}

/// Candidate no-alpha alternate policy plus retained device-branch provenance.
///
/// `render_depth_provenance` does not select output packing. The compositor
/// emits canonical unpacked RGB before unresolved device-format conversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftwareChampionshipCarSplitOracleProfile {
    pub raster: SoftwareChampionshipCarRasterProfile,
    pub render_depth_provenance: SoftwareC3dSplitRenderDepth,
    pub requested_filter: SoftwareC3dAlternateFilterRequest,
    pub oracle_filter: SoftwareC3dOracleFilter,
}

impl SoftwareChampionshipCarSplitOracleProfile {
    #[must_use]
    pub const fn new(
        render_depth: SoftwareC3dSplitRenderDepth,
        fog_mutation: bool,
        actor_rgb_override: bool,
    ) -> Self {
        Self {
            raster: SoftwareChampionshipCarRasterProfile {
                unified_indexed: false,
                texture_copy: false,
                point_filter: false,
                alpha_blend: false,
                fog_mutation,
                actor_rgb_override,
            },
            render_depth_provenance: render_depth,
            requested_filter: SoftwareC3dAlternateFilterRequest::Value2,
            oracle_filter: SoftwareC3dOracleFilter::BilinearClampedTexelCenter,
        }
    }
}

/// Index source selected by the championship binding operation.
#[derive(Clone, Copy, Debug)]
pub enum SoftwareChampionshipCarIndexedSource<'a> {
    UnifiedTexture(&'a SoftwareC3dSampledTexture),
    /// The reviewed car corpus maps the recovered opaque-black default to
    /// literal display index zero. This is deliberately not resource zero and
    /// does not validate an arbitrary current or pending palette.
    ReviewedCarPaletteMissingSelectorOpaqueBlackIndexZero,
}

/// Texture source for the bounded alternate championship compositor.
#[derive(Clone, Copy, Debug)]
pub enum SoftwareChampionshipCarSplitSource<'a> {
    /// A source-owned selected-format plane. The nominal type forces explicit
    /// conversion before this compositor can apply its oracle filter; legacy
    /// driver `Load` behavior remains unresolved.
    ///
    /// ```compile_fail
    /// use cmr_video::c3d_software::SoftwareC3dSampledTexture;
    /// use cmr_video::c3d_software_raster::SoftwareChampionshipCarSplitSource;
    /// fn bypass_conversion(source: &SoftwareC3dSampledTexture) {
    ///     let _ = SoftwareChampionshipCarSplitSource::ConvertedTexture(source);
    /// }
    /// ```
    ConvertedTexture(&'a SourceOwnedLogicalSelectedTexture),
    /// The recovered missing-selector path, accepted only when every prepared
    /// vertex has black diffuse RGB. MODULATE is then black independently of
    /// the unresolved texture-zero sample.
    BlackDiffuseDefault,
}

impl fmt::Display for SoftwareC3dRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "software C3D raster operation failed: {self:?}")
    }
}

impl std::error::Error for SoftwareC3dRasterError {}

/// Named source-owned RGB-to-index policy for the first software oracle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareC3dIndexedQuantization {
    /// Compare all 256 active-palette RGB entries by L1 distance, retain the
    /// lowest index on a tie, and apply no dithering.
    NearestRgbL1LowestIndexNoDither,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoftwareC3dComposeError {
    InvalidTargetExtent([u16; 2]),
    InvalidTargetLength { expected: usize, actual: usize },
    DepthTargetExtentMismatch { depth: [u16; 2], target: [u16; 2] },
    Raster(SoftwareC3dRasterError),
}

impl fmt::Display for SoftwareC3dComposeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "software C3D composition failed: {self:?}")
    }
}

impl std::error::Error for SoftwareC3dComposeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Raster(source) => Some(source),
            _ => None,
        }
    }
}

/// Row-major source-owned depth storage for the software compatibility path.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftwareC3dDepthBuffer {
    extent: [u16; 2],
    values: Box<[f32]>,
}

impl SoftwareC3dDepthBuffer {
    /// Source-owned initialization uses far value `1.0`, matching the value
    /// established by the recovered depth-only clear.
    pub fn new_far(extent: [u16; 2]) -> Result<Self, SoftwareC3dRasterError> {
        if extent[0] == 0 || extent[1] == 0 {
            return Err(SoftwareC3dRasterError::InvalidDepthExtent(extent));
        }
        let pixel_count = usize::from(extent[0]) * usize::from(extent[1]);
        Ok(Self {
            extent,
            values: vec![1.0; pixel_count].into_boxed_slice(),
        })
    }

    #[must_use]
    pub const fn extent(&self) -> [u16; 2] {
        self.extent
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub(crate) fn values_mut(&mut self) -> &mut [f32] {
        &mut self.values
    }

    /// A depth buffer from another renderer's values (row-major, `[0, 1]`),
    /// for passes drawn over its frame.
    pub fn from_values(extent: [u16; 2], values: Vec<f32>) -> Result<Self, SoftwareC3dRasterError> {
        if extent[0] == 0
            || extent[1] == 0
            || values.len() != usize::from(extent[0]) * usize::from(extent[1])
        {
            return Err(SoftwareC3dRasterError::InvalidDepthExtent(extent));
        }
        Ok(Self {
            extent,
            values: values.into_boxed_slice(),
        })
    }

    /// Performs the recovered depth-only clear without touching color.
    pub fn clear_far(&mut self) {
        self.values.fill(1.0);
    }

    /// Updates one depth value at `index` (`y * width + x`).
    pub fn set_value(&mut self, index: usize, value: f32) {
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value;
        }
    }

    #[cfg(test)]
    pub(crate) fn from_test_values(extent: [u16; 2], values: &[f32]) -> Self {
        assert_eq!(
            values.len(),
            usize::from(extent[0]) * usize::from(extent[1])
        );
        Self {
            extent,
            values: values.to_vec().into_boxed_slice(),
        }
    }
}

/// One depth-passing, point-sampled software C3D fragment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareC3dFragment {
    position: [u16; 2],
    depth: f32,
    uv: [f32; 2],
    texture_index: u8,
    diffuse_argb: [f32; 4],
    flat_diffuse: u32,
}

impl SoftwareC3dFragment {
    #[must_use]
    pub const fn position(self) -> [u16; 2] {
        self.position
    }

    #[must_use]
    pub const fn depth(self) -> f32 {
        self.depth
    }

    #[must_use]
    pub const fn uv(self) -> [f32; 2] {
        self.uv
    }

    #[must_use]
    pub const fn texture_index(self) -> u8 {
        self.texture_index
    }

    /// Screen-affine lit color for the later Gouraud/modulate profile.
    #[must_use]
    pub const fn diffuse_argb(self) -> [f32; 4] {
        self.diffuse_argb
    }

    /// Original first-index color for the later flat/copy profile.
    #[must_use]
    pub const fn flat_diffuse(self) -> u32 {
        self.flat_diffuse
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoftwareC3dFragmentOutcome {
    covered: usize,
    depth_passed: usize,
    depth_rejected: usize,
}

impl SoftwareC3dFragmentOutcome {
    #[must_use]
    pub const fn covered(self) -> usize {
        self.covered
    }

    #[must_use]
    pub const fn depth_passed(self) -> usize {
        self.depth_passed
    }

    #[must_use]
    pub const fn depth_rejected(self) -> usize {
        self.depth_rejected
    }
}

/// Immutable transform state consumed by one software batch preflight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoftwareC3dTransformState {
    world: C3dMatrixWords,
    view: C3dMatrixWords,
    projection: C3dMatrixWords,
    viewport: C3dViewportWords,
}

impl SoftwareC3dTransformState {
    #[must_use]
    pub const fn new(
        world: C3dMatrixWords,
        view: C3dMatrixWords,
        projection: C3dMatrixWords,
        viewport: C3dViewportWords,
    ) -> Self {
        Self {
            world,
            view,
            projection,
            viewport,
        }
    }

    /// Builds the transform state for a gameplay scene from the recovered E05
    /// matrices (`cmr_core::c3d_camera`): the camera VIEW is the affine inverse
    /// of `camera_world_q16`, the node WORLD is `node_world_q16`, the PROJECTION
    /// is the baseline left-handed perspective, and the viewport is the
    /// scene-independent D3DVIEWPORT2 for `width`x`height`. This is the public
    /// entry that drives the software oracle (via `preflight_root_c3d_batch`)
    /// with gameplay camera parameters instead of the fixed menu matrices.
    pub fn for_gameplay_scene(
        width: u16,
        height: u16,
        camera_world_q16: &[i32; 16],
        node_world_q16: &[i32; 16],
    ) -> Result<Self, RootMenuC3dViewportError> {
        let (w, h, zn, zf) = cmr_core::c3d_camera::gameplay_projection_default(width, height);
        Self::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            node_world_q16,
            (w, h, zn, zf),
        )
    }

    /// Gameplay transform builder with a caller-selected recovered projection.
    /// The horizon pass uses the shipped 25000-unit far-plane override while
    /// ordinary world geometry uses [`Self::for_gameplay_scene`].
    pub fn for_gameplay_scene_with_projection(
        width: u16,
        height: u16,
        camera_world_q16: &[i32; 16],
        node_world_q16: &[i32; 16],
        projection: (f32, f32, f32, f32),
    ) -> Result<Self, RootMenuC3dViewportError> {
        let (w, h, zn, zf) = projection;
        Ok(Self::new(
            cmr_core::c3d_camera::world_matrix_words(node_world_q16),
            cmr_core::c3d_camera::view_matrix_words(camera_world_q16),
            cmr_core::c3d_camera::projection_matrix_words(w, h, zn, zf),
            cmr_core::c3d_camera::viewport_words(width, height)?,
        ))
    }

    #[must_use]
    pub const fn world(&self) -> C3dMatrixWords {
        self.world
    }

    #[must_use]
    pub const fn view(&self) -> C3dMatrixWords {
        self.view
    }

    #[must_use]
    pub const fn projection(&self) -> C3dMatrixWords {
        self.projection
    }

    #[must_use]
    pub const fn viewport(&self) -> C3dViewportWords {
        self.viewport
    }
}

/// One clockwise, non-degenerate screen triangle ready for later coverage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareC3dPreparedTriangle {
    vertices: [SoftwareC3dScreenVertex; 3],
    flat_diffuse: u32,
}

impl SoftwareC3dPreparedTriangle {
    #[must_use]
    pub const fn vertices(&self) -> &[SoftwareC3dScreenVertex; 3] {
        &self.vertices
    }

    /// Diffuse value from the original triangle's first submitted index.
    #[must_use]
    pub const fn flat_diffuse(&self) -> u32 {
        self.flat_diffuse
    }
}

/// Deterministic result of validating, transforming, clipping, and culling a
/// source batch. Omission counters make every successful discard observable.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftwareC3dPreparedBatch {
    triangles: Box<[SoftwareC3dPreparedTriangle]>,
    source_triangle_count: usize,
    fully_clipped_source_triangle_count: usize,
    counter_clockwise_triangle_count: usize,
    degenerate_triangle_count: usize,
}

/// Geometry-only prepared batch for the branch-zero texture-COPY profile.
///
/// Its private inner batch prevents type-1 normal words from being exposed as
/// synthetic lit colors through the generic fragment API.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftwareC3dPreparedCopyBatch(SoftwareC3dPreparedBatch);

/// Type-2 lit geometry for the alternate Gouraud/MODULATE compositor.
#[derive(Clone, Debug, PartialEq)]
pub struct SoftwareC3dPreparedLitBatch(SoftwareC3dPreparedBatch);

impl SoftwareC3dPreparedCopyBatch {
    /// The prepared screen triangles that survived clipping and the back-face
    /// cull.
    #[must_use]
    pub fn triangles(&self) -> &[SoftwareC3dPreparedTriangle] {
        self.0.triangles()
    }

    /// Source triangle count before clipping and culling.
    #[must_use]
    pub const fn source_triangle_count(&self) -> usize {
        self.0.source_triangle_count()
    }

    /// Source triangles dropped entirely by homogeneous clipping.
    #[must_use]
    pub const fn fully_clipped_source_triangle_count(&self) -> usize {
        self.0.fully_clipped_source_triangle_count()
    }

    /// Source triangles dropped by the back-face (`D3DCULL_CCW`) cull.
    #[must_use]
    pub const fn counter_clockwise_triangle_count(&self) -> usize {
        self.0.counter_clockwise_triangle_count()
    }
}

impl SoftwareC3dPreparedBatch {
    #[must_use]
    pub fn triangles(&self) -> &[SoftwareC3dPreparedTriangle] {
        &self.triangles
    }

    #[must_use]
    pub const fn source_triangle_count(&self) -> usize {
        self.source_triangle_count
    }

    #[must_use]
    pub const fn fully_clipped_source_triangle_count(&self) -> usize {
        self.fully_clipped_source_triangle_count
    }

    #[must_use]
    pub const fn counter_clockwise_triangle_count(&self) -> usize {
        self.counter_clockwise_triangle_count
    }

    #[must_use]
    pub const fn degenerate_triangle_count(&self) -> usize {
        self.degenerate_triangle_count
    }
}

/// Validates and preflights one recovered source-ordered triangle-list batch.
///
/// Culling happens after clipping and the single viewport Y inversion. The
/// recovered `D3DCULL_CCW` state keeps positive signed screen areas.
pub fn preflight_root_c3d_batch(
    command: &RootMenuC3dDrawCommand,
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedBatch, SoftwareC3dRasterError> {
    let branch = command.vertex_mutation_branch();
    if branch != C3dVertexMutationBranch::Type2 {
        return Err(SoftwareC3dRasterError::UnsupportedVertexLayout {
            vertex_type: command.vertex_type(),
        });
    }
    let vertices = command
        .vertices()
        .iter()
        .copied()
        .map(|vertex| {
            RasterInputVertex::from_type2(
                C3dType2ResourceScaledVertex::new(vertex, branch)
                    .expect("type-2 branch was checked before conversion"),
            )
        })
        .collect::<Vec<_>>();
    preflight_indexed_triangle_list(&vertices, command.indices(), transforms)
}

/// Preflights either recovered 32-byte vertex layout for texture COPY/no-blend.
///
/// Type 1 contributes XYZ/UV and is checked for finite normals, but those
/// normals do not enter the COPY combiner. Type 2 retains its packed-color
/// path internally; neither layout's lighting payload is exposed by the
/// returned geometry-only wrapper.
pub fn preflight_root_c3d_copy_batch(
    command: &RootMenuC3dDrawCommand,
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedCopyBatch, SoftwareC3dRasterError> {
    let branch = command.vertex_mutation_branch();
    let vertices = command
        .vertices()
        .iter()
        .copied()
        .map(|vertex| match branch {
            C3dVertexMutationBranch::Type1 => {
                let vertex = C3dType1ResourceScaledVertex::new(vertex, branch)
                    .expect("type-1 branch supplies the type-1 layout");
                RasterInputVertex::from_type1_copy(vertex)
            }
            C3dVertexMutationBranch::Type2 => {
                let vertex = C3dType2ResourceScaledVertex::new(vertex, branch)
                    .expect("type-2 branch supplies the type-2 layout");
                Ok(RasterInputVertex::from_type2(vertex))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    preflight_indexed_triangle_list(&vertices, command.indices(), transforms)
        .map(SoftwareC3dPreparedCopyBatch)
}

/// Preflights one assembled-mesh LOD prim for the texture-COPY/no-blend profile.
///
/// The prim is a [`cmr_core::c3d_mesh::MeshLodDrawGroup`] prim: its deduplicated
/// 32-byte vertices (`cmr_core::c3d_batch::C3dConsecutiveBatch::unique_entries_32`)
/// and compact triangle indices (`indices`). Each vertex becomes a model-space
/// [`RasterInputVertex`] via [`RasterInputVertex::from_mesh_entry`]; the node
/// world matrix belongs in the WORLD slot of `transforms` (for example via
/// [`SoftwareC3dTransformState::for_gameplay_scene`]), not in the vertex
/// conversion. The result feeds the same clip/cull core and `compose_*` entries
/// as the menu and car batches, so a parsed stage or car `.c3d` mesh reaches
/// pixels through the recovered transform pipeline.
pub fn preflight_mesh_lod_prim(
    vertices: &[C3dEntry32],
    indices: &[u16],
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedCopyBatch, SoftwareC3dRasterError> {
    let raster = vertices
        .iter()
        .copied()
        .map(RasterInputVertex::from_mesh_entry)
        .collect::<Vec<_>>();
    preflight_indexed_triangle_list(&raster, indices, transforms).map(SoftwareC3dPreparedCopyBatch)
}

/// Preflights a car batch for the bounded COPY profile using already-prepared
/// Type-2 words from the explicit retained mutation state.
///
/// Profile and Type-2 checks precede geometry work, making unsupported state
/// rejection effect-free for callers that preflight before composition.
pub fn preflight_championship_car_indexed_copy_batch(
    command: &C3dDrawCommand,
    prepared_vertices: &[C3dType2PreDrawVertex],
    transforms: SoftwareC3dTransformState,
    profile: SoftwareChampionshipCarRasterProfile,
) -> Result<SoftwareC3dPreparedCopyBatch, SoftwareC3dRasterError> {
    validate_championship_car_raster_profile(profile)?;
    preflight_championship_car_prepared_type2_batch(
        command.vertex_mutation_branch(),
        command.vertices().len(),
        command.indices(),
        prepared_vertices,
        transforms,
    )
    .map(SoftwareC3dPreparedCopyBatch)
}

/// Preflights a car batch for the candidate alternate
/// split/Gouraud/MODULATE/no-alpha oracle policy.
pub fn preflight_championship_car_split_modulate_batch(
    command: &C3dDrawCommand,
    prepared_vertices: &[C3dType2PreDrawVertex],
    transforms: SoftwareC3dTransformState,
    profile: SoftwareChampionshipCarSplitOracleProfile,
) -> Result<SoftwareC3dPreparedLitBatch, SoftwareC3dRasterError> {
    validate_championship_car_split_modulate_no_alpha_oracle_profile(profile)?;
    preflight_championship_car_prepared_type2_batch(
        command.vertex_mutation_branch(),
        command.vertices().len(),
        command.indices(),
        prepared_vertices,
        transforms,
    )
    .map(SoftwareC3dPreparedLitBatch)
}

fn preflight_championship_car_prepared_type2_batch(
    branch: C3dVertexMutationBranch,
    command_vertex_count: usize,
    indices: &[u16],
    prepared_vertices: &[C3dType2PreDrawVertex],
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedBatch, SoftwareC3dRasterError> {
    if branch != C3dVertexMutationBranch::Type2 {
        return Err(SoftwareC3dRasterError::UnsupportedVertexLayout {
            vertex_type: branch.submitted_vertex_type(),
        });
    }
    if command_vertex_count != prepared_vertices.len() {
        return Err(SoftwareC3dRasterError::PreparedVertexCountMismatch {
            command: command_vertex_count,
            prepared: prepared_vertices.len(),
        });
    }
    let vertices = prepared_vertices
        .iter()
        .copied()
        .map(RasterInputVertex::from_type2_pre_draw)
        .collect::<Vec<_>>();
    preflight_indexed_triangle_list(&vertices, indices, transforms)
}

pub fn validate_championship_car_raster_profile(
    profile: SoftwareChampionshipCarRasterProfile,
) -> Result<(), SoftwareC3dRasterError> {
    if profile.unified_indexed
        && profile.texture_copy
        && profile.point_filter
        && !profile.alpha_blend
    {
        Ok(())
    } else {
        Err(SoftwareC3dRasterError::UnsupportedChampionshipCarProfile(
            profile,
        ))
    }
}

/// Accepts only the candidate no-alpha split/MODULATE oracle policy. The
/// requested numeric filter and the explicitly chosen oracle filter remain
/// separate; fog and actor-RGB may use any combination.
pub fn validate_championship_car_split_modulate_no_alpha_oracle_profile(
    profile: SoftwareChampionshipCarSplitOracleProfile,
) -> Result<(), SoftwareC3dRasterError> {
    if !profile.raster.unified_indexed
        && !profile.raster.texture_copy
        && !profile.raster.point_filter
        && !profile.raster.alpha_blend
    {
        Ok(())
    } else {
        Err(SoftwareC3dRasterError::UnsupportedChampionshipCarProfile(
            profile.raster,
        ))
    }
}

/// One pre-lit world-space vertex (`D3DLVERTEX`: position, diffuse ARGB,
/// texture coordinates; the specular is `0xff000000`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftwareC3dLitVertex {
    pub position: [f32; 3],
    pub diffuse: u32,
    pub uv: [f32; 2],
}

/// Preflights pre-lit vertices drawn as an indexed triangle list through the
/// same transform, clip and cull core as the meshes (`transforms` carries
/// the WORLD slot, identity for world-space vertices).
pub fn preflight_lit_triangle_list(
    vertices: &[SoftwareC3dLitVertex],
    indices: &[u16],
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedBatch, SoftwareC3dRasterError> {
    let raster = vertices
        .iter()
        .map(|vertex| RasterInputVertex {
            position: vertex.position,
            diffuse: vertex.diffuse,
            specular: 0xff00_0000,
            uv: vertex.uv,
        })
        .collect::<Vec<_>>();
    preflight_indexed_triangle_list(&raster, indices, transforms)
}

/// Visits the depth-passing fragments of an untextured batch (the caller
/// samples its own texture from the fragment's coordinates). Depth is
/// written to `depth` as for any batch; a caller drawing with z-write off
/// can use [`rasterize_c3d_fragments_untextured_no_zwrite`] to avoid cloning
/// the depth buffer.
pub fn rasterize_c3d_fragments_untextured(
    batch: &SoftwareC3dPreparedBatch,
    depth: &mut SoftwareC3dDepthBuffer,
    visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    rasterize_fragments_extent_only(batch, [1, 1], depth, visit)
}

/// Visits the depth-passing fragments of an untextured batch with
/// `ZWRITEENABLE = FALSE` (`depth` is read-only, avoiding a full-frame depth
/// buffer clone for shadow, lamp-flare, and tyre-mark passes).
pub fn rasterize_c3d_fragments_untextured_no_zwrite(
    batch: &SoftwareC3dPreparedBatch,
    depth: &SoftwareC3dDepthBuffer,
    mut visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    validate_prepared_triangles(batch)?;
    let [target_width, target_height] = depth.extent;
    let mut outcome = SoftwareC3dFragmentOutcome::default();
    for triangle in &batch.triangles {
        let positions = triangle.vertices.map(SoftwareC3dScreenVertex::position);
        let area = signed_screen_area(triangle.vertices);
        let Some([minimum_x, maximum_x]) =
            candidate_axis_bounds(positions.map(|position| position[0]), target_width)
        else {
            continue;
        };
        let Some([minimum_y, maximum_y]) =
            candidate_axis_bounds(positions.map(|position| position[1]), target_height)
        else {
            continue;
        };

        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                let sample = [f32::from(x) + 0.5, f32::from(y) + 0.5];
                let edges = [
                    edge(positions[1], positions[2], sample),
                    edge(positions[2], positions[0], sample),
                    edge(positions[0], positions[1], sample),
                ];
                let edge_pairs = [
                    [positions[1], positions[2]],
                    [positions[2], positions[0]],
                    [positions[0], positions[1]],
                ];
                if !(0..3).all(|index| edge_accepts(edges[index], edge_pairs[index])) {
                    continue;
                }
                outcome.covered += 1;

                let weights = edges.map(|value| f64::from(value) / f64::from(area));
                let interpolated_depth =
                    weighted_scalar(weights, triangle.vertices.map(|vertex| vertex.position[2]))
                        .clamp(0.0, 1.0) as f32;
                let target_index = usize::from(y) * usize::from(target_width) + usize::from(x);
                if interpolated_depth > depth.values[target_index] {
                    outcome.depth_rejected += 1;
                    continue;
                }

                let reciprocal_w = weighted_scalar(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::reciprocal_w),
                );
                let uv_over_w = weighted_pair(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::uv_over_w),
                );
                let uv = [
                    (uv_over_w[0] / reciprocal_w).clamp(0.0, 1.0) as f32,
                    (uv_over_w[1] / reciprocal_w).clamp(0.0, 1.0) as f32,
                ];
                let diffuse_argb = weighted_quad(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::diffuse_argb),
                )
                .map(|component| component.clamp(0.0, 1.0) as f32);

                outcome.depth_passed += 1;
                visit(SoftwareC3dFragment {
                    position: [x, y],
                    depth: interpolated_depth,
                    uv,
                    texture_index: 0,
                    diffuse_argb,
                    flat_diffuse: triangle.flat_diffuse,
                });
            }
        }
    }
    Ok(outcome)
}

/// Visits depth-passing fragments for one prepared batch in triangle/scanline
/// order using the deterministic point-filter compatibility policy.
///
/// All fallible validation completes before depth is mutated or `visit` runs.
pub fn rasterize_root_c3d_fragments(
    batch: &SoftwareC3dPreparedBatch,
    texture: &SoftwareC3dSampledTexture,
    depth: &mut SoftwareC3dDepthBuffer,
    visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    rasterize_fragments(
        batch,
        TextureView {
            extent: texture.extent(),
            indices: texture.indices(),
            palette_len: texture.palette().len(),
        },
        depth,
        visit,
    )
}

/// Renders the explicit branch-zero, alpha-disabled COPY profile into an
/// indexed target using a deterministic source-owned palette policy.
///
/// Texture RGB is mapped to the caller's active display palette by full-palette
/// Manhattan distance. Exact and equal-distance ties retain the lowest index;
/// index zero is an ordinary source and destination value. This mapping is a
/// reconstruction policy, not a claim about a particular legacy D3D driver.
///
/// Target validation and the fragment rasterizer's complete validation happen
/// before either the target or depth buffer is mutated.
pub fn compose_root_c3d_indexed_copy_no_blend(
    batch: &SoftwareC3dPreparedCopyBatch,
    texture: &SoftwareC3dSampledTexture,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    compose_indexed_copy_no_blend(
        &batch.0,
        IndexedTextureView {
            raster: TextureView {
                extent: texture.extent(),
                indices: texture.indices(),
                palette_len: texture.palette().len(),
            },
            palette: texture.palette(),
        },
        depth,
        target_extent,
        target_pixels,
        active_palette,
        quantization,
    )
}

/// Opaque COPY composition with the recovered stage color-key test. Fragments
/// sampling `color_key` do not update either color or depth.
pub fn compose_root_c3d_indexed_copy_color_key(
    batch: &SoftwareC3dPreparedCopyBatch,
    texture: &SoftwareC3dSampledTexture,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    active_palette: &[PaletteEntry; 256],
    color_key: u8,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    compose_indexed_copy_no_blend_with_color_key(
        &batch.0,
        IndexedTextureView {
            raster: TextureView {
                extent: texture.extent(),
                indices: texture.indices(),
                palette_len: texture.palette().len(),
            },
            palette: texture.palette(),
        },
        depth,
        target_extent,
        target_pixels,
        active_palette,
        IndexedCopyOptions {
            quantization: SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
            color_key: Some(color_key),
        },
    )
}

/// Copies sampled source indices directly into the indexed championship target.
///
/// No palette lookup or RGB quantization occurs. The terminal owner may commit
/// the pending car palette after this call, preserving the recovered ordering.
/// Target, source, and prepared geometry are fully validated before color or
/// depth is changed.
pub fn compose_championship_car_indexed_copy_no_blend(
    batch: &SoftwareC3dPreparedCopyBatch,
    source: SoftwareChampionshipCarIndexedSource<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    profile: SoftwareChampionshipCarRasterProfile,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    let default_index = [0u8];
    let texture = match source {
        SoftwareChampionshipCarIndexedSource::UnifiedTexture(texture) => TextureView {
            extent: texture.extent(),
            indices: texture.indices(),
            // The COPY profile consumes source indices, not retained palette
            // entries. Every u8 index is valid even when BMP metadata carries
            // fewer than 256 palette records.
            palette_len: 256,
        },
        SoftwareChampionshipCarIndexedSource::ReviewedCarPaletteMissingSelectorOpaqueBlackIndexZero => TextureView {
            extent: [1, 1],
            indices: &default_index,
            palette_len: 1,
        },
    };
    compose_championship_car_indexed_copy_view(
        &batch.0,
        texture,
        depth,
        target_extent,
        target_pixels,
        profile,
    )
}

/// Composes a candidate alternate championship policy into canonical RGB.
///
/// The caller supplies a source-owned selected-format materialization, so mask
/// quantization (when selected) necessarily precedes the explicit oracle
/// bilinear policy. This is separate from the observed numeric filter request
/// and makes no claim about the effective legacy device filter. Filtered RGB is multiplied by
/// screen-affine Gouraud diffuse RGB. Output is one canonical `[R,G,B]` triplet
/// per pixel before unresolved device-format conversion; retained 16/24/32-bit
/// depth is provenance only and does not select packing here.
pub fn compose_championship_car_split_modulate_canonical_rgb_oracle(
    batch: &SoftwareC3dPreparedLitBatch,
    source: SoftwareChampionshipCarSplitSource<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [[u8; 3]],
    profile: SoftwareChampionshipCarSplitOracleProfile,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    validate_championship_car_split_modulate_no_alpha_oracle_profile(profile)
        .map_err(SoftwareC3dComposeError::Raster)?;
    validate_rgb_target(target_extent, target_pixels, depth.extent())?;
    let target_width = usize::from(target_extent[0]);
    compose_championship_car_split_modulate_validated_target(
        batch,
        source,
        depth,
        |fragment, modulated| {
            let [x, y] = fragment.position();
            target_pixels[usize::from(y) * target_width + usize::from(x)] = modulated;
        },
    )
}

/// Composes the bounded alternate championship oracle into logical words for
/// a separately selected runtime framebuffer format.
///
/// The texture has already crossed the source-owned selected-texture
/// conversion boundary before bilinear filtering. Only depth-passing pixels
/// are packed through the target's independent RGB masks; uncovered and
/// depth-rejected logical words remain bit-exact. All fallible validation is
/// completed before either target words or depth values change.
///
/// Logical words are not a legacy byte buffer. This function makes no claim
/// about driver `Load`, effective device filtering, byte order, row pitch,
/// dithering, or alpha-enabled rendering.
pub fn compose_championship_car_split_modulate_logical_masked_rgb_oracle(
    batch: &SoftwareC3dPreparedLitBatch,
    source: SoftwareChampionshipCarSplitSource<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target: &mut LogicalMaskedRgbTarget,
    profile: SoftwareChampionshipCarSplitOracleProfile,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    preflight_championship_car_split_modulate_logical_masked_rgb_oracle(
        batch, source, depth, target, profile,
    )?;

    let target_extent = target.extent();
    let target_width = usize::from(target_extent[0]);
    compose_championship_car_split_modulate_validated_target(
        batch,
        source,
        depth,
        |fragment, modulated| {
            let [x, y] = fragment.position();
            target.pack_rgb8_at_source_owned_oracle(
                usize::from(y) * target_width + usize::from(x),
                modulated,
            );
        },
    )
}

/// Validates one alternate logical-word composition without changing depth or
/// target storage.
///
/// Backends use this after transform/clipping preflight and before the common
/// scene transaction performs depth clear, camera, or `BeginScene` effects.
/// Converted textures already retain checked storage lengths; this boundary
/// additionally checks the selected source extent, the black-default guard,
/// prepared triangles, target shape, depth extent, and profile tuple.
pub fn preflight_championship_car_split_modulate_logical_masked_rgb_oracle(
    batch: &SoftwareC3dPreparedLitBatch,
    source: SoftwareChampionshipCarSplitSource<'_>,
    depth: &SoftwareC3dDepthBuffer,
    target: &LogicalMaskedRgbTarget,
    profile: SoftwareChampionshipCarSplitOracleProfile,
) -> Result<(), SoftwareC3dComposeError> {
    validate_championship_car_split_modulate_no_alpha_oracle_profile(profile)
        .map_err(SoftwareC3dComposeError::Raster)?;
    validate_target_shape(target.extent(), target.words().len(), depth.extent())?;
    match source {
        SoftwareChampionshipCarSplitSource::ConvertedTexture(texture) => {
            let extent = texture.extent();
            if extent[0] == 0 || extent[1] == 0 {
                return Err(SoftwareC3dComposeError::Raster(
                    SoftwareC3dRasterError::InvalidTextureExtent(extent),
                ));
            }
        }
        SoftwareChampionshipCarSplitSource::BlackDiffuseDefault => {
            validate_black_alternate_default(&batch.0).map_err(SoftwareC3dComposeError::Raster)?;
        }
    }
    validate_prepared_triangles(&batch.0).map_err(SoftwareC3dComposeError::Raster)
}

fn compose_championship_car_split_modulate_validated_target(
    batch: &SoftwareC3dPreparedLitBatch,
    source: SoftwareChampionshipCarSplitSource<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    mut write: impl FnMut(SoftwareC3dFragment, [u8; 3]),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    let (texture, black_default) = match source {
        SoftwareChampionshipCarSplitSource::ConvertedTexture(texture) => (Some(texture), false),
        SoftwareChampionshipCarSplitSource::BlackDiffuseDefault => {
            validate_black_alternate_default(&batch.0).map_err(SoftwareC3dComposeError::Raster)?;
            (None, true)
        }
    };
    validate_prepared_triangles(&batch.0).map_err(SoftwareC3dComposeError::Raster)?;

    let texture_extent = texture.map_or([1, 1], SourceOwnedLogicalSelectedTexture::extent);
    // rasterize_fragments_extent_only performs its remaining extent and batch
    // validation before its first depth write or callback invocation.
    rasterize_fragments_extent_only(&batch.0, texture_extent, depth, |fragment| {
        let sampled = if black_default {
            [0.0; 3]
        } else {
            sample_bilinear_converted_rgb_oracle_clamped(
                texture.expect("non-default source retains a converted texture"),
                fragment.uv(),
            )
        };
        let diffuse = fragment.diffuse_argb();
        let modulated = std::array::from_fn(|channel| {
            (sampled[channel] * f64::from(diffuse[channel + 1]))
                .clamp(0.0, 255.0)
                .round() as u8
        });
        write(fragment, modulated);
    })
    .map_err(SoftwareC3dComposeError::Raster)
}

fn compose_championship_car_indexed_copy_view(
    batch: &SoftwareC3dPreparedBatch,
    texture: TextureView<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    profile: SoftwareChampionshipCarRasterProfile,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    validate_championship_car_raster_profile(profile).map_err(SoftwareC3dComposeError::Raster)?;
    validate_indexed_target(target_extent, target_pixels, depth.extent())?;
    let target_width = usize::from(target_extent[0]);
    rasterize_fragments(batch, texture, depth, |fragment| {
        let [x, y] = fragment.position();
        target_pixels[usize::from(y) * target_width + usize::from(x)] = fragment.texture_index();
    })
    .map_err(SoftwareC3dComposeError::Raster)
}

/// Applies the recovered row-vector transform chain and `D3DVIEWPORT2` clip
/// volume to one untransformed lit vertex.
pub fn transform_type2_c3d_vertex(
    vertex: C3dMode2ResourceScaledVertex,
    world: C3dMatrixWords,
    view: C3dMatrixWords,
    projection: C3dMatrixWords,
    viewport: C3dViewportWords,
) -> Result<SoftwareC3dClipVertex, SoftwareC3dRasterError> {
    let vertex = C3dType2ResourceScaledVertex::new(vertex, C3dVertexMutationBranch::Type2)
        .expect("the type-2 transform entry point supplies the type-2 layout");
    let input = RasterInputVertex::from_type2(vertex).into_clip_vertex();
    transform_vertex(input, world, view, projection, viewport)
}

/// Clips one source-ordered triangle against all six homogeneous clip planes.
///
/// The returned polygon retains source winding and has zero vertices when the
/// triangle is fully outside. Boundary points are inside by project policy.
pub fn clip_root_c3d_triangle(
    triangle: [SoftwareC3dClipVertex; 3],
) -> Result<Vec<SoftwareC3dClipVertex>, SoftwareC3dRasterError> {
    if !triangle.into_iter().all(SoftwareC3dClipVertex::is_finite) {
        return Err(SoftwareC3dRasterError::NonFiniteInput);
    }

    let mut polygon = triangle.to_vec();
    for plane in 0..CLIP_PLANE_COUNT {
        polygon = clip_polygon_against_plane(&polygon, plane)?;
        if polygon.is_empty() {
            break;
        }
    }
    Ok(polygon)
}

/// Performs homogeneous divide and the recovered viewport mapping.
pub fn project_root_c3d_polygon(
    polygon: &[SoftwareC3dClipVertex],
    viewport: C3dViewportWords,
) -> Result<Vec<SoftwareC3dScreenVertex>, SoftwareC3dRasterError> {
    let viewport = DecodedViewport::decode(viewport)?;
    polygon
        .iter()
        .copied()
        .map(|vertex| project_vertex(vertex, viewport))
        .collect()
}

fn transform_vertex(
    mut vertex: SoftwareC3dClipVertex,
    world: C3dMatrixWords,
    view: C3dMatrixWords,
    projection: C3dMatrixWords,
    viewport: C3dViewportWords,
) -> Result<SoftwareC3dClipVertex, SoftwareC3dRasterError> {
    if !vertex.is_finite()
        || !world
            .words()
            .map(f32::from_bits)
            .into_iter()
            .all(f32::is_finite)
        || !view
            .words()
            .map(f32::from_bits)
            .into_iter()
            .all(f32::is_finite)
        || !projection
            .words()
            .map(f32::from_bits)
            .into_iter()
            .all(f32::is_finite)
    {
        return Err(SoftwareC3dRasterError::NonFiniteInput);
    }
    let viewport = DecodedViewport::decode(viewport)?;
    vertex.clip_position = multiply_row_vector(vertex.clip_position, decode_matrix(world));
    vertex.clip_position = multiply_row_vector(vertex.clip_position, decode_matrix(view));
    vertex.clip_position = multiply_row_vector(vertex.clip_position, decode_matrix(projection));

    let [x, y, z, w] = vertex.clip_position;
    let clip_center_x = viewport.clip_x + viewport.clip_width * 0.5;
    let clip_center_y = viewport.clip_y - viewport.clip_height * 0.5;
    vertex.clip_position = [
        (x - clip_center_x * w) * (2.0 / viewport.clip_width),
        (y - clip_center_y * w) * (2.0 / viewport.clip_height),
        (z - viewport.min_z * w) * (1.0 / (viewport.max_z - viewport.min_z)),
        w,
    ];
    if !vertex.is_finite() {
        return Err(SoftwareC3dRasterError::NonFiniteResult);
    }
    Ok(vertex)
}

fn clip_polygon_against_plane(
    polygon: &[SoftwareC3dClipVertex],
    plane: usize,
) -> Result<Vec<SoftwareC3dClipVertex>, SoftwareC3dRasterError> {
    let mut output = Vec::with_capacity(polygon.len().saturating_add(1));
    let Some(mut previous) = polygon.last().copied() else {
        return Ok(output);
    };
    let mut previous_distance = clip_distance(previous.clip_position, plane);
    let mut previous_inside = previous_distance >= 0.0;

    for current in polygon.iter().copied() {
        let current_distance = clip_distance(current.clip_position, plane);
        let current_inside = current_distance >= 0.0;
        if current_inside != previous_inside {
            // Do not synthesize an endpoint already lying on this closed plane:
            // interpolation at exactly zero/one can perturb unrelated payload
            // bits and defeat whole-vertex duplicate suppression.
            if previous_distance != 0.0 && current_distance != 0.0 {
                let amount = previous_distance / (previous_distance - current_distance);
                let intersection = previous.interpolate(current, amount);
                if !intersection.is_finite() {
                    return Err(SoftwareC3dRasterError::NonFiniteResult);
                }
                push_distinct(&mut output, intersection);
            }
        }
        if current_inside {
            push_distinct(&mut output, current);
        }
        previous = current;
        previous_distance = current_distance;
        previous_inside = current_inside;
    }
    if output.len() > 1 && output.first() == output.last() {
        output.pop();
    }
    Ok(output)
}

fn push_distinct(output: &mut Vec<SoftwareC3dClipVertex>, vertex: SoftwareC3dClipVertex) {
    if output.last() != Some(&vertex) {
        output.push(vertex);
    }
}

fn clip_distance([x, y, z, w]: [f32; 4], plane: usize) -> f32 {
    match plane {
        0 => x + w,
        1 => w - x,
        2 => y + w,
        3 => w - y,
        4 => z,
        5 => w - z,
        _ => unreachable!("exactly six clip planes"),
    }
}

fn project_vertex(
    vertex: SoftwareC3dClipVertex,
    viewport: DecodedViewport,
) -> Result<SoftwareC3dScreenVertex, SoftwareC3dRasterError> {
    if !vertex.is_finite() {
        return Err(SoftwareC3dRasterError::NonFiniteInput);
    }
    let [x, y, z, w] = vertex.clip_position;
    if w <= 0.0 {
        return Err(SoftwareC3dRasterError::NonPositiveHomogeneousW);
    }
    let reciprocal_w = 1.0 / w;
    let normalized_x = x * reciprocal_w;
    let normalized_y = y * reciprocal_w;
    let normalized_z = z * reciprocal_w;
    let result = SoftwareC3dScreenVertex {
        position: [
            viewport.x + (normalized_x + 1.0) * viewport.width * 0.5,
            viewport.y + (1.0 - normalized_y) * viewport.height * 0.5,
            normalized_z,
        ],
        reciprocal_w,
        uv_over_w: [vertex.uv[0] * reciprocal_w, vertex.uv[1] * reciprocal_w],
        diffuse_argb: vertex.diffuse_argb,
        specular_argb: vertex.specular_argb,
    };
    if result
        .position
        .into_iter()
        .chain([result.reciprocal_w])
        .chain(result.uv_over_w)
        .all(f32::is_finite)
    {
        Ok(result)
    } else {
        Err(SoftwareC3dRasterError::NonFiniteResult)
    }
}

fn preflight_indexed_triangle_list(
    vertices: &[RasterInputVertex],
    indices: &[u16],
    transforms: SoftwareC3dTransformState,
) -> Result<SoftwareC3dPreparedBatch, SoftwareC3dRasterError> {
    let (source_triangles, remainder) = indices.as_chunks::<3>();
    if !remainder.is_empty() {
        return Err(SoftwareC3dRasterError::IncompleteIndexTriplet {
            index_count: indices.len(),
        });
    }

    let transformed = vertices
        .iter()
        .copied()
        .map(|vertex| {
            transform_vertex(
                vertex.into_clip_vertex(),
                transforms.world,
                transforms.view,
                transforms.projection,
                transforms.viewport,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut prepared = Vec::new();
    let mut fully_clipped = 0;
    let mut counter_clockwise = 0;
    let mut degenerate = 0;

    for (triangle_index, source_indices) in source_triangles.iter().enumerate() {
        let resolve = |corner: usize| {
            let source_index = source_indices[corner];
            transformed.get(usize::from(source_index)).copied().ok_or(
                SoftwareC3dRasterError::IndexOutOfRange {
                    triangle: triangle_index,
                    corner,
                    index: source_index,
                    vertex_count: transformed.len(),
                },
            )
        };
        let triangle = [resolve(0)?, resolve(1)?, resolve(2)?];
        let flat_diffuse = vertices[usize::from(source_indices[0])].diffuse;
        let clipped = clip_root_c3d_triangle(triangle)?;
        if clipped.is_empty() {
            fully_clipped += 1;
            continue;
        }
        if clipped.len() < 3 {
            degenerate += 1;
            continue;
        }
        let projected = project_root_c3d_polygon(&clipped, transforms.viewport)?;
        for fan_index in 1..projected.len() - 1 {
            let fan = [projected[0], projected[fan_index], projected[fan_index + 1]];
            let area = signed_screen_area(fan);
            if area == 0.0 {
                degenerate += 1;
            } else if area < 0.0 {
                counter_clockwise += 1;
            } else {
                prepared.push(SoftwareC3dPreparedTriangle {
                    vertices: fan,
                    flat_diffuse,
                });
            }
        }
    }

    Ok(SoftwareC3dPreparedBatch {
        triangles: prepared.into_boxed_slice(),
        source_triangle_count: indices.len() / 3,
        fully_clipped_source_triangle_count: fully_clipped,
        counter_clockwise_triangle_count: counter_clockwise,
        degenerate_triangle_count: degenerate,
    })
}

fn signed_screen_area(vertices: [SoftwareC3dScreenVertex; 3]) -> f32 {
    let [a, b, c] = vertices.map(SoftwareC3dScreenVertex::position);
    edge(a, b, [c[0], c[1]])
}

#[derive(Clone, Copy)]
struct TextureView<'a> {
    extent: [u16; 2],
    indices: &'a [u8],
    palette_len: usize,
}

#[derive(Clone, Copy)]
struct IndexedTextureView<'a> {
    raster: TextureView<'a>,
    palette: &'a [BitmapPaletteEntry],
}

#[derive(Clone, Copy)]
struct IndexedCopyOptions {
    quantization: SoftwareC3dIndexedQuantization,
    color_key: Option<u8>,
}

fn compose_indexed_copy_no_blend(
    batch: &SoftwareC3dPreparedBatch,
    texture: IndexedTextureView<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    compose_indexed_copy_no_blend_with_color_key(
        batch,
        texture,
        depth,
        target_extent,
        target_pixels,
        active_palette,
        IndexedCopyOptions {
            quantization,
            color_key: None,
        },
    )
}

fn compose_indexed_copy_no_blend_with_color_key(
    batch: &SoftwareC3dPreparedBatch,
    texture: IndexedTextureView<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    target_extent: [u16; 2],
    target_pixels: &mut [u8],
    active_palette: &[PaletteEntry; 256],
    options: IndexedCopyOptions,
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dComposeError> {
    validate_indexed_target(target_extent, target_pixels, depth.extent())?;
    let palette_map = match options.quantization {
        SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither => {
            full_palette_manhattan_map(texture.palette, active_palette)
        }
    };
    let target_width = usize::from(target_extent[0]);
    rasterize_fragments_with_color_key(
        batch,
        texture.raster,
        depth,
        options.color_key,
        |fragment| {
            let [x, y] = fragment.position();
            let target_index = usize::from(y) * target_width + usize::from(x);
            target_pixels[target_index] = palette_map[usize::from(fragment.texture_index())];
        },
    )
    .map_err(SoftwareC3dComposeError::Raster)
}

fn validate_indexed_target(
    extent: [u16; 2],
    pixels: &[u8],
    depth_extent: [u16; 2],
) -> Result<(), SoftwareC3dComposeError> {
    validate_target_shape(extent, pixels.len(), depth_extent)
}

fn validate_target_shape(
    extent: [u16; 2],
    actual_len: usize,
    depth_extent: [u16; 2],
) -> Result<(), SoftwareC3dComposeError> {
    if extent[0] == 0 || extent[1] == 0 {
        return Err(SoftwareC3dComposeError::InvalidTargetExtent(extent));
    }
    let expected = usize::from(extent[0]) * usize::from(extent[1]);
    if actual_len != expected {
        return Err(SoftwareC3dComposeError::InvalidTargetLength {
            expected,
            actual: actual_len,
        });
    }
    if extent != depth_extent {
        return Err(SoftwareC3dComposeError::DepthTargetExtentMismatch {
            depth: depth_extent,
            target: extent,
        });
    }
    Ok(())
}

fn validate_rgb_target(
    extent: [u16; 2],
    pixels: &[[u8; 3]],
    depth_extent: [u16; 2],
) -> Result<(), SoftwareC3dComposeError> {
    validate_target_shape(extent, pixels.len(), depth_extent)
}

fn full_palette_manhattan_map(
    texture_palette: &[BitmapPaletteEntry],
    active_palette: &[PaletteEntry; 256],
) -> Vec<u8> {
    texture_palette
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
        .collect()
}

fn validate_black_alternate_default(
    batch: &SoftwareC3dPreparedBatch,
) -> Result<(), SoftwareC3dRasterError> {
    for (triangle, prepared) in batch.triangles.iter().enumerate() {
        for (vertex, prepared) in prepared.vertices.iter().enumerate() {
            if prepared.diffuse_argb[1..] != [0.0; 3] {
                return Err(SoftwareC3dRasterError::NonBlackAlternateDefaultDiffuse {
                    triangle,
                    vertex,
                });
            }
        }
    }
    Ok(())
}

/// Filters only texels already materialized in the selected texture format.
fn sample_bilinear_converted_rgb_oracle_clamped(
    texture: &SourceOwnedLogicalSelectedTexture,
    uv: [f32; 2],
) -> [f64; 3] {
    let [width, height] = texture.extent().map(usize::from);
    let axis = |coordinate: f32, extent: usize| {
        let position = f64::from(coordinate.clamp(0.0, 1.0)) * extent as f64 - 0.5;
        let lower = position.floor();
        let fraction = position - lower;
        let lower_index = (lower as isize).clamp(0, extent as isize - 1) as usize;
        let upper_index = (lower as isize + 1).clamp(0, extent as isize - 1) as usize;
        (lower_index, upper_index, fraction)
    };
    let (left, right, horizontal) = axis(uv[0], width);
    let (top, bottom, vertical) = axis(uv[1], height);
    let texel_rgb = |x: usize, y: usize| {
        texture
            .texel_rgb8_source_owned_oracle(y * width + x)
            .expect("converted texture length matches its retained extent")
            .map(f64::from)
    };
    let top_left = texel_rgb(left, top);
    let top_right = texel_rgb(right, top);
    let bottom_left = texel_rgb(left, bottom);
    let bottom_right = texel_rgb(right, bottom);
    std::array::from_fn(|channel| {
        let top_value = top_left[channel] + (top_right[channel] - top_left[channel]) * horizontal;
        let bottom_value =
            bottom_left[channel] + (bottom_right[channel] - bottom_left[channel]) * horizontal;
        top_value + (bottom_value - top_value) * vertical
    })
}

fn rasterize_fragments(
    batch: &SoftwareC3dPreparedBatch,
    texture: TextureView<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    validate_texture(texture)?;
    rasterize_fragments_with_optional_index(batch, Some(texture), None, depth, visit)
}

fn rasterize_fragments_with_color_key(
    batch: &SoftwareC3dPreparedBatch,
    texture: TextureView<'_>,
    depth: &mut SoftwareC3dDepthBuffer,
    color_key: Option<u8>,
    visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    validate_texture(texture)?;
    rasterize_fragments_with_optional_index(batch, Some(texture), color_key, depth, visit)
}

fn rasterize_fragments_extent_only(
    batch: &SoftwareC3dPreparedBatch,
    texture_extent: [u16; 2],
    depth: &mut SoftwareC3dDepthBuffer,
    visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    if texture_extent[0] == 0 || texture_extent[1] == 0 {
        return Err(SoftwareC3dRasterError::InvalidTextureExtent(texture_extent));
    }
    rasterize_fragments_with_optional_index(batch, None, None, depth, visit)
}

fn rasterize_fragments_with_optional_index(
    batch: &SoftwareC3dPreparedBatch,
    indexed_texture: Option<TextureView<'_>>,
    color_key: Option<u8>,
    depth: &mut SoftwareC3dDepthBuffer,
    mut visit: impl FnMut(SoftwareC3dFragment),
) -> Result<SoftwareC3dFragmentOutcome, SoftwareC3dRasterError> {
    validate_prepared_triangles(batch)?;

    let [target_width, target_height] = depth.extent;
    let mut outcome = SoftwareC3dFragmentOutcome::default();
    for triangle in &batch.triangles {
        let positions = triangle.vertices.map(SoftwareC3dScreenVertex::position);
        let area = signed_screen_area(triangle.vertices);
        let Some([minimum_x, maximum_x]) =
            candidate_axis_bounds(positions.map(|position| position[0]), target_width)
        else {
            continue;
        };
        let Some([minimum_y, maximum_y]) =
            candidate_axis_bounds(positions.map(|position| position[1]), target_height)
        else {
            continue;
        };

        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                let sample = [f32::from(x) + 0.5, f32::from(y) + 0.5];
                let edges = [
                    edge(positions[1], positions[2], sample),
                    edge(positions[2], positions[0], sample),
                    edge(positions[0], positions[1], sample),
                ];
                let edge_pairs = [
                    [positions[1], positions[2]],
                    [positions[2], positions[0]],
                    [positions[0], positions[1]],
                ];
                if !(0..3).all(|index| edge_accepts(edges[index], edge_pairs[index])) {
                    continue;
                }
                outcome.covered += 1;

                let weights = edges.map(|value| f64::from(value) / f64::from(area));
                let interpolated_depth =
                    weighted_scalar(weights, triangle.vertices.map(|vertex| vertex.position[2]))
                        .clamp(0.0, 1.0) as f32;
                let target_index = usize::from(y) * usize::from(target_width) + usize::from(x);
                if interpolated_depth > depth.values[target_index] {
                    outcome.depth_rejected += 1;
                    continue;
                }

                let reciprocal_w = weighted_scalar(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::reciprocal_w),
                );
                let uv_over_w = weighted_pair(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::uv_over_w),
                );
                let uv = [
                    (uv_over_w[0] / reciprocal_w).clamp(0.0, 1.0) as f32,
                    (uv_over_w[1] / reciprocal_w).clamp(0.0, 1.0) as f32,
                ];
                let diffuse_argb = weighted_quad(
                    weights,
                    triangle.vertices.map(SoftwareC3dScreenVertex::diffuse_argb),
                )
                .map(|component| component.clamp(0.0, 1.0) as f32);
                let texture_index = indexed_texture
                    .map(|texture| sample_point_clamped(texture, uv))
                    .unwrap_or(0);

                if color_key == Some(texture_index) {
                    continue;
                }

                depth.values[target_index] = interpolated_depth;
                outcome.depth_passed += 1;
                visit(SoftwareC3dFragment {
                    position: [x, y],
                    depth: interpolated_depth,
                    uv,
                    texture_index,
                    diffuse_argb,
                    flat_diffuse: triangle.flat_diffuse,
                });
            }
        }
    }
    Ok(outcome)
}

fn validate_texture(texture: TextureView<'_>) -> Result<(), SoftwareC3dRasterError> {
    let [width, height] = texture.extent;
    if width == 0 || height == 0 {
        return Err(SoftwareC3dRasterError::InvalidTextureExtent(texture.extent));
    }
    let expected = usize::from(width) * usize::from(height);
    if texture.indices.len() != expected {
        return Err(SoftwareC3dRasterError::InvalidTextureIndexLength {
            expected,
            actual: texture.indices.len(),
        });
    }
    if let Some(index) = texture
        .indices
        .iter()
        .copied()
        .find(|index| usize::from(*index) >= texture.palette_len)
    {
        return Err(SoftwareC3dRasterError::TexturePaletteIndexOutOfRange {
            index,
            palette_len: texture.palette_len,
        });
    }
    Ok(())
}

fn validate_prepared_triangles(
    batch: &SoftwareC3dPreparedBatch,
) -> Result<(), SoftwareC3dRasterError> {
    for (triangle_index, triangle) in batch.triangles.iter().enumerate() {
        let valid_vertices = triangle.vertices.iter().all(|vertex| {
            vertex
                .position
                .into_iter()
                .chain([vertex.reciprocal_w])
                .chain(vertex.uv_over_w)
                .chain(vertex.diffuse_argb)
                .all(f32::is_finite)
                && vertex.reciprocal_w > 0.0
        });
        let area = signed_screen_area(triangle.vertices);
        if !valid_vertices || !area.is_finite() || area <= 0.0 {
            return Err(SoftwareC3dRasterError::InvalidPreparedTriangle {
                triangle: triangle_index,
            });
        }
    }
    Ok(())
}

fn candidate_axis_bounds(coordinates: [f32; 3], extent: u16) -> Option<[u16; 2]> {
    let minimum = coordinates.into_iter().fold(f32::INFINITY, f32::min);
    let maximum = coordinates.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let last = i64::from(extent - 1);
    let first_pixel = ((minimum - 0.5).ceil() as i64).clamp(0, last);
    let last_pixel = ((maximum - 0.5).floor() as i64).clamp(0, last);
    (first_pixel <= last_pixel).then_some([first_pixel as u16, last_pixel as u16])
}

fn edge(start: [f32; 3], end: [f32; 3], sample: [f32; 2]) -> f32 {
    if point_precedes(start, end) {
        raw_edge(start, end, sample)
    } else {
        -raw_edge(end, start, sample)
    }
}

fn raw_edge(start: [f32; 3], end: [f32; 3], sample: [f32; 2]) -> f32 {
    (end[0] - start[0]) * (sample[1] - start[1]) - (end[1] - start[1]) * (sample[0] - start[0])
}

fn point_precedes(left: [f32; 3], right: [f32; 3]) -> bool {
    left[0]
        .total_cmp(&right[0])
        .then_with(|| left[1].total_cmp(&right[1]))
        .is_le()
}

fn edge_accepts(value: f32, [start, end]: [[f32; 3]; 2]) -> bool {
    if value > 0.0 {
        return true;
    }
    if value != 0.0 {
        return false;
    }
    let delta_x = end[0] - start[0];
    let delta_y = end[1] - start[1];
    delta_y < 0.0 || (delta_y == 0.0 && delta_x > 0.0)
}

fn weighted_scalar(weights: [f64; 3], values: [f32; 3]) -> f64 {
    (weights[0] * f64::from(values[0]) + weights[1] * f64::from(values[1]))
        + weights[2] * f64::from(values[2])
}

fn weighted_pair(weights: [f64; 3], values: [[f32; 2]; 3]) -> [f64; 2] {
    std::array::from_fn(|component| weighted_scalar(weights, values.map(|value| value[component])))
}

fn weighted_quad(weights: [f64; 3], values: [[f32; 4]; 3]) -> [f64; 4] {
    std::array::from_fn(|component| weighted_scalar(weights, values.map(|value| value[component])))
}

fn sample_point_clamped(texture: TextureView<'_>, uv: [f32; 2]) -> u8 {
    let [width, height] = texture.extent.map(usize::from);
    let u = uv[0].clamp(0.0, 1.0);
    let v = uv[1].clamp(0.0, 1.0);
    let x = ((u * width as f32).floor() as usize).min(width - 1);
    let y = ((v * height as f32).floor() as usize).min(height - 1);
    texture.indices[y * width + x]
}

fn decode_matrix(matrix: C3dMatrixWords) -> [f32; 16] {
    matrix.words().map(f32::from_bits)
}

fn multiply_row_vector(vector: [f32; 4], matrix: [f32; 16]) -> [f32; 4] {
    std::array::from_fn(|column| {
        let first = vector[0] * matrix[column];
        let second = vector[1] * matrix[4 + column];
        let third = vector[2] * matrix[8 + column];
        let fourth = vector[3] * matrix[12 + column];
        ((first + second) + third) + fourth
    })
}

fn interpolate_array<const N: usize>(left: [f32; N], right: [f32; N], amount: f32) -> [f32; N] {
    std::array::from_fn(|index| left[index] + (right[index] - left[index]) * amount)
}

fn unpack_argb(value: u32) -> [f32; 4] {
    [
        ((value >> 24) & 0xff) as f32 / 255.0,
        ((value >> 16) & 0xff) as f32 / 255.0,
        ((value >> 8) & 0xff) as f32 / 255.0,
        (value & 0xff) as f32 / 255.0,
    ]
}

#[derive(Clone, Copy)]
struct RasterInputVertex {
    position: [f32; 3],
    diffuse: u32,
    specular: u32,
    uv: [f32; 2],
}

impl RasterInputVertex {
    fn into_clip_vertex(self) -> SoftwareC3dClipVertex {
        SoftwareC3dClipVertex {
            clip_position: [self.position[0], self.position[1], self.position[2], 1.0],
            diffuse_argb: unpack_argb(self.diffuse),
            specular_argb: unpack_argb(self.specular),
            uv: self.uv,
        }
    }
}

impl RasterInputVertex {
    fn from_type1_copy(
        vertex: C3dType1ResourceScaledVertex,
    ) -> Result<Self, SoftwareC3dRasterError> {
        if !vertex.normal().into_iter().all(f32::is_finite) {
            return Err(SoftwareC3dRasterError::NonFiniteInput);
        }
        Ok(Self {
            position: vertex.position(),
            // The geometry-only copy wrapper never exposes these carrier
            // values. Texture COPY ignores the lighting payload entirely.
            diffuse: 0,
            specular: 0,
            uv: vertex.uv(),
        })
    }

    fn from_type2(vertex: C3dType2ResourceScaledVertex) -> Self {
        let raw = vertex.vertex();
        Self {
            position: raw.first_triplet(),
            diffuse: vertex.diffuse(),
            specular: vertex.specular(),
            uv: raw.adjusted_pair(),
        }
    }

    fn from_type2_pre_draw(vertex: C3dType2PreDrawVertex) -> Self {
        let words = vertex.words();
        Self {
            position: [words[0], words[1], words[2]].map(f32::from_bits),
            diffuse: words[4],
            specular: words[5],
            uv: [words[6], words[7]].map(f32::from_bits),
        }
    }

    /// Builds a texture-COPY vertex from a raw 32-byte C3D entry (one prim vertex
    /// of a [`cmr_core::c3d_mesh::MeshLodDrawGroup`]). Position and UV are the
    /// Q16.16 components dequantized in model space (`component / 65536`); the
    /// node world transform is applied later through the WORLD slot of the
    /// transform state, so the vertex is not pre-transformed here. Diffuse and
    /// specular are zero, as in [`Self::from_type1_copy`] - the COPY combiner
    /// ignores the lighting payload, and the entry's normal words are not colors.
    fn from_mesh_entry(entry: C3dEntry32) -> Self {
        const Q16_SCALE: f32 = 1.0 / 65536.0;
        Self {
            position: entry
                .position_q16()
                .map(|component| component as f32 * Q16_SCALE),
            diffuse: 0,
            specular: 0,
            uv: cmr_core::c3d_render::stage_texture_uv(entry.uv_q16()),
        }
    }
}

#[derive(Clone, Copy)]
struct DecodedViewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    clip_x: f32,
    clip_y: f32,
    clip_width: f32,
    clip_height: f32,
    min_z: f32,
    max_z: f32,
}

impl DecodedViewport {
    fn decode(viewport: C3dViewportWords) -> Result<Self, SoftwareC3dRasterError> {
        let words = viewport.words();
        let decoded = Self {
            x: words[1] as f32,
            y: words[2] as f32,
            width: words[3] as f32,
            height: words[4] as f32,
            clip_x: f32::from_bits(words[5]),
            clip_y: f32::from_bits(words[6]),
            clip_width: f32::from_bits(words[7]),
            clip_height: f32::from_bits(words[8]),
            min_z: f32::from_bits(words[9]),
            max_z: f32::from_bits(words[10]),
        };
        let finite = [
            decoded.x,
            decoded.y,
            decoded.width,
            decoded.height,
            decoded.clip_x,
            decoded.clip_y,
            decoded.clip_width,
            decoded.clip_height,
            decoded.min_z,
            decoded.max_z,
        ]
        .into_iter()
        .all(f32::is_finite);
        if !finite
            || decoded.width <= 0.0
            || decoded.height <= 0.0
            || decoded.clip_width <= 0.0
            || decoded.clip_height <= 0.0
            || decoded.max_z <= decoded.min_z
        {
            return Err(SoftwareC3dRasterError::InvalidViewport);
        }
        Ok(decoded)
    }
}

#[cfg(test)]
mod tests {
    use cmr_core::{
        c3d_type2_fog::{C3dType2FogState, prepare_c3d_type2_pre_draw_vertices},
        championship_setup::CanonicalCar,
        display::DisplayExtent,
        root_menu_c3d_resources::{
            C3dEntryMaterialPolicy, C3dLoadPolicy, C3dPreparedArenaBuilder,
            C3dResourceAllocationPolicy, C3dSourceIdentity, C3dSurfaceLayoutGates,
            FrontendC3dOwner,
        },
        root_menu_c3d_transform::{
            ROOT_MENU_C3D_PROJECTION, ROOT_MENU_C3D_VIEW, root_menu_c3d_viewport,
            root_menu_c3d_world,
        },
    };
    use cmr_formats::{
        bitmap::BitmapLimits,
        c3d::{
            C3D_FORMAT_VERSION, C3D_GROUP1_ENTRY32_LEN, C3D_GROUP1_FIXED_LEN, C3D_HEADER_LEN,
            C3D_MAGIC, C3dFile, C3dLimits, parse_c3d,
        },
    };

    use super::*;
    use crate::{
        legacy_c3d_pixel_format::{
            KnownRgbLayout, RawDdPixelFormat, SelectedNonAlphaTextureFormat,
        },
        packed_rgb_target::{FramebufferRgbFormat, LogicalMaskedRgbTarget},
        selected_texture_conversion_oracle::{
            SourceOwnedSelectedTextureCategory, SourceOwnedTextureConversionOraclePolicy,
            convert_selected_texture_source_owned_oracle,
        },
    };

    const TEXTURE_CONVERSION_POLICY: SourceOwnedTextureConversionOraclePolicy =
        SourceOwnedTextureConversionOraclePolicy::ExactPaletteOrNearestMaskNoDither;

    fn vertex(position: [f32; 4], uv: [f32; 2]) -> SoftwareC3dClipVertex {
        SoftwareC3dClipVertex {
            clip_position: position,
            diffuse_argb: [0.5, 1.0, 0.0, 0.0],
            specular_argb: [1.0, 0.0, 0.0, 0.0],
            uv,
        }
    }

    fn one_vertex_c3d(uv_q16: [i32; 2]) -> C3dFile {
        let record_len = C3D_GROUP1_FIXED_LEN + C3D_GROUP1_ENTRY32_LEN;
        let mut record = vec![0u8; record_len];
        record[12..16].copy_from_slice(&(record_len as u32).to_le_bytes());
        record[16..18].copy_from_slice(&1u16.to_le_bytes());
        for (index, component) in [0, 0, 0, uv_q16[0], uv_q16[1], 0, 0, 0]
            .into_iter()
            .enumerate()
        {
            let offset = C3D_GROUP1_FIXED_LEN + index * 4;
            record[offset..offset + 4].copy_from_slice(&component.to_le_bytes());
        }

        let mut bytes = vec![0u8; C3D_HEADER_LEN];
        bytes[0..4].copy_from_slice(&C3D_MAGIC);
        bytes[4..8].copy_from_slice(&C3D_FORMAT_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        bytes[14..16].copy_from_slice(&1u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        bytes[24..28].copy_from_slice(&(C3D_HEADER_LEN as u32).to_le_bytes());
        bytes[28..32].copy_from_slice(&((C3D_HEADER_LEN + record_len) as u32).to_le_bytes());
        bytes.extend_from_slice(&record);
        parse_c3d(&bytes, C3dLimits::default()).unwrap()
    }

    fn raw(position: [f32; 3], diffuse: u32, uv: [f32; 2]) -> RasterInputVertex {
        RasterInputVertex {
            position,
            diffuse,
            specular: 0xff00_0000,
            uv,
        }
    }

    fn screen(
        position: [f32; 3],
        reciprocal_w: f32,
        uv: [f32; 2],
        diffuse_argb: [f32; 4],
    ) -> SoftwareC3dScreenVertex {
        SoftwareC3dScreenVertex {
            position,
            reciprocal_w,
            uv_over_w: [uv[0] * reciprocal_w, uv[1] * reciprocal_w],
            diffuse_argb,
            specular_argb: [0.0; 4],
        }
    }

    fn prepared_batch(triangles: Vec<SoftwareC3dPreparedTriangle>) -> SoftwareC3dPreparedBatch {
        SoftwareC3dPreparedBatch {
            triangles: triangles.into_boxed_slice(),
            source_triangle_count: 0,
            fully_clipped_source_triangle_count: 0,
            counter_clockwise_triangle_count: 0,
            degenerate_triangle_count: 0,
        }
    }

    fn triangle(
        vertices: [SoftwareC3dScreenVertex; 3],
        flat_diffuse: u32,
    ) -> SoftwareC3dPreparedTriangle {
        SoftwareC3dPreparedTriangle {
            vertices,
            flat_diffuse,
        }
    }

    fn texture<'a>(extent: [u16; 2], indices: &'a [u8]) -> TextureView<'a> {
        TextureView {
            extent,
            indices,
            palette_len: 256,
        }
    }

    fn indexed_texture<'a>(
        extent: [u16; 2],
        indices: &'a [u8],
        palette: &'a [BitmapPaletteEntry],
    ) -> IndexedTextureView<'a> {
        IndexedTextureView {
            raster: TextureView {
                extent,
                indices,
                palette_len: palette.len(),
            },
            palette,
        }
    }

    fn sampled_texture(fill: u8) -> SoftwareC3dSampledTexture {
        sampled_texture_with_palette_entry(fill, [0; 3])
    }

    fn convert_palette_texture(
        source: &SoftwareC3dSampledTexture,
    ) -> SourceOwnedLogicalSelectedTexture {
        let descriptor = RawDdPixelFormat::from_dwords([32, 0x20, 0, 8, 0, 0, 0, 0]);
        convert_selected_texture_source_owned_oracle(
            source,
            SelectedNonAlphaTextureFormat::Palette8(descriptor),
            TEXTURE_CONVERSION_POLICY,
        )
        .unwrap()
    }

    fn convert_rgb555_texture(
        source: &SoftwareC3dSampledTexture,
    ) -> SourceOwnedLogicalSelectedTexture {
        let descriptor =
            RawDdPixelFormat::from_dwords([32, 0x40, 0, 16, 0x7c00, 0x03e0, 0x001f, 0]);
        convert_selected_texture_source_owned_oracle(
            source,
            SelectedNonAlphaTextureFormat::OpaqueRgb16 {
                descriptor,
                layout: KnownRgbLayout::Rgb555,
            },
            TEXTURE_CONVERSION_POLICY,
        )
        .unwrap()
    }

    fn framebuffer_rgb565() -> FramebufferRgbFormat {
        FramebufferRgbFormat::from_runtime_surface_descriptor(RawDdPixelFormat::from_dwords([
            32, 0x40, 0, 16, 0xf800, 0x07e0, 0x001f, 0,
        ]))
        .unwrap()
    }

    fn sampled_texture_with_palette_entry(
        fill: u8,
        [red, green, blue]: [u8; 3],
    ) -> SoftwareC3dSampledTexture {
        const WIDTH: usize = 4;
        const HEIGHT: usize = 4;
        const PIXEL_OFFSET: usize = 14 + 40 + 256 * 4;
        let mut bitmap = vec![0; PIXEL_OFFSET + WIDTH * HEIGHT];
        let bitmap_len = bitmap.len() as u32;
        bitmap[0..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&bitmap_len.to_le_bytes());
        bitmap[10..14].copy_from_slice(&(PIXEL_OFFSET as u32).to_le_bytes());
        bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&(WIDTH as i32).to_le_bytes());
        bitmap[22..26].copy_from_slice(&(HEIGHT as i32).to_le_bytes());
        bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&8u16.to_le_bytes());
        bitmap[34..38].copy_from_slice(&((WIDTH * HEIGHT) as u32).to_le_bytes());
        bitmap[46..50].copy_from_slice(&256u32.to_le_bytes());
        let palette_offset = 14 + 40 + usize::from(fill) * 4;
        bitmap[palette_offset..palette_offset + 4].copy_from_slice(&[blue, green, red, 0]);
        bitmap[PIXEL_OFFSET..].fill(fill);

        let mut bytes = vec![0; 32 + 16 + bitmap.len()];
        bytes[0..4].copy_from_slice(&C3D_MAGIC);
        bytes[4..8].copy_from_slice(&C3D_FORMAT_VERSION.to_le_bytes());
        bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
        for index in 0..3 {
            bytes[20 + index * 4..24 + index * 4].copy_from_slice(&32u32.to_le_bytes());
        }
        bytes[32..44].copy_from_slice(b"TEST-TEXTURE");
        bytes[44..48].copy_from_slice(&(bitmap.len() as u32).to_le_bytes());
        bytes[48..].copy_from_slice(&bitmap);

        let file = parse_c3d(&bytes, C3dLimits::default()).unwrap();
        let mut builder = C3dPreparedArenaBuilder::new().unwrap();
        let object = builder
            .prepare_object(
                C3dSourceIdentity {
                    owner: FrontendC3dOwner::ChampionshipCar(CanonicalCar::Subaru),
                    path: "synthetic-copy-texture.c3d".into(),
                },
                &file,
                C3dLoadPolicy {
                    mode: 8,
                    caller_option: 1,
                    caller_flags: 0x18,
                    surface_constructor_flags: 0x15,
                    entry_resource_count: 0,
                    allocation: C3dResourceAllocationPolicy::AppendEveryResource,
                    surface_layout: C3dSurfaceLayoutGates::default(),
                    material: C3dEntryMaterialPolicy::FixedUniformWord(0x9696_9696),
                },
                BitmapLimits::default(),
            )
            .unwrap();
        let id = object.ordinal_resources()[0];
        let arena = builder.finish();
        SoftwareC3dSampledTexture::from_prepared_frontend(arena.resource(id).unwrap()).unwrap()
    }

    fn sampled_varying_2x2_texture() -> SoftwareC3dSampledTexture {
        sampled_quadrant_texture([[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]])
    }

    fn sampled_quadrant_texture(palette: [[u8; 3]; 4]) -> SoftwareC3dSampledTexture {
        const WIDTH: usize = 8;
        const HEIGHT: usize = 8;
        const PIXEL_OFFSET: usize = 14 + 40 + 256 * 4;
        let mut bitmap = vec![0; PIXEL_OFFSET + WIDTH * HEIGHT];
        let bitmap_len = bitmap.len() as u32;
        bitmap[0..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&bitmap_len.to_le_bytes());
        bitmap[10..14].copy_from_slice(&(PIXEL_OFFSET as u32).to_le_bytes());
        bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&(WIDTH as i32).to_le_bytes());
        bitmap[22..26].copy_from_slice(&(HEIGHT as i32).to_le_bytes());
        bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&8u16.to_le_bytes());
        bitmap[34..38].copy_from_slice(&((WIDTH * HEIGHT) as u32).to_le_bytes());
        bitmap[46..50].copy_from_slice(&256u32.to_le_bytes());
        for (index, [red, green, blue]) in palette.into_iter().enumerate() {
            let offset = 14 + 40 + index * 4;
            bitmap[offset..offset + 4].copy_from_slice(&[blue, green, red, 0]);
        }
        // BMP rows are bottom-up. The normalized top-left 4x4 quadrant uses
        // index 0, top-right 1, bottom-left 2, and bottom-right 3.
        for disk_y in 0..HEIGHT {
            let normalized_y = HEIGHT - 1 - disk_y;
            for x in 0..WIDTH {
                let index = usize::from(normalized_y >= 4) * 2 + usize::from(x >= 4);
                bitmap[PIXEL_OFFSET + disk_y * WIDTH + x] = index as u8;
            }
        }

        let mut bytes = vec![0; 32 + 16 + bitmap.len()];
        bytes[0..4].copy_from_slice(&C3D_MAGIC);
        bytes[4..8].copy_from_slice(&C3D_FORMAT_VERSION.to_le_bytes());
        bytes[16..18].copy_from_slice(&1u16.to_le_bytes());
        for index in 0..3 {
            bytes[20 + index * 4..24 + index * 4].copy_from_slice(&32u32.to_le_bytes());
        }
        bytes[32..44].copy_from_slice(b"TEST-ORACLE!");
        bytes[44..48].copy_from_slice(&(bitmap.len() as u32).to_le_bytes());
        bytes[48..].copy_from_slice(&bitmap);

        let file = parse_c3d(&bytes, C3dLimits::default()).unwrap();
        let mut builder = C3dPreparedArenaBuilder::new().unwrap();
        let object = builder
            .prepare_object(
                C3dSourceIdentity {
                    owner: FrontendC3dOwner::ChampionshipCar(CanonicalCar::Subaru),
                    path: "synthetic-oracle-filter-texture.c3d".into(),
                },
                &file,
                C3dLoadPolicy {
                    mode: 8,
                    caller_option: 1,
                    caller_flags: 0x18,
                    surface_constructor_flags: 0x15,
                    entry_resource_count: 0,
                    allocation: C3dResourceAllocationPolicy::AppendEveryResource,
                    surface_layout: C3dSurfaceLayoutGates {
                        force_quarter: true,
                        ..C3dSurfaceLayoutGates::default()
                    },
                    material: C3dEntryMaterialPolicy::FixedUniformWord(0x9696_9696),
                },
                BitmapLimits::default(),
            )
            .unwrap();
        let id = object.ordinal_resources()[0];
        let arena = builder.finish();
        let sampled =
            SoftwareC3dSampledTexture::from_prepared_frontend(arena.resource(id).unwrap()).unwrap();
        assert_eq!(sampled.extent(), [2, 2]);
        assert_eq!(sampled.indices(), [0, 1, 2, 3]);
        sampled
    }

    fn identity() -> C3dMatrixWords {
        ROOT_MENU_C3D_VIEW
    }

    fn viewport() -> C3dViewportWords {
        root_menu_c3d_viewport(DisplayExtent {
            width: 640,
            height: 480,
        })
        .unwrap()
    }

    fn identity_state() -> SoftwareC3dTransformState {
        SoftwareC3dTransformState::new(identity(), identity(), identity(), viewport())
    }

    #[test]
    fn gameplay_transform_state_composes_the_recovered_matrices() {
        let identity_q16 = cmr_core::c3d_camera::MATRIX_IDENTITY_Q16;
        // A camera translated to (2, 3, 5), identity node.
        let mut camera_world = identity_q16;
        camera_world[12] = 2 << 16;
        camera_world[13] = 3 << 16;
        camera_world[14] = 5 << 16;
        let state =
            SoftwareC3dTransformState::for_gameplay_scene(640, 480, &camera_world, &identity_q16)
                .unwrap();
        let (w, h, zn, zf) = cmr_core::c3d_camera::gameplay_projection_default(640, 480);
        assert_eq!(
            state.world(),
            cmr_core::c3d_camera::world_matrix_words(&identity_q16)
        );
        assert_eq!(
            state.view(),
            cmr_core::c3d_camera::view_matrix_words(&camera_world)
        );
        assert_eq!(
            state.projection(),
            cmr_core::c3d_camera::projection_matrix_words(w, h, zn, zf)
        );
        assert_eq!(
            state.viewport(),
            cmr_core::c3d_camera::viewport_words(640, 480).unwrap()
        );
        // An unrepresentable viewport extent is rejected.
        assert!(
            SoftwareC3dTransformState::for_gameplay_scene(0, 480, &identity_q16, &identity_q16)
                .is_err()
        );
    }

    fn alternate_profile() -> SoftwareChampionshipCarSplitOracleProfile {
        SoftwareChampionshipCarSplitOracleProfile::new(
            SoftwareC3dSplitRenderDepth::Bits32,
            false,
            false,
        )
    }

    fn lit_pixel_batch(diffuse: [[f32; 4]; 3]) -> SoftwareC3dPreparedLitBatch {
        SoftwareC3dPreparedLitBatch(prepared_batch(vec![triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.5, 0.5], diffuse[0]),
                screen([2.0, 0.0, 0.5], 1.0, [0.5, 0.5], diffuse[1]),
                screen([0.0, 2.0, 0.5], 1.0, [0.5, 0.5], diffuse[2]),
            ],
            0,
        )]))
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 0.000_2,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn root_origin_uses_row_vectors_and_viewport_clip_aspect() {
        let transformed = transform_vertex(
            vertex([0.0, 0.0, 0.0, 1.0], [0.25, 0.75]),
            root_menu_c3d_world(0),
            ROOT_MENU_C3D_VIEW,
            ROOT_MENU_C3D_PROJECTION,
            viewport(),
        )
        .unwrap();
        let screen = project_root_c3d_polygon(&[transformed], viewport()).unwrap()[0];

        assert_near(screen.position()[0], 329.599_6);
        assert_near(screen.position()[1], 237.907_5);
        assert_near(screen.position()[2], 0.950_951);
        assert_near(screen.reciprocal_w(), 0.05);
        assert_eq!(screen.uv_over_w(), [0.0125, 0.0375]);
    }

    #[test]
    fn viewport_clip_y_extent_maps_to_top_and_bottom() {
        let aspect: f32 = 640.0 / 480.0;
        let top = transform_vertex(
            vertex([0.0, aspect, 0.5, 1.0], [0.0, 0.0]),
            identity(),
            identity(),
            identity(),
            viewport(),
        )
        .unwrap();
        let bottom = transform_vertex(
            vertex([0.0, -aspect, 0.5, 1.0], [0.0, 0.0]),
            identity(),
            identity(),
            identity(),
            viewport(),
        )
        .unwrap();
        let projected = project_root_c3d_polygon(&[top, bottom], viewport()).unwrap();

        assert_near(top.clip_position()[1], 1.0);
        assert_near(bottom.clip_position()[1], -1.0);
        assert_near(projected[0].position()[1], 0.0);
        assert_near(projected[1].position()[1], 480.0);
    }

    #[test]
    fn clips_each_homogeneous_plane_and_interpolates_attributes() {
        let inside_a = vertex([0.0, 0.0, 0.5, 1.0], [0.0, 0.0]);
        let inside_b = vertex([0.5, 0.0, 0.5, 1.0], [1.0, 0.0]);
        let outside_positions = [
            [-2.0, 0.0, 0.5, 1.0],
            [2.0, 0.0, 0.5, 1.0],
            [0.0, -2.0, 0.5, 1.0],
            [0.0, 2.0, 0.5, 1.0],
            [0.0, 0.0, -1.0, 1.0],
            [0.0, 0.0, 2.0, 1.0],
        ];

        for (plane, outside_position) in outside_positions.into_iter().enumerate() {
            let outside = vertex(outside_position, [0.5, 1.0]);
            let polygon = clip_root_c3d_triangle([inside_a, inside_b, outside]).unwrap();
            assert_eq!(polygon.len(), 4, "plane {plane}");
            assert!(polygon.iter().all(|candidate| {
                (0..CLIP_PLANE_COUNT).all(|candidate_plane| {
                    clip_distance(candidate.clip_position(), candidate_plane) >= 0.0
                })
            }));
            assert!(polygon.iter().any(|candidate| {
                clip_distance(candidate.clip_position(), plane).abs() <= f32::EPSILON
                    && candidate.uv()[1] > 0.0
                    && candidate.uv()[1] < 1.0
            }));
        }
    }

    #[test]
    fn fully_outside_triangle_is_rejected_and_boundary_is_closed() {
        let outside = clip_root_c3d_triangle([
            vertex([-3.0, 0.0, 0.5, 1.0], [0.0, 0.0]),
            vertex([-2.0, 0.5, 0.5, 1.0], [0.0, 0.0]),
            vertex([-2.0, -0.5, 0.5, 1.0], [0.0, 0.0]),
        ])
        .unwrap();
        assert!(outside.is_empty());

        let boundary = [
            vertex([-1.0, 0.0, 0.0, 1.0], [0.0, 0.0]),
            vertex([1.0, 0.0, 1.0, 1.0], [1.0, 0.0]),
            vertex([0.0, 1.0, 0.5, 1.0], [0.5, 1.0]),
        ];
        assert_eq!(clip_root_c3d_triangle(boundary).unwrap(), boundary);
    }

    #[test]
    fn projection_retains_perspective_interpolation_terms() {
        let projected = project_root_c3d_polygon(
            &[
                vertex([0.0, 0.0, 0.5, 1.0], [0.0, 1.0]),
                vertex([0.0, 0.0, 1.0, 2.0], [1.0, 0.5]),
            ],
            viewport(),
        )
        .unwrap();

        assert_eq!(projected[0].reciprocal_w(), 1.0);
        assert_eq!(projected[0].uv_over_w(), [0.0, 1.0]);
        assert_eq!(projected[1].reciprocal_w(), 0.5);
        assert_eq!(projected[1].uv_over_w(), [0.5, 0.25]);
    }

    #[test]
    fn clips_in_homogeneous_space_before_divide() {
        let inside = vertex([0.0, 0.0, 0.5, 1.0], [0.0, 0.0]);
        let outside = vertex([-3.0, 1.0, 1.0, 2.0], [1.0, 1.0]);
        let polygon = clip_root_c3d_triangle([outside, inside, inside]).unwrap();
        let generated = polygon
            .iter()
            .copied()
            .find(|candidate| {
                let [x, _, _, w] = candidate.clip_position();
                x == -w && candidate.uv() == [0.5, 0.5]
            })
            .unwrap();
        let projected = project_root_c3d_polygon(&[generated], viewport()).unwrap()[0];

        assert_eq!(generated.clip_position(), [-1.5, 0.5, 0.75, 1.5]);
        assert_near(projected.reciprocal_w(), 2.0 / 3.0);
        assert_near(projected.uv_over_w()[0], 1.0 / 3.0);
        assert_near(projected.uv_over_w()[1], 1.0 / 3.0);
    }

    #[test]
    fn indexed_preflight_fans_clipped_polygon_and_preserves_flat_source_color() {
        let aspect = 640.0 / 480.0;
        let vertices = [
            raw([-1.5, -0.5 * aspect, 0.5], 0x9611_2233, [0.0, 0.0]),
            raw([0.0, 0.5 * aspect, 0.5], 0x9644_5566, [0.5, 1.0]),
            raw([0.5, -0.5 * aspect, 0.5], 0x9677_8899, [1.0, 0.0]),
        ];

        let batch =
            preflight_indexed_triangle_list(&vertices, &[0, 1, 2], identity_state()).unwrap();

        assert_eq!(batch.source_triangle_count(), 1);
        assert_eq!(batch.triangles().len(), 2);
        assert_eq!(batch.fully_clipped_source_triangle_count(), 0);
        assert_eq!(batch.counter_clockwise_triangle_count(), 0);
        assert_eq!(batch.degenerate_triangle_count(), 0);
        assert!(
            batch
                .triangles()
                .iter()
                .all(|triangle| triangle.flat_diffuse() == 0x9611_2233)
        );
    }

    #[test]
    fn indexed_preflight_culls_screen_counter_clockwise_and_discards_degenerate() {
        let aspect = 640.0 / 480.0;
        let vertices = [
            raw([-0.5, -0.5 * aspect, 0.5], 1, [0.0, 0.0]),
            raw([0.0, 0.5 * aspect, 0.5], 2, [0.5, 1.0]),
            raw([0.5, -0.5 * aspect, 0.5], 3, [1.0, 0.0]),
        ];
        let indices = [0, 1, 2, 0, 2, 1, 0, 0, 0];

        let batch = preflight_indexed_triangle_list(&vertices, &indices, identity_state()).unwrap();

        assert_eq!(batch.source_triangle_count(), 3);
        assert_eq!(batch.triangles().len(), 1);
        assert_eq!(batch.counter_clockwise_triangle_count(), 1);
        assert_eq!(batch.degenerate_triangle_count(), 1);
    }

    #[test]
    fn boundary_crossing_does_not_create_a_duplicate_fan_vertex() {
        let aspect = 640.0 / 480.0;
        let mut outside = vertex([-2.0, 1.0, 0.5, 1.0], [1.0, 1.0]);
        outside.diffuse_argb = unpack_argb(0xff01_0000);
        let mut boundary = vertex([-1.0, 0.0, 0.5, 1.0], [f32::MIN_POSITIVE, 0.123]);
        boundary.diffuse_argb = unpack_argb(0xff03_0000);
        let inside = vertex([0.0, -0.5, 0.5, 1.0], [0.5, 0.0]);
        let clipped = clip_root_c3d_triangle([outside, boundary, inside]).unwrap();
        assert_eq!(clipped.len(), 3);
        assert_eq!(clipped[1], boundary);

        let vertices = [
            raw([-1.0, 0.0, 0.5], 1, [0.0, 0.0]),
            raw([-2.0, aspect, 0.5], 2, [0.0, 1.0]),
            raw([0.0, -0.5 * aspect, 0.5], 3, [1.0, 0.0]),
        ];

        let batch =
            preflight_indexed_triangle_list(&vertices, &[0, 1, 2], identity_state()).unwrap();

        assert_eq!(batch.triangles().len(), 1);
        assert_eq!(batch.degenerate_triangle_count(), 0);
        assert_eq!(batch.counter_clockwise_triangle_count(), 0);
    }

    #[test]
    fn indexed_preflight_reports_malformed_indices_without_panicking() {
        let empty = preflight_indexed_triangle_list(&[], &[], identity_state()).unwrap();
        assert!(empty.triangles().is_empty());
        assert_eq!(empty.source_triangle_count(), 0);
        assert_eq!(empty.fully_clipped_source_triangle_count(), 0);
        assert_eq!(empty.counter_clockwise_triangle_count(), 0);
        assert_eq!(empty.degenerate_triangle_count(), 0);

        let one = [raw([0.0, 0.0, 0.5], 0, [0.0, 0.0])];
        assert_eq!(
            preflight_indexed_triangle_list(&one, &[0, 0], identity_state()),
            Err(SoftwareC3dRasterError::IncompleteIndexTriplet { index_count: 2 })
        );
        assert_eq!(
            preflight_indexed_triangle_list(&one, &[0, 1, 0], identity_state()),
            Err(SoftwareC3dRasterError::IndexOutOfRange {
                triangle: 0,
                corner: 1,
                index: 1,
                vertex_count: 1,
            })
        );
        assert_eq!(
            preflight_indexed_triangle_list(&[], &[0, 0, 0], identity_state()),
            Err(SoftwareC3dRasterError::IndexOutOfRange {
                triangle: 0,
                corner: 0,
                index: 0,
                vertex_count: 0,
            })
        );
    }

    #[test]
    fn non_finite_inputs_and_nonpositive_w_are_rejected() {
        let error = transform_vertex(
            vertex([f32::NAN, 0.0, 0.0, 1.0], [0.0, 0.0]),
            identity(),
            identity(),
            identity(),
            viewport(),
        )
        .unwrap_err();
        assert_eq!(error, SoftwareC3dRasterError::NonFiniteInput);

        for w in [0.0, -1.0] {
            let error =
                project_root_c3d_polygon(&[vertex([0.0, 0.0, 0.0, w], [0.0, 0.0])], viewport())
                    .unwrap_err();
            assert_eq!(error, SoftwareC3dRasterError::NonPositiveHomogeneousW);
        }
    }

    #[test]
    fn top_left_pixel_centers_give_shared_edges_exactly_one_owner() {
        let color = [1.0, 0.0, 0.0, 0.0];
        let a = screen([0.5, 0.5, 0.5], 1.0, [0.0, 0.0], color);
        let b = screen([2.5, 0.5, 0.5], 1.0, [0.0, 0.0], color);
        let c = screen([0.5, 2.5, 0.5], 1.0, [0.0, 0.0], color);
        let d = screen([2.5, 2.5, 0.5], 1.0, [0.0, 0.0], color);
        let batch = prepared_batch(vec![
            triangle([a, b, c], 0x1111_1111),
            triangle([b, d, c], 0x2222_2222),
        ]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([3, 3]).unwrap();
        let mut fragments = Vec::new();

        let outcome = rasterize_fragments(&batch, texture([1, 1], &[0]), &mut depth, |fragment| {
            fragments.push(fragment)
        })
        .unwrap();

        assert_eq!(outcome.covered(), 4);
        assert_eq!(outcome.depth_passed(), 4);
        assert_eq!(outcome.depth_rejected(), 0);
        assert!(
            fragments
                .iter()
                .all(|fragment| fragment.texture_index() == 0)
        );
        assert_eq!(
            fragments
                .iter()
                .map(|fragment| (fragment.position(), fragment.flat_diffuse()))
                .collect::<Vec<_>>(),
            [
                ([0, 0], 0x1111_1111),
                ([1, 0], 0x1111_1111),
                ([0, 1], 0x1111_1111),
                ([1, 1], 0x2222_2222),
            ]
        );
    }

    #[test]
    fn reversed_fractional_edges_are_exact_complements_without_cracks_or_overlap() {
        let fixtures = [
            (
                [33.293_736, 0.688_705_3],
                [33.706_264, 0.311_294_7],
                [34.0, 1.0],
                [33.0, 0.0],
                [33, 0],
            ),
            (
                [64.580_11, 0.162_179_86],
                [64.419_89, 0.837_820_2],
                [64.0, 1.0],
                [65.0, 0.0],
                [64, 0],
            ),
        ];

        for (a, b, first_outer, second_outer, target) in fixtures {
            let sample = [f32::from(target[0]) + 0.5, f32::from(target[1]) + 0.5];
            let a3 = [a[0], a[1], 0.5];
            let b3 = [b[0], b[1], 0.5];
            assert_eq!(edge(a3, b3, sample), -edge(b3, a3, sample));

            let make_vertex =
                |point: [f32; 2]| screen([point[0], point[1], 0.5], 1.0, [0.0, 0.0], [0.0; 4]);
            let batch = prepared_batch(vec![
                triangle(
                    [make_vertex(a), make_vertex(b), make_vertex(first_outer)],
                    1,
                ),
                triangle(
                    [make_vertex(b), make_vertex(a), make_vertex(second_outer)],
                    2,
                ),
            ]);
            let mut depth = SoftwareC3dDepthBuffer::new_far([66, 2]).unwrap();
            let mut owners = Vec::new();
            rasterize_fragments(&batch, texture([1, 1], &[0]), &mut depth, |fragment| {
                if fragment.position() == target {
                    owners.push(fragment.flat_diffuse());
                }
            })
            .unwrap();

            assert_eq!(owners.len(), 1, "shared sample {sample:?}: {owners:?}");
        }
    }

    #[test]
    fn preflight_and_fragment_validation_share_the_canonical_area_predicate() {
        let points = [
            [782.551_15, 846.957_7, 0.5],
            [-67.476_28, 277.881_74, 0.5],
            [507.540_13, 662.843_44, 0.5],
        ];
        let [a, b, c] = points;
        let differently_ordered = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert_eq!(differently_ordered, 0.015_625);

        let vertices = points.map(|position| screen(position, 1.0, [0.0, 0.0], [0.0; 4]));
        assert_eq!(signed_screen_area(vertices).to_bits(), (-0.0_f32).to_bits());
        let batch = prepared_batch(vec![triangle(vertices, 0)]);
        assert_eq!(
            validate_prepared_triangles(&batch),
            Err(SoftwareC3dRasterError::InvalidPreparedTriangle { triangle: 0 })
        );
    }

    #[test]
    fn fragment_interpolation_is_perspective_for_uv_and_affine_for_depth() {
        let batch = prepared_batch(vec![triangle(
            [
                screen([1.0, 1.0, 1.0 / 8.0], 1.0, [0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
                screen([5.0, 1.0, 5.0 / 8.0], 0.5, [1.0, 0.0], [0.0, 1.0, 0.0, 0.0]),
                screen(
                    [1.0, 5.0, 7.0 / 8.0],
                    0.25,
                    [0.0, 1.0],
                    [0.0, 0.0, 1.0, 0.0],
                ),
            ],
            0x9600_0000,
        )]);
        let indices = (0..16).collect::<Vec<_>>();
        let mut depth = SoftwareC3dDepthBuffer::new_far([6, 6]).unwrap();
        let mut selected = None;

        rasterize_fragments(&batch, texture([4, 4], &indices), &mut depth, |fragment| {
            if fragment.position() == [2, 2] {
                selected = Some(fragment);
            }
        })
        .unwrap();
        let fragment = selected.unwrap();

        assert_near(fragment.depth(), 19.0 / 32.0);
        assert_near(fragment.uv()[0], 6.0 / 17.0);
        assert_near(fragment.uv()[1], 3.0 / 17.0);
        assert_eq!(fragment.texture_index(), 1);
        assert_near(fragment.diffuse_argb()[0], 0.25);
        assert_near(fragment.diffuse_argb()[1], 0.375);
        assert_near(fragment.diffuse_argb()[2], 0.375);
    }

    #[test]
    fn mesh_entry_uses_the_shared_stage_uv_preprocessing() {
        let file = one_vertex_c3d([0x1_0001, -0x8000]);
        let entry = file
            .group1_records()
            .next()
            .unwrap()
            .entries_32()
            .next()
            .unwrap();
        let vertex = RasterInputVertex::from_mesh_entry(entry);
        assert_eq!(
            vertex.uv,
            cmr_core::c3d_render::stage_texture_uv([0x1_0001, -0x8000])
        );
    }

    #[test]
    fn corrected_stage_uvs_cover_a_quadrant_texture_without_an_internal_crack() {
        let uv00 = cmr_core::c3d_render::stage_texture_uv([0, -0x1_0000]);
        let uv10 = cmr_core::c3d_render::stage_texture_uv([0x1_0000, -0x1_0000]);
        let uv01 = cmr_core::c3d_render::stage_texture_uv([0, 0]);
        let uv11 = cmr_core::c3d_render::stage_texture_uv([0x1_0000, 0]);
        let at = |x, y, uv| screen([x, y, 0.5], 1.0, uv, [0.0; 4]);
        let batch = prepared_batch(vec![
            triangle(
                [at(0.0, 0.0, uv00), at(4.0, 0.0, uv10), at(0.0, 4.0, uv01)],
                0,
            ),
            triangle(
                [at(4.0, 0.0, uv10), at(4.0, 4.0, uv11), at(0.0, 4.0, uv01)],
                0,
            ),
        ]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([4, 4]).unwrap();
        let mut sampled = [u8::MAX; 16];

        rasterize_fragments(
            &batch,
            texture([2, 2], &[10, 11, 20, 21]),
            &mut depth,
            |fragment| {
                let [x, y] = fragment.position();
                sampled[usize::from(y) * 4 + usize::from(x)] = fragment.texture_index();
            },
        )
        .unwrap();

        assert_eq!(
            sampled,
            [
                10, 10, 11, 11, 10, 10, 11, 11, 20, 20, 21, 21, 20, 20, 21, 21
            ]
        );
    }

    #[test]
    fn finite_extreme_interpolation_stays_finite_before_clamping() {
        let mut vertices = [
            screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], [1.0; 4]),
            screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], [1.0; 4]),
            screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], [1.0; 4]),
        ];
        for vertex in &mut vertices {
            vertex.uv_over_w = [f32::MAX; 2];
        }
        let batch = prepared_batch(vec![triangle(vertices, 0)]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut selected = None;

        rasterize_fragments(&batch, texture([1, 1], &[0]), &mut depth, |fragment| {
            selected = Some(fragment);
        })
        .unwrap();

        let fragment = selected.unwrap();
        assert_eq!(fragment.uv(), [1.0, 1.0]);
        assert!(fragment.uv().into_iter().all(f32::is_finite));
        assert!(fragment.diffuse_argb().into_iter().all(f32::is_finite));
    }

    #[test]
    fn indexed_copy_palette_map_uses_full_l1_lowest_ties_and_ignores_flags() {
        let mut active = [PaletteEntry {
            red: 250,
            green: 250,
            blue: 250,
            flags: 0,
        }; 256];
        active[0] = PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 1,
        };
        active[1] = PaletteEntry {
            red: 2,
            green: 0,
            blue: 0,
            flags: 2,
        };
        active[2] = PaletteEntry {
            red: 10,
            green: 20,
            blue: 30,
            flags: 3,
        };
        active[3] = PaletteEntry {
            flags: 4,
            ..active[2]
        };
        active[255] = PaletteEntry {
            red: 254,
            green: 255,
            blue: 255,
            flags: 255,
        };
        let texture_palette = [
            BitmapPaletteEntry {
                red: 1,
                green: 0,
                blue: 0,
                reserved: 255,
            },
            BitmapPaletteEntry {
                red: 10,
                green: 20,
                blue: 30,
                reserved: 127,
            },
            BitmapPaletteEntry {
                red: 255,
                green: 255,
                blue: 255,
                reserved: 99,
            },
        ];

        assert_eq!(
            full_palette_manhattan_map(&texture_palette, &active),
            [0, 2, 255]
        );

        for (index, entry) in active.iter_mut().enumerate() {
            entry.flags = index as u8;
        }
        assert_eq!(
            full_palette_manhattan_map(&texture_palette, &active),
            [0, 2, 255]
        );
    }

    #[test]
    fn indexed_copy_writes_index_zero_sample_and_preserves_uncovered_pixels() {
        let color = [1.0, 0.25, 0.5, 0.75];
        let batch = prepared_batch(vec![triangle(
            [
                screen([1.0, 1.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([3.0, 1.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([1.0, 3.0, 0.5], 1.0, [0.0, 0.0], color),
            ],
            0x0102_0304,
        )]);
        let texture_palette = [BitmapPaletteEntry {
            red: 12,
            green: 34,
            blue: 56,
            reserved: 0,
        }];
        let mut active = [PaletteEntry {
            red: 255,
            green: 255,
            blue: 255,
            flags: 0,
        }; 256];
        active[7] = PaletteEntry {
            red: 12,
            green: 34,
            blue: 56,
            flags: 200,
        };
        let mut depth = SoftwareC3dDepthBuffer::new_far([3, 3]).unwrap();
        let mut target = [99; 9];

        let outcome = compose_indexed_copy_no_blend(
            &batch,
            indexed_texture([1, 1], &[0], &texture_palette),
            &mut depth,
            [3, 3],
            &mut target,
            &active,
            SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
        )
        .unwrap();

        assert_eq!(outcome.covered(), 1);
        assert_eq!(outcome.depth_passed(), 1);
        assert_eq!(target, [99, 99, 99, 99, 7, 99, 99, 99, 99]);
        assert_eq!(
            depth.values(),
            [1.0, 1.0, 1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 1.0]
        );

        let changed_color = [0.0, 1.0, 0.0, 0.0];
        let changed_diffuse = prepared_batch(vec![triangle(
            [
                screen([1.0, 1.0, 0.5], 1.0, [0.0, 0.0], changed_color),
                screen([3.0, 1.0, 0.5], 1.0, [0.0, 0.0], changed_color),
                screen([1.0, 3.0, 0.5], 1.0, [0.0, 0.0], changed_color),
            ],
            0xfedc_ba98,
        )]);
        depth.clear_far();
        target.fill(99);
        compose_indexed_copy_no_blend(
            &changed_diffuse,
            indexed_texture([1, 1], &[0], &texture_palette),
            &mut depth,
            [3, 3],
            &mut target,
            &active,
            SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
        )
        .unwrap();
        assert_eq!(target, [99, 99, 99, 99, 7, 99, 99, 99, 99]);

        active[0] = active[7];
        active[7] = PaletteEntry {
            red: 255,
            green: 255,
            blue: 255,
            flags: 0,
        };
        depth.clear_far();
        target.fill(99);
        compose_indexed_copy_no_blend(
            &changed_diffuse,
            indexed_texture([1, 1], &[0], &texture_palette),
            &mut depth,
            [3, 3],
            &mut target,
            &active,
            SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
        )
        .unwrap();
        assert_eq!(target, [99, 99, 99, 99, 0, 99, 99, 99, 99]);
    }

    #[test]
    fn indexed_color_key_skips_both_color_and_depth_writes() {
        let batch = prepared_batch(vec![triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
            ],
            0,
        )]);
        let texture_palette = [BitmapPaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            reserved: 0,
        }];
        let active = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [77];

        let outcome = compose_indexed_copy_no_blend_with_color_key(
            &batch,
            indexed_texture([1, 1], &[0], &texture_palette),
            &mut depth,
            [1, 1],
            &mut target,
            &active,
            IndexedCopyOptions {
                quantization: SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
                color_key: Some(0),
            },
        )
        .unwrap();

        assert_eq!(outcome.covered(), 1);
        assert_eq!(outcome.depth_passed(), 0);
        assert_eq!(target, [77]);
        assert_eq!(depth.values(), [1.0]);
    }

    #[test]
    fn championship_copy_preserves_texture_index_without_palette_quantization() {
        let color = [0.1, 0.2, 0.3, 0.4];
        let batch = prepared_batch(vec![triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], color),
            ],
            0x9696_9696,
        )]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [3];

        let outcome = compose_championship_car_indexed_copy_view(
            &batch,
            texture([1, 1], &[217]),
            &mut depth,
            [1, 1],
            &mut target,
            SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
        )
        .unwrap();

        assert_eq!(outcome.depth_passed(), 1);
        assert_eq!(target, [217]);
        assert_eq!(depth.values(), [0.5]);
    }

    #[test]
    fn public_championship_present_resource_copies_index_zero_as_ordinary_data() {
        let batch = SoftwareC3dPreparedCopyBatch(prepared_batch(vec![triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
            ],
            0,
        )]));
        let texture = sampled_texture(0);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [91];

        let outcome = compose_championship_car_indexed_copy_no_blend(
            &batch,
            SoftwareChampionshipCarIndexedSource::UnifiedTexture(&texture),
            &mut depth,
            [1, 1],
            &mut target,
            SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
        )
        .unwrap();

        assert_eq!(outcome.depth_passed(), 1);
        assert_eq!(target, [0]);
        assert_eq!(depth.values(), [0.5]);
    }

    #[test]
    fn public_championship_copy_ignores_type2_diffuse_and_specular_payloads() {
        let aspect = 640.0 / 480.0;
        let batch = |diffuse, specular| {
            let vertices = [
                RasterInputVertex {
                    specular,
                    ..raw([-0.5, -0.5 * aspect, 0.5], diffuse, [0.0, 0.0])
                },
                RasterInputVertex {
                    specular,
                    ..raw([0.0, 0.5 * aspect, 0.5], diffuse, [0.0, 0.0])
                },
                RasterInputVertex {
                    specular,
                    ..raw([0.5, -0.5 * aspect, 0.5], diffuse, [0.0, 0.0])
                },
            ];
            SoftwareC3dPreparedCopyBatch(
                preflight_indexed_triangle_list(&vertices, &[0, 1, 2], identity_state()).unwrap(),
            )
        };
        let texture = sampled_texture(217);
        let profile = SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND;
        let mut first_depth = SoftwareC3dDepthBuffer::new_far([640, 480]).unwrap();
        let mut second_depth = SoftwareC3dDepthBuffer::new_far([640, 480]).unwrap();
        let mut first_target = vec![11; 640 * 480];
        let mut second_target = first_target.clone();

        compose_championship_car_indexed_copy_no_blend(
            &batch(0x0000_0000, 0xffff_ffff),
            SoftwareChampionshipCarIndexedSource::UnifiedTexture(&texture),
            &mut first_depth,
            [640, 480],
            &mut first_target,
            profile,
        )
        .unwrap();
        compose_championship_car_indexed_copy_no_blend(
            &batch(0xffff_ffff, 0x0000_0000),
            SoftwareChampionshipCarIndexedSource::UnifiedTexture(&texture),
            &mut second_depth,
            [640, 480],
            &mut second_target,
            profile,
        )
        .unwrap();

        assert!(first_target.contains(&217));
        assert_eq!(first_target, second_target);
        assert_eq!(first_depth.values(), second_depth.values());
    }

    #[test]
    fn public_championship_missing_source_writes_zero_ignores_diffuse_and_obeys_depth() {
        let batch_at = |depth_value, diffuse| {
            prepared_batch(vec![triangle(
                [
                    screen([0.0, 0.0, depth_value], 1.0, [0.0, 0.0], diffuse),
                    screen([2.0, 0.0, depth_value], 1.0, [0.0, 0.0], diffuse),
                    screen([0.0, 2.0, depth_value], 1.0, [0.0, 0.0], diffuse),
                ],
                0xff00_0000,
            )])
        };
        let profile = SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND;
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [91];

        compose_championship_car_indexed_copy_no_blend(
            &SoftwareC3dPreparedCopyBatch(batch_at(0.25, [0.0; 4])),
            SoftwareChampionshipCarIndexedSource::ReviewedCarPaletteMissingSelectorOpaqueBlackIndexZero,
            &mut depth,
            [1, 1],
            &mut target,
            profile,
        )
        .unwrap();
        assert_eq!(target, [0]);

        depth.clear_far();
        target[0] = 91;
        compose_championship_car_indexed_copy_no_blend(
            &SoftwareC3dPreparedCopyBatch(batch_at(0.25, [1.0; 4])),
            SoftwareChampionshipCarIndexedSource::ReviewedCarPaletteMissingSelectorOpaqueBlackIndexZero,
            &mut depth,
            [1, 1],
            &mut target,
            profile,
        )
        .unwrap();
        assert_eq!(target, [0]);

        target[0] = 77;
        let outcome = compose_championship_car_indexed_copy_no_blend(
            &SoftwareC3dPreparedCopyBatch(batch_at(0.75, [1.0; 4])),
            SoftwareChampionshipCarIndexedSource::ReviewedCarPaletteMissingSelectorOpaqueBlackIndexZero,
            &mut depth,
            [1, 1],
            &mut target,
            profile,
        )
        .unwrap();
        assert_eq!(outcome.depth_rejected(), 1);
        assert_eq!(target, [77]);
        assert_eq!(depth.values(), [0.25]);
    }

    #[test]
    fn unsupported_championship_profiles_and_invalid_sources_are_effect_free() {
        let batch = prepared_batch(vec![triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
                screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], [0.0; 4]),
            ],
            0,
        )]);

        let supported = SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND;
        for profile in [
            SoftwareChampionshipCarRasterProfile {
                unified_indexed: false,
                ..supported
            },
            SoftwareChampionshipCarRasterProfile {
                texture_copy: false,
                ..supported
            },
            SoftwareChampionshipCarRasterProfile {
                point_filter: false,
                ..supported
            },
            SoftwareChampionshipCarRasterProfile {
                alpha_blend: true,
                ..supported
            },
        ] {
            let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
            let before_depth = depth.values().to_vec();
            let mut target = [44];
            assert_eq!(
                compose_championship_car_indexed_copy_view(
                    &batch,
                    texture([1, 1], &[9]),
                    &mut depth,
                    [1, 1],
                    &mut target,
                    profile,
                ),
                Err(SoftwareC3dComposeError::Raster(
                    SoftwareC3dRasterError::UnsupportedChampionshipCarProfile(profile)
                ))
            );
            assert_eq!(target, [44]);
            assert_eq!(depth.values(), before_depth);
        }
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let before_depth = depth.values().to_vec();
        let mut target = [44];
        assert!(matches!(
            compose_championship_car_indexed_copy_view(
                &batch,
                texture([2, 1], &[9]),
                &mut depth,
                [1, 1],
                &mut target,
                SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::InvalidTextureIndexLength { .. }
            ))
        ));
        assert_eq!(target, [44]);
        assert_eq!(depth.values(), before_depth);
    }

    #[test]
    fn championship_validator_accepts_all_copy_no_blend_mutation_profiles() {
        for profile in [
            SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
            SoftwareChampionshipCarRasterProfile::MUTATION_ENABLED_ACTOR_RGB_DISABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
            SoftwareChampionshipCarRasterProfile::MUTATION_DISABLED_ACTOR_RGB_ENABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
            SoftwareChampionshipCarRasterProfile::MUTATION_ENABLED_ACTOR_RGB_ENABLED_UNIFIED_INDEXED_COPY_NO_BLEND,
        ] {
            assert_eq!(validate_championship_car_raster_profile(profile), Ok(()));
        }
    }

    #[test]
    fn alternate_validator_accepts_candidate_no_alpha_policy_and_exact_provenance() {
        for (render_depth, raw) in [
            (SoftwareC3dSplitRenderDepth::Bits16, 0x10),
            (SoftwareC3dSplitRenderDepth::Bits24, 0x18),
            (SoftwareC3dSplitRenderDepth::Bits32, 0x20),
        ] {
            assert_eq!(render_depth as u32, raw);
            for (fog, actor_rgb) in [(false, false), (true, false), (false, true), (true, true)] {
                let profile =
                    SoftwareChampionshipCarSplitOracleProfile::new(render_depth, fog, actor_rgb);
                assert_eq!(
                    validate_championship_car_split_modulate_no_alpha_oracle_profile(profile),
                    Ok(())
                );
                assert_eq!(profile.render_depth_provenance, render_depth);
                assert_eq!(profile.requested_filter as u32, 2);
                assert_eq!(
                    profile.oracle_filter,
                    SoftwareC3dOracleFilter::BilinearClampedTexelCenter
                );
                assert!(validate_championship_car_raster_profile(profile.raster).is_err());
            }
        }

        let alternate = alternate_profile();
        for raster in [
            SoftwareChampionshipCarRasterProfile {
                unified_indexed: true,
                ..alternate.raster
            },
            SoftwareChampionshipCarRasterProfile {
                texture_copy: true,
                ..alternate.raster
            },
            SoftwareChampionshipCarRasterProfile {
                point_filter: true,
                ..alternate.raster
            },
            SoftwareChampionshipCarRasterProfile {
                alpha_blend: true,
                ..alternate.raster
            },
        ] {
            let profile = SoftwareChampionshipCarSplitOracleProfile {
                raster,
                ..alternate
            };
            assert_eq!(
                validate_championship_car_split_modulate_no_alpha_oracle_profile(profile),
                Err(SoftwareC3dRasterError::UnsupportedChampionshipCarProfile(
                    raster
                ))
            );
        }
    }

    #[test]
    fn renderer_branch_decoder_preserves_priority_and_indexed_zero() {
        use SoftwareC3dRendererBranchSelection::{Indexed, Split};

        assert_eq!(decode_software_c3d_renderer_branch(0), Indexed);
        assert_eq!(decode_software_c3d_renderer_branch(0x80), Indexed);
        assert_eq!(
            decode_software_c3d_renderer_branch(0x100),
            Split(SoftwareC3dSplitRenderDepth::Bits32)
        );
        assert_eq!(
            decode_software_c3d_renderer_branch(0x200 | 0x100),
            Split(SoftwareC3dSplitRenderDepth::Bits24)
        );
        assert_eq!(
            decode_software_c3d_renderer_branch(0x400 | 0x200 | 0x100),
            Split(SoftwareC3dSplitRenderDepth::Bits16)
        );
    }

    #[test]
    fn alternate_preflight_consumes_prepared_type2_words_and_checks_layout_and_count() {
        let aspect: f32 = 640.0 / 480.0;
        let words = [
            [-0.5, -0.5 * aspect, 0.5],
            [0.0, 0.5 * aspect, 0.5],
            [0.5, -0.5 * aspect, 0.5],
        ]
        .map(|position| {
            [
                position[0].to_bits(),
                position[1].to_bits(),
                position[2].to_bits(),
                0,
                0xff12_3456,
                0xffab_cdef,
                0.5f32.to_bits(),
                0.5f32.to_bits(),
            ]
        });
        let prepared =
            prepare_c3d_type2_pre_draw_vertices(words, C3dType2FogState::DISABLED).unwrap();
        let batch = preflight_championship_car_prepared_type2_batch(
            C3dVertexMutationBranch::Type2,
            3,
            &[0, 1, 2],
            &prepared,
            identity_state(),
        )
        .unwrap();

        assert_eq!(batch.triangles().len(), 1);
        for vertex in batch.triangles()[0].vertices() {
            assert_eq!(vertex.diffuse_argb(), unpack_argb(0xff12_3456));
            assert_eq!(vertex.specular_argb(), unpack_argb(0xffab_cdef));
        }
        assert_eq!(
            preflight_championship_car_prepared_type2_batch(
                C3dVertexMutationBranch::Type2,
                4,
                &[0, 1, 2],
                &prepared,
                identity_state(),
            ),
            Err(SoftwareC3dRasterError::PreparedVertexCountMismatch {
                command: 4,
                prepared: 3,
            })
        );
        assert_eq!(
            preflight_championship_car_prepared_type2_batch(
                C3dVertexMutationBranch::Type1,
                3,
                &[0, 1, 2],
                &prepared,
                identity_state(),
            ),
            Err(SoftwareC3dRasterError::UnsupportedVertexLayout { vertex_type: 1 })
        );
    }

    #[test]
    fn explicit_oracle_bilinear_sampler_uses_clamped_texel_centers() {
        let palette = [
            BitmapPaletteEntry {
                red: 255,
                green: 0,
                blue: 0,
                reserved: 0,
            },
            BitmapPaletteEntry {
                red: 0,
                green: 255,
                blue: 0,
                reserved: 0,
            },
            BitmapPaletteEntry {
                red: 0,
                green: 0,
                blue: 255,
                reserved: 0,
            },
            BitmapPaletteEntry {
                red: 255,
                green: 255,
                blue: 255,
                reserved: 0,
            },
        ];
        let texture = indexed_texture([2, 2], &[0, 1, 2, 3], &palette);
        let source = sampled_varying_2x2_texture();
        let converted = convert_palette_texture(&source);

        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [0.25, 0.25]),
            [255.0, 0.0, 0.0]
        );
        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [0.75, 0.25]),
            [0.0, 255.0, 0.0]
        );
        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [0.25, 0.75]),
            [0.0, 0.0, 255.0]
        );
        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [0.75, 0.75]),
            [255.0; 3]
        );
        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [0.5, 0.5]),
            [127.5; 3]
        );
        assert_eq!(
            sample_bilinear_converted_rgb_oracle_clamped(&converted, [-1.0, 2.0]),
            [0.0, 0.0, 255.0]
        );
        assert_eq!(sample_point_clamped(texture.raster, [0.5, 0.5]), 3);
    }

    #[test]
    fn public_alternate_compositor_uses_explicit_oracle_filter_and_gouraud_modulate() {
        let texture = sampled_varying_2x2_texture();
        let converted = convert_palette_texture(&texture);
        let batch = lit_pixel_batch([
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
        ]);
        // The explicit oracle filter averages the four texels to 127.5 RGB;
        // pixel-center Gouraud barycentrics produce diffuse red 0.25. Rounded
        // MODULATE output is therefore red 32, unlike point sampling index 3.
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [[200; 3]];
        let outcome = compose_championship_car_split_modulate_canonical_rgb_oracle(
            &batch,
            SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
            &mut depth,
            [1, 1],
            &mut target,
            alternate_profile(),
        )
        .unwrap();

        assert_eq!(outcome.depth_passed(), 1);
        assert_eq!(target, [[32, 0, 0]]);
        assert_eq!(depth.values(), [0.5]);
        assert_ne!(
            target[0],
            [64, 0, 0],
            "oracle filter must differ from point"
        );
    }

    #[test]
    fn selected_rgb_quantization_precedes_bilinear_filtering() {
        let source = sampled_quadrant_texture([[0, 0, 0], [5, 0, 0], [0, 0, 0], [5, 0, 0]]);
        let converted = convert_rgb555_texture(&source);
        let batch = lit_pixel_batch([[1.0; 4]; 3]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [[200; 3]];

        compose_championship_car_split_modulate_canonical_rgb_oracle(
            &batch,
            SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
            &mut depth,
            [1, 1],
            &mut target,
            alternate_profile(),
        )
        .unwrap();

        assert_eq!(target, [[4, 0, 0]]);
        assert_ne!(target, [[3, 0, 0]], "raw-RGB filtering would occur first");
    }

    #[test]
    fn alternate_compositor_keeps_selected_texture_and_framebuffer_masks_distinct() {
        let source = sampled_texture_with_palette_entry(1, [0, 129, 0]);
        let converted_555 = convert_rgb555_texture(&source);
        let batch = lit_pixel_batch([[1.0; 4]; 3]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = LogicalMaskedRgbTarget::new([1, 1], framebuffer_rgb565()).unwrap();

        let outcome = compose_championship_car_split_modulate_logical_masked_rgb_oracle(
            &batch,
            SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted_555),
            &mut depth,
            &mut target,
            alternate_profile(),
        )
        .unwrap();

        assert_eq!(outcome.depth_passed(), 1);
        // RGB555 materialization decodes green 129 as 132; the independent
        // RGB565 framebuffer then packs 132 to six-bit value 33 (0x420).
        assert_eq!(target.words(), &[0x0420]);
        assert_eq!(
            converted_555.category(),
            SourceOwnedSelectedTextureCategory::OpaqueRgb16(KnownRgbLayout::Rgb555)
        );
        assert_eq!(target.format().layout().masks(), [0xf800, 0x07e0, 0x001f]);
    }

    #[test]
    fn logical_masked_alternate_composition_errors_are_atomic() {
        let converted = convert_palette_texture(&sampled_varying_2x2_texture());
        let mut malformed = lit_pixel_batch([[1.0; 4]; 3]);
        malformed.0.triangles[0].vertices[0].position[0] = f32::NAN;
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = LogicalMaskedRgbTarget::new([1, 1], framebuffer_rgb565()).unwrap();
        target
            .replace_from_canonical_rgb([1, 1], &[[17, 203, 49]])
            .unwrap();
        let words_before = target.words().to_vec();
        let depth_before = depth.values().to_vec();

        assert_eq!(
            compose_championship_car_split_modulate_logical_masked_rgb_oracle(
                &malformed,
                SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
                &mut depth,
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::InvalidPreparedTriangle { triangle: 0 }
            ))
        );
        assert_eq!(target.words(), words_before);
        assert_eq!(depth.values(), depth_before);

        let mut mismatched_depth = SoftwareC3dDepthBuffer::new_far([2, 1]).unwrap();
        let mismatched_depth_before = mismatched_depth.values().to_vec();
        assert_eq!(
            compose_championship_car_split_modulate_logical_masked_rgb_oracle(
                &lit_pixel_batch([[1.0; 4]; 3]),
                SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
                &mut mismatched_depth,
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::DepthTargetExtentMismatch {
                depth: [2, 1],
                target: [1, 1],
            })
        );
        assert_eq!(target.words(), words_before);
        assert_eq!(mismatched_depth.values(), mismatched_depth_before);
    }

    #[test]
    fn logical_masked_alternate_composition_preserves_rejected_and_uncovered_words() {
        let converted = convert_palette_texture(&sampled_varying_2x2_texture());
        let batch = lit_pixel_batch([[1.0; 4]; 3]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([2, 1]).unwrap();
        let mut target = LogicalMaskedRgbTarget::new([2, 1], framebuffer_rgb565()).unwrap();
        target
            .replace_from_canonical_rgb([2, 1], &[[17, 203, 49], [91, 7, 222]])
            .unwrap();
        let words_before = target.words().to_vec();

        let outcome = compose_championship_car_split_modulate_logical_masked_rgb_oracle(
            &batch,
            SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
            &mut depth,
            &mut target,
            alternate_profile(),
        )
        .unwrap();

        assert_eq!(outcome.covered(), 1);
        assert_eq!(outcome.depth_passed(), 1);
        assert_ne!(target.words()[0], words_before[0]);
        assert_eq!(target.words()[1], words_before[1]);

        let mut rejected_depth = SoftwareC3dDepthBuffer::new_far([2, 1]).unwrap();
        rejected_depth.values[0] = 0.0;
        let mut rejected_target =
            LogicalMaskedRgbTarget::new([2, 1], framebuffer_rgb565()).unwrap();
        rejected_target
            .replace_from_canonical_rgb([2, 1], &[[17, 203, 49], [91, 7, 222]])
            .unwrap();
        let rejected_words_before = rejected_target.words().to_vec();
        let rejected = compose_championship_car_split_modulate_logical_masked_rgb_oracle(
            &batch,
            SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
            &mut rejected_depth,
            &mut rejected_target,
            alternate_profile(),
        )
        .unwrap();
        assert_eq!(rejected.depth_rejected(), 1);
        assert_eq!(rejected.depth_passed(), 0);
        assert_eq!(rejected_target.words(), rejected_words_before);
        assert_eq!(rejected_depth.values(), &[0.0, 1.0]);
    }

    #[test]
    fn alternate_modulate_ignores_fog_specular_but_consumes_actor_rgb_diffuse() {
        let texture = sampled_texture_with_palette_entry(1, [255; 3]);
        let converted = convert_palette_texture(&texture);
        let batch = |diffuse: [f32; 4], specular: [f32; 4]| {
            let mut batch = lit_pixel_batch([diffuse; 3]);
            for vertex in &mut batch.0.triangles[0].vertices {
                vertex.specular_argb = specular;
            }
            batch
        };
        let render = |batch: &SoftwareC3dPreparedLitBatch| {
            let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
            let mut target = [[99; 3]];
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                batch,
                SoftwareChampionshipCarSplitSource::ConvertedTexture(&converted),
                &mut depth,
                [1, 1],
                &mut target,
                alternate_profile(),
            )
            .unwrap();
            target[0]
        };

        let original = render(&batch([1.0, 128.0 / 255.0, 64.0 / 255.0, 0.0], [0.0; 4]));
        let changed_fog = render(&batch(
            [1.0, 128.0 / 255.0, 64.0 / 255.0, 0.0],
            [1.0, 0.9, 0.1, 0.8],
        ));
        let actor_rgb = render(&batch(
            [1.0, 64.0 / 255.0, 128.0 / 255.0, 0.0],
            [1.0, 0.9, 0.1, 0.8],
        ));
        assert_eq!(
            (original, changed_fog, actor_rgb),
            ([128, 64, 0], [128, 64, 0], [64, 128, 0])
        );
    }

    #[test]
    fn alternate_default_is_bounded_to_black_diffuse_and_errors_are_effect_free() {
        let black = lit_pixel_batch([[1.0, 0.0, 0.0, 0.0]; 3]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [[44; 3]];
        compose_championship_car_split_modulate_canonical_rgb_oracle(
            &black,
            SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
            &mut depth,
            [1, 1],
            &mut target,
            alternate_profile(),
        )
        .unwrap();
        assert_eq!(target, [[0; 3]]);

        let non_black = lit_pixel_batch([[1.0, 1.0, 0.0, 0.0]; 3]);
        depth.clear_far();
        target[0] = [44; 3];
        let before_depth = depth.values().to_vec();
        assert_eq!(
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                &non_black,
                SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
                &mut depth,
                [1, 1],
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::NonBlackAlternateDefaultDiffuse {
                    triangle: 0,
                    vertex: 0,
                }
            ))
        );
        assert_eq!(target, [[44; 3]]);
        assert_eq!(depth.values(), before_depth);

        assert_eq!(
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                &black,
                SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
                &mut depth,
                [2, 1],
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::InvalidTargetLength {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(target, [[44; 3]]);
        assert_eq!(depth.values(), before_depth);

        let mut mismatched_depth = SoftwareC3dDepthBuffer::new_far([2, 1]).unwrap();
        let before_mismatched_depth = mismatched_depth.values().to_vec();
        assert_eq!(
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                &black,
                SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
                &mut mismatched_depth,
                [1, 1],
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::DepthTargetExtentMismatch {
                depth: [2, 1],
                target: [1, 1],
            })
        );
        assert_eq!(target, [[44; 3]]);
        assert_eq!(mismatched_depth.values(), before_mismatched_depth);

        let mut malformed = black.clone();
        malformed.0.triangles[0].vertices[0].position[0] = f32::NAN;
        assert_eq!(
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                &malformed,
                SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
                &mut depth,
                [1, 1],
                &mut target,
                alternate_profile(),
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::InvalidPreparedTriangle { triangle: 0 }
            ))
        );
        assert_eq!(target, [[44; 3]]);
        assert_eq!(depth.values(), before_depth);

        let unsupported = SoftwareChampionshipCarSplitOracleProfile {
            raster: SoftwareChampionshipCarRasterProfile {
                alpha_blend: true,
                ..alternate_profile().raster
            },
            ..alternate_profile()
        };
        assert!(matches!(
            compose_championship_car_split_modulate_canonical_rgb_oracle(
                &black,
                SoftwareChampionshipCarSplitSource::BlackDiffuseDefault,
                &mut depth,
                [1, 1],
                &mut target,
                unsupported,
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::UnsupportedChampionshipCarProfile(_)
            ))
        ));
        assert_eq!(target, [[44; 3]]);
        assert_eq!(depth.values(), before_depth);
    }

    #[test]
    fn indexed_copy_less_equal_layering_changes_color_only_on_depth_pass() {
        let batch_at = |value| {
            let color = [0.0; 4];
            prepared_batch(vec![triangle(
                [
                    screen([0.0, 0.0, value], 1.0, [0.0, 0.0], color),
                    screen([2.0, 0.0, value], 1.0, [0.0, 0.0], color),
                    screen([0.0, 2.0, value], 1.0, [0.0, 0.0], color),
                ],
                0,
            )])
        };
        let active = std::array::from_fn(|index| PaletteEntry {
            red: index as u8,
            green: 0,
            blue: 0,
            flags: 0,
        });
        let palette = |red| {
            [BitmapPaletteEntry {
                red,
                green: 0,
                blue: 0,
                reserved: 0,
            }]
        };
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut target = [77];

        for (incoming, red, expected_pass, expected_target) in [
            (0.5, 4, true, 4),
            (0.5, 9, true, 9),
            (0.75, 12, false, 9),
            (0.25, 15, true, 15),
        ] {
            let outcome = compose_indexed_copy_no_blend(
                &batch_at(incoming),
                indexed_texture([1, 1], &[0], &palette(red)),
                &mut depth,
                [1, 1],
                &mut target,
                &active,
                SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither,
            )
            .unwrap();
            assert_eq!(outcome.depth_passed(), usize::from(expected_pass));
            assert_eq!(target, [expected_target]);
        }
        assert_eq!(depth.values(), [0.25]);
    }

    #[test]
    fn indexed_copy_validation_is_effect_free_for_color_and_depth() {
        let color = [0.0; 4];
        let valid = triangle(
            [
                screen([0.0, 0.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([2.0, 0.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([0.0, 2.0, 0.5], 1.0, [0.0, 0.0], color),
            ],
            0,
        );
        let valid_batch = prepared_batch(vec![valid]);
        let texture_palette = [BitmapPaletteEntry {
            red: 1,
            green: 2,
            blue: 3,
            reserved: 0,
        }];
        let active = [PaletteEntry {
            red: 1,
            green: 2,
            blue: 3,
            flags: 0,
        }; 256];
        let policy = SoftwareC3dIndexedQuantization::NearestRgbL1LowestIndexNoDither;

        for (extent, length, expected) in [
            (
                [0, 1],
                0,
                SoftwareC3dComposeError::InvalidTargetExtent([0, 1]),
            ),
            (
                [1, 1],
                0,
                SoftwareC3dComposeError::InvalidTargetLength {
                    expected: 1,
                    actual: 0,
                },
            ),
            (
                [2, 1],
                2,
                SoftwareC3dComposeError::DepthTargetExtentMismatch {
                    depth: [1, 1],
                    target: [2, 1],
                },
            ),
        ] {
            let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
            let before_depth = depth.clone();
            let mut target = vec![88; length];
            let before_target = target.clone();
            assert_eq!(
                compose_indexed_copy_no_blend(
                    &valid_batch,
                    indexed_texture([1, 1], &[0], &texture_palette),
                    &mut depth,
                    extent,
                    &mut target,
                    &active,
                    policy,
                ),
                Err(expected)
            );
            assert_eq!(depth, before_depth);
            assert_eq!(target, before_target);
        }

        let mut invalid = valid;
        invalid.vertices[0].reciprocal_w = 0.0;
        let invalid_batch = prepared_batch(vec![valid, invalid]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let before_depth = depth.clone();
        let mut target = [88];
        assert_eq!(
            compose_indexed_copy_no_blend(
                &invalid_batch,
                indexed_texture([1, 1], &[0], &texture_palette),
                &mut depth,
                [1, 1],
                &mut target,
                &active,
                policy,
            ),
            Err(SoftwareC3dComposeError::Raster(
                SoftwareC3dRasterError::InvalidPreparedTriangle { triangle: 1 }
            ))
        );
        assert_eq!(depth, before_depth);
        assert_eq!(target, [88]);
    }

    #[test]
    fn point_sampler_clamps_and_uses_floor_times_extent() {
        let indices = [10, 11, 12, 13, 20, 21, 22, 23];
        let texture = texture([4, 2], &indices);
        let cases: [([f32; 2], u8); 9] = [
            ([-0.25, -1.0], 10),
            ([0.0, 0.0], 10),
            ([0.20, 0.25], 10),
            ([0.25, 0.25], 11),
            ([0.5, 0.25], 12),
            ([0.75, 0.25], 13),
            ([1.0, 0.25], 13),
            ([2.0, 0.5], 23),
            ([0.5, 1.0], 22),
        ];

        for (uv, expected) in cases {
            assert_eq!(sample_point_clamped(texture, uv), expected, "{uv:?}");
        }
    }

    #[test]
    fn stage_uv_preprocessing_is_neither_raw_clamp_nor_generic_repeat() {
        let indices: Vec<u8> = (0..16).collect();
        let texture = texture([4, 4], &indices);

        let first_raw = [1.25, -0.75];
        let first_corrected = cmr_core::c3d_render::stage_texture_uv([0x1_4000, -0xc000]);
        assert_eq!(first_corrected, [0.25, 0.25]);
        assert_eq!(sample_point_clamped(texture, first_corrected), 5);
        assert_eq!(sample_point_clamped(texture, first_raw), 3);

        let second_corrected = cmr_core::c3d_render::stage_texture_uv([0x2_4000, 0x4000]);
        assert_eq!(second_corrected, [1.25, 1.25]);
        assert_eq!(sample_point_clamped(texture, second_corrected), 15);
        assert_eq!(sample_point_clamped(texture, [0.25, 0.25]), 5);
    }

    #[test]
    fn less_equal_depth_writes_equal_and_nearer_fragments_only() {
        let make_batch = |depth| {
            let value = screen([0.0, 0.0, depth], 1.0, [0.0, 0.0], [0.0; 4]);
            let right = screen([2.0, 0.0, depth], 1.0, [0.0, 0.0], [0.0; 4]);
            let bottom = screen([0.0, 2.0, depth], 1.0, [0.0, 0.0], [0.0; 4]);
            prepared_batch(vec![triangle([value, right, bottom], 0)])
        };
        let mut depth = SoftwareC3dDepthBuffer::new_far([1, 1]).unwrap();
        let mut written = Vec::new();

        for (incoming, index, expected_pass) in [
            (1.00, 5, true),
            (0.75, 10, true),
            (0.80, 20, false),
            (0.75, 30, true),
            (0.25, 40, true),
        ] {
            let outcome = rasterize_fragments(
                &make_batch(incoming),
                texture([1, 1], &[index]),
                &mut depth,
                |fragment| written.push(fragment.texture_index()),
            )
            .unwrap();
            assert_eq!(outcome.depth_passed(), usize::from(expected_pass));
            assert_eq!(outcome.depth_rejected(), usize::from(!expected_pass));
        }

        assert_eq!(written, [5, 10, 30, 40]);
        assert_eq!(depth.values(), [0.25]);
        depth.clear_far();
        assert_eq!(depth.values(), [1.0]);
    }

    #[test]
    fn target_bounds_and_validation_fail_before_depth_mutation() {
        assert_eq!(
            SoftwareC3dDepthBuffer::new_far([0, 3]),
            Err(SoftwareC3dRasterError::InvalidDepthExtent([0, 3]))
        );
        let color = [0.0; 4];
        let batch = prepared_batch(vec![triangle(
            [
                screen([-1.0, -1.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([4.0, -1.0, 0.5], 1.0, [0.0, 0.0], color),
                screen([-1.0, 4.0, 0.5], 1.0, [0.0, 0.0], color),
            ],
            0,
        )]);
        let mut depth = SoftwareC3dDepthBuffer::new_far([3, 3]).unwrap();
        let mut positions = Vec::new();
        rasterize_fragments(&batch, texture([1, 1], &[0]), &mut depth, |fragment| {
            positions.push(fragment.position());
        })
        .unwrap();
        assert_eq!(positions, [[0, 0], [1, 0], [0, 1]]);

        depth.clear_far();
        let before = depth.clone();
        assert_eq!(
            rasterize_fragments(&batch, texture([2, 2], &[0, 1, 2]), &mut depth, |_| panic!(
                "validation must precede visits"
            ),),
            Err(SoftwareC3dRasterError::InvalidTextureIndexLength {
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(depth, before);

        let mut visits = 0;
        assert_eq!(
            rasterize_fragments(
                &batch,
                TextureView {
                    extent: [0, 1],
                    indices: &[],
                    palette_len: 256,
                },
                &mut depth,
                |_| visits += 1,
            ),
            Err(SoftwareC3dRasterError::InvalidTextureExtent([0, 1]))
        );
        assert_eq!(depth, before);
        assert_eq!(visits, 0);

        assert_eq!(
            rasterize_fragments(
                &batch,
                TextureView {
                    extent: [1, 1],
                    indices: &[7],
                    palette_len: 7,
                },
                &mut depth,
                |_| visits += 1,
            ),
            Err(SoftwareC3dRasterError::TexturePaletteIndexOutOfRange {
                index: 7,
                palette_len: 7,
            })
        );
        assert_eq!(depth, before);
        assert_eq!(visits, 0);

        let valid = batch.triangles[0];
        let mut invalid = valid;
        invalid.vertices[0].reciprocal_w = 0.0;
        let invalid_batch = prepared_batch(vec![valid, valid, invalid]);
        assert_eq!(
            rasterize_fragments(&invalid_batch, texture([1, 1], &[0]), &mut depth, |_| {
                visits += 1
            },),
            Err(SoftwareC3dRasterError::InvalidPreparedTriangle { triangle: 2 })
        );
        assert_eq!(depth, before);
        assert_eq!(visits, 0);
    }
}
