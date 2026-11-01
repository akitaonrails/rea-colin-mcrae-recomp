//! Pure command planning for the recovered normal single-player race HUD core.
//!
//! This slice intentionally contains only commands whose source identity,
//! rectangle, value formatting, and source order are statically pinned. The
//! original preparation routine remains outside this module: callers provide
//! its per-player rev, speed, gear, progress, and timer values explicitly.
//! Unrecovered optional layers remain typed plan gaps instead of guessed draw
//! commands.
//!
//! The complete fifteen-segment 640x480 rev strip and its strict threshold
//! chain are observed in `FUN_0045f530` (REA Evidence
//! `ev_610f7816ee6dc0dcaf81f71233b97871fea7570f5a9dfc817f563f2ebd14ee24`);
//! its packed source/destination descriptors are initialized by
//! `FUN_0045dda0` (REA Evidence
//! `ev_74e424d7591426f4eaba851dca243dfac707dcf112a6a1f805f413fcaa11caa0`).

use std::fmt::Write;

use crate::display::DisplayExtent;

pub const REFERENCE_EXTENT: DisplayExtent = DisplayExtent {
    width: 640,
    height: 480,
};
pub const LEGACY_BITMAP_FLAGS: u8 = 0x08;
pub const LEGACY_TEXT_FLAGS: u8 = 0x11;

const COMMON_GAUGE_COLOR: HudColor = HudColor::new(0xff, 0xff, 0xff, 0xc0);
const TIMER_COLOR: HudColor = HudColor::new(0xff, 0xff, 0xff, 0xff);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HudRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl HudRect {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HudPoint {
    pub x: i32,
    pub y: i32,
}

impl HudPoint {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HudColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl HudColor {
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// Stable identities for the exact source-owned files needed by this slice.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RaceHudBitmap {
    CommonGauge640,
    ProgressRedSpot,
    HorizontalBar,
    HorizontalBarEnd,
    HorizontalBarEndRight,
    TopLeftCorner,
    BottomLeftCurve,
    VerticalBar,
    ProgressBarTop,
    WhitePixel,
    CountdownDigits,
    CountdownGo,
}

impl RaceHudBitmap {
    #[must_use]
    pub const fn relative_path(self) -> &'static str {
        match self {
            Self::CommonGauge640 => "images/osdnew/revs/640x480.bmp",
            Self::ProgressRedSpot => "images/osdnew/High/common/redspot.bmp",
            Self::HorizontalBar => "images/osdnew/High/common/hbar.bmp",
            Self::HorizontalBarEnd => "images/osdnew/High/common/hbarend.bmp",
            Self::HorizontalBarEndRight => "images/osdnew/High/common/hbarend2.bmp",
            Self::TopLeftCorner => "images/osdnew/High/common/tlc.bmp",
            Self::BottomLeftCurve => "images/osdnew/High/common/curve.bmp",
            Self::VerticalBar => "images/osdnew/High/common/vbar.bmp",
            Self::ProgressBarTop => "images/osdnew/High/common/pbartop.bmp",
            Self::WhitePixel => "images/osdnew/High/common/white.bmp",
            Self::CountdownDigits => "images/cntdown2.bmp",
            Self::CountdownGo => "images/go!.bmp",
        }
    }
}

/// Start-line countdown / `"GO!"` banner state (`FUN_00423570` / `FUN_00423700`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RaceHudCountdown {
    /// Phase 4 digit `5..=1` shrinking from full screen height over 20 ticks
    /// (`ticks_left` in `0..=19`).
    Digit { digit: u8, ticks_left: u8 },
    /// Phase 5 `"GO!"` banner expanding and fading out over 19 ticks
    /// (`ticks_left` in `1..=19`).
    Go { ticks_left: u8 },
}

/// One row of the top-left 4-row in-race classification table (`FUN_00464930`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaceHudClassificationRow {
    /// 1-based rank (`1..=16`).
    pub rank: u8,
    /// 3-letter uppercase driver abbreviation (`"MCR"`, `" V"`, etc.).
    pub code: String,
    /// Whether this row belongs to the active player (`0x57afd4`).
    pub is_player: bool,
}

/// Inputs for the in-race checkpoint HUD layers (`FUN_00464130`, `FUN_00464930`,
/// `FUN_00463080`, `FUN_0047fe50`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RaceHudCheckpointInputs {
    /// Current car progress along the stage (`0..=0x1_0000`).
    pub progress_q16: i32,
    /// Cumulative 3D distance fractions (`0..=0x1_0000`) for the intermediate
    /// split gates (`FUN_00462230` / `FUN_00463080`).
    pub split_progress_q16: Vec<i32>,
    /// Per-sector green (`true`: gained/equal time vs reference) or red
    /// (`false`: lost time vs reference) indicator for completed sectors
    /// (`0x5ffba0` in `FUN_00463080`).
    pub sector_gained: Vec<bool>,
    /// Visible 4-row classification table (`FUN_00464710` / `FUN_00464930`),
    /// populated once at least one split has been crossed (`splits_passed >= 1`).
    pub classification_rows: Vec<RaceHudClassificationRow>,
    /// Whether the player gained or held rank at the latest split (`0x464b89`:
    /// `curr_rank <= prev_rank` -> green bar `[0x00, 0xff, 0x00, 0xc8]`, else
    /// red bar `[0xff, 0x00, 0x00, 0xc8]`).
    pub player_rank_gained_or_held: bool,
    /// Remaining display ticks (`120..=0`, `0x5ffc2c`) for the split readout
    /// (`FUN_0047fe50`).
    pub split_display_ticks: u8,
    /// Most recently latched player split time in centiseconds (`0x5ffb30`).
    pub last_split_centiseconds: Option<u32>,
    /// Reference (fastest rival) cumulative split time in centiseconds at the
    /// most recently crossed split (`0x5ffa1c[split_idx]`).
    pub reference_split_centiseconds: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RaceHudFont {
    Numbers640,
    TimeNumbers640,
    PlaceNumbers640,
    Result640,
}

impl RaceHudFont {
    #[must_use]
    pub const fn relative_path(self) -> &'static str {
        match self {
            Self::Numbers640 => "fonts/640x480/numbers.fon",
            Self::TimeNumbers640 => "fonts/640x480/timenum.fon",
            Self::PlaceNumbers640 => "fonts/640x480/placenum.fon",
            Self::Result640 => "fonts/640x480/result.fon",
        }
    }
}

/// Raw recovered renderer arguments. Their color-key and blend meanings are
/// not yet established, so this type deliberately does not name a policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LegacyBitmapArguments {
    pub mode: u8,
    pub flags: u8,
}

impl LegacyBitmapArguments {
    const OPAQUE_CALL: Self = Self {
        mode: 2,
        flags: LEGACY_BITMAP_FLAGS,
    };
    const TINTED_CALL: Self = Self {
        mode: 1,
        flags: LEGACY_BITMAP_FLAGS,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HudTextPosition {
    /// Normalized coordinates passed to the recovered font draw routine.
    Q16(HudPoint),
    /// Pixel coordinates passed to the recovered timer font routine.
    Pixels(HudPoint),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RaceHudCommand {
    Bitmap {
        bitmap: RaceHudBitmap,
        source: HudRect,
        destination: HudRect,
        legacy: LegacyBitmapArguments,
    },
    TintedBitmap {
        bitmap: RaceHudBitmap,
        source: HudRect,
        destination: HudRect,
        color: HudColor,
        legacy: LegacyBitmapArguments,
    },
    Text {
        font: RaceHudFont,
        text: String,
        position: HudTextPosition,
        color: HudColor,
        flags: u8,
    },
}

/// Raw choice made by the recovered configuration-byte branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommonGaugeInset {
    ConfigLowThreeBitsZero,
    ConfigLowThreeBitsNonZero,
}

/// Values produced by race-state preparation and consumed without mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RaceHudPreparedInputs {
    pub viewport: DisplayExtent,
    pub rev_q16: i32,
    pub speed: u32,
    pub gear: i8,
    pub progress_q16: i32,
    pub elapsed_centiseconds: u32,
    pub gauge_inset: CommonGaugeInset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RaceHudIncomplete {
    CommonGaugeBorderFragments,
    ProgressAuxiliaryMarkers,
    ProgressOutsideRecoveredRange { progress_q16: i32 },
    BitmapRasterPolicy,
    FontGlyphRasterPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RaceHudPlan {
    commands: Vec<RaceHudCommand>,
    incomplete: Vec<RaceHudIncomplete>,
}

impl RaceHudPlan {
    #[must_use]
    pub fn commands(&self) -> &[RaceHudCommand] {
        &self.commands
    }

    #[must_use]
    pub fn incomplete(&self) -> &[RaceHudIncomplete] {
        &self.incomplete
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RaceHudPlanError {
    EmptyViewport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RevSegment {
    threshold_q16: Option<i32>,
    source: HudRect,
    destination: HudRect,
}

const REV_SEGMENTS: [RevSegment; 15] = [
    RevSegment {
        threshold_q16: None,
        source: HudRect::new(3, 119, 14, 6),
        destination: HudRect::new(492, 464, 14, 6),
    },
    RevSegment {
        threshold_q16: Some(0x2000),
        source: HudRect::new(19, 114, 25, 11),
        destination: HudRect::new(498, 459, 25, 11),
    },
    RevSegment {
        threshold_q16: Some(0x3000),
        source: HudRect::new(51, 112, 34, 13),
        destination: HudRect::new(508, 457, 34, 13),
    },
    RevSegment {
        threshold_q16: Some(0x4000),
        source: HudRect::new(89, 112, 42, 13),
        destination: HudRect::new(523, 457, 42, 13),
    },
    RevSegment {
        threshold_q16: Some(0x5000),
        source: HudRect::new(3, 96, 46, 13),
        destination: HudRect::new(546, 457, 46, 13),
    },
    RevSegment {
        threshold_q16: Some(0x6000),
        source: HudRect::new(51, 91, 48, 18),
        destination: HudRect::new(570, 452, 48, 18),
    },
    RevSegment {
        threshold_q16: Some(0x7000),
        source: HudRect::new(3, 72, 44, 21),
        destination: HudRect::new(584, 444, 44, 21),
    },
    RevSegment {
        threshold_q16: Some(0x8000),
        source: HudRect::new(89, 71, 42, 22),
        destination: HudRect::new(589, 433, 42, 22),
    },
    RevSegment {
        threshold_q16: Some(0x9000),
        source: HudRect::new(3, 51, 41, 20),
        destination: HudRect::new(590, 422, 41, 20),
    },
    RevSegment {
        threshold_q16: Some(0xa000),
        source: HudRect::new(51, 51, 41, 19),
        destination: HudRect::new(590, 411, 41, 19),
    },
    RevSegment {
        threshold_q16: Some(0xb000),
        source: HudRect::new(3, 31, 41, 17),
        destination: HudRect::new(590, 400, 41, 17),
    },
    RevSegment {
        threshold_q16: Some(0xc000),
        source: HudRect::new(51, 30, 41, 15),
        destination: HudRect::new(590, 389, 41, 15),
    },
    RevSegment {
        threshold_q16: Some(0xd000),
        source: HudRect::new(3, 15, 41, 14),
        destination: HudRect::new(590, 378, 41, 14),
    },
    RevSegment {
        threshold_q16: Some(0xe000),
        source: HudRect::new(51, 17, 41, 12),
        destination: HudRect::new(590, 368, 41, 12),
    },
    RevSegment {
        threshold_q16: Some(0xf000),
        source: HudRect::new(3, 2, 41, 11),
        destination: HudRect::new(590, 357, 41, 11),
    },
];

/// Plans the exact recovered subset in native source order.
pub fn plan_normal_single_player_race_hud(
    input: RaceHudPreparedInputs,
) -> Result<RaceHudPlan, RaceHudPlanError> {
    if input.viewport.width == 0 || input.viewport.height == 0 {
        return Err(RaceHudPlanError::EmptyViewport);
    }

    let mut commands = Vec::with_capacity(24);
    let mut incomplete = vec![
        RaceHudIncomplete::CommonGaugeBorderFragments,
        RaceHudIncomplete::ProgressAuxiliaryMarkers,
        RaceHudIncomplete::BitmapRasterPolicy,
        RaceHudIncomplete::FontGlyphRasterPolicy,
    ];

    push_bitmap(
        &mut commands,
        input.viewport,
        RaceHudBitmap::CommonGauge640,
        HudRect::new(35, 127, 147, 71),
        HudRect::new(487, 402, 147, 71),
    );
    push_bitmap(
        &mut commands,
        input.viewport,
        RaceHudBitmap::CommonGauge640,
        HudRect::new(135, 41, 47, 87),
        HudRect::new(587, 316, 47, 87),
    );
    let (inset_source, inset_destination) = match input.gauge_inset {
        CommonGaugeInset::ConfigLowThreeBitsZero => {
            (HudRect::new(111, 20, 18, 9), HudRect::new(521, 440, 18, 9))
        }
        CommonGaugeInset::ConfigLowThreeBitsNonZero => {
            (HudRect::new(148, 20, 17, 8), HudRect::new(521, 440, 17, 8))
        }
    };
    push_bitmap(
        &mut commands,
        input.viewport,
        RaceHudBitmap::CommonGauge640,
        inset_source,
        inset_destination,
    );

    for segment in REV_SEGMENTS {
        if segment
            .threshold_q16
            .is_some_and(|threshold| input.rev_q16 <= threshold)
        {
            continue;
        }
        push_tinted_bitmap(
            &mut commands,
            input.viewport,
            segment.source,
            segment.destination,
        );
    }

    commands.push(RaceHudCommand::Text {
        font: RaceHudFont::Numbers640,
        text: input.speed.to_string(),
        position: HudTextPosition::Q16(HudPoint::new(speed_x(input.speed), 0xda9f)),
        color: COMMON_GAUGE_COLOR,
        flags: LEGACY_TEXT_FLAGS,
    });
    commands.push(RaceHudCommand::Text {
        font: RaceHudFont::Numbers640,
        text: gear_text(input.gear),
        position: HudTextPosition::Q16(HudPoint::new(0xf1a9, 0xac49)),
        color: COMMON_GAUGE_COLOR,
        flags: LEGACY_TEXT_FLAGS,
    });

    push_bitmap(
        &mut commands,
        input.viewport,
        RaceHudBitmap::HorizontalBar,
        HudRect::new(0, 0, 4, 4),
        HudRect::new(468, 469, 20, 4),
    );
    push_bitmap(
        &mut commands,
        input.viewport,
        RaceHudBitmap::HorizontalBarEnd,
        HudRect::new(0, 0, 4, 4),
        HudRect::new(464, 469, 4, 4),
    );

    if (0..=0x1_0000).contains(&input.progress_q16) {
        let y = 300 - q16_mul_floor(300, input.progress_q16);
        push_bitmap(
            &mut commands,
            input.viewport,
            RaceHudBitmap::ProgressRedSpot,
            HudRect::new(0, 0, 32, 32),
            HudRect::new(610, y, 32, 32),
        );
    } else {
        incomplete.push(RaceHudIncomplete::ProgressOutsideRecoveredRange {
            progress_q16: input.progress_q16,
        });
    }

    commands.push(RaceHudCommand::Text {
        font: RaceHudFont::TimeNumbers640,
        text: timer_text(input.elapsed_centiseconds),
        position: HudTextPosition::Pixels(HudPoint::new(
            scale_horizontal(28, input.viewport),
            scale_vertical(428, input.viewport),
        )),
        color: TIMER_COLOR,
        flags: LEGACY_TEXT_FLAGS,
    });

    Ok(RaceHudPlan {
        commands,
        incomplete,
    })
}

/// Plans the common gauge and progress-rail border fragments (`FUN_0045dac0`
/// and the `FUN_0045f530` progress-rail descriptors initialized in
/// `FUN_0045d050`).
pub fn plan_common_gauge_border_fragments(
    viewport: DisplayExtent,
) -> Result<Vec<RaceHudCommand>, RaceHudPlanError> {
    if viewport.width == 0 || viewport.height == 0 {
        return Err(RaceHudPlanError::EmptyViewport);
    }
    let mut commands = Vec::with_capacity(12);
    for (bitmap, source, destination) in [
        (
            RaceHudBitmap::ProgressBarTop,
            HudRect::new(0, 0, 16, 4),
            HudRect::new(618, 11, 16, 4),
        ),
        (
            RaceHudBitmap::VerticalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(630, 15, 4, 340),
        ),
        (
            RaceHudBitmap::VerticalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(618, 15, 4, 302),
        ),
        (
            RaceHudBitmap::HorizontalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(16, 11, 122, 4),
        ),
        (
            RaceHudBitmap::HorizontalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(15, 91, 123, 4),
        ),
        (
            RaceHudBitmap::HorizontalBarEndRight,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(138, 11, 4, 4),
        ),
        (
            RaceHudBitmap::HorizontalBarEndRight,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(138, 91, 4, 4),
        ),
        (
            RaceHudBitmap::TopLeftCorner,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(12, 11, 4, 4),
        ),
        (
            RaceHudBitmap::BottomLeftCurve,
            HudRect::new(0, 0, 16, 16),
            HudRect::new(12, 457, 16, 16),
        ),
        (
            RaceHudBitmap::HorizontalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(28, 469, 124, 4),
        ),
        (
            RaceHudBitmap::VerticalBar,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(12, 15, 4, 442),
        ),
        (
            RaceHudBitmap::HorizontalBarEndRight,
            HudRect::new(0, 0, 4, 4),
            HudRect::new(152, 469, 4, 4),
        ),
    ] {
        push_bitmap(&mut commands, viewport, bitmap, source, destination);
    }
    Ok(commands)
}

/// Plans the centered start-line countdown digit (`5..=1`, `FUN_00423570`) or
/// `"GO!"` banner (`FUN_00423700`) for `viewport`.
#[must_use]
pub fn plan_race_countdown_overlay(
    viewport: DisplayExtent,
    countdown: RaceHudCountdown,
) -> Option<RaceHudCommand> {
    if viewport.width == 0 || viewport.height == 0 {
        return None;
    }
    let vw = viewport.width as i32;
    let vh = viewport.height as i32;
    match countdown {
        RaceHudCountdown::Digit { digit, ticks_left } => {
            let source = match digit {
                5 => HudRect::new(0, 0, 44, 64),
                4 => HudRect::new(51, 0, 50, 64),
                3 => HudRect::new(104, 0, 46, 64),
                2 => HudRect::new(157, 0, 47, 64),
                1 => HudRect::new(213, 0, 35, 64),
                _ => return None,
            };
            let ticks = i32::from(ticks_left.min(19));
            if ticks <= 0 {
                return None;
            }
            let scale_q16 = (ticks << 16) / 20;
            let dst_h = ((i64::from(scale_q16) * i64::from(vh)) >> 16) as i32;
            let dst_w =
                ((((i64::from(scale_q16) * i64::from(source.width)) / 64) * i64::from(vh)) >> 16)
                    as i32;
            if dst_w <= 0 || dst_h <= 0 {
                return None;
            }
            let alpha = (((scale_q16 * 255) >> 16) + 1).clamp(0, 255) as u8;
            Some(RaceHudCommand::TintedBitmap {
                bitmap: RaceHudBitmap::CountdownDigits,
                source,
                destination: HudRect::new((vw - dst_w) / 2, (vh - dst_h) / 2, dst_w, dst_h),
                color: HudColor::new(0xff, 0xff, 0xff, alpha),
                legacy: LegacyBitmapArguments::TINTED_CALL,
            })
        }
        RaceHudCountdown::Go { ticks_left } => {
            let ticks = i32::from(ticks_left.clamp(1, 19));
            let t_q16 = ((20 - ticks) << 16) / 20;
            let w_scale_q16 = ((i64::from(t_q16) * 0xcccc) >> 16) as i32;
            let dst_w = ((i64::from(w_scale_q16) * i64::from(vw)) >> 16) as i32;
            let dst_h = ((i64::from(t_q16) * i64::from(vh)) >> 16) as i32;
            if dst_w <= 0 || dst_h <= 0 {
                return None;
            }
            let alpha = (((0x10000 - t_q16) * 255) >> 16).clamp(0, 255) as u8;
            Some(RaceHudCommand::TintedBitmap {
                bitmap: RaceHudBitmap::CountdownGo,
                source: HudRect::new(0, 0, 128, 64),
                destination: HudRect::new((vw - dst_w) / 2, (vh - dst_h) / 2, dst_w, dst_h),
                color: HudColor::new(0xff, 0xff, 0xff, alpha),
                legacy: LegacyBitmapArguments::TINTED_CALL,
            })
        }
    }
}

/// Plans the in-race checkpoint HUD elements (`FUN_00464930`, `FUN_00463080`,
/// `FUN_0047fe50`):
/// 1. Top-left 4-row classification table background bars (`0x465446..0x465b23`)
///    and, once Split 1+ has been crossed, the 4 rank numbers (`placenum.fon`)
///    and 3-letter driver codes (`result.fon`).
/// 2. Right-side progress-rail completed sector color bars (`white.bmp`) and
///    white split tick marks (`hbar.bmp`, `FUN_00463080`).
/// 3. Bottom-left latched split time (`timenum.fon` at `(28, 404)` in cyan) and
///    top-center split delta (`timenum.fon` centered at `Q16(0x8000, 0x10a3)`
///    in green/red/white, `FUN_0047fe50`).
#[must_use]
pub fn plan_race_checkpoint_hud(
    viewport: DisplayExtent,
    checkpoint: &RaceHudCheckpointInputs,
) -> Vec<RaceHudCommand> {
    if viewport.width == 0 || viewport.height == 0 {
        return Vec::new();
    }
    let mut commands = Vec::with_capacity(28);

    // 1. Top-left 4-row classification bars and rank/driver labels (`FUN_00464930`).
    const ROW_Y_Q16: [i32; 4] = [0x08f5, 0x1333, 0x1d70, 0x27ae];
    for (row_idx, &y_q16) in ROW_Y_Q16.iter().enumerate() {
        let row_entry = checkpoint.classification_rows.get(row_idx);
        let bar_color = if row_entry.is_some_and(|row| row.is_player) {
            if checkpoint.player_rank_gained_or_held {
                HudColor::new(0x00, 0xff, 0x00, 0xc8)
            } else {
                HudColor::new(0xff, 0x00, 0x00, 0xc8)
            }
        } else if row_idx % 2 == 0 {
            HudColor::new(0xa0, 0xa0, 0xa0, 0xc8)
        } else {
            HudColor::new(0x80, 0x80, 0x80, 0xc8)
        };
        commands.push(RaceHudCommand::TintedBitmap {
            bitmap: RaceHudBitmap::WhitePixel,
            source: HudRect::new(0, 0, 1, 1),
            destination: scale_destination(
                HudRect::new(16, 15 + 19 * (row_idx as i32), 125, 19),
                viewport,
            ),
            color: bar_color,
            legacy: LegacyBitmapArguments::TINTED_CALL,
        });
        if let Some(row) = row_entry {
            commands.push(RaceHudCommand::Text {
                font: RaceHudFont::PlaceNumbers640,
                text: row.rank.to_string(),
                position: HudTextPosition::Q16(HudPoint::new(0x072b, y_q16)),
                color: TIMER_COLOR,
                flags: 0x11,
            });
            commands.push(RaceHudCommand::Text {
                font: RaceHudFont::Result640,
                text: row.code.clone(),
                position: HudTextPosition::Q16(HudPoint::new(0x3851, y_q16 + 0x028f)),
                color: TIMER_COLOR,
                flags: 0x13,
            });
        }
    }

    // 2. Right-side progress-rail sector bars and split tick marks (`FUN_00463080`).
    let mut prev_split_q16 = 0_i32;
    for (idx, &split_q16) in checkpoint.split_progress_q16.iter().enumerate() {
        let clamped_q16 = split_q16.clamp(0, 0x1_0000);
        if clamped_q16 < checkpoint.progress_q16 && clamped_q16 > prev_split_q16 {
            let prev_y = 315 - q16_mul_floor(300, prev_split_q16);
            let cur_y = 315 - q16_mul_floor(300, clamped_q16);
            let height = (prev_y - cur_y).max(1);
            let gained = checkpoint.sector_gained.get(idx).copied().unwrap_or(true);
            let sector_color = if gained {
                HudColor::new(0x00, 0xff, 0x00, 0x80)
            } else {
                HudColor::new(0xff, 0x00, 0x00, 0x80)
            };
            commands.push(RaceHudCommand::TintedBitmap {
                bitmap: RaceHudBitmap::WhitePixel,
                source: HudRect::new(0, 0, 1, 1),
                destination: scale_destination(HudRect::new(621, cur_y, 10, height), viewport),
                color: sector_color,
                legacy: LegacyBitmapArguments::TINTED_CALL,
            });
        }
        prev_split_q16 = clamped_q16;
    }
    for &split_q16 in &checkpoint.split_progress_q16 {
        if (0x028f..0xfd71).contains(&split_q16) {
            let tick_y = 315 - q16_mul_floor(300, split_q16) - 2;
            push_bitmap(
                &mut commands,
                viewport,
                RaceHudBitmap::HorizontalBar,
                HudRect::new(0, 0, 4, 4),
                HudRect::new(618, tick_y, 16, 4),
            );
        }
    }

    // 3. Active split time & top-center split delta (`FUN_0047fe50`).
    if checkpoint.split_display_ticks > 0
        && let Some(split_cs) = checkpoint.last_split_centiseconds
    {
        let alpha = if checkpoint.split_display_ticks >= 25 {
            0xff
        } else {
            (u16::from(checkpoint.split_display_ticks) * 10).min(255) as u8
        };
        // Latched split clock at pixel `(28, 404)` (`0x480272..0x48029a`) in
        // cyan `[0x20, 0xff, 0xff, alpha]` (`0x4f659c`).
        commands.push(RaceHudCommand::Text {
            font: RaceHudFont::TimeNumbers640,
            text: timer_text(split_cs),
            position: HudTextPosition::Pixels(HudPoint::new(
                scale_horizontal(28, viewport),
                scale_vertical(404, viewport),
            )),
            color: HudColor::new(0x20, 0xff, 0xff, alpha),
            flags: LEGACY_TEXT_FLAGS,
        });
        // Top-center split delta vs fastest rival (`0x4802a9..0x4803f9`).
        if let Some(ref_cs) = checkpoint.reference_split_centiseconds {
            let (delta_str, delta_color) = if split_cs < ref_cs {
                (
                    format!("-{}", timer_text(ref_cs - split_cs)),
                    HudColor::new(0x40, 0xff, 0x20, alpha),
                )
            } else if split_cs > ref_cs {
                (
                    format!("+{}", timer_text(split_cs - ref_cs)),
                    HudColor::new(0xff, 0x40, 0x20, alpha),
                )
            } else {
                (
                    "00:00.00".to_owned(),
                    HudColor::new(0xff, 0xff, 0xff, alpha),
                )
            };
            commands.push(RaceHudCommand::Text {
                font: RaceHudFont::TimeNumbers640,
                text: delta_str,
                position: HudTextPosition::Q16(HudPoint::new(0x8000, 0x10a3)),
                color: delta_color,
                flags: 0x12,
            });
        }
    }

    commands
}

fn push_bitmap(
    commands: &mut Vec<RaceHudCommand>,
    viewport: DisplayExtent,
    bitmap: RaceHudBitmap,
    source: HudRect,
    destination: HudRect,
) {
    commands.push(RaceHudCommand::Bitmap {
        bitmap,
        source,
        destination: scale_destination(destination, viewport),
        legacy: LegacyBitmapArguments::OPAQUE_CALL,
    });
}

fn push_tinted_bitmap(
    commands: &mut Vec<RaceHudCommand>,
    viewport: DisplayExtent,
    source: HudRect,
    destination: HudRect,
) {
    commands.push(RaceHudCommand::TintedBitmap {
        bitmap: RaceHudBitmap::CommonGauge640,
        source,
        destination: scale_destination(destination, viewport),
        color: COMMON_GAUGE_COLOR,
        legacy: LegacyBitmapArguments::TINTED_CALL,
    });
}

fn scale_destination(rectangle: HudRect, viewport: DisplayExtent) -> HudRect {
    HudRect::new(
        scale_horizontal(rectangle.x, viewport),
        scale_vertical(rectangle.y, viewport),
        scale_horizontal(rectangle.width, viewport),
        scale_vertical(rectangle.height, viewport),
    )
}

fn scale_horizontal(value: i32, viewport: DisplayExtent) -> i32 {
    ((i64::from(value) * i64::from(viewport.width)) / 640) as i32
}

fn scale_vertical(value: i32, viewport: DisplayExtent) -> i32 {
    ((i64::from(value) * i64::from(viewport.height)) / 480) as i32
}

fn q16_mul_floor(value: i32, q16: i32) -> i32 {
    ((i64::from(value) * i64::from(q16)) >> 16) as i32
}

fn speed_x(speed: u32) -> i32 {
    if speed < 10 {
        0xd8d4
    } else if speed < 100 {
        0xd6c8
    } else {
        0xd4bc
    }
}

fn gear_text(gear: i8) -> String {
    match gear {
        0 => "N".to_owned(),
        7 => "R".to_owned(),
        value => value.to_string(),
    }
}

fn timer_text(centiseconds: u32) -> String {
    let minutes = centiseconds / 6_000;
    let seconds = (centiseconds / 100) % 60;
    let hundredths = centiseconds % 100;
    let mut text = String::new();
    write!(text, "{minutes:02}:{seconds:02}.{hundredths:02}")
        .expect("writing to a String cannot fail");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: DisplayExtent = DisplayExtent {
        width: 640,
        height: 480,
    };

    fn input() -> RaceHudPreparedInputs {
        RaceHudPreparedInputs {
            viewport: VIEWPORT,
            rev_q16: 0x2800,
            speed: 87,
            gear: 3,
            progress_q16: 0x8000,
            elapsed_centiseconds: 7_345,
            gauge_inset: CommonGaugeInset::ConfigLowThreeBitsZero,
        }
    }

    #[test]
    fn normal_plan_preserves_recovered_source_order_and_values() {
        let plan = plan_normal_single_player_race_hud(input()).unwrap();

        assert_eq!(plan.commands.len(), 11);
        assert_eq!(
            plan.commands[0],
            bitmap(
                RaceHudBitmap::CommonGauge640,
                HudRect::new(35, 127, 147, 71),
                HudRect::new(487, 402, 147, 71),
            )
        );
        assert_eq!(
            plan.commands[4],
            tinted(
                HudRect::new(19, 114, 25, 11),
                HudRect::new(498, 459, 25, 11),
            )
        );
        assert_eq!(
            plan.commands[5],
            text(
                RaceHudFont::Numbers640,
                "87",
                HudTextPosition::Q16(HudPoint::new(0xd6c8, 0xda9f)),
                COMMON_GAUGE_COLOR,
            )
        );
        assert_eq!(
            plan.commands[6],
            text(
                RaceHudFont::Numbers640,
                "3",
                HudTextPosition::Q16(HudPoint::new(0xf1a9, 0xac49)),
                COMMON_GAUGE_COLOR,
            )
        );
        assert_eq!(
            plan.commands[9],
            bitmap(
                RaceHudBitmap::ProgressRedSpot,
                HudRect::new(0, 0, 32, 32),
                HudRect::new(610, 150, 32, 32),
            )
        );
        assert_eq!(
            plan.commands[10],
            text(
                RaceHudFont::TimeNumbers640,
                "01:13.45",
                HudTextPosition::Pixels(HudPoint::new(28, 428)),
                TIMER_COLOR,
            )
        );
    }

    #[test]
    fn destinations_and_timer_pixels_scale_from_640_by_480_with_truncation() {
        let mut scaled = input();
        scaled.viewport = DisplayExtent {
            width: 960,
            height: 720,
        };
        let plan = plan_normal_single_player_race_hud(scaled).unwrap();

        let RaceHudCommand::Bitmap { destination, .. } = plan.commands[0] else {
            panic!("first command must be a bitmap");
        };
        assert_eq!(destination, HudRect::new(730, 603, 220, 106));
        let RaceHudCommand::Text { position, .. } = &plan.commands[10] else {
            panic!("last command must be timer text");
        };
        assert_eq!(*position, HudTextPosition::Pixels(HudPoint::new(42, 642)));
    }

    #[test]
    fn speed_alignment_gear_labels_and_inset_branch_are_explicit() {
        for (speed, x) in [(9, 0xd8d4), (10, 0xd6c8), (99, 0xd6c8), (100, 0xd4bc)] {
            let mut case = input();
            case.speed = speed;
            let plan = plan_normal_single_player_race_hud(case).unwrap();
            let RaceHudCommand::Text { position, .. } = &plan.commands[5] else {
                panic!("speed command must be text");
            };
            assert_eq!(*position, HudTextPosition::Q16(HudPoint::new(x, 0xda9f)));
        }

        for (gear, expected) in [(0, "N"), (7, "R"), (-1, "-1")] {
            let mut case = input();
            case.gear = gear;
            let plan = plan_normal_single_player_race_hud(case).unwrap();
            let RaceHudCommand::Text { text, .. } = &plan.commands[6] else {
                panic!("gear command must be text");
            };
            assert_eq!(text, expected);
        }

        let mut alternate = input();
        alternate.gauge_inset = CommonGaugeInset::ConfigLowThreeBitsNonZero;
        let plan = plan_normal_single_player_race_hud(alternate).unwrap();
        assert_eq!(
            plan.commands[2],
            bitmap(
                RaceHudBitmap::CommonGauge640,
                HudRect::new(148, 20, 17, 8),
                HudRect::new(521, 440, 17, 8),
            )
        );
    }

    #[test]
    fn complete_rev_segments_preserve_recovered_order_and_descriptors() {
        let mut case = input();
        case.rev_q16 = 0x1_0000;
        let plan = plan_normal_single_player_race_hud(case).unwrap();

        let actual: Vec<_> = plan.commands[3..3 + REV_SEGMENTS.len()]
            .iter()
            .map(|command| match command {
                RaceHudCommand::TintedBitmap {
                    source,
                    destination,
                    ..
                } => (*source, *destination),
                _ => panic!("rev command must be a tinted bitmap"),
            })
            .collect();
        let expected: Vec<_> = REV_SEGMENTS
            .iter()
            .map(|segment| (segment.source, segment.destination))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn rev_thresholds_are_strict_and_add_one_segment_at_a_time() {
        for threshold_step in 2..=15 {
            let threshold = threshold_step * 0x1000;
            let mut at_threshold = input();
            at_threshold.rev_q16 = threshold;
            let at_threshold = plan_normal_single_player_race_hud(at_threshold).unwrap();
            assert_eq!(tinted_count(&at_threshold), threshold_step as usize - 1);

            let mut above_threshold = input();
            above_threshold.rev_q16 = threshold + 1;
            let above_threshold = plan_normal_single_player_race_hud(above_threshold).unwrap();
            assert_eq!(tinted_count(&above_threshold), threshold_step as usize);
        }
    }

    #[test]
    fn incomplete_progress_path_is_typed_and_not_fabricated() {
        let mut case = input();
        case.rev_q16 = 0x4000;
        case.progress_q16 = -1;
        let plan = plan_normal_single_player_race_hud(case).unwrap();

        assert!(
            plan.incomplete
                .contains(&RaceHudIncomplete::ProgressOutsideRecoveredRange { progress_q16: -1 })
        );
        assert!(!plan.commands.iter().any(|command| matches!(
            command,
            RaceHudCommand::Bitmap {
                bitmap: RaceHudBitmap::ProgressRedSpot,
                ..
            }
        )));
    }

    #[test]
    fn empty_viewport_is_rejected() {
        let mut case = input();
        case.viewport.width = 0;
        assert_eq!(
            plan_normal_single_player_race_hud(case),
            Err(RaceHudPlanError::EmptyViewport)
        );
    }

    fn bitmap(bitmap: RaceHudBitmap, source: HudRect, destination: HudRect) -> RaceHudCommand {
        RaceHudCommand::Bitmap {
            bitmap,
            source,
            destination,
            legacy: LegacyBitmapArguments::OPAQUE_CALL,
        }
    }

    fn tinted(source: HudRect, destination: HudRect) -> RaceHudCommand {
        RaceHudCommand::TintedBitmap {
            bitmap: RaceHudBitmap::CommonGauge640,
            source,
            destination,
            color: COMMON_GAUGE_COLOR,
            legacy: LegacyBitmapArguments::TINTED_CALL,
        }
    }

    fn text(
        font: RaceHudFont,
        value: &str,
        position: HudTextPosition,
        color: HudColor,
    ) -> RaceHudCommand {
        RaceHudCommand::Text {
            font,
            text: value.to_owned(),
            position,
            color,
            flags: LEGACY_TEXT_FLAGS,
        }
    }

    fn tinted_count(plan: &RaceHudPlan) -> usize {
        plan.commands
            .iter()
            .filter(|command| matches!(command, RaceHudCommand::TintedBitmap { .. }))
            .count()
    }

    #[test]
    fn common_gauge_border_fragments_emit_twelve_commands_at_640x480_only() {
        let cmds = plan_common_gauge_border_fragments(DisplayExtent {
            width: 640,
            height: 480,
        })
        .unwrap();
        assert_eq!(cmds.len(), 12);
        assert_eq!(
            plan_common_gauge_border_fragments(DisplayExtent {
                width: 0,
                height: 480,
            }),
            Err(RaceHudPlanError::EmptyViewport)
        );
    }

    #[test]
    fn countdown_overlay_plans_digit_and_go_quads() {
        let viewport = DisplayExtent {
            width: 640,
            height: 480,
        };
        let Some(RaceHudCommand::TintedBitmap {
            bitmap,
            source,
            destination,
            color,
            ..
        }) = plan_race_countdown_overlay(
            viewport,
            RaceHudCountdown::Digit {
                digit: 5,
                ticks_left: 19,
            },
        )
        else {
            panic!("expected countdown digit command");
        };
        assert_eq!(bitmap, RaceHudBitmap::CountdownDigits);
        assert_eq!(source, HudRect::new(0, 0, 44, 64));
        assert_eq!(destination.height, 455);
        assert_eq!(color.alpha, 243);

        let Some(RaceHudCommand::TintedBitmap {
            bitmap: go_bmp,
            source: go_src,
            destination: go_dst,
            color: go_color,
            ..
        }) = plan_race_countdown_overlay(viewport, RaceHudCountdown::Go { ticks_left: 1 })
        else {
            panic!("expected countdown GO command");
        };
        assert_eq!(go_bmp, RaceHudBitmap::CountdownGo);
        assert_eq!(go_src.width, 128);
        assert_eq!(go_src.height, 64);
        assert!(go_dst.width > 128 && go_dst.height > 64);
        assert!(go_color.alpha > 0);
    }

    #[test]
    fn checkpoint_hud_plans_classification_rows_sector_bars_and_split_delta() {
        let viewport = DisplayExtent {
            width: 640,
            height: 480,
        };
        let cmds = plan_race_checkpoint_hud(
            viewport,
            &RaceHudCheckpointInputs {
                progress_q16: 0x4000,
                split_progress_q16: vec![0x2000, 0x6000],
                sector_gained: vec![true, false],
                classification_rows: vec![
                    RaceHudClassificationRow {
                        rank: 1,
                        code: "MCR".to_owned(),
                        is_player: false,
                    },
                    RaceHudClassificationRow {
                        rank: 2,
                        code: "PLR".to_owned(),
                        is_player: true,
                    },
                ],
                player_rank_gained_or_held: true,
                split_display_ticks: 100,
                last_split_centiseconds: Some(2450),
                reference_split_centiseconds: Some(2500),
            },
        );
        // 4 top-left bars + 2*2 text labels + 1 completed sector bar + 2 split ticks + 2 split texts = 13
        assert_eq!(cmds.len(), 13);
        assert!(cmds.iter().any(|cmd| matches!(
            cmd,
            RaceHudCommand::Text {
                font: RaceHudFont::TimeNumbers640,
                text,
                flags: 0x12,
                ..
            } if text == "-00:00.50"
        )));
    }
}
