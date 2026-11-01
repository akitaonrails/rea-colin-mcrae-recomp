//! Headless presentation capture for the indexed software framebuffer.
//!
//! Capturing an owned indexed frame makes the completed compatibility output
//! observable without assigning native window, DirectDraw Flip, synchronization,
//! or windowed-copy semantics to this backend.

use std::fmt;

use cmr_core::{display::DisplayExtent, root_menu_frame::RootFrameHooks};
use cmr_formats::riff_palette::PaletteEntry;
use sha2::{Digest, Sha256};

use crate::{PaletteHandle, SoftwareIndexedBackend, SoftwareVideoError, SurfaceHandle};

/// Version tag prepended to the canonical indexed-frame digest stream.
pub const SOFTWARE_PRESENTED_FRAME_DIGEST_FORMAT: &[u8; 8] = b"CMRFRM01";

/// Bytes per pixel in the native presentation adapter's packed RGBA8 output.
pub const SOFTWARE_RGBA8_BYTES_PER_PIXEL: usize = 4;

/// A tightly packed, row-major RGBA8 copy of an indexed compatibility frame.
///
/// There is no padding between rows. Each pixel is emitted as red, green,
/// blue, then an opaque alpha byte. [`PaletteEntry::flags`] is deliberately
/// not interpreted as alpha: RIFF PAL flags are control/reserved bits, not a
/// recovered transparency channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoftwareRgba8Frame {
    extent: DisplayExtent,
    pixels: Box<[u8]>,
    cas_applied: bool,
}

impl SoftwareRgba8Frame {
    #[must_use]
    pub const fn extent(&self) -> DisplayExtent {
        self.extent
    }

    /// Packed RGBA8 bytes in top-to-bottom, left-to-right pixel order.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Number of bytes between the start of adjacent rows.
    #[must_use]
    pub const fn row_pitch(&self) -> usize {
        self.extent.width as usize * SOFTWARE_RGBA8_BYTES_PER_PIXEL
    }

    /// Returns whether GPU contrast-adaptive sharpening (CAS) was already applied.
    #[must_use]
    pub const fn cas_applied(&self) -> bool {
        self.cas_applied
    }
}

/// Validation or allocation failure while adapting an indexed frame to RGBA8.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SoftwareRgba8Error {
    InvalidExtent(DisplayExtent),
    SizeOverflow(DisplayExtent),
    InvalidIndexedPixelLength { expected: usize, actual: usize },
    AllocationFailed { bytes: usize },
}

impl fmt::Display for SoftwareRgba8Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "software RGBA8 conversion failed: {self:?}")
    }
}

impl std::error::Error for SoftwareRgba8Error {}

/// Expands a tightly packed indexed frame into a tightly packed RGBA8 frame.
///
/// The input length must be exactly `width * height`. Palette indices preserve
/// their row-major order. Output alpha is always 255; palette flags are ignored.
pub fn expand_indexed_frame_rgba8(
    extent: DisplayExtent,
    indexed_pixels: &[u8],
    palette: &[PaletteEntry; 256],
) -> Result<SoftwareRgba8Frame, SoftwareRgba8Error> {
    if extent.width == 0 || extent.height == 0 {
        return Err(SoftwareRgba8Error::InvalidExtent(extent));
    }

    let indexed_len = usize::from(extent.width)
        .checked_mul(usize::from(extent.height))
        .ok_or(SoftwareRgba8Error::SizeOverflow(extent))?;
    if indexed_pixels.len() != indexed_len {
        return Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
            expected: indexed_len,
            actual: indexed_pixels.len(),
        });
    }
    let rgba_len = indexed_len
        .checked_mul(SOFTWARE_RGBA8_BYTES_PER_PIXEL)
        .ok_or(SoftwareRgba8Error::SizeOverflow(extent))?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(rgba_len)
        .map_err(|_| SoftwareRgba8Error::AllocationFailed { bytes: rgba_len })?;
    for &index in indexed_pixels {
        let entry = palette[usize::from(index)];
        rgba.extend_from_slice(&[entry.red, entry.green, entry.blue, u8::MAX]);
    }

    debug_assert_eq!(rgba.len(), rgba_len);
    Ok(SoftwareRgba8Frame {
        extent,
        pixels: rgba.into_boxed_slice(),
        cas_applied: false,
    })
}

/// Stable checksums for one complete indexed software frame.
///
/// The canonical stream consists of [`SOFTWARE_PRESENTED_FRAME_DIGEST_FORMAT`],
/// little-endian `u16` width and height, little-endian `u32` pixel count, all
/// 256 palette entries as red/green/blue/flags bytes, and the row-major indexed
/// pixels. These checksums identify deterministic output; they do not
/// authenticate the frame or its asset provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoftwarePresentedFrameDigest {
    pub extent: DisplayExtent,
    pub indexed_pixel_count: u32,
    pub indexed_pixels_sha256: String,
    pub palette_rgb_flags_sha256: String,
    pub canonical_frame_sha256: String,
}

/// One immutable indexed frame emitted by the headless software oracle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SoftwarePresentedFrame {
    extent: DisplayExtent,
    pixels: Box<[u8]>,
    palette: Box<[PaletteEntry; 256]>,
    rgba8: Option<Box<[u8]>>,
    hi_res_rgba8: Option<(DisplayExtent, Box<[u8]>, bool)>,
}

impl SoftwarePresentedFrame {
    /// Builds a presented frame from an indexed framebuffer and its display
    /// palette, for a software path that produces indexed pixels outside the
    /// root-menu backend (such as the C3D stage scene composer). The pixel count
    /// must equal `extent.width * extent.height`; the pixels are row-major
    /// palette indices.
    pub fn from_indexed(
        extent: DisplayExtent,
        pixels: Box<[u8]>,
        palette: Box<[PaletteEntry; 256]>,
    ) -> Result<Self, SoftwareRgba8Error> {
        if extent.width == 0 || extent.height == 0 {
            return Err(SoftwareRgba8Error::InvalidExtent(extent));
        }
        let expected = usize::from(extent.width)
            .checked_mul(usize::from(extent.height))
            .ok_or(SoftwareRgba8Error::SizeOverflow(extent))?;
        if pixels.len() != expected {
            return Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(SoftwarePresentedFrame {
            extent,
            pixels,
            palette,
            rgba8: None,
            hi_res_rgba8: None,
        })
    }

    /// Attaches an optional true-color RGBA8 presentation plane (`width * height * 4`
    /// bytes) produced by the hardware GPU stage renderer so the native window host
    /// presents per-texture palettes, bilinear filtering, and Gouraud lighting
    /// without 256-color quantization while keeping the indexed plane intact.
    pub fn with_rgba8(mut self, rgba8: Box<[u8]>) -> Result<Self, SoftwareRgba8Error> {
        let expected = usize::from(self.extent.width)
            .checked_mul(usize::from(self.extent.height))
            .and_then(|px| px.checked_mul(SOFTWARE_RGBA8_BYTES_PER_PIXEL))
            .ok_or(SoftwareRgba8Error::SizeOverflow(self.extent))?;
        if rgba8.len() != expected {
            return Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected,
                actual: rgba8.len(),
            });
        }
        self.rgba8 = Some(rgba8);
        Ok(self)
    }

    /// Attaches a high-resolution true-color RGBA8 presentation plane (`extent.width * extent.height * 4`
    /// bytes) produced by the modern hardware GPU compositor.
    pub fn with_hi_res_rgba8(
        mut self,
        extent: DisplayExtent,
        rgba8: Box<[u8]>,
        cas_applied: bool,
    ) -> Result<Self, SoftwareRgba8Error> {
        if extent.width == 0 || extent.height == 0 {
            return Err(SoftwareRgba8Error::InvalidExtent(extent));
        }
        let expected = usize::from(extent.width)
            .checked_mul(usize::from(extent.height))
            .and_then(|px| px.checked_mul(SOFTWARE_RGBA8_BYTES_PER_PIXEL))
            .ok_or(SoftwareRgba8Error::SizeOverflow(extent))?;
        if rgba8.len() != expected {
            return Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected,
                actual: rgba8.len(),
            });
        }
        self.hi_res_rgba8 = Some((extent, rgba8, cas_applied));
        Ok(self)
    }

    #[must_use]
    pub fn rgba8(&self) -> Option<&[u8]> {
        self.rgba8.as_deref()
    }

    #[must_use]
    pub const fn extent(&self) -> DisplayExtent {
        self.extent
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[must_use]
    pub fn palette(&self) -> &[PaletteEntry; 256] {
        &self.palette
    }

    /// Produces packed RGBA8 bytes suitable for a streaming texture upload.
    ///
    /// Alpha is always opaque; palette flags are not an alpha channel.
    pub fn expand_rgba8(&self) -> Result<SoftwareRgba8Frame, SoftwareRgba8Error> {
        if let Some((extent, rgba8, cas_applied)) = &self.hi_res_rgba8 {
            return Ok(SoftwareRgba8Frame {
                extent: *extent,
                pixels: rgba8.clone(),
                cas_applied: *cas_applied,
            });
        }
        if let Some(rgba8) = &self.rgba8 {
            let mut pixels = rgba8.clone();
            for px in pixels.as_chunks_mut::<4>().0 {
                px[3] = u8::MAX;
            }
            return Ok(SoftwareRgba8Frame {
                extent: self.extent,
                pixels,
                cas_applied: false,
            });
        }
        expand_indexed_frame_rgba8(self.extent, &self.pixels, &self.palette)
    }

    /// Consumes this frame and produces packed RGBA8 bytes without cloning an
    /// attached true-color or high-resolution plane.
    pub fn into_rgba8(self) -> Result<SoftwareRgba8Frame, SoftwareRgba8Error> {
        if let Some((extent, pixels, cas_applied)) = self.hi_res_rgba8 {
            return Ok(SoftwareRgba8Frame {
                extent,
                pixels,
                cas_applied,
            });
        }
        if let Some(mut pixels) = self.rgba8 {
            for px in pixels.as_chunks_mut::<4>().0 {
                px[3] = u8::MAX;
            }
            return Ok(SoftwareRgba8Frame {
                extent: self.extent,
                pixels,
                cas_applied: false,
            });
        }
        expand_indexed_frame_rgba8(self.extent, &self.pixels, &self.palette)
    }

    /// Returns this frame under a fullscreen black quad of `alpha`
    /// (`FUN_00423170`, blended SRCALPHA/INVSRCALPHA): each colour scaled by
    /// `(255 - alpha) / 255`, then matched to the nearest palette entry (L1,
    /// first minimum) so what is drawn later keeps its own colours. The
    /// re-quantization is this indexed backend's policy.
    #[must_use]
    pub fn under_black_quad(&self, alpha: u8) -> Self {
        if alpha == 0 {
            return self.clone();
        }
        let keep = u32::from(0xff - alpha);
        let scale = |c: u8| ((u32::from(c) * keep) / 255) as u8;
        let map: [u8; 256] = std::array::from_fn(|i| {
            let e = self.palette[i];
            let rgb = [scale(e.red), scale(e.green), scale(e.blue)];
            let mut best = (u32::MAX, 0u8);
            for (index, c) in self.palette.iter().enumerate() {
                let d = u32::from(rgb[0].abs_diff(c.red))
                    + u32::from(rgb[1].abs_diff(c.green))
                    + u32::from(rgb[2].abs_diff(c.blue));
                if d < best.0 {
                    best = (d, index as u8);
                }
            }
            best.1
        });
        let rgba8 = self.rgba8.as_ref().map(|rgba| {
            let mut out = rgba.clone();
            for px in out.as_chunks_mut::<4>().0 {
                px[0] = scale(px[0]);
                px[1] = scale(px[1]);
                px[2] = scale(px[2]);
            }
            out
        });
        Self {
            extent: self.extent,
            pixels: self.pixels.iter().map(|&p| map[usize::from(p)]).collect(),
            palette: self.palette.clone(),
            rgba8,
            hi_res_rgba8: None,
        }
    }

    /// Returns this frame with every palette RGB channel scaled by `level/255`
    /// (flags preserved): a whole-frame modulate toward black. Because every
    /// pixel shares one palette, a palette scale and a fullscreen
    /// modulate-by-`(level,level,level)` quad produce the identical per-pixel
    /// multiply; `0xff` leaves the frame unchanged (full brightness) and `0`
    /// drives it to black. The pixels are shared unchanged; only the palette is
    /// rebuilt, so this cannot fail. (The screen fade is a black quad drawn
    /// under later 2D, [`Self::under_black_quad`].)
    #[must_use]
    pub fn modulated(&self, level: u8) -> Self {
        let scale = u32::from(level);
        let mut palette = self.palette.clone();
        for entry in palette.iter_mut() {
            entry.red = ((u32::from(entry.red) * scale) / 255) as u8;
            entry.green = ((u32::from(entry.green) * scale) / 255) as u8;
            entry.blue = ((u32::from(entry.blue) * scale) / 255) as u8;
        }
        let rgba8 = self.rgba8.as_ref().map(|rgba| {
            let mut out = rgba.clone();
            for px in out.as_chunks_mut::<4>().0 {
                px[0] = ((u32::from(px[0]) * scale) / 255) as u8;
                px[1] = ((u32::from(px[1]) * scale) / 255) as u8;
                px[2] = ((u32::from(px[2]) * scale) / 255) as u8;
            }
            out
        });
        Self {
            extent: self.extent,
            pixels: self.pixels.clone(),
            palette,
            rgba8,
            hi_res_rgba8: None,
        }
    }

    /// Computes stable component and canonical checksums for this frame.
    #[must_use]
    pub fn digest_summary(&self) -> SoftwarePresentedFrameDigest {
        let indexed_pixel_count = u32::from(self.extent.width) * u32::from(self.extent.height);
        debug_assert_eq!(self.pixels.len(), indexed_pixel_count as usize);

        let mut palette_digest = Sha256::new();
        for entry in self.palette.iter() {
            palette_digest.update([entry.red, entry.green, entry.blue, entry.flags]);
        }

        let mut canonical_digest = Sha256::new();
        canonical_digest.update(SOFTWARE_PRESENTED_FRAME_DIGEST_FORMAT);
        canonical_digest.update(self.extent.width.to_le_bytes());
        canonical_digest.update(self.extent.height.to_le_bytes());
        canonical_digest.update(indexed_pixel_count.to_le_bytes());
        for entry in self.palette.iter() {
            canonical_digest.update([entry.red, entry.green, entry.blue, entry.flags]);
        }
        canonical_digest.update(&self.pixels);

        SoftwarePresentedFrameDigest {
            extent: self.extent,
            indexed_pixel_count,
            indexed_pixels_sha256: sha256_hex(Sha256::digest(&self.pixels)),
            palette_rgb_flags_sha256: sha256_hex(palette_digest.finalize()),
            canonical_frame_sha256: sha256_hex(canonical_digest.finalize()),
        }
    }
}

fn sha256_hex(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = digest.as_ref();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for &byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SoftwarePresentationError {
    InvalidSequence(&'static str),
    Video(SoftwareVideoError),
    DestinationIsNotBackBuffer {
        destination: SurfaceHandle,
    },
    DestinationExtentMismatch {
        expected: DisplayExtent,
        actual: DisplayExtent,
    },
    MissingActivePalette,
    PaletteNotAttached,
    InvalidPixelLength {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for SoftwarePresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "software headless presentation failed: {self:?}")
    }
}

impl std::error::Error for SoftwarePresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Video(source) => Some(source),
            _ => None,
        }
    }
}

impl RootFrameHooks for SoftwareIndexedBackend {
    type FinalError = SoftwarePresentationError;

    fn present_root_frame(
        &mut self,
        destination: Self::Surface,
        destination_extent: DisplayExtent,
    ) -> Result<(), Self::FinalError> {
        if self.c3d.scene_is_active() {
            return Err(SoftwarePresentationError::InvalidSequence(
                "present_root_frame during active C3D scene",
            ));
        }
        if self.back != Some(destination) {
            return Err(SoftwarePresentationError::DestinationIsNotBackBuffer { destination });
        }
        if !self.palette_uploaded {
            return Err(SoftwarePresentationError::MissingActivePalette);
        }
        let palette = self
            .palette
            .ok_or(SoftwarePresentationError::MissingActivePalette)?;
        let frame = {
            let surface = self
                .surface(destination)
                .map_err(SoftwarePresentationError::Video)?;
            let actual = DisplayExtent {
                width: surface.width as u16,
                height: surface.height as u16,
            };
            if actual != destination_extent {
                return Err(SoftwarePresentationError::DestinationExtentMismatch {
                    expected: destination_extent,
                    actual,
                });
            }
            if surface.palette != Some(PaletteHandle(0)) {
                return Err(SoftwarePresentationError::PaletteNotAttached);
            }
            let expected =
                usize::from(destination_extent.width) * usize::from(destination_extent.height);
            if surface.pixels.len() != expected {
                return Err(SoftwarePresentationError::InvalidPixelLength {
                    expected,
                    actual: surface.pixels.len(),
                });
            }
            SoftwarePresentedFrame {
                extent: destination_extent,
                pixels: surface.pixels.clone().into_boxed_slice(),
                palette: Box::new(palette),
                rgba8: None,
                hi_res_rgba8: None,
            }
        };
        self.presented_frame = Some(frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use cmr_core::{
        display::{
            CooperativeMode, DisplaySurfaces, IndexedMode, IndexedVideoBackend, SurfaceTopology,
        },
        root_menu_c3d_scene::RootMenuC3dSceneBackend,
        root_menu_c3d_transform::{ROOT_MENU_C3D_VIEW, root_menu_c3d_viewport},
    };

    use super::*;

    const EXTENT: DisplayExtent = DisplayExtent {
        width: 4,
        height: 2,
    };

    fn palette(seed: u8) -> [PaletteEntry; 256] {
        std::array::from_fn(|index| PaletteEntry {
            red: seed.wrapping_add(index as u8),
            green: seed,
            blue: index as u8,
            flags: (index as u8) ^ seed,
        })
    }

    #[test]
    fn from_indexed_builds_a_presentable_frame_and_rejects_bad_input() {
        let pixels = vec![3u8; 8].into_boxed_slice(); // 4 * 2
        let frame =
            SoftwarePresentedFrame::from_indexed(EXTENT, pixels, Box::new(palette(1))).unwrap();
        assert_eq!(frame.extent(), EXTENT);
        assert_eq!(frame.pixels(), &[3u8; 8]);
        assert_eq!(frame.palette(), &palette(1));
        // It expands exactly like a backend-emitted frame.
        assert_eq!(
            frame.expand_rgba8().unwrap(),
            expand_indexed_frame_rgba8(EXTENT, &[3u8; 8], &palette(1)).unwrap()
        );

        // One pixel short is rejected with the expected/actual lengths.
        let short = vec![0u8; 7].into_boxed_slice();
        assert_eq!(
            SoftwarePresentedFrame::from_indexed(EXTENT, short, Box::new(palette(1))),
            Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected: 8,
                actual: 7,
            })
        );

        // A zero extent is rejected before the length check.
        let zero = DisplayExtent {
            width: 0,
            height: 2,
        };
        assert_eq!(
            SoftwarePresentedFrame::from_indexed(
                zero,
                Vec::<u8>::new().into_boxed_slice(),
                Box::new(palette(1)),
            ),
            Err(SoftwareRgba8Error::InvalidExtent(zero))
        );
    }

    #[test]
    fn modulated_scales_the_palette_rgb_and_preserves_flags_and_pixels() {
        let pixels = vec![5u8; 8].into_boxed_slice();
        let frame =
            SoftwarePresentedFrame::from_indexed(EXTENT, pixels, Box::new(palette(10))).unwrap();

        // Full brightness (0xff) leaves the frame unchanged.
        let full = frame.modulated(0xff);
        assert_eq!(full.palette(), frame.palette());
        assert_eq!(full.pixels(), frame.pixels());

        // Black (0) zeroes every RGB channel but keeps the flags and pixels.
        let black = frame.modulated(0);
        for (scaled, original) in black.palette().iter().zip(frame.palette().iter()) {
            assert_eq!(scaled.red, 0);
            assert_eq!(scaled.green, 0);
            assert_eq!(scaled.blue, 0);
            assert_eq!(scaled.flags, original.flags);
        }
        assert_eq!(black.pixels(), frame.pixels());
        assert_eq!(black.extent(), frame.extent());

        // Half (0x80) scales each channel by 128/255 with truncation.
        let half = frame.modulated(0x80);
        for (scaled, original) in half.palette().iter().zip(frame.palette().iter()) {
            assert_eq!(scaled.red, ((u32::from(original.red) * 0x80) / 255) as u8);
            assert_eq!(
                scaled.green,
                ((u32::from(original.green) * 0x80) / 255) as u8
            );
            assert_eq!(scaled.blue, ((u32::from(original.blue) * 0x80) / 255) as u8);
            assert_eq!(scaled.flags, original.flags);
        }
    }

    fn surfaces_only() -> (SoftwareIndexedBackend, DisplaySurfaces<SurfaceHandle>) {
        let mut backend = SoftwareIndexedBackend::default();
        backend.create_device().unwrap();
        backend
            .set_cooperative_mode(CooperativeMode::Windowed)
            .unwrap();
        let surfaces = backend
            .create_surfaces(
                SurfaceTopology::PrimaryAndOffscreen,
                IndexedMode {
                    extent: EXTENT,
                    bits_per_pixel: 8,
                },
            )
            .unwrap();
        (backend, surfaces)
    }

    fn initialized_backend() -> (
        SoftwareIndexedBackend,
        DisplaySurfaces<SurfaceHandle>,
        PaletteHandle,
    ) {
        let (mut backend, surfaces) = surfaces_only();
        let entries = palette(3);
        let palette = backend.create_palette(&entries).unwrap();
        backend
            .attach_palette(surfaces.visible_front, palette)
            .unwrap();
        backend.attach_palette(surfaces.back, palette).unwrap();
        backend.upload_palette(palette, &entries).unwrap();
        (backend, surfaces, palette)
    }

    fn set_back_pixels(backend: &mut SoftwareIndexedBackend, back: SurfaceHandle, pixels: &[u8]) {
        backend.surfaces[back.0].as_mut().unwrap().pixels = pixels.to_vec();
    }

    #[test]
    fn capture_owns_pixels_and_palette_without_mutating_surfaces() {
        let (mut backend, surfaces, palette_handle) = initialized_backend();
        let first_pixels = [1, 2, 3, 4, 5, 6, 7, 8];
        set_back_pixels(&mut backend, surfaces.back, &first_pixels);
        let front_before = backend.visible_pixels().unwrap().to_vec();
        let back_before = backend.back_pixels().unwrap().to_vec();

        backend.present_root_frame(surfaces.back, EXTENT).unwrap();

        let captured = backend.presented_frame().unwrap().clone();
        assert_eq!(captured.extent(), EXTENT);
        assert_eq!(captured.pixels(), first_pixels);
        assert_eq!(captured.palette(), &palette(3));
        assert_eq!(backend.visible_pixels().unwrap(), front_before);
        assert_eq!(backend.back_pixels().unwrap(), back_before);

        set_back_pixels(&mut backend, surfaces.back, &[9; 8]);
        backend
            .upload_palette(palette_handle, &palette(17))
            .unwrap();
        assert_eq!(captured.pixels(), first_pixels);
        assert_eq!(captured.palette(), &palette(3));
        assert_eq!(backend.presented_frame(), Some(&captured));

        backend.present_root_frame(surfaces.back, EXTENT).unwrap();
        assert_eq!(backend.presented_frame().unwrap().pixels(), &[9; 8]);
        assert_eq!(backend.presented_frame().unwrap().palette(), &palette(17));
        assert_eq!(backend.visible_pixels().unwrap(), front_before);
        assert_eq!(backend.back_pixels().unwrap(), &[9; 8]);
    }

    #[test]
    fn completed_frame_can_be_taken_without_cloning_and_the_slot_is_cleared() {
        let (mut backend, surfaces, _) = initialized_backend();
        let first_pixels = [1, 2, 3, 4, 5, 6, 7, 8];
        set_back_pixels(&mut backend, surfaces.back, &first_pixels);
        backend.present_root_frame(surfaces.back, EXTENT).unwrap();

        let taken = backend.take_presented_frame().unwrap();

        assert_eq!(taken.extent(), EXTENT);
        assert_eq!(taken.pixels(), first_pixels);
        assert!(backend.presented_frame().is_none());
        assert!(backend.take_presented_frame().is_none());

        let second_pixels = [9, 10, 11, 12, 13, 14, 15, 16];
        set_back_pixels(&mut backend, surfaces.back, &second_pixels);
        backend.present_root_frame(surfaces.back, EXTENT).unwrap();
        assert_eq!(
            backend.take_presented_frame().unwrap().pixels(),
            second_pixels
        );
    }

    #[test]
    fn every_capture_validation_error_preserves_prior_snapshot_and_surfaces() {
        let (mut backend, surfaces, _) = initialized_backend();
        let baseline_pixels = [11, 12, 13, 14, 15, 16, 17, 18];
        set_back_pixels(&mut backend, surfaces.back, &baseline_pixels);
        backend.present_root_frame(surfaces.back, EXTENT).unwrap();
        let baseline = backend.presented_frame().unwrap().clone();
        let front_before = backend.visible_pixels().unwrap().to_vec();
        let back_before = backend.back_pixels().unwrap().to_vec();

        assert!(matches!(
            backend.present_root_frame(surfaces.visible_front, EXTENT),
            Err(SoftwarePresentationError::DestinationIsNotBackBuffer { .. })
        ));
        assert!(matches!(
            backend.present_root_frame(
                surfaces.back,
                DisplayExtent {
                    width: EXTENT.width - 1,
                    height: EXTENT.height,
                }
            ),
            Err(SoftwarePresentationError::DestinationExtentMismatch { .. })
        ));

        backend.set_root_c3d_view(ROOT_MENU_C3D_VIEW).unwrap();
        backend
            .set_root_c3d_viewport(root_menu_c3d_viewport(EXTENT).unwrap())
            .unwrap();
        backend.begin_root_c3d_scene(surfaces.back).unwrap();
        assert_eq!(
            backend.present_root_frame(surfaces.back, EXTENT),
            Err(SoftwarePresentationError::InvalidSequence(
                "present_root_frame during active C3D scene"
            ))
        );
        backend.end_root_c3d_scene().unwrap();

        backend.surfaces[surfaces.back.0].as_mut().unwrap().palette = None;
        assert_eq!(
            backend.present_root_frame(surfaces.back, EXTENT),
            Err(SoftwarePresentationError::PaletteNotAttached)
        );
        backend.surfaces[surfaces.back.0].as_mut().unwrap().palette = Some(PaletteHandle(0));

        backend.surfaces[surfaces.back.0]
            .as_mut()
            .unwrap()
            .pixels
            .pop();
        assert_eq!(
            backend.present_root_frame(surfaces.back, EXTENT),
            Err(SoftwarePresentationError::InvalidPixelLength {
                expected: 8,
                actual: 7,
            })
        );
        backend.surfaces[surfaces.back.0]
            .as_mut()
            .unwrap()
            .pixels
            .push(18);

        assert_eq!(backend.presented_frame(), Some(&baseline));
        assert_eq!(backend.visible_pixels().unwrap(), front_before);
        assert_eq!(backend.back_pixels().unwrap(), back_before);
    }

    #[test]
    fn capture_requires_an_uploaded_active_palette() {
        let (mut backend, surfaces) = surfaces_only();
        assert_eq!(
            backend.present_root_frame(surfaces.back, EXTENT),
            Err(SoftwarePresentationError::MissingActivePalette)
        );
        assert!(backend.presented_frame().is_none());
    }

    #[test]
    fn digest_summary_matches_the_canonical_version_one_vector() {
        let palette = std::array::from_fn(|index| {
            let index = index as u8;
            PaletteEntry {
                red: index,
                green: 255 - index,
                blue: index ^ 0x5a,
                flags: index.rotate_left(1),
            }
        });
        let frame = SoftwarePresentedFrame {
            extent: DisplayExtent {
                width: 2,
                height: 2,
            },
            pixels: Box::from([0, 1, 2, 255]),
            palette: Box::new(palette),
            rgba8: None,
            hi_res_rgba8: None,
        };

        assert_eq!(SOFTWARE_PRESENTED_FRAME_DIGEST_FORMAT, b"CMRFRM01");
        assert_eq!(
            frame.digest_summary(),
            SoftwarePresentedFrameDigest {
                extent: frame.extent(),
                indexed_pixel_count: 4,
                indexed_pixels_sha256:
                    "3d1f57c984978ef98a18378c8166c1cb8ede02c03eeb6aee7e2f121dfeee3e56".to_owned(),
                palette_rgb_flags_sha256:
                    "b11f77b1394c63690850b4f92ae5dd451be670d81969e31508b834c434cdf88d".to_owned(),
                canonical_frame_sha256:
                    "3316bd72a0814a72a34076a21cfaf97b9ae5f47dc95e32e820f1cf84f087a154".to_owned(),
            }
        );
    }

    #[test]
    fn digest_summary_matches_the_all_zero_boundary_vector() {
        let frame = SoftwarePresentedFrame {
            extent: DisplayExtent {
                width: 1,
                height: 1,
            },
            pixels: Box::from([0]),
            palette: Box::new(
                [PaletteEntry {
                    red: 0,
                    green: 0,
                    blue: 0,
                    flags: 0,
                }; 256],
            ),
            rgba8: None,
            hi_res_rgba8: None,
        };

        assert_eq!(
            frame.digest_summary().canonical_frame_sha256,
            "fce9803d5b73c116ad1d6c650f2db4bd19aaea7285fe3f9355abaa9b5b07224c"
        );
    }

    #[test]
    fn canonical_digest_is_sensitive_to_extent_palette_and_pixels() {
        let palette = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];
        let baseline = SoftwarePresentedFrame {
            extent: DisplayExtent {
                width: 2,
                height: 2,
            },
            pixels: Box::from([0, 1, 2, 3]),
            palette: Box::new(palette),
            rgba8: None,
            hi_res_rgba8: None,
        };
        let mut changed_extent = baseline.clone();
        changed_extent.extent = DisplayExtent {
            width: 1,
            height: 4,
        };
        let mut changed_palette = baseline.clone();
        changed_palette.palette[0].flags = 1;
        let mut changed_pixels = baseline.clone();
        changed_pixels.pixels[0] = 1;

        let baseline_digest = baseline.digest_summary();
        let extent_digest = changed_extent.digest_summary();
        let palette_digest = changed_palette.digest_summary();
        let pixels_digest = changed_pixels.digest_summary();

        assert_ne!(
            extent_digest.canonical_frame_sha256,
            baseline_digest.canonical_frame_sha256
        );
        assert_ne!(
            palette_digest.canonical_frame_sha256,
            baseline_digest.canonical_frame_sha256
        );
        assert_ne!(
            pixels_digest.canonical_frame_sha256,
            baseline_digest.canonical_frame_sha256
        );
        assert_eq!(
            extent_digest.indexed_pixels_sha256,
            baseline_digest.indexed_pixels_sha256
        );
        assert_eq!(
            palette_digest.indexed_pixels_sha256,
            baseline_digest.indexed_pixels_sha256
        );
        assert_eq!(
            pixels_digest.palette_rgb_flags_sha256,
            baseline_digest.palette_rgb_flags_sha256
        );
    }

    #[test]
    fn rgba8_expansion_preserves_row_major_order_and_has_no_row_padding() {
        let mut palette = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];
        palette[1] = PaletteEntry {
            red: 10,
            green: 11,
            blue: 12,
            flags: 0,
        };
        palette[2] = PaletteEntry {
            red: 20,
            green: 21,
            blue: 22,
            flags: 0,
        };
        palette[3] = PaletteEntry {
            red: 30,
            green: 31,
            blue: 32,
            flags: 0,
        };
        let extent = DisplayExtent {
            width: 3,
            height: 2,
        };

        let rgba = expand_indexed_frame_rgba8(extent, &[1, 2, 3, 3, 2, 1], &palette).unwrap();

        assert_eq!(rgba.extent(), extent);
        assert_eq!(rgba.row_pitch(), 12);
        assert_eq!(rgba.pixels().len(), rgba.row_pitch() * 2);
        assert_eq!(
            rgba.pixels(),
            &[
                10, 11, 12, 255, 20, 21, 22, 255, 30, 31, 32, 255, 30, 31, 32, 255, 20, 21, 22,
                255, 10, 11, 12, 255,
            ]
        );
    }

    #[test]
    fn rgba8_expansion_uses_opaque_alpha_independently_of_palette_flags() {
        let palette = std::array::from_fn(|index| PaletteEntry {
            red: 7,
            green: 8,
            blue: 9,
            flags: index as u8,
        });

        let rgba = expand_indexed_frame_rgba8(
            DisplayExtent {
                width: 4,
                height: 1,
            },
            &[0, 1, 128, 255],
            &palette,
        )
        .unwrap();

        assert_eq!(
            rgba.pixels(),
            &[7, 8, 9, 255, 7, 8, 9, 255, 7, 8, 9, 255, 7, 8, 9, 255]
        );
    }

    #[test]
    fn rgba8_expansion_rejects_short_and_long_index_buffers() {
        let palette = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];
        let extent = DisplayExtent {
            width: 2,
            height: 2,
        };

        assert_eq!(
            expand_indexed_frame_rgba8(extent, &[0; 3], &palette),
            Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected: 4,
                actual: 3,
            })
        );
        assert_eq!(
            expand_indexed_frame_rgba8(extent, &[0; 5], &palette),
            Err(SoftwareRgba8Error::InvalidIndexedPixelLength {
                expected: 4,
                actual: 5,
            })
        );
    }

    #[test]
    fn rgba8_expansion_rejects_zero_width_or_height() {
        let palette = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];

        for extent in [
            DisplayExtent {
                width: 0,
                height: 1,
            },
            DisplayExtent {
                width: 1,
                height: 0,
            },
        ] {
            assert_eq!(
                expand_indexed_frame_rgba8(extent, &[], &palette),
                Err(SoftwareRgba8Error::InvalidExtent(extent))
            );
        }
    }

    #[test]
    fn captured_frame_rgba8_method_uses_the_same_contract() {
        let mut palette = [PaletteEntry {
            red: 0,
            green: 0,
            blue: 0,
            flags: 0,
        }; 256];
        palette[42] = PaletteEntry {
            red: 1,
            green: 2,
            blue: 3,
            flags: 254,
        };
        let frame = SoftwarePresentedFrame {
            extent: DisplayExtent {
                width: 1,
                height: 1,
            },
            pixels: Box::from([42]),
            palette: Box::new(palette),
            rgba8: None,
            hi_res_rgba8: None,
        };

        assert_eq!(frame.expand_rgba8().unwrap().pixels(), &[1, 2, 3, 255]);
    }
}
