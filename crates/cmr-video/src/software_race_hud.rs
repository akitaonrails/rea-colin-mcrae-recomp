//! Indexed software composition for recovered race-HUD commands.
//!
//! Bitmap commands use the source-owned semantic oracle selected for the
//! compatibility baseline: index zero is transparent, point sampling uses
//! center-nearest stretching, RGB results are quantized against the active
//! frame palette by full-palette Manhattan distance, and ties select the lowest
//! index. Font glyphs retain their separate recovered direct-index copy rule.

use std::fmt;

use cmr_core::race_hud_plan::{
    HudColor, HudRect, HudTextPosition, RaceHudBitmap, RaceHudCommand, RaceHudFont,
};
use cmr_formats::bitmap::IndexedBitmap;
use cmr_formats::font::{
    BitmapFont, PointI32, RectI16, TextAlignment, TextLayoutError, TextLayoutLimits, layout_text,
};
use cmr_formats::riff_palette::PaletteEntry;

use crate::{SoftwarePresentedFrame, SoftwareRgba8Error};

/// Observable work completed by [`compose_race_hud_commands`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RaceHudComposeOutcome {
    pub bitmap_commands: usize,
    pub tinted_bitmap_commands: usize,
    pub text_commands: usize,
    pub glyphs_drawn: usize,
}

/// A selected glyph rectangle was not a valid positive rectangle inside its
/// font atlas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidRaceHudGlyphSource {
    pub command_index: usize,
    pub font: RaceHudFont,
    pub code: u8,
    pub source: RectI16,
    pub atlas_width: u32,
    pub atlas_height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidRaceHudBitmapRectangle {
    pub command_index: usize,
    pub bitmap: RaceHudBitmap,
    pub source: HudRect,
    pub destination: HudRect,
    pub bitmap_width: u32,
    pub bitmap_height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RaceHudComposeError {
    Layout {
        command_index: usize,
        font: RaceHudFont,
        source: TextLayoutError,
    },
    InvalidGlyphSource(InvalidRaceHudGlyphSource),
    InvalidBitmapRectangle(InvalidRaceHudBitmapRectangle),
    InvalidBitmapPaletteIndex {
        command_index: usize,
        bitmap: RaceHudBitmap,
        index: u8,
        palette_entries: usize,
    },
    ArithmeticOverflow,
    Frame(SoftwareRgba8Error),
}

impl fmt::Display for RaceHudComposeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "race HUD composition failed: {self:?}")
    }
}

impl std::error::Error for RaceHudComposeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Layout { source, .. } => Some(source),
            Self::Frame(source) => Some(source),
            Self::InvalidGlyphSource(_)
            | Self::InvalidBitmapRectangle(_)
            | Self::InvalidBitmapPaletteIndex { .. }
            | Self::ArithmeticOverflow => None,
        }
    }
}

/// Composes all race-HUD commands into an owned indexed frame in source order.
///
/// Nonzero font-atlas indices replace destination indices unchanged. Atlas
/// index zero is transparent. Destination pixels outside the frame are
/// clipped; selected glyph source rectangles must be wholly inside the atlas.
/// Q16 positions are scaled independently by the frame width and height using
/// signed Q16 multiplication and an arithmetic right shift.
pub fn compose_race_hud_commands<'font, 'bitmap>(
    frame: SoftwarePresentedFrame,
    commands: &[RaceHudCommand],
    mut resolve_font: impl FnMut(RaceHudFont) -> &'font BitmapFont,
    mut resolve_bitmap: impl FnMut(RaceHudBitmap) -> &'bitmap IndexedBitmap,
) -> Result<(SoftwarePresentedFrame, RaceHudComposeOutcome), RaceHudComposeError> {
    let extent = frame.extent();
    let frame_width = usize::from(extent.width);
    let frame_height = usize::from(extent.height);
    let mut pixels = frame.pixels().to_vec();
    let palette = Box::new(*frame.palette());
    let mut outcome = RaceHudComposeOutcome::default();

    for (command_index, command) in commands.iter().enumerate() {
        match command {
            RaceHudCommand::Bitmap {
                bitmap,
                source,
                destination,
                ..
            } => {
                compose_bitmap(
                    command_index,
                    *bitmap,
                    resolve_bitmap(*bitmap),
                    *source,
                    *destination,
                    BitmapDiffuse::Ordinary,
                    &mut pixels,
                    frame_width,
                    frame_height,
                    palette.as_ref(),
                )?;
                outcome.bitmap_commands += 1;
            }
            RaceHudCommand::TintedBitmap {
                bitmap,
                source,
                destination,
                color,
                ..
            } => {
                compose_bitmap(
                    command_index,
                    *bitmap,
                    resolve_bitmap(*bitmap),
                    *source,
                    *destination,
                    BitmapDiffuse::Tinted(*color),
                    &mut pixels,
                    frame_width,
                    frame_height,
                    palette.as_ref(),
                )?;
                outcome.tinted_bitmap_commands += 1;
            }
            RaceHudCommand::Text {
                font,
                text,
                position,
                color,
                flags,
            } => {
                outcome.text_commands += 1;
                let font_data = resolve_font(*font);
                let anchor = text_anchor(*position, extent.width, extent.height)?;
                let plan = layout_text(
                    font_data,
                    text.as_bytes(),
                    anchor,
                    i32::from(extent.width),
                    TextAlignment::from_legacy_flags(*flags),
                    TextLayoutLimits::default(),
                )
                .map_err(|source| RaceHudComposeError::Layout {
                    command_index,
                    font: *font,
                    source,
                })?;

                let mut quantization = BitmapQuantizationCache::new(BitmapDiffuse::Tinted(*color));
                for blit in plan.blits {
                    validate_source(command_index, *font, blit.code, blit.source, font_data)?;
                    blit_glyph(
                        &mut pixels,
                        frame_width,
                        frame_height,
                        font_data,
                        blit.source,
                        blit.destination.x,
                        blit.destination.y,
                        palette.as_ref(),
                        &mut quantization,
                    )?;
                    outcome.glyphs_drawn += 1;
                }
            }
        }
    }

    let frame = SoftwarePresentedFrame::from_indexed(extent, pixels.into_boxed_slice(), palette)
        .map_err(RaceHudComposeError::Frame)?;
    Ok((frame, outcome))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BitmapDiffuse {
    Ordinary,
    Tinted(HudColor),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ValidatedBitmapRectangle {
    source_x: usize,
    source_y: usize,
    source_width: u32,
    source_height: u32,
    destination_x: i64,
    destination_y: i64,
    destination_width: u32,
    destination_height: u32,
}

#[allow(clippy::too_many_arguments)]
fn compose_bitmap(
    command_index: usize,
    bitmap_id: RaceHudBitmap,
    bitmap: &IndexedBitmap,
    source: HudRect,
    destination: HudRect,
    diffuse: BitmapDiffuse,
    target: &mut [u8],
    target_width: usize,
    target_height: usize,
    active_palette: &[PaletteEntry; 256],
) -> Result<(), RaceHudComposeError> {
    let rectangle =
        validate_bitmap_rectangle(command_index, bitmap_id, bitmap, source, destination)?;
    let source_stride =
        usize::try_from(bitmap.width).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
    let left = rectangle.destination_x.max(0);
    let top = rectangle.destination_y.max(0);
    let right = rectangle
        .destination_x
        .checked_add(i64::from(rectangle.destination_width))
        .ok_or(RaceHudComposeError::ArithmeticOverflow)?
        .min(target_width as i64);
    let bottom = rectangle
        .destination_y
        .checked_add(i64::from(rectangle.destination_height))
        .ok_or(RaceHudComposeError::ArithmeticOverflow)?
        .min(target_height as i64);
    let mut quantization = BitmapQuantizationCache::new(diffuse);

    for target_y in top..bottom {
        let destination_offset_y = u32::try_from(target_y - rectangle.destination_y)
            .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
        let sampled_y = center_nearest_offset(
            destination_offset_y,
            rectangle.source_height,
            rectangle.destination_height,
        );
        let source_row = rectangle
            .source_y
            .checked_add(sampled_y as usize)
            .and_then(|y| y.checked_mul(source_stride))
            .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
        let target_row = usize::try_from(target_y)
            .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?
            .checked_mul(target_width)
            .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
        for target_x in left..right {
            let destination_offset_x = u32::try_from(target_x - rectangle.destination_x)
                .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
            let sampled_x = center_nearest_offset(
                destination_offset_x,
                rectangle.source_width,
                rectangle.destination_width,
            );
            let source_offset = source_row
                .checked_add(rectangle.source_x)
                .and_then(|offset| offset.checked_add(sampled_x as usize))
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            let source_index = *bitmap
                .pixels
                .get(source_offset)
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            if source_index == 0 {
                continue;
            }
            let source_color = bitmap.palette.get(usize::from(source_index)).ok_or(
                RaceHudComposeError::InvalidBitmapPaletteIndex {
                    command_index,
                    bitmap: bitmap_id,
                    index: source_index,
                    palette_entries: bitmap.palette.len(),
                },
            )?;
            let target_offset = target_row
                .checked_add(
                    usize::try_from(target_x)
                        .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?,
                )
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            let destination_index = *target
                .get(target_offset)
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            let destination_color = active_palette[usize::from(destination_index)];
            target[target_offset] = quantization.resolve(
                source_index,
                destination_index,
                source_color,
                destination_color,
                active_palette,
            );
        }
    }
    Ok(())
}

/// Memoizes the deterministic RGB-to-index oracle for one bitmap command.
///
/// Ordinary diffuse ignores the destination, so one result per source index is
/// sufficient. Tinted source-over depends on both indexed operands. Keeping the
/// pair table local preserves command ordering while avoiding a 256-entry
/// palette search for every covered destination pixel.
enum BitmapQuantizationCache {
    Ordinary {
        diffuse: BitmapDiffuse,
        by_source: Box<[Option<u8>; 256]>,
    },
    Tinted {
        diffuse: BitmapDiffuse,
        by_source_destination: Box<[Option<u8>]>,
    },
}

impl BitmapQuantizationCache {
    fn new(diffuse: BitmapDiffuse) -> Self {
        match diffuse {
            BitmapDiffuse::Ordinary => Self::Ordinary {
                diffuse,
                by_source: Box::new([None; 256]),
            },
            BitmapDiffuse::Tinted(_) => Self::Tinted {
                diffuse,
                by_source_destination: vec![None; 256 * 256].into_boxed_slice(),
            },
        }
    }

    fn resolve(
        &mut self,
        source_index: u8,
        destination_index: u8,
        source_color: &cmr_formats::bitmap::BitmapPaletteEntry,
        destination_color: PaletteEntry,
        active_palette: &[PaletteEntry; 256],
    ) -> u8 {
        match self {
            Self::Ordinary { diffuse, by_source } => {
                let entry = &mut by_source[usize::from(source_index)];
                *entry.get_or_insert_with(|| {
                    nearest_palette_index(
                        bitmap_rgb(source_color, destination_color, *diffuse),
                        active_palette,
                    )
                })
            }
            Self::Tinted {
                diffuse,
                by_source_destination,
            } => {
                let pair = usize::from(source_index) * 256 + usize::from(destination_index);
                let entry = &mut by_source_destination[pair];
                *entry.get_or_insert_with(|| {
                    nearest_palette_index(
                        bitmap_rgb(source_color, destination_color, *diffuse),
                        active_palette,
                    )
                })
            }
        }
    }
}

fn validate_bitmap_rectangle(
    command_index: usize,
    bitmap_id: RaceHudBitmap,
    bitmap: &IndexedBitmap,
    source: HudRect,
    destination: HudRect,
) -> Result<ValidatedBitmapRectangle, RaceHudComposeError> {
    let source_right = i64::from(source.x).checked_add(i64::from(source.width));
    let source_bottom = i64::from(source.y).checked_add(i64::from(source.height));
    let destination_right = i64::from(destination.x).checked_add(i64::from(destination.width));
    let destination_bottom = i64::from(destination.y).checked_add(i64::from(destination.height));
    let valid = source.x >= 0
        && source.y >= 0
        && source.width > 0
        && source.height > 0
        && destination.width > 0
        && destination.height > 0
        && source_right.is_some_and(|right| right <= i64::from(bitmap.width))
        && source_bottom.is_some_and(|bottom| bottom <= i64::from(bitmap.height))
        && destination_right.is_some()
        && destination_bottom.is_some();
    if !valid {
        return Err(RaceHudComposeError::InvalidBitmapRectangle(
            InvalidRaceHudBitmapRectangle {
                command_index,
                bitmap: bitmap_id,
                source,
                destination,
                bitmap_width: bitmap.width,
                bitmap_height: bitmap.height,
            },
        ));
    }
    Ok(ValidatedBitmapRectangle {
        source_x: source.x as usize,
        source_y: source.y as usize,
        source_width: source.width as u32,
        source_height: source.height as u32,
        destination_x: i64::from(destination.x),
        destination_y: i64::from(destination.y),
        destination_width: destination.width as u32,
        destination_height: destination.height as u32,
    })
}

fn center_nearest_offset(destination_offset: u32, source_len: u32, destination_len: u32) -> u32 {
    let numerator = (u64::from(destination_offset) * 2 + 1) * u64::from(source_len);
    let denominator = u64::from(destination_len) * 2;
    u32::try_from(numerator / denominator)
        .expect("center-nearest sampling stays inside a u32 source extent")
}

fn bitmap_rgb(
    source: &cmr_formats::bitmap::BitmapPaletteEntry,
    destination: PaletteEntry,
    diffuse: BitmapDiffuse,
) -> [u8; 3] {
    let source = [source.red, source.green, source.blue];
    match diffuse {
        BitmapDiffuse::Ordinary => source.map(|channel| modulate_channel(channel, 200)),
        BitmapDiffuse::Tinted(color) => {
            let modulated = [
                modulate_channel(source[0], color.red),
                modulate_channel(source[1], color.green),
                modulate_channel(source[2], color.blue),
            ];
            let destination = [destination.red, destination.green, destination.blue];
            std::array::from_fn(|index| {
                source_over_channel(modulated[index], destination[index], color.alpha)
            })
        }
    }
}

pub(crate) fn modulate_channel(source: u8, diffuse: u8) -> u8 {
    ((u16::from(source) * u16::from(diffuse)) / 255) as u8
}

pub(crate) fn source_over_channel(source: u8, destination: u8, alpha: u8) -> u8 {
    let inverse_alpha = 255 - u16::from(alpha);
    ((u16::from(source) * u16::from(alpha) + u16::from(destination) * inverse_alpha) / 255) as u8
}

pub(crate) fn nearest_palette_index(rgb: [u8; 3], palette: &[PaletteEntry; 256]) -> u8 {
    let mut best_index = 0;
    let mut best_distance = u16::MAX;
    for (index, entry) in palette.iter().enumerate() {
        let distance = u16::from(rgb[0].abs_diff(entry.red))
            + u16::from(rgb[1].abs_diff(entry.green))
            + u16::from(rgb[2].abs_diff(entry.blue));
        if distance < best_distance {
            best_index = index as u8;
            best_distance = distance;
            if distance == 0 {
                break;
            }
        }
    }
    best_index
}

fn text_anchor(
    position: HudTextPosition,
    frame_width: u16,
    frame_height: u16,
) -> Result<PointI32, RaceHudComposeError> {
    let point = match position {
        HudTextPosition::Pixels(point) => PointI32 {
            x: point.x,
            y: point.y,
        },
        HudTextPosition::Q16(point) => PointI32 {
            x: q16_to_pixels(point.x, frame_width)?,
            y: q16_to_pixels(point.y, frame_height)?,
        },
    };
    Ok(point)
}

fn q16_to_pixels(value: i32, extent: u16) -> Result<i32, RaceHudComposeError> {
    let scaled = i64::from(value)
        .checked_mul(i64::from(extent))
        .ok_or(RaceHudComposeError::ArithmeticOverflow)?
        >> 16;
    i32::try_from(scaled).map_err(|_| RaceHudComposeError::ArithmeticOverflow)
}

fn validate_source(
    command_index: usize,
    font: RaceHudFont,
    code: u8,
    source: RectI16,
    font_data: &BitmapFont,
) -> Result<(), RaceHudComposeError> {
    let atlas = font_data.atlas();
    let valid = source.x >= 0
        && source.y >= 0
        && source.width > 0
        && source.height > 0
        && i32::from(source.x)
            .checked_add(i32::from(source.width))
            .is_some_and(|end| end <= atlas.width as i32)
        && i32::from(source.y)
            .checked_add(i32::from(source.height))
            .is_some_and(|end| end <= atlas.height as i32);
    if valid {
        Ok(())
    } else {
        Err(RaceHudComposeError::InvalidGlyphSource(
            InvalidRaceHudGlyphSource {
                command_index,
                font,
                code,
                source,
                atlas_width: atlas.width,
                atlas_height: atlas.height,
            },
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn blit_glyph(
    destination: &mut [u8],
    destination_width: usize,
    destination_height: usize,
    font: &BitmapFont,
    source: RectI16,
    destination_x: i32,
    destination_y: i32,
    active_palette: &[PaletteEntry; 256],
    quantization: &mut BitmapQuantizationCache,
) -> Result<(), RaceHudComposeError> {
    let atlas = font.atlas();
    let atlas_width =
        usize::try_from(atlas.width).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
    let source_x =
        usize::try_from(source.x).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
    let source_y =
        usize::try_from(source.y).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
    let width =
        usize::try_from(source.width).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;
    let height =
        usize::try_from(source.height).map_err(|_| RaceHudComposeError::ArithmeticOverflow)?;

    for row in 0..height {
        let target_y = i64::from(destination_y) + row as i64;
        if target_y < 0 || target_y >= destination_height as i64 {
            continue;
        }
        let atlas_row = source_y
            .checked_add(row)
            .and_then(|y| y.checked_mul(atlas_width))
            .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
        let target_row = usize::try_from(target_y)
            .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?
            .checked_mul(destination_width)
            .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
        for column in 0..width {
            let target_x = i64::from(destination_x) + column as i64;
            if target_x < 0 || target_x >= destination_width as i64 {
                continue;
            }
            let source_index = atlas_row
                .checked_add(source_x)
                .and_then(|index| index.checked_add(column))
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            let atlas_index = *atlas
                .pixels
                .get(source_index)
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            if atlas_index == 0 {
                continue;
            }
            let destination_index = target_row
                .checked_add(
                    usize::try_from(target_x)
                        .map_err(|_| RaceHudComposeError::ArithmeticOverflow)?,
                )
                .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
            let resolved_index = match atlas.palette.get(usize::from(atlas_index)) {
                Some(source_color)
                    if [source_color.red, source_color.green, source_color.blue] != [0, 0, 0] =>
                {
                    let dst_idx = *destination
                        .get(destination_index)
                        .ok_or(RaceHudComposeError::ArithmeticOverflow)?;
                    let destination_color = active_palette[usize::from(dst_idx)];
                    quantization.resolve(
                        atlas_index,
                        dst_idx,
                        source_color,
                        destination_color,
                        active_palette,
                    )
                }
                _ => atlas_index,
            };
            *destination
                .get_mut(destination_index)
                .ok_or(RaceHudComposeError::ArithmeticOverflow)? = resolved_index;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use cmr_core::race_hud_plan::{
        HudColor, HudPoint, HudRect, LegacyBitmapArguments, RaceHudBitmap,
    };
    use cmr_formats::{
        bitmap::{BitmapMetadata, BitmapPaletteEntry},
        font::{FontLimits, parse_legacy_font},
        riff_palette::PaletteEntry,
    };

    use super::*;

    const COLOR: HudColor = HudColor::new(255, 255, 255, 255);

    #[test]
    fn zero_is_transparent_and_nonzero_atlas_indices_are_preserved() {
        let font = font(2, 2, &[0, 5, 7, 0], glyph(b'A', 0, 0, 2, 2));
        let command = text_command("A", HudTextPosition::Pixels(HudPoint::new(1, 1)), 0);
        let (frame, outcome) = compose_text(frame(4, 4, 9), &[command], &font).unwrap();

        assert_eq!(outcome.text_commands, 1);
        assert_eq!(outcome.glyphs_drawn, 1);
        assert_eq!(frame.pixels()[5], 9);
        assert_eq!(frame.pixels()[6], 5);
        assert_eq!(frame.pixels()[9], 7);
        assert_eq!(frame.pixels()[10], 9);
    }

    #[test]
    fn q16_positions_scale_by_extent_while_pixel_positions_are_direct() {
        let font = font(1, 1, &[6], glyph(b'A', 0, 0, 1, 1));
        let commands = [
            text_command("A", HudTextPosition::Q16(HudPoint::new(0x8000, 0x8000)), 0),
            text_command("A", HudTextPosition::Pixels(HudPoint::new(1, 2)), 0),
        ];
        let (frame, _) = compose_text(frame(8, 6, 0), &commands, &font).unwrap();

        assert_eq!(frame.pixels()[3 * 8 + 4], 6);
        assert_eq!(frame.pixels()[2 * 8 + 1], 6);
    }

    #[test]
    fn legacy_alignment_flags_are_applied_before_rasterization() {
        let font = font(2, 1, &[3, 4], glyph(b'A', 0, 0, 2, 1));
        let command = text_command("A", HudTextPosition::Pixels(HudPoint::new(4, 2)), 0x22);
        let (frame, _) = compose_text(frame(8, 5, 0), &[command], &font).unwrap();

        assert_eq!(&frame.pixels()[2 * 8 + 3..2 * 8 + 5], &[3, 4]);
    }

    #[test]
    fn offscreen_destination_is_clipped_without_shifting_the_source() {
        let font = font(2, 2, &[1, 2, 3, 4], glyph(b'A', 0, 0, 2, 2));
        let command = text_command("A", HudTextPosition::Pixels(HudPoint::new(-1, -1)), 0);
        let (frame, _) = compose_text(frame(2, 2, 8), &[command], &font).unwrap();

        assert_eq!(frame.pixels(), &[4, 8, 8, 8]);
    }

    #[test]
    fn malformed_glyph_source_is_rejected_before_indexing_the_atlas() {
        let font = font(2, 2, &[1, 2, 3, 4], glyph(b'A', 1, 0, 2, 1));
        let command = text_command("A", HudTextPosition::Pixels(HudPoint::new(0, 0)), 0);
        let error = compose_text(frame(2, 2, 8), &[command], &font).unwrap_err();

        assert_eq!(
            error,
            RaceHudComposeError::InvalidGlyphSource(InvalidRaceHudGlyphSource {
                command_index: 0,
                font: RaceHudFont::Numbers640,
                code: b'A',
                source: RectI16 {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                atlas_width: 2,
                atlas_height: 2,
            })
        );
    }

    #[test]
    fn bitmap_index_zero_is_transparent() {
        let font = font(1, 1, &[1], glyph(b'A', 0, 0, 1, 1));
        let bitmap = bitmap(1, 1, &[0], &[]);
        let command = bitmap_command(HudRect::new(0, 0, 1, 1), HudRect::new(0, 0, 1, 1));
        let (frame, outcome) =
            compose_race_hud_commands(frame(1, 1, 7), &[command], |_| &font, |_| &bitmap).unwrap();

        assert_eq!(frame.pixels(), &[7]);
        assert_eq!(outcome.bitmap_commands, 1);
    }

    #[test]
    fn ordinary_bitmap_uses_fixed_200_over_255_diffuse() {
        let font = font(1, 1, &[1], glyph(b'A', 0, 0, 1, 1));
        let bitmap = bitmap(1, 1, &[1], &[(1, [255, 128, 0])]);
        let palette = active_palette(&[(11, [200, 100, 0])]);
        let command = bitmap_command(HudRect::new(0, 0, 1, 1), HudRect::new(0, 0, 1, 1));
        let (frame, _) = compose_race_hud_commands(
            frame_with_palette(1, 1, &[0], palette),
            &[command],
            |_| &font,
            |_| &bitmap,
        )
        .unwrap();

        assert_eq!(frame.pixels(), &[11]);
    }

    #[test]
    fn tinted_bitmap_modulates_then_applies_straight_source_over() {
        let font = font(1, 1, &[1], glyph(b'A', 0, 0, 1, 1));
        let bitmap = bitmap(1, 1, &[1], &[(1, [200, 100, 50])]);
        let palette = active_palette(&[(3, [20, 40, 80]), (4, [60, 70, 39])]);
        let command = tinted_bitmap_command(
            HudRect::new(0, 0, 1, 1),
            HudRect::new(0, 0, 1, 1),
            HudColor::new(128, 255, 0, 128),
        );
        let (frame, _) = compose_race_hud_commands(
            frame_with_palette(1, 1, &[3], palette),
            &[command],
            |_| &font,
            |_| &bitmap,
        )
        .unwrap();

        assert_eq!(frame.pixels(), &[4]);
    }

    #[test]
    fn bitmap_quantization_uses_lowest_index_for_equal_manhattan_distance() {
        let font = font(1, 1, &[1], glyph(b'A', 0, 0, 1, 1));
        let bitmap = bitmap(1, 1, &[1], &[(1, [255, 0, 0])]);
        let palette = active_palette(&[(1, [199, 0, 0]), (2, [201, 0, 0])]);
        let command = bitmap_command(HudRect::new(0, 0, 1, 1), HudRect::new(0, 0, 1, 1));
        let (frame, _) = compose_race_hud_commands(
            frame_with_palette(1, 1, &[0], palette),
            &[command],
            |_| &font,
            |_| &bitmap,
        )
        .unwrap();

        assert_eq!(frame.pixels(), &[1]);
    }

    #[test]
    fn bitmap_quantization_cache_preserves_destination_dependency_only_for_tint() {
        let source = BitmapPaletteEntry {
            red: 255,
            green: 255,
            blue: 255,
            reserved: 0,
        };
        let palette = active_palette(&[
            (1, [200, 200, 200]),
            (2, [100, 100, 100]),
            (3, [228, 228, 228]),
        ]);

        let mut ordinary = BitmapQuantizationCache::new(BitmapDiffuse::Ordinary);
        let ordinary_on_dark = ordinary.resolve(1, 2, &source, palette[2], &palette);
        let ordinary_on_light = ordinary.resolve(1, 3, &source, palette[3], &palette);
        assert_eq!(ordinary_on_dark, 1);
        assert_eq!(ordinary_on_light, 1);

        let mut tinted =
            BitmapQuantizationCache::new(BitmapDiffuse::Tinted(HudColor::new(255, 255, 255, 128)));
        let tinted_on_dark = tinted.resolve(1, 2, &source, palette[2], &palette);
        let tinted_on_light = tinted.resolve(1, 3, &source, palette[3], &palette);
        assert_eq!(tinted_on_dark, 1);
        assert_eq!(tinted_on_light, 3);
    }

    #[test]
    fn bitmap_stretch_maps_destination_centers_to_nearest_source_centers() {
        let font = font(1, 1, &[1], glyph(b'A', 0, 0, 1, 1));
        let bitmap = bitmap(2, 1, &[1, 2], &[(1, [255, 0, 0]), (2, [0, 255, 0])]);
        let palette = active_palette(&[(5, [200, 0, 0]), (6, [0, 200, 0])]);
        let command = bitmap_command(HudRect::new(0, 0, 2, 1), HudRect::new(0, 0, 4, 1));
        let (frame, _) = compose_race_hud_commands(
            frame_with_palette(4, 1, &[0; 4], palette),
            &[command],
            |_| &font,
            |_| &bitmap,
        )
        .unwrap();

        assert_eq!(frame.pixels(), &[5, 5, 6, 6]);
    }

    #[test]
    fn bitmap_text_and_tinted_bitmap_execute_in_command_order() {
        let font = font(1, 1, &[7], glyph(b'A', 0, 0, 1, 1));
        let ordinary = bitmap(1, 1, &[1], &[(1, [255, 0, 0])]);
        let tinted = bitmap(1, 1, &[1], &[(1, [255, 255, 255])]);
        let palette = active_palette(&[(5, [200, 0, 0]), (7, [100, 0, 0]), (9, [49, 0, 0])]);
        let commands = [
            bitmap_command(HudRect::new(0, 0, 1, 1), HudRect::new(0, 0, 1, 1)),
            text_command("A", HudTextPosition::Pixels(HudPoint::new(0, 0)), 0),
            tinted_bitmap_command(
                HudRect::new(0, 0, 1, 1),
                HudRect::new(0, 0, 1, 1),
                HudColor::new(0, 0, 0, 128),
            ),
        ];
        let (frame, outcome) = compose_race_hud_commands(
            frame_with_palette(1, 1, &[0], palette),
            &commands,
            |_| &font,
            |bitmap| match bitmap {
                RaceHudBitmap::CommonGauge640 => &ordinary,
                RaceHudBitmap::ProgressRedSpot => &tinted,
                _ => unreachable!(),
            },
        )
        .unwrap();

        assert_eq!(frame.pixels(), &[9]);
        assert_eq!(outcome.bitmap_commands, 1);
        assert_eq!(outcome.text_commands, 1);
        assert_eq!(outcome.tinted_bitmap_commands, 1);
        assert_eq!(outcome.glyphs_drawn, 1);
    }

    fn text_command(text: &str, position: HudTextPosition, flags: u8) -> RaceHudCommand {
        RaceHudCommand::Text {
            font: RaceHudFont::Numbers640,
            text: text.to_owned(),
            position,
            color: COLOR,
            flags,
        }
    }

    fn bitmap_command(source: HudRect, destination: HudRect) -> RaceHudCommand {
        RaceHudCommand::Bitmap {
            bitmap: RaceHudBitmap::CommonGauge640,
            source,
            destination,
            legacy: LegacyBitmapArguments { mode: 2, flags: 8 },
        }
    }

    fn tinted_bitmap_command(
        source: HudRect,
        destination: HudRect,
        color: HudColor,
    ) -> RaceHudCommand {
        RaceHudCommand::TintedBitmap {
            bitmap: RaceHudBitmap::ProgressRedSpot,
            source,
            destination,
            color,
            legacy: LegacyBitmapArguments { mode: 1, flags: 8 },
        }
    }

    fn compose_text(
        frame: SoftwarePresentedFrame,
        commands: &[RaceHudCommand],
        font: &BitmapFont,
    ) -> Result<(SoftwarePresentedFrame, RaceHudComposeOutcome), RaceHudComposeError> {
        let unused_bitmap = bitmap(1, 1, &[0], &[]);
        compose_race_hud_commands(frame, commands, |_| font, |_| &unused_bitmap)
    }

    fn frame(width: u16, height: u16, fill: u8) -> SoftwarePresentedFrame {
        let palette = std::array::from_fn(|index| PaletteEntry {
            red: index as u8,
            green: index as u8,
            blue: index as u8,
            flags: 0,
        });
        let pixels = vec![fill; usize::from(width) * usize::from(height)];
        frame_with_palette(width, height, &pixels, palette)
    }

    fn frame_with_palette(
        width: u16,
        height: u16,
        pixels: &[u8],
        palette: [PaletteEntry; 256],
    ) -> SoftwarePresentedFrame {
        let extent = cmr_core::display::DisplayExtent { width, height };
        SoftwarePresentedFrame::from_indexed(
            extent,
            pixels.to_vec().into_boxed_slice(),
            Box::new(palette),
        )
        .unwrap()
    }

    fn active_palette(entries: &[(u8, [u8; 3])]) -> [PaletteEntry; 256] {
        let mut palette = [PaletteEntry {
            red: 0,
            green: 255,
            blue: 255,
            flags: 0,
        }; 256];
        for &(index, rgb) in entries {
            palette[usize::from(index)] = PaletteEntry {
                red: rgb[0],
                green: rgb[1],
                blue: rgb[2],
                flags: index,
            };
        }
        palette
    }

    fn bitmap(width: u32, height: u32, pixels: &[u8], entries: &[(u8, [u8; 3])]) -> IndexedBitmap {
        let mut palette = vec![
            BitmapPaletteEntry {
                red: 0,
                green: 0,
                blue: 0,
                reserved: 0,
            };
            256
        ];
        for &(index, rgb) in entries {
            palette[usize::from(index)] = BitmapPaletteEntry {
                red: rgb[0],
                green: rgb[1],
                blue: rgb[2],
                reserved: index,
            };
        }
        IndexedBitmap {
            width,
            height,
            source_was_top_down: true,
            palette,
            pixels: pixels.to_vec(),
            row_stride: width,
            stored_pixels: pixels.to_vec(),
            metadata: BitmapMetadata {
                reserved_1: 0,
                reserved_2: 0,
                x_pixels_per_meter: 0,
                y_pixels_per_meter: 0,
                important_colors: 0,
            },
        }
    }

    fn glyph(code: u8, x: i16, y: i16, width: i16, height: i16) -> [u8; 16] {
        let mut record = [0; 16];
        record[0] = code;
        record[2..4].copy_from_slice(&x.to_le_bytes());
        record[4..6].copy_from_slice(&y.to_le_bytes());
        record[6..8].copy_from_slice(&width.to_le_bytes());
        record[8..10].copy_from_slice(&height.to_le_bytes());
        record
    }

    fn font(width: u32, height: u32, normalized_pixels: &[u8], glyph: [u8; 16]) -> BitmapFont {
        const PIXEL_OFFSET: usize = 0x436;
        let stride = (width as usize + 3) & !3;
        let trailer = PIXEL_OFFSET + stride * height as usize;
        let mut bytes = vec![0; trailer + 24 + 16];
        bytes[0..2].copy_from_slice(b"BM");
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[28..30].copy_from_slice(&8u16.to_le_bytes());
        for y in 0..height as usize {
            let stored_y = height as usize - 1 - y;
            let source = y * width as usize;
            let destination = PIXEL_OFFSET + stored_y * stride;
            bytes[destination..destination + width as usize]
                .copy_from_slice(&normalized_pixels[source..source + width as usize]);
        }
        bytes[trailer + 1] = 1;
        bytes[trailer + 4..trailer + 8].copy_from_slice(&1u32.to_le_bytes());
        bytes[trailer + 24..trailer + 40].copy_from_slice(&glyph);
        parse_legacy_font(&bytes, FontLimits::default()).unwrap()
    }

    #[test]
    fn text_glyph_quantizes_and_tints_font_palette_against_active_palette() {
        const PIXEL_OFFSET: usize = 0x436;
        let width = 2u32;
        let height = 1u32;
        let stride = 4usize;
        let trailer = PIXEL_OFFSET + stride * height as usize;
        let mut bytes = vec![0u8; trailer + 24 + 16];
        bytes[0..2].copy_from_slice(b"BM");
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[28..30].copy_from_slice(&8u16.to_le_bytes());
        // Font palette entry 5 = white (BGR0 = 255, 255, 255, 0) at 0x36 + 5 * 4.
        let pal_entry = 0x36 + 5 * 4;
        bytes[pal_entry..pal_entry + 4].copy_from_slice(&[255, 255, 255, 0]);
        bytes[PIXEL_OFFSET] = 5;
        bytes[PIXEL_OFFSET + 1] = 0;
        bytes[trailer + 1] = 1;
        bytes[trailer + 4..trailer + 8].copy_from_slice(&1u32.to_le_bytes());
        bytes[trailer + 24..trailer + 40].copy_from_slice(&glyph(b'1', 0, 0, 2, 1));
        let parsed_font = parse_legacy_font(&bytes, FontLimits::default()).unwrap();

        let palette = active_palette(&[(42, [200, 100, 50])]);
        let command = RaceHudCommand::Text {
            font: RaceHudFont::Numbers640,
            text: "1".to_owned(),
            position: HudTextPosition::Pixels(HudPoint::new(1, 0)),
            color: HudColor::new(200, 100, 50, 255),
            flags: 0,
        };
        let (frame, _) = compose_text(
            frame_with_palette(4, 2, &[9; 8], palette),
            &[command],
            &parsed_font,
        )
        .unwrap();
        // Font index 5 (white) tinted by (200,100,50) quantizes to active_palette index 42;
        // font index 0 stays transparent (9).
        assert_eq!(frame.pixels()[1], 42);
        assert_eq!(frame.pixels()[2], 9);
    }
}
