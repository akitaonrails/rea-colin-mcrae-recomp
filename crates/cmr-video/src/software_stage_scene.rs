//! Software composition of a walkable C3D stage/car scene (reconstruction item
//! E08, the pixel half of the stage scene session).
//!
//! [`cmr_core::c3d_stage_scene::StageScene`] is the backend-neutral walk: it
//! yields a draw order plus each node's world matrix and active-LOD draw group.
//! [`compose_stage_scene_frame`] drives that walk through the existing software
//! rasterizer - for each prim it builds the gameplay transform state
//! ([`SoftwareC3dTransformState::for_gameplay_scene`]), preflights the mesh prim
//! ([`preflight_mesh_lod_prim`]), and composites it into one shared indexed
//! framebuffer and [`SoftwareC3dDepthBuffer`] via
//! [`compose_root_c3d_indexed_copy_no_blend`] - exactly the path the occlusion
//! oracle proved order-independent, so the draw order only matters once alpha
//! blending is wired (the current COPY path is opaque, Z-buffered).
//!
//! Binding a prim's group-2 material ordinal to a texture is the separate E03
//! step; this composer **defers** it to a caller-supplied `resolve_texture`
//! closure (mirroring how [`cmr_core::c3d_mesh::assemble_lod_draw_group`] defers
//! the material selectors). A prim whose ordinal the caller cannot resolve is
//! skipped and counted, so the composer stays honest about the unresolved
//! binding rather than inventing one.

use std::fmt;

use cmr_core::c3d_stage_scene::StageScene;
use cmr_core::display::DisplayExtent;
use cmr_core::root_menu_c3d_resources::FrontendC3dOwner;
use cmr_core::root_menu_c3d_transform::RootMenuC3dViewportError;
use cmr_formats::bitmap::BitmapLimits;
use cmr_formats::c3d::{C3dAnimationReferenceError, C3dFile, C3dResourceError, C3dResourceView};
use cmr_formats::riff_palette::PaletteEntry;

use crate::c3d_software::{SoftwareC3dBitmapError, SoftwareC3dSampledTexture};
use crate::c3d_software_raster::{
    SoftwareC3dComposeError, SoftwareC3dDepthBuffer, SoftwareC3dIndexedQuantization,
    SoftwareC3dRasterError, SoftwareC3dTransformState, compose_root_c3d_indexed_copy_color_key,
    compose_root_c3d_indexed_copy_no_blend, preflight_mesh_lod_prim,
};
use crate::{SoftwarePresentedFrame, SoftwareRgba8Error};

/// The counters a [`compose_stage_scene_frame`] run accumulates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StageSceneFrameStats {
    /// Prims composited into the framebuffer.
    pub prims_composed: usize,
    /// Prims skipped because `resolve_texture` returned `None` for the ordinal.
    pub unresolved_prims: usize,
    /// Drawable nodes whose active LOD held no geometry (an empty/sentinel LOD).
    pub empty_lod_nodes: usize,
    /// Fragments the rasterizer produced across all composited prims.
    pub covered: usize,
    /// Fragments that passed the depth test (were written).
    pub depth_passed: usize,
    /// Fragments the depth test rejected (occluded).
    pub depth_rejected: usize,
}

/// A composed stage-scene frame: the indexed framebuffer, its display palette,
/// its depth buffer, and the run statistics.
#[derive(Clone, Debug)]
pub struct StageSceneFrame {
    extent: [u16; 2],
    pixels: Vec<u8>,
    palette: Box<[PaletteEntry; 256]>,
    depth: SoftwareC3dDepthBuffer,
    stats: StageSceneFrameStats,
    rgba8: Option<Vec<u8>>,
    projection: Option<(f32, f32, f32, f32)>,
}

impl StageSceneFrame {
    fn add_stats(&mut self, other: &StageSceneFrame) {
        self.stats.prims_composed += other.stats.prims_composed;
        self.stats.unresolved_prims += other.stats.unresolved_prims;
        self.stats.empty_lod_nodes += other.stats.empty_lod_nodes;
        self.stats.covered += other.stats.covered;
        self.stats.depth_passed += other.stats.depth_passed;
        self.stats.depth_rejected += other.stats.depth_rejected;
    }

    /// A frame another renderer composed (its indices, palette and depth),
    /// for passes drawn over it.
    pub fn from_parts(
        extent: [u16; 2],
        pixels: Vec<u8>,
        palette: [PaletteEntry; 256],
        depth: SoftwareC3dDepthBuffer,
    ) -> Self {
        Self {
            extent,
            pixels,
            palette: Box::new(palette),
            depth,
            stats: StageSceneFrameStats::default(),
            rgba8: None,
            projection: None,
        }
    }

    /// Attaches a true-color RGBA8 plane (`width * height * 4` bytes) from the
    /// hardware GPU stage renderer so post-blend passes (shadows, skid marks,
    /// particles, lamp flares, night pool) also blend in true 24-bit color.
    #[must_use]
    pub fn with_rgba8(mut self, rgba8: Vec<u8>) -> Self {
        self.rgba8 = Some(rgba8);
        self
    }

    /// Overrides the projection parameters (`(w, h, z_near, z_far)`) used by
    /// post-blend passes (`blend_shadows`, `blend_sprites`, `blend_coloured_triangles`,
    /// `apply_night_pool`) so widescreen / extended draw-distance projections match.
    #[must_use]
    pub fn with_projection(mut self, projection: (f32, f32, f32, f32)) -> Self {
        self.projection = Some(projection);
        self
    }

    fn effective_projection(&self) -> (f32, f32, f32, f32) {
        self.projection.unwrap_or_else(|| {
            cmr_core::c3d_camera::gameplay_projection_default(self.extent[0], self.extent[1])
        })
    }

    /// The frame extent `[width, height]`.
    pub fn extent(&self) -> [u16; 2] {
        self.extent
    }

    /// The composed indexed framebuffer (`width * height` palette indices).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The display palette the frame was composed against.
    pub fn palette(&self) -> &[PaletteEntry; 256] {
        &self.palette
    }

    /// The depth buffer after composition.
    pub fn depth(&self) -> &SoftwareC3dDepthBuffer {
        &self.depth
    }

    /// Temporarily masks pixels whose RGBA8 alpha is `0` (written by a car
    /// layer in `WgpuStageRenderer`) to `depth = -1.0` so pre-car passes
    /// (`apply_night_pool`, `blend_coloured_triangles`, `blend_shadows`) cannot
    /// draw over the car mesh even when a shadow vertex is pulled `1.0 m`
    /// toward the camera (`FUN_0047bee0`), and restores their RGBA8 alpha to
    /// `255`. Returns the `(pixel_index, original_depth)` list for
    /// [`Self::restore_masked_depths`].
    pub fn mask_car_depths_from_rgba8_alpha(&mut self) -> Vec<(usize, f32)> {
        let Some(rgba) = self.rgba8.as_mut() else {
            return Vec::new();
        };
        let total = usize::from(self.extent[0]) * usize::from(self.extent[1]);
        let mut saved = Vec::new();
        for at in 0..total {
            if let Some(alpha) = rgba.get_mut(at * 4 + 3)
                && *alpha == 0
            {
                *alpha = 255;
                let orig = self.depth.values()[at];
                saved.push((at, orig));
                self.depth.set_value(at, -1.0);
            }
        }
        saved
    }

    /// Restores depths masked by [`Self::mask_car_depths_from_rgba8_alpha`] so
    /// post-car passes (particles and lamp flares) depth-test against the car's
    /// true depth.
    pub fn restore_masked_depths(&mut self, saved: &[(usize, f32)]) {
        for &(at, orig) in saved {
            self.depth.set_value(at, orig);
        }
    }

    /// The run statistics.
    pub fn stats(&self) -> &StageSceneFrameStats {
        &self.stats
    }

    /// Bridges the indexed frame into a [`SoftwarePresentedFrame`] the platform
    /// host can present (via `expand_rgba8` + a streaming texture upload),
    /// reusing the same presentation type the root-menu backend emits.
    pub fn to_presented_frame(&self) -> Result<SoftwarePresentedFrame, SoftwareRgba8Error> {
        let [width, height] = self.extent;
        let frame = SoftwarePresentedFrame::from_indexed(
            DisplayExtent { width, height },
            self.pixels.clone().into_boxed_slice(),
            self.palette.clone(),
        )?;
        if let Some(rgba8) = &self.rgba8 {
            frame.with_rgba8(rgba8.clone().into_boxed_slice())
        } else {
            Ok(frame)
        }
    }

    /// Places this world/foreground pass over an independently rendered
    /// background pass. Pixels whose foreground depth remains at the clear
    /// value (`1.0`) retain the background; every depth-writing foreground
    /// fragment replaces it. This models the shipped horizon as a background
    /// C3D pass without treating palette index zero as transparency.
    pub fn over_background(
        mut self,
        background: &StageSceneFrame,
    ) -> Result<Self, StageSceneComposeError> {
        if self.extent != background.extent {
            return Err(StageSceneComposeError::LayerExtentMismatch {
                foreground: self.extent,
                background: background.extent,
            });
        }
        for ((pixel, depth), background_pixel) in self
            .pixels
            .iter_mut()
            .zip(self.depth.values())
            .zip(background.pixels.iter())
        {
            if *depth == 1.0 {
                *pixel = *background_pixel;
            }
        }
        self.add_stats(background);
        Ok(self)
    }

    /// Draws the car shadows (`FUN_004a51f0`) over this world pass: each
    /// shadow's pre-lit world-space triangles are depth-tested against the
    /// frame without writing depth (`ZWRITEENABLE` off), and each passing
    /// fragment blends `SRCALPHA`/`INVSRCALPHA` with the texture MODULATE of
    /// the black vertex colour, so it darkens the pixel by the vertex alpha
    /// times the alpha map's texel (`64 x 64`, the ARGB4444 upload keeps its
    /// high nibble); the colour re-quantizes to the frame's palette (the
    /// backend's nearest-colour policy). Point sampling, as for the meshes.
    pub fn blend_shadows(
        &mut self,
        camera_world_q16: &[i32; 16],
        shadows: &[(
            Vec<crate::c3d_software_raster::SoftwareC3dLitVertex>,
            Vec<u16>,
        )],
        alpha: &[u8],
    ) -> Result<(), StageSceneComposeError> {
        const MAP: usize = 64;
        let [width, height] = self.extent;
        let identity = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000,
        ];
        let transforms = SoftwareC3dTransformState::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            &identity,
            self.effective_projection(),
        )?;
        let palette = *self.palette;
        let mut cache = std::collections::HashMap::<(u8, u8), u8>::new();
        let mut per_shadow_levels = std::collections::HashMap::<usize, u8>::new();
        for (vertices, indices) in shadows {
            // `FUN_004a51f0` at `0x4a5915` sets `D3DCULL_NONE` (`push 0; call 0x4a24e0`)
            // because the sun-projected outer fan points can wind either CW or CCW.
            let two_sided = cmr_core::c3d_mesh::two_sided_indices(indices);
            let batch = crate::c3d_software_raster::preflight_lit_triangle_list(
                vertices, &two_sided, transforms,
            )?;
            let depth = &self.depth;
            per_shadow_levels.clear();
            crate::c3d_software_raster::rasterize_c3d_fragments_untextured_no_zwrite(
                &batch,
                depth,
                |fragment| {
                    let [u, v] = fragment.uv();
                    let tx = ((u * MAP as f32) as usize).min(MAP - 1);
                    let ty = ((v * MAP as f32) as usize).min(MAP - 1);
                    let texel = alpha.get(ty * MAP + tx).copied().unwrap_or(0) >> 4;
                    let vertex_alpha = fragment.diffuse_argb()[0];
                    let a = (f32::from(texel) / 15.0) * vertex_alpha;
                    let level = (a * 255.0).round().clamp(0.0, 255.0) as u8;
                    if level == 0 {
                        return;
                    }
                    let [x, y] = fragment.position();
                    let at = usize::from(y) * usize::from(width) + usize::from(x);
                    let entry = per_shadow_levels.entry(at).or_insert(0);
                    *entry = (*entry).max(level);
                },
            )?;
            let pixels = &mut self.pixels;
            let mut rgba8 = self.rgba8.as_deref_mut();
            for (&at, &level) in &per_shadow_levels {
                let under = pixels[at];
                let keep = 1.0 - f32::from(level) / 255.0;
                pixels[at] = *cache.entry((under, level)).or_insert_with(|| {
                    let entry = palette[usize::from(under)];
                    let rgb = [entry.red, entry.green, entry.blue]
                        .map(|c| (f32::from(c) * keep).round() as u8);
                    nearest_index(rgb, &palette)
                });
                if let Some(rgba) = rgba8.as_deref_mut()
                    && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                {
                    px[0] = (f32::from(px[0]) * keep).round() as u8;
                    px[1] = (f32::from(px[1]) * keep).round() as u8;
                    px[2] = (f32::from(px[2]) * keep).round() as u8;
                }
            }
        }
        Ok(())
    }

    /// The particle sprites (`FUN_004a4460`, kind 2): each quad's corners in
    /// world units, its texture keyed on index 0 and modulated by the
    /// vertex grey, blended by the vertex alpha over the frame
    /// (SRCALPHA/INVSRCALPHA) and re-quantized to the frame palette. The
    /// sprites are depth-tested against the scene and write a scratch depth
    /// copy among themselves (the native's z-write is inherited, unknown).
    pub fn blend_sprites(
        &mut self,
        camera_world_q16: &[i32; 16],
        sprites: &[SpriteQuad<'_>],
    ) -> Result<(), StageSceneComposeError> {
        if sprites.is_empty() {
            return Ok(());
        }
        let [width, height] = self.extent;
        let identity = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000,
        ];
        let transforms = SoftwareC3dTransformState::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            &identity,
            self.effective_projection(),
        )?;
        let palette = *self.palette;
        let mut cache = std::collections::HashMap::<(u8, [u8; 3], u8, u8), u8>::new();
        let mut scratch = self.depth.clone();
        for sprite in sprites {
            let (u0, u1) = if sprite.flip { (1.0, 0.0) } else { (0.0, 1.0) };
            let uv = [[u0, 1.0], [u0, 0.0], [u1, 1.0], [u1, 0.0]];
            let diffuse = (u32::from(sprite.alpha) << 24) | (u32::from(sprite.grey) * 0x01_0101);
            let vertices: Vec<crate::c3d_software_raster::SoftwareC3dLitVertex> = (0..4)
                .map(|i| crate::c3d_software_raster::SoftwareC3dLitVertex {
                    position: sprite.corners[i],
                    diffuse,
                    uv: uv[i],
                })
                .collect();
            let batch = crate::c3d_software_raster::preflight_lit_triangle_list(
                &vertices,
                &[0, 3, 2, 0, 1, 3],
                transforms,
            )?;
            let texture = sprite.texture;
            let (tw, th) = (texture.width as usize, texture.height as usize);
            let pixels = &mut self.pixels;
            let mut rgba8 = self.rgba8.as_deref_mut();
            let grey = u32::from(sprite.grey);
            let alpha = sprite.alpha;
            crate::c3d_software_raster::rasterize_c3d_fragments_untextured(
                &batch,
                &mut scratch,
                |fragment| {
                    let [u, v] = fragment.uv();
                    let tx = ((u * tw as f32) as usize).min(tw.saturating_sub(1));
                    let ty = ((v * th as f32) as usize).min(th.saturating_sub(1));
                    let Some(&index) = texture.pixels.get(ty * tw + tx) else {
                        return;
                    };
                    if index == 0 {
                        return;
                    }
                    let Some(entry) = texture.palette.get(usize::from(index)) else {
                        return;
                    };
                    let rgb = [entry.red, entry.green, entry.blue]
                        .map(|c| ((u32::from(c) * grey) / 255) as u8);
                    let [x, y] = fragment.position();
                    let at = usize::from(y) * usize::from(width) + usize::from(x);
                    let under = pixels[at];
                    // Backend policy: the blend went to a 16-bit display; an
                    // ordered 4x4 offset keeps a translucent blend from
                    // quantizing back to the colour under it.
                    let bayer = BAYER_4X4[usize::from(y & 3)][usize::from(x & 3)];
                    pixels[at] = dithered_blend(&mut cache, &palette, under, rgb, alpha, bayer);
                    if let Some(rgba) = rgba8.as_deref_mut()
                        && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                    {
                        let a = u16::from(alpha);
                        let inv = 255 - a;
                        px[0] = ((u16::from(rgb[0]) * a + u16::from(px[0]) * inv) / 255) as u8;
                        px[1] = ((u16::from(rgb[1]) * a + u16::from(px[1]) * inv) / 255) as u8;
                        px[2] = ((u16::from(rgb[2]) * a + u16::from(px[2]) * inv) / 255) as u8;
                    }
                },
            )?;
        }
        Ok(())
    }

    /// Additive camera-billboard lamp flares (`FUN_004a4330`/`FUN_004a4460`,
    /// kind 6): each quad is depth-tested against the scene without updating
    /// the scene depth (`ZWRITEENABLE` off), modulated by `alpha` (`0xFF000000
    /// | a<<16 | a<<8 | a`), and added to the underlying palette colour.
    pub fn blend_additive_sprites(
        &mut self,
        camera_world_q16: &[i32; 16],
        sprites: &[SpriteQuad<'_>],
    ) -> Result<(), StageSceneComposeError> {
        if sprites.is_empty() {
            return Ok(());
        }
        let [width, height] = self.extent;
        let identity = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000,
        ];
        let transforms = SoftwareC3dTransformState::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            &identity,
            self.effective_projection(),
        )?;
        let palette = *self.palette;
        let mut cache = std::collections::HashMap::<(u8, [u8; 3], u8), u8>::new();
        for sprite in sprites {
            if sprite.alpha == 0 {
                continue;
            }
            let (u0, u1) = if sprite.flip { (1.0, 0.0) } else { (0.0, 1.0) };
            let uv = [[u0, 1.0], [u0, 0.0], [u1, 1.0], [u1, 0.0]];
            let a32 = u32::from(sprite.alpha);
            let diffuse = 0xff00_0000 | (a32 * 0x01_0101);
            let vertices: Vec<crate::c3d_software_raster::SoftwareC3dLitVertex> = (0..4)
                .map(|i| crate::c3d_software_raster::SoftwareC3dLitVertex {
                    position: sprite.corners[i],
                    diffuse,
                    uv: uv[i],
                })
                .collect();
            let batch = crate::c3d_software_raster::preflight_lit_triangle_list(
                &vertices,
                &[0, 3, 2, 0, 1, 3],
                transforms,
            )?;
            let texture = sprite.texture;
            let (tw, th) = (texture.width as usize, texture.height as usize);
            let depth = &self.depth;
            let pixels = &mut self.pixels;
            let mut rgba8 = self.rgba8.as_deref_mut();
            let scale = u32::from(sprite.alpha);
            crate::c3d_software_raster::rasterize_c3d_fragments_untextured_no_zwrite(
                &batch,
                depth,
                |fragment| {
                    let [u, v] = fragment.uv();
                    let tx = ((u * tw as f32) as usize).min(tw.saturating_sub(1));
                    let ty = ((v * th as f32) as usize).min(th.saturating_sub(1));
                    let Some(&index) = texture.pixels.get(ty * tw + tx) else {
                        return;
                    };
                    if index == 0 {
                        return;
                    }
                    let Some(entry) = texture.palette.get(usize::from(index)) else {
                        return;
                    };
                    let add_rgb = [entry.red, entry.green, entry.blue]
                        .map(|c| ((u32::from(c) * scale) / 255) as u8);
                    if add_rgb == [0, 0, 0] {
                        return;
                    }
                    let [x, y] = fragment.position();
                    let at = usize::from(y) * usize::from(width) + usize::from(x);
                    let under = pixels[at];
                    let bayer = BAYER_4X4[usize::from(y & 3)][usize::from(x & 3)];
                    pixels[at] = *cache.entry((under, add_rgb, bayer)).or_insert_with(|| {
                        let base = palette[usize::from(under)];
                        let offset = (f32::from(bayer) - 7.5) * DITHER_STEP;
                        let add = |b: u8, s: u8| {
                            (f32::from(b) + f32::from(s) + offset)
                                .round()
                                .clamp(0.0, 255.0) as u8
                        };
                        nearest_index(
                            [
                                add(base.red, add_rgb[0]),
                                add(base.green, add_rgb[1]),
                                add(base.blue, add_rgb[2]),
                            ],
                            &palette,
                        )
                    });
                    if let Some(rgba) = rgba8.as_deref_mut()
                        && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                    {
                        px[0] = px[0].saturating_add(add_rgb[0]);
                        px[1] = px[1].saturating_add(add_rgb[1]);
                        px[2] = px[2].saturating_add(add_rgb[2]);
                    }
                },
            )?;
        }
        Ok(())
    }

    /// Night road light pool (`FUN_004a8a60`/`FUN_004a8750`): modulates
    /// visible stage pixels (`depth < 1.0`) by the circular XZ pool grey byte
    /// (`0x28..=0xff`) around `pool_light_q16` with radius `pool_radius_q16`.
    /// When `exclude_car_sphere_q16` is `Some((centre, radius))`, pixels within
    /// that 3D sphere keep their NIGHT car model look.
    pub fn apply_night_pool(
        &mut self,
        camera_world_q16: &[i32; 16],
        pool_light_q16: [i32; 3],
        pool_radius_q16: i32,
        exclude_car_sphere_q16: Option<([i32; 3], i32)>,
    ) {
        let [width, height] = self.extent;
        if width == 0 || height == 0 {
            return;
        }
        let (pw, ph, zn, zf) = self.effective_projection();
        let m00 = pw * 0.5;
        let m11 = ph * 0.5;
        let w_half = f32::from(width) * 0.5;
        let h_half = f32::from(height) * 0.5;
        let aspect = f32::from(width) / f32::from(height);
        let to_f = |q: i32| q as f32 / 65536.0;
        let cam_right = [
            to_f(camera_world_q16[0]),
            to_f(camera_world_q16[1]),
            to_f(camera_world_q16[2]),
        ];
        let cam_up = [
            to_f(camera_world_q16[4]),
            to_f(camera_world_q16[5]),
            to_f(camera_world_q16[6]),
        ];
        let cam_fwd = [
            to_f(camera_world_q16[8]),
            to_f(camera_world_q16[9]),
            to_f(camera_world_q16[10]),
        ];
        let cam_pos = [
            to_f(camera_world_q16[12]),
            to_f(camera_world_q16[13]),
            to_f(camera_world_q16[14]),
        ];
        let car_excl = exclude_car_sphere_q16.map(|(c, r)| {
            let rf = to_f(r);
            ([to_f(c[0]), to_f(c[1]), to_f(c[2])], rf * rf)
        });
        let palette = *self.palette;
        let mut lut = [[0u8; 256]; 256];
        let mut lut_ready = [false; 256];
        let depth_vals = self.depth.values();
        let w_usize = usize::from(width);

        for y in 0..usize::from(height) {
            let ny = 1.0 - ((y as f32 + 0.5) / h_half);
            let y_coeff = (ny * aspect) / m11;
            let row = y * w_usize;
            for x in 0..w_usize {
                let at = row + x;
                let z_ndc = depth_vals[at];
                if !(0.0..1.0).contains(&z_ndc) {
                    continue;
                }
                let denom = zf - z_ndc * (zf - zn);
                if denom <= 1e-5 {
                    continue;
                }
                let z_view = (zn * zf) / denom;
                let nx = ((x as f32 + 0.5) / w_half) - 1.0;
                let x_view = (nx * z_view) / m00;
                let y_view = y_coeff * z_view;
                let wx =
                    cam_pos[0] + x_view * cam_right[0] + y_view * cam_up[0] + z_view * cam_fwd[0];
                let wy =
                    cam_pos[1] + x_view * cam_right[1] + y_view * cam_up[1] + z_view * cam_fwd[1];
                let wz =
                    cam_pos[2] + x_view * cam_right[2] + y_view * cam_up[2] + z_view * cam_fwd[2];
                if let Some((cc, r2)) = car_excl {
                    let dx = wx - cc[0];
                    let dy = wy - cc[1];
                    let dz = wz - cc[2];
                    if dx * dx + dy * dy + dz * dz <= r2 {
                        continue;
                    }
                }
                let v_q16 = [
                    (wx * 65536.0) as i32,
                    (wy * 65536.0) as i32,
                    (wz * 65536.0) as i32,
                ];
                let g = cmr_core::car_headlights::pool_grey_4a8750(
                    pool_light_q16,
                    v_q16,
                    pool_radius_q16,
                );
                if g == 0xff {
                    continue;
                }
                let gi = usize::from(g);
                if !lut_ready[gi] {
                    let g32 = u32::from(g);
                    for (idx, entry) in palette.iter().enumerate() {
                        let rgb = [entry.red, entry.green, entry.blue]
                            .map(|c| ((u32::from(c) * g32) / 255) as u8);
                        lut[gi][idx] = nearest_index(rgb, &palette);
                    }
                    lut_ready[gi] = true;
                }
                let under = self.pixels[at];
                self.pixels[at] = lut[gi][usize::from(under)];
                if let Some(rgba) = self.rgba8.as_deref_mut()
                    && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                {
                    let g32 = u32::from(g);
                    px[0] = ((u32::from(px[0]) * g32) / 255) as u8;
                    px[1] = ((u32::from(px[1]) * g32) / 255) as u8;
                    px[2] = ((u32::from(px[2]) * g32) / 255) as u8;
                }
            }
        }
    }

    /// Vertex-coloured triangles in world units, blended by their vertex
    /// alpha (the tyre marks, `FUN_004a9cd0`'s untextured buffer), depth
    /// tested against the frame without writing it, and re-quantized with
    /// the sprites' ordered dither.
    pub fn blend_coloured_triangles(
        &mut self,
        camera_world_q16: &[i32; 16],
        triangles: &[([[f32; 3]; 3], [u32; 3])],
    ) -> Result<(), StageSceneComposeError> {
        if triangles.is_empty() {
            return Ok(());
        }
        let [width, height] = self.extent;
        let identity = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000,
        ];
        let transforms = SoftwareC3dTransformState::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            &identity,
            self.effective_projection(),
        )?;
        let palette = *self.palette;
        let mut vertices = Vec::with_capacity(triangles.len() * 3);
        let mut indices = Vec::with_capacity(triangles.len() * 3);
        for (points, colours) in triangles {
            for (point, &colour) in points.iter().zip(colours) {
                indices.push(vertices.len() as u16);
                vertices.push(crate::c3d_software_raster::SoftwareC3dLitVertex {
                    position: *point,
                    diffuse: colour,
                    uv: [0.0, 0.0],
                });
            }
        }
        let mut cache = std::collections::HashMap::<(u8, [u8; 3], u8, u8), u8>::new();
        // One batch per 21000 vertices keeps the u16 indices in range.
        for (vchunk, ichunk) in vertices.chunks(21_000).zip(indices.chunks(21_000)) {
            let base = ichunk.first().copied().unwrap_or(0);
            let local: Vec<u16> = ichunk.iter().map(|i| i - base).collect();
            let two_sided = cmr_core::c3d_mesh::two_sided_indices(&local);
            let batch = crate::c3d_software_raster::preflight_lit_triangle_list(
                vchunk, &two_sided, transforms,
            )?;
            let depth = &self.depth;
            let pixels = &mut self.pixels;
            let mut rgba8 = self.rgba8.as_deref_mut();
            crate::c3d_software_raster::rasterize_c3d_fragments_untextured_no_zwrite(
                &batch,
                depth,
                |fragment| {
                    let [a, r, g, b] = fragment.diffuse_argb();
                    let to8 = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
                    let rgb = [to8(r), to8(g), to8(b)];
                    let alpha = to8(a);
                    let [x, y] = fragment.position();
                    let at = usize::from(y) * usize::from(width) + usize::from(x);
                    pixels[at] = dithered_blend(
                        &mut cache,
                        &palette,
                        pixels[at],
                        rgb,
                        alpha,
                        BAYER_4X4[usize::from(y & 3)][usize::from(x & 3)],
                    );
                    if let Some(rgba) = rgba8.as_deref_mut()
                        && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                    {
                        let a16 = u16::from(alpha);
                        let inv = 255 - a16;
                        px[0] = ((u16::from(rgb[0]) * a16 + u16::from(px[0]) * inv) / 255) as u8;
                        px[1] = ((u16::from(rgb[1]) * a16 + u16::from(px[1]) * inv) / 255) as u8;
                        px[2] = ((u16::from(rgb[2]) * a16 + u16::from(px[2]) * inv) / 255) as u8;
                    }
                },
            )?;
        }
        Ok(())
    }

    /// Depth-composites a later independently rendered opaque world graph into
    /// this frame. The nearer depth wins per pixel and `other` wins equal-depth
    /// ties, reproducing `LessEqual` submission to one shared depth buffer while
    /// keeping the graphs' C3D resource namespaces independent.
    pub fn merge_nearest(
        mut self,
        other: &StageSceneFrame,
    ) -> Result<Self, StageSceneComposeError> {
        if self.extent != other.extent {
            return Err(StageSceneComposeError::LayerExtentMismatch {
                foreground: self.extent,
                background: other.extent,
            });
        }
        for ((pixel, depth), (other_pixel, other_depth)) in self
            .pixels
            .iter_mut()
            .zip(self.depth.values_mut())
            .zip(other.pixels.iter().zip(other.depth.values()))
        {
            if other_depth <= depth {
                *pixel = *other_pixel;
                *depth = *other_depth;
            }
        }
        self.add_stats(other);
        Ok(self)
    }
}

/// The ordered-dither matrix and step the sprite blend quantizes with.
const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
const DITHER_STEP: f32 = 2.0;

/// `rgb` over palette entry `under` at `alpha` (SRCALPHA/INVSRCALPHA),
/// offset by the ordered dither and matched to the palette.
fn dithered_blend(
    cache: &mut std::collections::HashMap<(u8, [u8; 3], u8, u8), u8>,
    palette: &[PaletteEntry; 256],
    under: u8,
    rgb: [u8; 3],
    alpha: u8,
    bayer: u8,
) -> u8 {
    *cache.entry((under, rgb, alpha, bayer)).or_insert_with(|| {
        let base = palette[usize::from(under)];
        let a = f32::from(alpha) / 255.0;
        let offset = (f32::from(bayer) - 7.5) * DITHER_STEP;
        let mix = |b: u8, s: u8| {
            (f32::from(b) * (1.0 - a) + f32::from(s) * a + offset)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        nearest_index(
            [
                mix(base.red, rgb[0]),
                mix(base.green, rgb[1]),
                mix(base.blue, rgb[2]),
            ],
            palette,
        )
    })
}

/// One particle sprite for [`StageSceneFrame::blend_sprites`]: the four
/// corners (v0 `(a0, b0)`, v1 `(a0, b1)`, v2 `(a1, b0)`, v3 `(a1, b1)`) in
/// world units, the vertex alpha and grey, the U mirror and the texture.
#[derive(Clone, Copy, Debug)]
pub struct SpriteQuad<'a> {
    pub corners: [[f32; 3]; 4],
    pub alpha: u8,
    pub grey: u8,
    pub flip: bool,
    pub texture: &'a cmr_formats::bitmap::IndexedBitmap,
}

/// The palette entry nearest `rgb` (Manhattan distance, lowest index on
/// ties — the backend's quantization policy).
fn nearest_index(rgb: [u8; 3], palette: &[PaletteEntry; 256]) -> u8 {
    let mut best = (u32::MAX, 0u8);
    for (index, entry) in palette.iter().enumerate() {
        let d = u32::from(rgb[0].abs_diff(entry.red))
            + u32::from(rgb[1].abs_diff(entry.green))
            + u32::from(rgb[2].abs_diff(entry.blue));
        if d < best.0 {
            best = (d, index as u8);
        }
    }
    best.1
}

/// Composites a walkable stage scene into a software frame.
///
/// Walks [`StageScene::draw_order`]; for each drawable node's active-LOD draw
/// group it composites every prim whose group-2 material ordinal
/// (`prim.selector()`) the caller's `resolve_texture` resolves to a texture. The
/// node's world matrix goes in the WORLD slot (via `for_gameplay_scene`), so a
/// prim's model-space vertices are transformed by its node placement; `camera`
/// supplies the VIEW inverse. Unresolved prims and empty LODs are skipped and
/// counted. The framebuffer starts cleared to index `0` and the depth buffer to
/// the far plane; composition is opaque COPY under the hardware-Z contract.
pub fn compose_stage_scene_frame<'tex, R>(
    scene: &StageScene<'_>,
    camera_world_q16: &[i32; 16],
    extent: [u16; 2],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
    resolve_texture: R,
) -> Result<StageSceneFrame, StageSceneComposeError>
where
    R: FnMut(i16) -> Option<&'tex SoftwareC3dSampledTexture>,
{
    compose_stage_scene_frame_with_projection(
        scene,
        camera_world_q16,
        extent,
        active_palette,
        quantization,
        cmr_core::c3d_camera::gameplay_projection_default(extent[0], extent[1]),
        resolve_texture,
    )
}

/// Composes a stage scene using one of the recovered projection parameter
/// sets. This is used by the horizon pass, whose native renderer temporarily
/// raises the far plane to 25000 world units.
pub fn compose_stage_scene_frame_with_projection<'tex, R>(
    scene: &StageScene<'_>,
    camera_world_q16: &[i32; 16],
    extent: [u16; 2],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
    projection: (f32, f32, f32, f32),
    resolve_texture: R,
) -> Result<StageSceneFrame, StageSceneComposeError>
where
    R: FnMut(i16) -> Option<&'tex SoftwareC3dSampledTexture>,
{
    compose_stage_scene_frame_profile(
        scene,
        camera_world_q16,
        extent,
        active_palette,
        quantization,
        projection,
        None,
        resolve_texture,
    )
}

/// Composes a world graph with the recovered stage index-zero color key. A
/// keyed texel updates neither color nor depth.
pub fn compose_stage_scene_frame_color_key<'tex, R>(
    scene: &StageScene<'_>,
    camera_world_q16: &[i32; 16],
    extent: [u16; 2],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
    color_key: u8,
    resolve_texture: R,
) -> Result<StageSceneFrame, StageSceneComposeError>
where
    R: FnMut(i16) -> Option<&'tex SoftwareC3dSampledTexture>,
{
    compose_stage_scene_frame_profile(
        scene,
        camera_world_q16,
        extent,
        active_palette,
        quantization,
        cmr_core::c3d_camera::gameplay_projection_default(extent[0], extent[1]),
        Some(color_key),
        resolve_texture,
    )
}

#[allow(clippy::too_many_arguments)]
fn compose_stage_scene_frame_profile<'tex, R>(
    scene: &StageScene<'_>,
    camera_world_q16: &[i32; 16],
    extent: [u16; 2],
    active_palette: &[PaletteEntry; 256],
    quantization: SoftwareC3dIndexedQuantization,
    projection: (f32, f32, f32, f32),
    color_key: Option<u8>,
    mut resolve_texture: R,
) -> Result<StageSceneFrame, StageSceneComposeError>
where
    R: FnMut(i16) -> Option<&'tex SoftwareC3dSampledTexture>,
{
    let [width, height] = extent;
    let mut pixels = vec![0u8; usize::from(width) * usize::from(height)];
    let mut depth = SoftwareC3dDepthBuffer::new_far(extent)?;
    let mut stats = StageSceneFrameStats::default();

    for op in scene.draw_order() {
        let world = scene.world(op.node);
        let Some(mesh) = scene.mesh(op.node) else {
            continue;
        };
        let Some(draw_group) = mesh.active() else {
            stats.empty_lod_nodes += 1;
            continue;
        };
        // The transform state depends only on the node (its world matrix), so it
        // is built once and reused across the node's prims. Nodes whose active
        // LOD carries Y_BILLBOARD_FLAG (`0x04`, `FUN_004a18e0`) rotate around Y
        // to face the camera's XZ forward direction using their Group-0 basis
        // row scales.
        let effective_world = if draw_group.is_y_billboard() {
            cmr_core::c3d_camera::y_billboard_world_q16(world, camera_world_q16)
        } else {
            *world
        };
        let transforms = SoftwareC3dTransformState::for_gameplay_scene_with_projection(
            width,
            height,
            camera_world_q16,
            &effective_world,
            projection,
        )?;
        for prim in draw_group.prims() {
            let Some(texture) = resolve_texture(prim.selector()) else {
                stats.unresolved_prims += 1;
                continue;
            };
            let doubled_indices;
            let indices = if draw_group.is_two_sided() {
                doubled_indices = cmr_core::c3d_mesh::two_sided_indices(prim.indices());
                doubled_indices.as_slice()
            } else {
                prim.indices()
            };
            let batch = preflight_mesh_lod_prim(prim.unique_entries_32(), indices, transforms)?;
            let outcome = if let Some(color_key) = color_key {
                compose_root_c3d_indexed_copy_color_key(
                    &batch,
                    texture,
                    &mut depth,
                    extent,
                    &mut pixels,
                    active_palette,
                    color_key,
                )?
            } else {
                compose_root_c3d_indexed_copy_no_blend(
                    &batch,
                    texture,
                    &mut depth,
                    extent,
                    &mut pixels,
                    active_palette,
                    quantization,
                )?
            };
            stats.prims_composed += 1;
            stats.covered += outcome.covered();
            stats.depth_passed += outcome.depth_passed();
            stats.depth_rejected += outcome.depth_rejected();
        }
    }

    Ok(StageSceneFrame {
        extent,
        pixels,
        palette: Box::new(*active_palette),
        depth,
        stats,
        rgba8: None,
        projection: Some(projection),
    })
}

/// Why [`compose_stage_scene_frame`] failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageSceneComposeError {
    /// The gameplay viewport could not be built for the frame extent.
    Viewport(RootMenuC3dViewportError),
    /// A prim failed transform/clip preflight, or the depth buffer was invalid.
    Raster(SoftwareC3dRasterError),
    /// A prim could not be composited into the target.
    Compose(SoftwareC3dComposeError),
    /// Independently rendered layers did not use the same target size.
    LayerExtentMismatch {
        foreground: [u16; 2],
        background: [u16; 2],
    },
}

impl From<RootMenuC3dViewportError> for StageSceneComposeError {
    fn from(error: RootMenuC3dViewportError) -> Self {
        StageSceneComposeError::Viewport(error)
    }
}

impl From<SoftwareC3dRasterError> for StageSceneComposeError {
    fn from(error: SoftwareC3dRasterError) -> Self {
        StageSceneComposeError::Raster(error)
    }
}

impl From<SoftwareC3dComposeError> for StageSceneComposeError {
    fn from(error: SoftwareC3dComposeError) -> Self {
        StageSceneComposeError::Compose(error)
    }
}

impl fmt::Display for StageSceneComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageSceneComposeError::Viewport(error) => write!(f, "stage scene viewport: {error}"),
            StageSceneComposeError::Raster(error) => write!(f, "stage scene prim raster: {error}"),
            StageSceneComposeError::Compose(error) => write!(f, "stage scene compose: {error}"),
            StageSceneComposeError::LayerExtentMismatch {
                foreground,
                background,
            } => write!(
                f,
                "stage scene layer extents differ: foreground {foreground:?}, background {background:?}"
            ),
        }
    }
}

impl std::error::Error for StageSceneComposeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StageSceneComposeError::Viewport(error) => Some(error),
            StageSceneComposeError::Raster(error) => Some(error),
            StageSceneComposeError::Compose(error) => Some(error),
            StageSceneComposeError::LayerExtentMismatch { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(extent: [u16; 2], pixels: &[u8], depth: &[f32], prims: usize) -> StageSceneFrame {
        StageSceneFrame {
            extent,
            pixels: pixels.to_vec(),
            palette: Box::new(
                [PaletteEntry {
                    red: 0,
                    green: 0,
                    blue: 0,
                    flags: 0,
                }; 256],
            ),
            depth: SoftwareC3dDepthBuffer::from_test_values(extent, depth),
            stats: StageSceneFrameStats {
                prims_composed: prims,
                ..StageSceneFrameStats::default()
            },
            rgba8: None,
            projection: None,
        }
    }

    #[test]
    fn foreground_depth_not_palette_zero_selects_layer_coverage() {
        let background = frame([3, 1], &[3, 4, 5], &[0.5, 0.5, 0.5], 1);
        let foreground = frame([3, 1], &[0, 0, 9], &[1.0, 0.25, 0.25], 2);

        let composited = foreground.over_background(&background).unwrap();
        assert_eq!(composited.pixels(), &[3, 0, 9]);
        assert_eq!(composited.stats().prims_composed, 3);
    }

    #[test]
    fn layered_frames_require_matching_extents() {
        let background = frame([1, 1], &[3], &[0.5], 1);
        let foreground = frame([2, 1], &[0, 0], &[1.0, 1.0], 2);
        assert!(matches!(
            foreground.over_background(&background),
            Err(StageSceneComposeError::LayerExtentMismatch { .. })
        ));
    }

    #[test]
    fn independently_rendered_world_graphs_merge_by_nearest_depth() {
        let base = frame([4, 1], &[1, 2, 3, 7], &[0.8, 0.2, 1.0, 0.5], 1);
        let objects = frame([4, 1], &[4, 5, 6, 8], &[0.4, 0.7, 0.9, 0.5], 2);
        let merged = base.merge_nearest(&objects).unwrap();
        assert_eq!(merged.pixels(), &[4, 2, 6, 8]);
        assert_eq!(merged.depth().values(), &[0.4, 0.2, 0.9, 0.5]);
        assert_eq!(merged.stats().prims_composed, 3);
    }
}

/// The group-2 textures of one stage/car C3D scene, prepared and sampled into
/// the software [`SoftwareC3dSampledTexture`] form, indexed by group-2 ordinal.
///
/// This resolves the material binding the C3D composer defers: each group-2 blob
/// record becomes one sampled texture at its blob-record position, so a face's
/// `group2_ordinal` (`prim.selector()`) indexes directly into
/// [`texture_for_ordinal`](Self::texture_for_ordinal) - the same positional
/// face -> texture mapping the loader resolves through its material slot table
/// (`rea.official-eu.c3d-material-binding`). An untextured ordinal (`-1`) yields
/// `None`.
///
/// Each group-2 record is classified and decoded in place, so `by_ordinal` is
/// positional (one slot per group-2 record): a slot holds the sampled texture
/// when it decoded, or `None` for an unsupported palette-less/direct-24-bit
/// source. An inline `BM` bitmap must decode or preparation fails closed.
/// `ANIM` composites resolve their selected target frame.
#[derive(Clone, Debug)]
pub struct StageC3dTextures {
    by_ordinal: Vec<Option<SoftwareC3dSampledTexture>>,
}

impl StageC3dTextures {
    /// Prepares every group-2 record of `scene` into its sampled form, indexed
    /// by group-2 ordinal.
    ///
    /// A record is classified ([`C3dResourceView`]) then decoded per kind:
    /// - `BM` bitmap bytes decode through the frontend BM path and **must**
    ///   succeed, or the whole preparation fails
    ///   ([`StageC3dTextureError::Bitmap`]) - a malformed shipped-format bitmap
    ///   is a real defect, not a material to drop.
    /// - A non-`BM`/non-`ANIM` blob is the engine inline texture format; indexed
    ///   variants decode, while direct 24-bit and palette-less sources stay empty.
    /// - An `ANIM` composite resolves frame zero unless the caller selects another.
    ///
    /// A record whose `ANIM` framing is itself malformed aborts preparation
    /// ([`StageC3dTextureError::Classify`]), since that is corrupt input, not an
    /// unsupported-but-valid texture.
    pub fn prepare(
        scene: &C3dFile,
        bitmap_limits: BitmapLimits,
    ) -> Result<Self, StageC3dTextureError> {
        Self::prepare_with_animation_frames(scene, bitmap_limits, &[])
    }

    /// Prepare materials with a caller-selected frame per `ANIM` ordinal.
    /// Missing entries select frame zero, the static-loading baseline.
    pub fn prepare_with_animation_frames(
        scene: &C3dFile,
        bitmap_limits: BitmapLimits,
        animation_frames: &[usize],
    ) -> Result<Self, StageC3dTextureError> {
        let mut by_ordinal = Vec::with_capacity(scene.blob_records().len());
        for (ordinal, record) in scene.blob_records().enumerate() {
            if matches!(
                record.classify_resource(),
                Ok(C3dResourceView::Animation(_))
            ) {
                let frame = animation_frames.get(ordinal).copied().unwrap_or(0);
                let mut frames = scene
                    .animation_frame_references(ordinal)
                    .map_err(|source| StageC3dTextureError::Animation { ordinal, source })?;
                let frame_count = frames.len();
                let target = frames
                    .nth(frame)
                    .ok_or(StageC3dTextureError::AnimationFrame {
                        ordinal,
                        frame,
                        frame_count,
                    })?
                    .map_err(|_| StageC3dTextureError::AnimationFrame {
                        ordinal,
                        frame,
                        frame_count,
                    })?
                    .target();
                by_ordinal.push(Self::sample_material(target, ordinal, bitmap_limits)?);
            } else {
                by_ordinal.push(Self::sample_material(record, ordinal, bitmap_limits)?);
            }
        }
        Ok(StageC3dTextures { by_ordinal })
    }

    fn sample_material(
        record: cmr_formats::c3d::C3dBlobRecord<'_>,
        ordinal: usize,
        bitmap_limits: BitmapLimits,
    ) -> Result<Option<SoftwareC3dSampledTexture>, StageC3dTextureError> {
        match record
            .classify_resource()
            .map_err(|source| StageC3dTextureError::Classify { ordinal, source })?
        {
            C3dResourceView::BitmapBytes(bytes) => Ok(Some(
                SoftwareC3dSampledTexture::from_bm_blob(
                    bytes,
                    FrontendC3dOwner::Stage,
                    bitmap_limits,
                )
                .map_err(|source| StageC3dTextureError::Bitmap { ordinal, source })?,
            )),
            C3dResourceView::Unknown(_) => Ok(record.engine_bitmap().ok().and_then(|bitmap| {
                SoftwareC3dSampledTexture::from_engine_bitmap(&bitmap, FrontendC3dOwner::Stage).ok()
            })),
            C3dResourceView::Animation(_) => Ok(None),
        }
    }

    /// The sampled texture for a face's group-2 ordinal, or `None` for the
    /// untextured sentinel (`-1`), an out-of-range ordinal, or an ordinal whose
    /// record decoded to an empty slot (a composite or unsupported texture).
    pub fn texture_for_ordinal(&self, ordinal: i16) -> Option<&SoftwareC3dSampledTexture> {
        usize::try_from(ordinal)
            .ok()
            .and_then(|index| self.by_ordinal.get(index))
            .and_then(|slot| slot.as_ref())
    }

    /// The number of group-2 ordinal slots (decoded or empty).
    pub fn len(&self) -> usize {
        self.by_ordinal.len()
    }

    /// Whether the scene has no group-2 records.
    pub fn is_empty(&self) -> bool {
        self.by_ordinal.is_empty()
    }
}

/// Why [`StageC3dTextures::prepare`] failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StageC3dTextureError {
    /// A group-2 record's resource framing (an `ANIM` composite header) is
    /// malformed and could not be classified.
    Classify {
        ordinal: usize,
        source: C3dResourceError,
    },
    /// An `ANIM` selection could not be resolved.
    Animation {
        ordinal: usize,
        source: C3dAnimationReferenceError,
    },
    /// The selected frame or its target ordinal was invalid.
    AnimationFrame {
        ordinal: usize,
        frame: usize,
        frame_count: usize,
    },
    /// A group-2 `BM` bitmap blob could not be decoded or sampled.
    Bitmap {
        ordinal: usize,
        source: SoftwareC3dBitmapError,
    },
}

impl fmt::Display for StageC3dTextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StageC3dTextureError::Classify { ordinal, source } => {
                write!(f, "stage C3D group-2 ordinal {ordinal} classify: {source}")
            }
            StageC3dTextureError::Animation { ordinal, source } => {
                write!(f, "stage C3D group-2 ordinal {ordinal} animation: {source}")
            }
            StageC3dTextureError::AnimationFrame {
                ordinal,
                frame,
                frame_count,
            } => write!(
                f,
                "stage C3D group-2 ordinal {ordinal} frame {frame} outside {frame_count} frames or invalid target"
            ),
            StageC3dTextureError::Bitmap { ordinal, source } => {
                write!(f, "stage C3D group-2 ordinal {ordinal} bitmap: {source}")
            }
        }
    }
}

impl std::error::Error for StageC3dTextureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StageC3dTextureError::Classify { source, .. } => Some(source),
            StageC3dTextureError::Animation { source, .. } => Some(source),
            StageC3dTextureError::AnimationFrame { .. } => None,
            StageC3dTextureError::Bitmap { source, .. } => Some(source),
        }
    }
}
