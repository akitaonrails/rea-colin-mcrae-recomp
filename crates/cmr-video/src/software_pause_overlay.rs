//! Indexed software composition of the pause menu
//! ([`cmr_core::pause_menu_draw::PauseDrawPlan`]) over the world frame.
//!
//! Order follows the native flush: the panel quad (untextured, blended
//! source-over by its alpha byte, or opaque) and then the lane-1 text. The
//! text takes the queued glyph path: each atlas texel converts to ARGB4444
//! (black transparent, every other colour opaque, as the recovered upload
//! rule does) and is multiplied by the colour record, the `MODULATE` texture
//! blend. Rasterising those primitives is driver-defined, so the blend
//! rounding (`x * y / 255`), the nibble expansion (`n * 17`) and the
//! nearest-palette quantization (full-palette Manhattan distance, lowest index
//! on ties, shared with the race HUD) are this backend's documented policy.

use cmr_core::overlay_alt::OverlayQuad;
use cmr_core::pause_menu_draw::{PauseDrawPlan, PauseFont, PauseTextRun};
use cmr_formats::font::BitmapFont;

use crate::software_race_hud::{modulate_channel, nearest_palette_index, source_over_channel};
use crate::{SoftwarePresentedFrame, SoftwareRgba8Error};

/// What [`compose_pause_overlay`] drew.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PauseOverlayOutcome {
    pub panel_pixels: usize,
    pub glyphs_drawn: usize,
}

/// The ARGB4444 texel of one atlas palette entry, expanded back to 8 bits,
/// or `None` for the transparent black.
fn texel(rgb: [u8; 3]) -> Option<[u8; 3]> {
    if rgb == [0, 0, 0] {
        return None;
    }
    Some(rgb.map(|channel| (channel >> 4) * 17))
}

/// Draw the panel and the text runs of `plan` over `frame`.
pub fn compose_pause_overlay<'a>(
    frame: SoftwarePresentedFrame,
    plan: &PauseDrawPlan,
    fonts: impl Fn(PauseFont) -> &'a BitmapFont,
) -> Result<(SoftwarePresentedFrame, PauseOverlayOutcome), SoftwareRgba8Error> {
    compose_text_overlay(frame, &[(plan.panel, plan.panel_color)], &plan.runs, fonts)
}

/// Draw untextured rectangles (in order, each blended source-over by its
/// alpha byte) and then the text runs over `frame`.
pub fn compose_text_overlay<'a>(
    frame: SoftwarePresentedFrame,
    quads: &[(OverlayQuad, [u8; 4])],
    runs: &[PauseTextRun],
    fonts: impl Fn(PauseFont) -> &'a BitmapFont,
) -> Result<(SoftwarePresentedFrame, PauseOverlayOutcome), SoftwareRgba8Error> {
    let extent = frame.extent();
    let width = usize::from(extent.width);
    let height = usize::from(extent.height);
    let palette = Box::new(*frame.palette());
    let mut pixels = frame.pixels().to_vec();
    let mut rgba8 = frame.rgba8().map(<[u8]>::to_vec);
    let mut outcome = PauseOverlayOutcome::default();

    for &(quad, [red, green, blue, alpha]) in quads {
        let left = usize::from(quad.x).min(width);
        let top = usize::from(quad.y).min(height);
        let right = (usize::from(quad.x) + usize::from(quad.w)).min(width);
        let bottom = (usize::from(quad.y) + usize::from(quad.h)).min(height);
        let mut panel_cache = [None::<u8>; 256];
        for y in top..bottom {
            for x in left..right {
                let at = y * width + x;
                let pixel = &mut pixels[at];
                let destination = usize::from(*pixel);
                *pixel = *panel_cache[destination].get_or_insert_with(|| {
                    let under = palette[destination];
                    let rgb = [
                        source_over_channel(red, under.red, alpha),
                        source_over_channel(green, under.green, alpha),
                        source_over_channel(blue, under.blue, alpha),
                    ];
                    nearest_palette_index(rgb, &palette)
                });
                if let Some(rgba) = rgba8.as_deref_mut()
                    && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                {
                    px[0] = source_over_channel(red, px[0], alpha);
                    px[1] = source_over_channel(green, px[1], alpha);
                    px[2] = source_over_channel(blue, px[2], alpha);
                }
                outcome.panel_pixels += 1;
            }
        }
    }

    for run in runs {
        let font = fonts(run.font);
        let atlas = font.atlas();
        let atlas_width = atlas.width as usize;
        let (clip_x, clip_y, clip_right, clip_bottom) =
            run.clip
                .map_or((0, 0, width as i32, height as i32), |clip| {
                    (
                        i32::from(clip.x),
                        i32::from(clip.y),
                        i32::from(clip.x) + i32::from(clip.w),
                        i32::from(clip.y) + i32::from(clip.h),
                    )
                });
        let (clip_x, clip_y) = (clip_x.max(0), clip_y.max(0));
        let (clip_right, clip_bottom) =
            (clip_right.min(width as i32), clip_bottom.min(height as i32));
        let mut glyph_cache = [None::<Option<(u8, [u8; 3])>>; 256];
        for blit in &run.plan.blits {
            outcome.glyphs_drawn += 1;
            for row in 0..i32::from(blit.source.height) {
                let target_y = blit.destination.y + row;
                if target_y < clip_y || target_y >= clip_bottom {
                    continue;
                }
                for column in 0..i32::from(blit.source.width) {
                    let target_x = blit.destination.x + column;
                    if target_x < clip_x || target_x >= clip_right {
                        continue;
                    }
                    let source_x = i32::from(blit.source.x) + column;
                    let source_y = i32::from(blit.source.y) + row;
                    let Some(&index) = usize::try_from(source_y)
                        .ok()
                        .zip(usize::try_from(source_x).ok())
                        .filter(|&(_, x)| x < atlas_width)
                        .and_then(|(y, x)| atlas.pixels.get(y * atlas_width + x))
                    else {
                        continue;
                    };
                    let resolved = *glyph_cache[usize::from(index)].get_or_insert_with(|| {
                        let entry = atlas.palette.get(usize::from(index))?;
                        let texel = texel([entry.red, entry.green, entry.blue])?;
                        let rgb = [
                            modulate_channel(texel[0], run.color[0]),
                            modulate_channel(texel[1], run.color[1]),
                            modulate_channel(texel[2], run.color[2]),
                        ];
                        Some((nearest_palette_index(rgb, &palette), rgb))
                    });
                    if let Some((color, rgb)) = resolved {
                        let at = target_y as usize * width + target_x as usize;
                        pixels[at] = color;
                        if let Some(rgba) = rgba8.as_deref_mut()
                            && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                        {
                            px[0] = rgb[0];
                            px[1] = rgb[1];
                            px[2] = rgb[2];
                        }
                    }
                }
            }
        }
    }

    let mut frame =
        SoftwarePresentedFrame::from_indexed(extent, pixels.into_boxed_slice(), palette)?;
    if let Some(rgba8) = rgba8 {
        frame = frame.with_rgba8(rgba8.into_boxed_slice())?;
    }
    Ok((frame, outcome))
}

/// Draw the results' queued icons (`FUN_0049b1d0`, lane 1, after the text)
/// over `frame`: each source rectangle of its sheet stretched to the
/// destination (nearest texel; the queue's flush is not decoded, so the
/// sampling is this backend's policy), cut to its clip and the frame, with
/// the sheet's black keyed out as the upload's ARGB4444 rule does and every
/// other colour (nibble-expanded) quantized to the frame's palette. A
/// missing sheet draws nothing.
pub fn compose_icon_overlay<'a>(
    frame: SoftwarePresentedFrame,
    icons: &[cmr_core::results_draw::IconBlit],
    sheets: impl Fn(cmr_core::results_draw::IconSheet) -> Option<&'a cmr_formats::bitmap::IndexedBitmap>,
) -> Result<SoftwarePresentedFrame, SoftwareRgba8Error> {
    let extent = frame.extent();
    let (width, height) = (i32::from(extent.width), i32::from(extent.height));
    let palette = Box::new(*frame.palette());
    let mut pixels = frame.pixels().to_vec();
    let mut rgba8 = frame.rgba8().map(<[u8]>::to_vec);
    for icon in icons {
        let Some(sheet) = sheets(icon.sheet) else {
            continue;
        };
        let [sx, sy, sw, sh] = icon.src.map(i32::from);
        let [dx, dy, dw, dh] = icon.dst.map(i32::from);
        if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
            continue;
        }
        let (mut left, mut top, mut right, mut bottom) = (0, 0, width, height);
        if let Some(clip) = icon.clip {
            left = left.max(i32::from(clip.x));
            top = top.max(i32::from(clip.y));
            right = right.min(i32::from(clip.x) + i32::from(clip.w));
            bottom = bottom.min(i32::from(clip.y) + i32::from(clip.h));
        }
        let mut cache = [None::<Option<(u8, [u8; 3])>>; 256];
        for row in 0..dh {
            let y = dy + row;
            let texel_y = sy + row * sh / dh;
            if y < top || y >= bottom || texel_y < 0 || texel_y as u32 >= sheet.height {
                continue;
            }
            for column in 0..dw {
                let x = dx + column;
                let texel_x = sx + column * sw / dw;
                if x < left || x >= right || texel_x < 0 || texel_x as u32 >= sheet.width {
                    continue;
                }
                let index =
                    sheet.pixels[texel_y as usize * sheet.width as usize + texel_x as usize];
                let colour = *cache[usize::from(index)].get_or_insert_with(|| {
                    let entry = sheet.palette.get(usize::from(index))?;
                    texel([entry.red, entry.green, entry.blue])
                        .map(|rgb| (nearest_palette_index(rgb, &palette), rgb))
                });
                if let Some((colour, rgb)) = colour {
                    let at = y as usize * width as usize + x as usize;
                    pixels[at] = colour;
                    if let Some(rgba) = rgba8.as_deref_mut()
                        && let Some(px) = rgba.get_mut(at * 4..at * 4 + 4)
                    {
                        px[0] = rgb[0];
                        px[1] = rgb[1];
                        px[2] = rgb[2];
                    }
                }
            }
        }
    }
    let mut frame =
        SoftwarePresentedFrame::from_indexed(extent, pixels.into_boxed_slice(), palette)?;
    if let Some(rgba8) = rgba8 {
        frame = frame.with_rgba8(rgba8.into_boxed_slice())?;
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmr_core::display::DisplayExtent;
    use cmr_core::pause_menu_draw::text_q16;
    use cmr_formats::font::{FontLimits, parse_legacy_font};
    use cmr_formats::riff_palette::PaletteEntry;

    fn palette() -> Box<[PaletteEntry; 256]> {
        Box::new(std::array::from_fn(|index| {
            let level = index as u8;
            PaletteEntry {
                red: level,
                green: level,
                blue: level,
                flags: 0,
            }
        }))
    }

    fn frame(fill: u8) -> SoftwarePresentedFrame {
        let extent = DisplayExtent {
            width: 8,
            height: 4,
        };
        SoftwarePresentedFrame::from_indexed(extent, vec![fill; 32].into_boxed_slice(), palette())
            .unwrap()
    }

    /// A 4x4 atlas: index 0 black (transparent), index 1 white; one 2x2 glyph.
    fn font() -> BitmapFont {
        let mut file = vec![0u8; 0x436 + 16];
        file[0..2].copy_from_slice(b"BM");
        file[18..22].copy_from_slice(&4i32.to_le_bytes());
        file[22..26].copy_from_slice(&4i32.to_le_bytes());
        file[28..30].copy_from_slice(&8u16.to_le_bytes());
        file[54 + 4..54 + 7].copy_from_slice(&[0xff, 0xff, 0xff]);
        for byte in &mut file[0x436..0x436 + 16] {
            *byte = 1;
        }
        let mut trailer = [0u8; 24];
        trailer[1] = 1;
        trailer[4..8].copy_from_slice(&2i32.to_le_bytes());
        file.extend_from_slice(&trailer);
        let mut glyph = [0u8; 16];
        glyph[0] = b'A';
        glyph[6..8].copy_from_slice(&2i16.to_le_bytes());
        glyph[8..10].copy_from_slice(&2i16.to_le_bytes());
        file.extend_from_slice(&glyph);
        parse_legacy_font(&file, FontLimits::default()).unwrap()
    }

    #[test]
    fn the_translucent_panel_halves_the_world() {
        let plan = PauseDrawPlan {
            panel: OverlayQuad {
                x: 2,
                y: 1,
                w: 4,
                h: 2,
            },
            panel_color: [0, 0, 0, 0x80],
            runs: Vec::new(),
        };
        let font = font();
        let (out, outcome) = compose_pause_overlay(frame(200), &plan, |_| &font).unwrap();
        assert_eq!(outcome.panel_pixels, 8);
        let expected = source_over_channel(0, 200, 0x80);
        assert_eq!(out.pixels()[8 + 2], expected);
        assert_eq!(out.pixels()[0], 200, "outside the panel");
    }

    #[test]
    fn glyphs_take_the_colour_record_over_the_panel() {
        let font = font();
        let (text, _, _) = text_q16(&font, b"A", 0, 0, 0, 8, 4).unwrap();
        let plan = PauseDrawPlan {
            panel: OverlayQuad {
                x: 0,
                y: 0,
                w: 8,
                h: 4,
            },
            panel_color: [0x14, 0x14, 0x14, 0xff],
            runs: vec![PauseTextRun {
                font: PauseFont::Result,
                color: [0x80, 0x80, 0x80, 0xff],
                plan: text,
                clip: None,
            }],
        };
        let (out, outcome) = compose_pause_overlay(frame(200), &plan, |_| &font).unwrap();
        assert_eq!(outcome.glyphs_drawn, 1);
        assert_eq!(out.pixels()[0], modulate_channel(0xff, 0x80));
        assert_eq!(out.pixels()[3], 0x14, "the opaque panel elsewhere");
    }

    #[test]
    fn black_texels_are_transparent() {
        assert_eq!(texel([0, 0, 0]), None);
        assert_eq!(texel([0xff, 0x10, 0x0f]), Some([0xff, 0x11, 0]));
    }
}
