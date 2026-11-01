//! The front end's Options subtree (root entry 4) as `FUN_004be540` builds
//! it, and the callbacks that read and write the cmRally.cfg record `img`:
//! Options `0xd453b8`, Settings `0xd4bf80`, Graphics `0xd41718`, Graphics
//! detail `0xd4c8c8`, Advanced graphics `0xd478f0`, Sound `0xd4ad98`, the
//! hi-scores `0xd49bb8` with its three tables, and Difficulty `0xd49718`.
//! Every menu uses the setups' sound table `0xd90060` and wraps. Items are
//! built by `FUN_004ae720` (value: count, initial, wrap), `FUN_004ae840`
//! (toggle: two wrapping values), `FUN_004ae9a0` (submenu) and
//! `FUN_004ae950` (back), each keyed by `FUN_004aea00(m, draw, activate,
//! key)`.
//!
//! The callbacks touch `img`, never the live block: `FUN_004b93f0` copies the
//! bytes into it at the next commit. The device-dependent rules of the
//! advanced graphics (display devices, the 3D device, the video memory a
//! resolution needs) read the host's [`DisplayCaps`]. A description of
//! disassembly, not original source. Evidence:
//! `rea.official-eu.frontend-options`.

use crate::config::LEGACY_CONFIG_SIZE;
use crate::descriptor_menu::{ItemKind, Menu, MenuId, MenuItem};
use crate::frontend::ScreenId;
use crate::frontend_menu::ValueWidget;
use crate::modern_settings::{
    AntiAliasingMode, AspectRatioMode, DrawDistanceScale, FpsLimit, ModernPreset, ModernResolution,
    ModernSettings, TextureFilterMode, UpscalingMode,
};
use crate::scalar_tween::ScalarTween;

type Record = [u8; LEGACY_CONFIG_SIZE];

/// An options screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OptionScreen {
    Options,
    Settings,
    Graphics,
    Detail,
    Advanced,
    Sound,
    HiScores,
    BestStage,
    BestRally,
    ChampionshipScores,
    Difficulty,
    Modern,
}

impl OptionScreen {
    pub const ALL: [Self; 12] = [
        Self::Options,
        Self::Settings,
        Self::Graphics,
        Self::Detail,
        Self::Advanced,
        Self::Sound,
        Self::HiScores,
        Self::BestStage,
        Self::BestRally,
        Self::ChampionshipScores,
        Self::Difficulty,
        Self::Modern,
    ];

    /// The menu object's address.
    pub const fn screen(self) -> ScreenId {
        ScreenId::from_legacy(match self {
            Self::Options => 0x00d4_53b8,
            Self::Settings => 0x00d4_bf80,
            Self::Graphics => 0x00d4_1718,
            Self::Detail => 0x00d4_c8c8,
            Self::Advanced => 0x00d4_78f0,
            Self::Sound => 0x00d4_ad98,
            Self::HiScores => 0x00d4_9bb8,
            Self::BestStage => 0x00d4_9020,
            Self::BestRally => 0x00d4_4f10,
            Self::ChampionshipScores => 0x00d4_c428,
            Self::Difficulty => 0x00d4_9718,
            Self::Modern => 0x00d4_7900,
        })
    }

    /// The camera target the create call passes (argument 4).
    pub const fn camera_target(self) -> u32 {
        match self {
            Self::Options => 0x0050_7a80,
            Self::Settings => 0x0050_7aa8,
            Self::Graphics | Self::Detail | Self::Advanced | Self::Modern => 0x0050_7ac8,
            Self::Sound => 0x0050_7ad0,
            Self::HiScores | Self::BestStage | Self::BestRally | Self::ChampionshipScores => {
                0x0050_7ab0
            }
            Self::Difficulty => 0x0050_7ab8,
        }
    }

    pub const fn title_id(self) -> i32 {
        match self {
            Self::Options => 4,
            Self::Settings => 7,
            Self::Graphics => 0xb,
            Self::Detail => 0xe2,
            Self::Advanced => 0xe8,
            Self::Sound => 0xc,
            Self::HiScores => 8,
            Self::BestStage => 0x2c,
            Self::BestRally => 0x2d,
            Self::ChampionshipScores => 0x2e,
            Self::Difficulty => 0x66,
            Self::Modern => modern_text::TITLE_ENHANCED,
        }
    }

    pub fn of(screen: ScreenId) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.screen() == screen)
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Synthetic text IDs for the modern graphics & gamepad menus.
pub mod modern_text {
    pub const TITLE_ENHANCED: i32 = 0x300;
    pub const LABEL_ENHANCED: i32 = 0x301;
    pub const LABEL_PRESET: i32 = 0x302;
    pub const LABEL_ASPECT: i32 = 0x303;
    pub const LABEL_FPS: i32 = 0x304;
    pub const LABEL_DRAW_DISTANCE: i32 = 0x305;
    pub const LABEL_MSAA: i32 = 0x306;
    pub const LABEL_UPSCALING: i32 = 0x307;
    pub const LABEL_RUMBLE: i32 = 0x308;
    pub const LABEL_VSYNC: i32 = 0x309;
    pub const LABEL_HD_ART: i32 = 0x30a;

    pub const fn lookup(text_id: i32) -> Option<&'static [u8]> {
        match text_id {
            TITLE_ENHANCED | LABEL_ENHANCED => Some(b"HUD"),
            LABEL_PRESET => Some(b"PRESET"),
            LABEL_ASPECT => Some(b"ASPECT RATIO"),
            LABEL_FPS => Some(b"FRAME RATE"),
            LABEL_DRAW_DISTANCE => Some(b"DRAW DISTANCE"),
            LABEL_MSAA => Some(b"ANTI-ALIASING"),
            LABEL_UPSCALING => Some(b"UPSCALING"),
            LABEL_RUMBLE => Some(b"RUMBLE"),
            LABEL_VSYNC => Some(b"VSYNC"),
            LABEL_HD_ART => Some(b"HD ART PACK"),
            _ => None,
        }
    }
}

/// Item keys.
pub mod key {
    pub const BACK: i32 = 8;
    pub const SETTINGS: i32 = 9;
    pub const HI_SCORES: i32 = 0xa;
    pub const DIFFICULTY: i32 = 0xb;
    pub const GRAPHICS: i32 = 0xe;
    pub const SOUND: i32 = 0xf;
    pub const CONTROLS_1: i32 = 0x10;
    pub const CONTROLS_2: i32 = 0x11;
    pub const ARROWS: i32 = 4;
    pub const GAME_DISPLAYS: i32 = 5;
    pub const SPLIT: i32 = 2;
    pub const WIPES: i32 = 6;
    pub const OBJECT_DETAIL: i32 = 0xc;
    pub const TRACK_DISTANCE: i32 = 0xd;
    pub const OBJECT_DISTANCE: i32 = 0xe;
    pub const CAR_RESOLUTION: i32 = 0xf;
    pub const WEATHER: i32 = 0x10;
    pub const CAR_EFFECTS: i32 = 0x11;
    pub const DEVICE: i32 = 7;
    pub const CHIPSET: i32 = 8;
    pub const RESOLUTION: i32 = 9;
    pub const TEXTURES: i32 = 0xa;
    pub const SMOOTHNESS: i32 = 0xb;
    pub const EFFECTS_VOLUME: i32 = 0x21;
    pub const SPEECH_VOLUME: i32 = 0x22;
    pub const MUSIC_VOLUME: i32 = 0x23;
    pub const BEST_STAGE: i32 = 0x13;
    pub const BEST_RALLY: i32 = 0x14;
    pub const CHAMPIONSHIP: i32 = 0x15;
    pub const COUNTRY: i32 = 0x16;
    pub const SCORE_DIFFICULTY: i32 = 0x17;
    pub const NOVICE: i32 = 0x18;
    pub const INTERMEDIATE: i32 = 0x19;
    pub const EXPERT: i32 = 0x1a;
    pub const MODERN_ENHANCED: i32 = 0x30;
    pub const MODERN_PRESET: i32 = 0x31;
    pub const MODERN_ASPECT: i32 = 0x32;
    pub const MODERN_FPS: i32 = 0x33;
    pub const MODERN_DRAW_DISTANCE: i32 = 0x34;
    pub const MODERN_MSAA: i32 = 0x35;
    pub const MODERN_UPSCALING: i32 = 0x36;
    pub const MODERN_HD_ART: i32 = 0x37;
}

/// The display hardware the advanced graphics rules ask about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayCaps {
    /// `FUN_004a5f60` (`DAT_00b73c38`): display devices.
    pub devices: i32,
    /// `FUN_004a5f50` (`DAT_00b73c3c`): a 3D device exists.
    pub has_3d: bool,
    /// `DAT_00b73c40`: the device cannot do car effects or car resolution.
    pub limited_b73c40: bool,
    /// `DAT_00502838`.
    pub flag_502838: bool,
    /// `FUN_00498b40(res)`: the video memory holds resolution `res` (0
    /// also answers for the high texture quality).
    pub resolution_fits: [bool; 4],
}

impl DisplayCaps {
    /// A host with one display device and a 3D device that holds every
    /// resolution (harness default until the host reports its own).
    pub const DEFAULT: Self = Self {
        devices: 1,
        has_3d: true,
        limited_b73c40: false,
        flag_502838: true,
        resolution_fits: [true; 4],
    };
}

/// A front-end effect of an options callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsEffect {
    /// `FUN_004c9860(v * 20)` / `FUN_004c97f0(v * 20)`: the front end's
    /// effects and music volumes (percent).
    Volumes { effects: i32, music: i32 },
    /// `FUN_004c98e0(m, 0, 1)`: the preview cue (slot 2).
    PreviewCue,
    /// Modern display/graphics settings changed in `OptionScreen::Modern` or
    /// `OptionScreen::Advanced`.
    ModernChanged(ModernSettings),
}

fn value_item(text: i32, key: i32, count: i32, value: i32, wrap: bool, draw: u32) -> MenuItem {
    let mut item = MenuItem::new(ItemKind::Value, text, draw);
    item.key = key;
    item.value = ValueWidget {
        value,
        value_count: count,
        wrap,
    };
    item
}

/// `FUN_004ae840`.
fn toggle(text: i32, key: i32, on: bool) -> MenuItem {
    value_item(text, key, 2, i32::from(on), true, 0)
}

fn submenu(text: i32, key: i32, target: MenuId, draw: u32) -> MenuItem {
    let mut item = MenuItem::new(ItemKind::Submenu(target), text, draw);
    item.key = key;
    item
}

fn back(text: i32, draw: u32) -> MenuItem {
    let mut item = MenuItem::new(ItemKind::Back, text, draw);
    item.key = key::BACK;
    item
}

/// `FUN_004aeb50`: a keyed item's value, -1 without one.
pub fn value(menu: &Menu, key: i32) -> i32 {
    menu.find(key).map_or(-1, |i| menu.items[i].value.value)
}

/// A keyed item's value write; a missing item's lands on `menu + 0x34`
/// (harmless) and is dropped here.
fn set_value(menu: &mut Menu, key: i32, value: i32) {
    if let Some(i) = menu.find(key) {
        menu.items[i].value.value = value;
    }
}

fn set_enabled(menu: &mut Menu, key: i32, enabled: bool) {
    if let Some(i) = menu.find(key) {
        menu.items[i].enabled = enabled;
    }
}

fn bit(byte: u8, shift: u8) -> i32 {
    i32::from((byte >> shift) & 1)
}

/// The row screens' slide templates (`0x50b2f8` Options, `0x5095c0`
/// Settings, `0x509548` Difficulty, `0x509640` Hi-scores), from the user's
/// executable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowTemplates {
    pub options: Vec<crate::frontend_flow::SlideChannel>,
    pub settings: Vec<crate::frontend_flow::SlideChannel>,
    pub difficulty: Vec<crate::frontend_flow::SlideChannel>,
    pub hi_scores: Vec<crate::frontend_flow::SlideChannel>,
}

impl RowTemplates {
    /// Parse `n` 8-byte channel records (`{i16 a0, a1, a2; u8 in, out}`).
    pub fn parse_channels(bytes: &[u8], n: usize) -> Vec<crate::frontend_flow::SlideChannel> {
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .take(n)
            .map(|c| crate::frontend_flow::SlideChannel {
                angles: [
                    i16::from_le_bytes([c[0], c[1]]),
                    i16::from_le_bytes([c[2], c[3]]),
                    i16::from_le_bytes([c[4], c[5]]),
                ],
                delay_in: c[6],
                delay_out: c[7],
            })
            .collect()
    }

    fn of(&self, screen: OptionScreen) -> Option<&[crate::frontend_flow::SlideChannel]> {
        match screen {
            OptionScreen::Options => Some(&self.options),
            OptionScreen::Settings => Some(&self.settings),
            OptionScreen::Difficulty => Some(&self.difficulty),
            OptionScreen::HiScores => Some(&self.hi_scores),
            _ => None,
        }
    }
}

/// Whether `screen` slides its rows on the arc (`FUN_004c4410`) rather than
/// a value panel.
pub const fn is_row_screen(screen: OptionScreen) -> bool {
    matches!(
        screen,
        OptionScreen::Options
            | OptionScreen::Settings
            | OptionScreen::Difficulty
            | OptionScreen::HiScores
    )
}

/// A row screen's item key → channel (`0x4cf040`, `0x4ca2f0`, `0x4ca050`,
/// `0x4cba60`).
pub fn row_slot(screen: OptionScreen, item_key: i32) -> Option<usize> {
    let keys: &[i32] = match screen {
        OptionScreen::Options => &[key::SETTINGS, key::HI_SCORES, key::DIFFICULTY, key::BACK],
        OptionScreen::Settings => &[
            key::GRAPHICS,
            key::SOUND,
            key::CONTROLS_1,
            key::CONTROLS_2,
            key::BACK,
        ],
        OptionScreen::Difficulty => &[key::NOVICE, key::INTERMEDIATE, key::EXPERT, key::BACK],
        OptionScreen::HiScores => &[
            key::CHAMPIONSHIP,
            key::BEST_RALLY,
            key::BEST_STAGE,
            key::BACK,
        ],
        _ => &[],
    };
    keys.iter().position(|&k| k == item_key)
}

/// The panel's rest x (`0x1999`).
pub const PANEL_REST_X: i32 = 0x1999;
/// The panel's y (`0x509590`).
pub const PANEL_Y: i32 = 0x2666;

/// The subtree's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionsScreens {
    ids: [MenuId; 12],
    templates: RowTemplates,
    /// The row channels' tweens and positions (`d90550`/`d900e0`/`d90098`/
    /// `d904c0`).
    row_tweens: Vec<ScalarTween>,
    pub rows: [(i32, i32); 5],
    /// The panel slide `0xd900d4`, its x `DAT_00d9011c` and the amplitude
    /// `0x509594` (0x8000 until the first value panel's enter writes 0xb333).
    panel_tween: Option<ScalarTween>,
    pub panel_x: i32,
    amplitude: i32,
    /// `DAT_00d90064 == 0`: the sound screen mutes the value cue.
    pub value_cue_muted: bool,
    /// `DAT_00d900d5` and the captured volumes `d900d8`/`d900dc`/`d900d0`.
    volume_latch: Option<[i32; 3]>,
    /// `DAT_00d90128`: the last resolution chosen.
    resolution_d90128: i32,
    /// `DAT_00d90504`: the controls' player (0 or 1).
    pub player: u8,
    /// Whether modern settings menus are enabled.
    pub modern_mode: bool,
    /// Current modern settings edited by `OptionScreen::Modern` and `OptionScreen::Advanced`.
    pub modern: ModernSettings,
}

impl OptionsScreens {
    /// Add the subtree's menus to `set`, below `root`, with `controls` the
    /// control setup screen the Settings items open.
    pub fn build(
        set: &mut crate::descriptor_menu::MenuSet,
        root: MenuId,
        controls: MenuId,
        img: &Record,
    ) -> Self {
        let base = set.menus.len();
        let id = |screen: OptionScreen| MenuId(base + screen.index());
        let menu = |screen: OptionScreen, parent: MenuId| {
            Menu::new(
                screen.title_id(),
                Some(parent),
                Some(crate::frontend_sounds::SETUP_SOUNDS),
            )
        };
        use OptionScreen as S;
        let mut menus = Vec::new();
        // Options: draws `0x4cf040`.
        let mut m = menu(S::Options, root);
        m.items = vec![
            submenu(7, key::SETTINGS, id(S::Settings), 0x004c_f040),
            submenu(8, key::HI_SCORES, id(S::HiScores), 0x004c_f040),
            submenu(0x66, key::DIFFICULTY, id(S::Difficulty), 0x004c_f040),
            back(0xa, 0x004c_f040),
        ];
        callbacks(
            &mut m,
            [
                0x004c_efc0,
                0x004c_ef90,
                0x004c_f020,
                0x004c_ef10,
                0x004c_ef50,
            ],
        );
        menus.push(m);
        // Settings: draws `0x4ca2f0`; both control items run `0x4cda60`.
        let mut m = menu(S::Settings, id(S::Options));
        let controls_item = |text, key| {
            let mut item = submenu(text, key, controls, 0x004c_a2f0);
            item.activate = Some(0x004c_da60);
            item
        };
        m.items = vec![
            submenu(0xb, key::GRAPHICS, id(S::Graphics), 0x004c_a2f0),
            submenu(0xc, key::SOUND, id(S::Sound), 0x004c_a2f0),
            controls_item(0xd, key::CONTROLS_1),
            controls_item(0xe, key::CONTROLS_2),
            back(0xa, 0x004c_a2f0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_a260,
                0x004c_a210,
                0x004c_a2d0,
                0x004c_a160,
                0x004c_a1a0,
            ],
        );
        menus.push(m);
        // Graphics.
        let flags = img[0x10];
        let mut m = menu(S::Graphics, id(S::Settings));
        m.items = vec![
            toggle(0x79, key::ARROWS, flags >> 2 & 1 != 0),
            value_item(
                0x7a,
                key::GAME_DISPLAYS,
                3,
                i32::from(flags >> 3 & 3),
                true,
                0,
            ),
            toggle(0x7b, key::SPLIT, false),
            toggle(0x7c, key::WIPES, false),
            submenu(0xe0, 0, id(S::Detail), 0),
            submenu(0xe1, 0, id(S::Advanced), 0),
            back(0xdf, 0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_a520,
                0x004c_a810,
                0x004c_aed0,
                0x004c_a390,
                0x004c_a3f0,
            ],
        );
        menus.push(m);
        // Graphics detail: the builder starts both distance items at
        // `img[0x15]`.
        let detail = i32::from(img[0x15]);
        let mut m = menu(S::Detail, id(S::Graphics));
        m.items = vec![
            toggle(0xe3, key::OBJECT_DETAIL, true),
            value_item(0xe4, key::OBJECT_DISTANCE, 4, detail, false, 0),
            value_item(0xe5, key::TRACK_DISTANCE, 4, detail, false, 0),
            value_item(0xf9, key::CAR_RESOLUTION, 3, i32::from(img[0x18]), false, 0),
            toggle(0xe6, key::WEATHER, true),
            toggle(0xe7, key::CAR_EFFECTS, true),
            back(0xdf, 0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_a6f0,
                0x004c_a900,
                0x004c_aed0,
                0x004c_a390,
                0x004c_a3f0,
            ],
        );
        menus.push(m);
        // Advanced graphics (the device count is set at entry).
        let mut m = menu(S::Advanced, id(S::Graphics));
        m.items = vec![
            value_item(0xe9, key::DEVICE, 1, i32::from(img[0x11]), true, 0),
            value_item(0xea, key::CHIPSET, 8, i32::from(img[0x12]), true, 0),
            value_item(0xc7, key::RESOLUTION, 4, i32::from(img[0x13]), true, 0),
            value_item(0xeb, key::TEXTURES, 2, i32::from(img[0x14]), true, 0),
            value_item(0xec, key::SMOOTHNESS, 2, img[0x1c] as i32, true, 0),
            back(0xdf, 0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_a600,
                0x004c_aaa0,
                0x004c_aed0,
                0x004c_a390,
                0x004c_a3f0,
            ],
        );
        menus.push(m);
        // Sound: three volumes 0..=5 starting at 3, no wrap.
        let mut m = menu(S::Sound, id(S::Settings));
        m.items = vec![
            value_item(0x11, key::EFFECTS_VOLUME, 6, 3, false, 0),
            value_item(0x12, key::SPEECH_VOLUME, 6, 3, false, 0),
            value_item(0x13, key::MUSIC_VOLUME, 6, 3, false, 0),
            back(0xdf, 0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_ac90,
                0x004c_adb0,
                0x004c_aed0,
                0x004c_a390,
                0x004c_a3f0,
            ],
        );
        menus.push(m);
        // Hi-scores: draws `0x4cba60`.
        let mut m = menu(S::HiScores, id(S::Options));
        m.items = vec![
            submenu(
                0x2e,
                key::CHAMPIONSHIP,
                id(S::ChampionshipScores),
                0x004c_ba60,
            ),
            submenu(0x2d, key::BEST_RALLY, id(S::BestRally), 0x004c_ba60),
            submenu(0x2c, key::BEST_STAGE, id(S::BestStage), 0x004c_ba60),
            back(0xa, 0x004c_ba60),
        ];
        callbacks(
            &mut m,
            [
                0x004c_b9e0,
                0x004c_b9b0,
                0x004c_ba40,
                0x004c_b930,
                0x004c_b970,
            ],
        );
        menus.push(m);
        // The three tables: one value item each, whose confirm (`0x4cb920`)
        // returns to the hi-scores.
        for (screen, key, text, count) in [
            (S::BestStage, key::COUNTRY, 0xcd, 8),
            (S::BestRally, key::COUNTRY, 0xcd, 8),
            (S::ChampionshipScores, key::SCORE_DIFFICULTY, 0x66, 3),
        ] {
            let mut m = menu(screen, id(S::HiScores));
            let mut item = value_item(text, key, count, 0, true, 0);
            item.activate = Some(0x004c_b920);
            m.items = vec![item];
            callbacks(
                &mut m,
                [
                    0x004c_a460,
                    0x004c_a420,
                    0x004c_aed0,
                    0x004c_a390,
                    0x004c_a3f0,
                ],
            );
            menus.push(m);
        }
        // Difficulty: draws `0x4ca050`; the three levels are back items
        // with activate `0x4ca0e0`.
        let mut m = menu(S::Difficulty, id(S::Options));
        let level = |text, key| {
            let mut item = back(text, 0x004c_a050);
            item.key = key;
            item.activate = Some(0x004c_a0e0);
            item
        };
        m.items = vec![
            level(0x67, key::NOVICE),
            level(0x68, key::INTERMEDIATE),
            level(0x69, key::EXPERT),
            back(0xa, 0x004c_a050),
        ];
        callbacks(
            &mut m,
            [
                0x004c_9f60,
                0x004c_9f50,
                0x004c_a030,
                0x004c_9ea0,
                0x004c_9ee0,
            ],
        );
        menus.push(m);
        // Modern HUD / Overlay graphics screen (`OptionScreen::Modern`).
        let modern_defaults = ModernSettings::default();
        let mut m = menu(S::Modern, id(S::Graphics));
        m.items = vec![
            toggle(0x79, key::ARROWS, flags >> 2 & 1 != 0),
            value_item(
                0x7a,
                key::GAME_DISPLAYS,
                3,
                i32::from(flags >> 3 & 3),
                true,
                0,
            ),
            toggle(0x7b, key::SPLIT, false),
            toggle(0x7c, key::WIPES, false),
            back(0xdf, 0),
        ];
        callbacks(
            &mut m,
            [
                0x004c_a520,
                0x004c_a810,
                0x004c_aed0,
                0x004c_a390,
                0x004c_a3f0,
            ],
        );
        menus.push(m);
        for m in menus {
            set.add(m);
        }
        Self {
            ids: std::array::from_fn(|i| MenuId(base + i)),
            templates: RowTemplates::default(),
            row_tweens: Vec::new(),
            rows: [(0, 0); 5],
            panel_tween: None,
            panel_x: 0,
            amplitude: 0x8000,
            value_cue_muted: false,
            volume_latch: None,
            resolution_d90128: 0,
            player: 0,
            modern_mode: false,
            modern: modern_defaults,
        }
    }

    /// Enable the modern in-game graphics menus:
    /// - `OptionScreen::Graphics` prioritizes `PRESET` first, followed by core
    ///   display options (`RESOLUTION`, `ASPECT RATIO`, `FRAME RATE`) and
    ///   submenus (`ADVANCED`, `DETAIL`, `HUD`).
    /// - `OptionScreen::Advanced` houses all granular GPU rendering options
    ///   (`VSYNC`, `ANTI-ALIASING`, `UPSCALING`, `TEXTURES`, `DRAW DISTANCE`).
    /// - `OptionScreen::Modern` (`HUD`) houses the classic HUD & camera toggles
    ///   (`ARROWS`, `GAME DISPLAYS`, `SPLIT`, `WIPES`).
    pub fn enable_modern_mode(
        &mut self,
        set: &mut crate::descriptor_menu::MenuSet,
        settings: ModernSettings,
    ) {
        self.modern_mode = true;
        self.modern = settings;
        let modern_id = self.id(OptionScreen::Modern);
        let detail_id = self.id(OptionScreen::Detail);
        let advanced_id = self.id(OptionScreen::Advanced);
        let graphics_id = self.id(OptionScreen::Graphics);

        let graphics = set.menu_mut(graphics_id);
        graphics.items = vec![
            value_item(
                modern_text::LABEL_PRESET,
                key::MODERN_PRESET,
                ModernPreset::COUNT,
                settings.preset.index(),
                true,
                0,
            ),
            value_item(
                0xc7,
                key::RESOLUTION,
                ModernResolution::COUNT,
                settings.resolution.index(),
                true,
                0,
            ),
            value_item(
                modern_text::LABEL_ASPECT,
                key::MODERN_ASPECT,
                AspectRatioMode::COUNT,
                settings.aspect_ratio.index(),
                true,
                0,
            ),
            value_item(
                modern_text::LABEL_FPS,
                key::MODERN_FPS,
                FpsLimit::COUNT,
                settings.fps_limit.index(),
                true,
                0,
            ),
            submenu(0xe1, 0, advanced_id, 0),
            submenu(0xe0, 0, detail_id, 0),
            submenu(
                modern_text::LABEL_ENHANCED,
                key::MODERN_ENHANCED,
                modern_id,
                0,
            ),
            back(0xdf, 0),
        ];
        graphics.selected = 0;

        let advanced = set.menu_mut(advanced_id);
        advanced.items = vec![
            value_item(
                modern_text::LABEL_VSYNC,
                key::SMOOTHNESS,
                2,
                i32::from(settings.vsync),
                true,
                0,
            ),
            value_item(
                modern_text::LABEL_MSAA,
                key::MODERN_MSAA,
                AntiAliasingMode::COUNT,
                settings.msaa.index(),
                true,
                0,
            ),
            value_item(
                modern_text::LABEL_UPSCALING,
                key::MODERN_UPSCALING,
                UpscalingMode::COUNT,
                settings.upscaling.index(),
                true,
                0,
            ),
            value_item(
                0xeb,
                key::TEXTURES,
                TextureFilterMode::COUNT,
                settings.texture_filter.index(),
                true,
                0,
            ),
            value_item(
                modern_text::LABEL_DRAW_DISTANCE,
                key::MODERN_DRAW_DISTANCE,
                DrawDistanceScale::COUNT,
                settings.draw_distance.index(),
                true,
                0,
            ),
            toggle(
                modern_text::LABEL_HD_ART,
                key::MODERN_HD_ART,
                settings.hd_art,
            ),
            back(0xdf, 0),
        ];
        advanced.selected = 0;
    }

    fn sync_modern_graphics_items(menu: &mut Menu, settings: &ModernSettings) {
        set_value(menu, key::MODERN_PRESET, settings.preset.index());
        set_value(menu, key::RESOLUTION, settings.resolution.index());
        set_value(menu, key::MODERN_ASPECT, settings.aspect_ratio.index());
        set_value(menu, key::MODERN_FPS, settings.fps_limit.index());
    }

    fn sync_modern_advanced_items(menu: &mut Menu, settings: &ModernSettings) {
        set_value(menu, key::SMOOTHNESS, i32::from(settings.vsync));
        set_value(menu, key::MODERN_MSAA, settings.msaa.index());
        set_value(menu, key::MODERN_UPSCALING, settings.upscaling.index());
        set_value(menu, key::TEXTURES, settings.texture_filter.index());
        set_value(
            menu,
            key::MODERN_DRAW_DISTANCE,
            settings.draw_distance.index(),
        );
        set_value(menu, key::MODERN_HD_ART, i32::from(settings.hd_art));
    }

    /// The row screens' templates.
    pub fn set_templates(&mut self, templates: RowTemplates) {
        self.templates = templates;
    }

    /// The enter callbacks' presentation: the rows slide in on the arc
    /// (`FUN_004c4410(.., 1)`), or the panel from `-0x3333 - A` to `0x1999`
    /// with the slide cue (`FUN_004ca4c0`, which runs before the layout
    /// writes, so the first panel of the process uses `A = 0x8000`). The
    /// navigation stops until the commit sees the slide end. Returns the
    /// cue slots played.
    pub fn enter_presentation(
        &mut self,
        screen: OptionScreen,
        menu: &mut Menu,
        no_animation: bool,
    ) -> Vec<u32> {
        menu.nav = crate::descriptor_menu::MenuNav::NONE;
        let slide = |from: i32, to: i32, delay: u8| {
            if no_animation {
                ScalarTween::ease_out(to, to, 15, 0)
            } else {
                ScalarTween::ease_out(from, to, 15, i32::from(delay))
            }
        };
        if let Some(channels) = self.templates.of(screen) {
            self.row_tweens = channels
                .iter()
                .map(|c| slide(i32::from(c.angles[0]), i32::from(c.angles[1]), c.delay_in))
                .collect();
            return Vec::new();
        }
        let a = self.amplitude;
        self.panel_tween = Some(slide(-0x3333 - a, PANEL_REST_X, 0));
        self.panel_x = -a;
        self.amplitude = 0xb333;
        if screen == OptionScreen::Sound {
            self.value_cue_muted = true;
        }
        vec![crate::frontend_sounds::SLIDE_CUE]
    }

    /// The commit's presentation: the rows' arc positions (`FUN_004c43b0`)
    /// or the panel's x (`0x4ca420`); the navigation returns once the slide
    /// has ended.
    pub fn commit_presentation(
        &mut self,
        screen: OptionScreen,
        menu: &mut Menu,
        place: &dyn Fn(i32) -> (i32, i32),
    ) {
        let idle = if is_row_screen(screen) {
            let mut idle = true;
            for (k, tween) in self.row_tweens.iter_mut().enumerate() {
                if !tween.is_finished() {
                    let angle = tween.evaluate_and_advance();
                    if let Some(row) = self.rows.get_mut(k) {
                        *row = place(angle);
                    }
                    idle = false;
                }
            }
            idle
        } else if let Some(tween) = self.panel_tween.as_mut()
            && !tween.is_finished()
        {
            self.panel_x = tween.evaluate_and_advance();
            false
        } else {
            true
        };
        if idle {
            menu.nav = crate::descriptor_menu::MenuNav::ALL;
        }
    }

    /// The post callbacks' presentation: the rows slide out (`a1 -> a2`), or
    /// the panel back out to the left (an ease-in) with the slide cue and the
    /// value cue restored. Returns the cue slots played.
    pub fn post_presentation(
        &mut self,
        screen: OptionScreen,
        menu: &mut Menu,
        no_animation: bool,
    ) -> Vec<u32> {
        menu.nav = crate::descriptor_menu::MenuNav::NONE;
        if let Some(channels) = self.templates.of(screen) {
            self.row_tweens = channels
                .iter()
                .map(|c| {
                    let (from, to) = (i32::from(c.angles[1]), i32::from(c.angles[2]));
                    if no_animation {
                        ScalarTween::ease_out(to, to, 15, 0)
                    } else {
                        ScalarTween::ease_out(from, to, 15, i32::from(c.delay_out))
                    }
                })
                .collect();
            return Vec::new();
        }
        self.value_cue_muted = false;
        let to = -0x3333 - self.amplitude;
        self.panel_tween = Some(if no_animation {
            ScalarTween::ease_out(to, to, 15, 0)
        } else {
            ScalarTween::ease_in(PANEL_REST_X, to, 15, 0)
        });
        self.panel_x = PANEL_REST_X;
        vec![crate::frontend_sounds::SLIDE_CUE]
    }

    /// The menu of `screen`.
    pub fn id(&self, screen: OptionScreen) -> MenuId {
        self.ids[screen.index()]
    }

    /// The screen of menu `id`.
    pub fn screen_of(&self, id: MenuId) -> Option<OptionScreen> {
        OptionScreen::ALL.into_iter().find(|&s| self.id(s) == id)
    }

    /// The enter callbacks' record reads (`arg` is the activation's argument:
    /// 0 when entered from the parent).
    pub fn enter(
        &mut self,
        screen: OptionScreen,
        arg: u8,
        menu: &mut Menu,
        img: &Record,
        caps: &DisplayCaps,
    ) {
        let flags = img[0x10];
        match screen {
            // `0x4ca520`.
            OptionScreen::Graphics => {
                if self.modern_mode {
                    Self::sync_modern_graphics_items(menu, &self.modern);
                } else if arg == 0 {
                    set_value(menu, key::ARROWS, bit(flags, 2));
                    set_value(menu, key::GAME_DISPLAYS, i32::from(flags >> 3 & 3));
                    set_value(menu, key::SPLIT, bit(flags, 0));
                    set_value(menu, key::WIPES, bit(flags, 5));
                }
            }
            // `0x4ca6f0`.
            OptionScreen::Detail if arg == 0 => {
                set_value(menu, key::OBJECT_DETAIL, i32::from(img[0x15]));
                set_value(menu, key::TRACK_DISTANCE, i32::from(img[0x16]));
                set_value(menu, key::OBJECT_DISTANCE, i32::from(img[0x17]));
                set_value(menu, key::CAR_RESOLUTION, i32::from(img[0x18]));
                set_value(menu, key::WEATHER, bit(flags, 6));
                set_value(menu, key::CAR_EFFECTS, bit(flags, 7));
            }
            // `0x4ca600`: the values, the device count from the host.
            OptionScreen::Advanced => {
                if self.modern_mode {
                    Self::sync_modern_advanced_items(menu, &self.modern);
                } else {
                    if let Some(i) = menu.find(key::DEVICE) {
                        menu.items[i].value.value_count = caps.devices;
                    }
                    set_value(menu, key::DEVICE, i32::from(img[0x11]));
                    set_value(menu, key::CHIPSET, i32::from(img[0x12]));
                    set_value(menu, key::RESOLUTION, i32::from(img[0x13]));
                    set_value(menu, key::TEXTURES, i32::from(img[0x14]));
                    set_value(
                        menu,
                        key::SMOOTHNESS,
                        i32::from_le_bytes([img[0x1c], img[0x1d], img[0x1e], img[0x1f]]),
                    );
                }
            }
            OptionScreen::Modern => {
                set_value(menu, key::ARROWS, bit(flags, 2));
                set_value(menu, key::GAME_DISPLAYS, i32::from(flags >> 3 & 3));
                set_value(menu, key::SPLIT, bit(flags, 0));
                set_value(menu, key::WIPES, bit(flags, 5));
            }
            // `0x4cac90`: the volumes (the two keys it also loads do not
            // exist in this build).
            OptionScreen::Sound => {
                set_value(menu, key::EFFECTS_VOLUME, i32::from(img[0xd] & 0x7f));
                set_value(menu, key::SPEECH_VOLUME, i32::from(img[0xe] & 0x7f));
                set_value(menu, key::MUSIC_VOLUME, i32::from(img[0xf] & 0x7f));
            }
            // `0x4c9f60`: select the record's difficulty; expert needs the
            // unlock `img[0x2b] & 1`.
            OptionScreen::Difficulty if arg == 0 => {
                let selected = match (img[0x20] >> 3) & 3 {
                    0 => key::NOVICE,
                    2 => key::EXPERT,
                    _ => key::INTERMEDIATE,
                };
                if let Some(i) = menu.find(selected) {
                    menu.selected = i;
                }
                set_enabled(menu, key::EXPERT, img[0x2b] & 1 != 0);
            }
            _ => {}
        }
    }

    /// The commit callbacks' record writes (every step).
    pub fn commit(
        &mut self,
        screen: OptionScreen,
        menu: &mut Menu,
        img: &mut Record,
        caps: &DisplayCaps,
        effects: &mut Vec<OptionsEffect>,
    ) {
        match screen {
            // `0x4ca810`; then `FUN_004ad710(!wipes)` (the host reads the
            // "no animation" flag from `img[0x10]`).
            OptionScreen::Graphics => {
                if self.modern_mode {
                    self.commit_modern_graphics(menu, img, effects);
                } else {
                    Self::commit_hud_flags(menu, img);
                }
            }
            // `0x4ca900`.
            OptionScreen::Detail => {
                let limited = caps.limited_b73c40 || matches!(render_mode(img[0x12]), 2 | 3 | 6);
                img[0x15] = value(menu, key::OBJECT_DETAIL) as u8;
                img[0x16] = value(menu, key::TRACK_DISTANCE) as u8;
                img[0x17] = value(menu, key::OBJECT_DISTANCE) as u8;
                if limited {
                    set_value(menu, key::CAR_RESOLUTION, 0);
                }
                img[0x18] = value(menu, key::CAR_RESOLUTION) as u8;
                img[0x10] = (img[0x10] & !0x40) | ((value(menu, key::WEATHER) as u8 & 1) << 6);
                if limited {
                    set_value(menu, key::CAR_EFFECTS, 0);
                }
                img[0x10] = (img[0x10] & !0x80) | ((value(menu, key::CAR_EFFECTS) as u8 & 1) << 7);
            }
            // `0x4caaa0`.
            OptionScreen::Advanced => self.commit_advanced(menu, img, caps, effects),
            OptionScreen::Modern => Self::commit_hud_flags(menu, img),
            // `0x4cadb0`.
            OptionScreen::Sound => {
                let volumes = [
                    value(menu, key::EFFECTS_VOLUME),
                    value(menu, key::SPEECH_VOLUME),
                    value(menu, key::MUSIC_VOLUME),
                ];
                let latch = self.volume_latch.get_or_insert(volumes);
                effects.push(OptionsEffect::Volumes {
                    effects: volumes[0] * 20,
                    music: volumes[2] * 20,
                });
                if *latch != volumes {
                    *latch = volumes;
                    effects.push(OptionsEffect::PreviewCue);
                }
                for (at, volume) in [0xd, 0xe, 0xf].into_iter().zip(volumes) {
                    img[at] = (img[at] & 0x80) | (volume as u8 & 0x7f);
                }
            }
            _ => {}
        }
    }

    fn commit_hud_flags(menu: &Menu, img: &mut Record) {
        let mut flags = img[0x10];
        flags = (flags & !4) | (u8::from(value(menu, key::ARROWS) != 0) << 2);
        flags = (flags & !1) | u8::from(value(menu, key::SPLIT) != 0);
        flags = (flags & !0x18) | ((value(menu, key::GAME_DISPLAYS) as u8 & 3) << 3);
        flags = (flags & !0x20) | (u8::from(value(menu, key::WIPES) != 0) << 5);
        img[0x10] = flags;
    }

    /// `0x4caaa0`: the display device, the render mode (6 without a 3D
    /// device), the resolution the video memory allows, the texture quality
    /// and the smoothness.
    fn commit_advanced(
        &mut self,
        menu: &mut Menu,
        img: &mut Record,
        caps: &DisplayCaps,
        effects: &mut Vec<OptionsEffect>,
    ) {
        if self.modern_mode {
            let old = self.modern;
            self.modern.vsync = value(menu, key::SMOOTHNESS) != 0;
            self.modern.msaa = AntiAliasingMode::from_index(value(menu, key::MODERN_MSAA));
            self.modern.upscaling = UpscalingMode::from_index(value(menu, key::MODERN_UPSCALING));
            self.modern.texture_filter = TextureFilterMode::from_index(value(menu, key::TEXTURES));
            self.modern.draw_distance =
                DrawDistanceScale::from_index(value(menu, key::MODERN_DRAW_DISTANCE));
            self.modern.hd_art = value(menu, key::MODERN_HD_ART) != 0;
            self.modern.refresh_preset_tag();
            self.modern.apply_to_legacy_record(img, false);
            if self.modern != old {
                effects.push(OptionsEffect::ModernChanged(self.modern));
            }
            return;
        }
        let device = value(menu, key::DEVICE);
        img[0x11] = device as u8;
        if !caps.has_3d {
            set_value(menu, key::CHIPSET, 6);
        }
        img[0x12] = value(menu, key::CHIPSET) as u8;
        if caps.limited_b73c40 || matches!(render_mode(img[0x12]), 2 | 3 | 6) {
            img[0x10] &= !0x80;
        }
        let fits = |res: i32| {
            usize::try_from(res)
                .ok()
                .and_then(|r| caps.resolution_fits.get(r).copied())
                .unwrap_or(false)
        };
        let mut res = value(menu, key::RESOLUTION);
        if !caps.flag_502838 && device == 1 && res == 0 {
            res = if self.resolution_d90128 > 0 {
                if fits(3) { 3 } else { 2 }
            } else {
                1
            };
        }
        if self.resolution_d90128 > res {
            if !fits(res) && res != 0 {
                res -= 1;
            }
        } else if !fits(res) {
            res = 0;
        }
        set_value(menu, key::RESOLUTION, res);
        self.resolution_d90128 = res;
        img[0x13] = res as u8;
        if !fits(0) {
            set_value(menu, key::TEXTURES, 0);
        }
        img[0x14] = value(menu, key::TEXTURES) as u8;
        img[0x1c..0x20].copy_from_slice(&value(menu, key::SMOOTHNESS).to_le_bytes());
    }

    fn commit_modern_graphics(
        &mut self,
        menu: &mut Menu,
        img: &mut Record,
        effects: &mut Vec<OptionsEffect>,
    ) {
        let old = self.modern;
        let menu_preset = ModernPreset::from_index(value(menu, key::MODERN_PRESET));
        if menu_preset != self.modern.preset && menu_preset != ModernPreset::Custom {
            self.modern.apply_preset(menu_preset);
            Self::sync_modern_graphics_items(menu, &self.modern);
        } else {
            self.modern.resolution = ModernResolution::from_index(value(menu, key::RESOLUTION));
            self.modern.aspect_ratio = AspectRatioMode::from_index(value(menu, key::MODERN_ASPECT));
            self.modern.fps_limit = FpsLimit::from_index(value(menu, key::MODERN_FPS));
            self.modern.refresh_preset_tag();
            set_value(menu, key::MODERN_PRESET, self.modern.preset.index());
        }
        self.modern.apply_to_legacy_record(img, false);
        if self.modern != old {
            effects.push(OptionsEffect::ModernChanged(self.modern));
        }
    }

    /// The activate callbacks: `0x4cda60` (the controls' player),
    /// `0x4ca0e0` (the difficulty, which also resets the saved championship
    /// position) and `0x4cb920` (back to the hi-scores, returned).
    pub fn activate(
        &mut self,
        screen: OptionScreen,
        item: &MenuItem,
        img: &mut Record,
    ) -> Option<MenuId> {
        match (screen, item.activate) {
            (OptionScreen::Settings, Some(0x004c_da60)) => {
                self.player = u8::from(item.key != key::CONTROLS_1);
                None
            }
            (OptionScreen::Difficulty, Some(0x004c_a0e0)) => {
                let level = match item.key {
                    key::NOVICE => 0,
                    key::INTERMEDIATE => 1,
                    _ => 2,
                };
                img[0x20] = (img[0x20] & !0x18) | (level << 3);
                img[0x180] &= 3;
                img[0x181] = 0;
                None
            }
            (_, Some(0x004c_b920)) => Some(self.id(OptionScreen::HiScores)),
            _ => None,
        }
    }
}

fn callbacks(menu: &mut Menu, [enter, commit, predraw, post, leave]: [u32; 5]) {
    menu.on_enter = Some(enter);
    menu.on_commit = Some(commit);
    menu.on_predraw = Some(predraw);
    menu.on_post = Some(post);
    menu.on_leave = Some(leave);
}

/// `FUN_00498af0` after `FUN_00498b00(img[0x12])`: the render mode, 6 above
/// 7.
pub const fn render_mode(chipset: u8) -> u8 {
    if chipset > 7 { 6 } else { chipset }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor_menu::MenuSet;

    fn caps() -> DisplayCaps {
        DisplayCaps {
            devices: 2,
            has_3d: true,
            limited_b73c40: false,
            flag_502838: true,
            resolution_fits: [true; 4],
        }
    }

    fn build() -> (MenuSet, OptionsScreens, Box<Record>) {
        let mut set = MenuSet::default();
        set.add(Menu::new(-1, None, None));
        let mut img = Box::new([0u8; LEGACY_CONFIG_SIZE]);
        img[0x10] = 0x14;
        img[0x15] = 2;
        let options = OptionsScreens::build(&mut set, MenuId(0), MenuId(0), &img);
        (set, options, img)
    }

    #[test]
    fn the_tree_links_each_screen_to_its_parent() {
        let (set, options, _) = build();
        let menu = |s| set.menu(options.id(s));
        assert_eq!(menu(OptionScreen::Options).parent, Some(MenuId(0)));
        assert_eq!(
            menu(OptionScreen::Detail).parent,
            Some(options.id(OptionScreen::Graphics))
        );
        assert_eq!(
            menu(OptionScreen::Graphics).items[0].value,
            ValueWidget {
                value: 1,
                value_count: 2,
                wrap: true
            },
            "arrows from img[0x10] bit 2"
        );
        assert_eq!(menu(OptionScreen::Detail).items[1].value.value, 2);
        assert!(menu(OptionScreen::Options).wrap);
        assert_eq!(
            OptionScreen::of(ScreenId::from_legacy(0x00d4_9718)),
            Some(OptionScreen::Difficulty)
        );
    }

    #[test]
    fn graphics_and_detail_write_the_display_flags() {
        let (mut set, mut options, mut img) = build();
        let graphics = options.id(OptionScreen::Graphics);
        let m = set.menu_mut(graphics);
        options.enter(OptionScreen::Graphics, 0, m, &img, &caps());
        set_value(m, key::WIPES, 1);
        set_value(m, key::GAME_DISPLAYS, 2);
        set_value(m, key::ARROWS, 0);
        options.commit(
            OptionScreen::Graphics,
            m,
            &mut img,
            &caps(),
            &mut Vec::new(),
        );
        assert_eq!(img[0x10], 0x20 | (2 << 3));
        let detail = options.id(OptionScreen::Detail);
        let m = set.menu_mut(detail);
        img[0x10] = 0xc0;
        img[0x18] = 2;
        options.enter(OptionScreen::Detail, 0, m, &img, &caps());
        assert_eq!(value(m, key::CAR_EFFECTS), 1);
        // A software render mode drops car effects and car resolution.
        img[0x12] = 6;
        options.commit(OptionScreen::Detail, m, &mut img, &caps(), &mut Vec::new());
        assert_eq!((img[0x18], img[0x10] & 0xc0), (0, 0x40));
        assert_eq!(value(m, key::CAR_RESOLUTION), 0);
    }

    #[test]
    fn advanced_graphics_follows_the_device_rules() {
        let (mut set, mut options, mut img) = build();
        let m = set.menu_mut(options.id(OptionScreen::Advanced));
        img[0x11] = 1;
        let mut caps = caps();
        caps.flag_502838 = false;
        caps.has_3d = false;
        caps.resolution_fits = [true, true, false, false];
        options.enter(OptionScreen::Advanced, 0, m, &img, &caps);
        assert_eq!(m.items[0].value.value_count, 2);
        options.commit(OptionScreen::Advanced, m, &mut img, &caps, &mut Vec::new());
        assert_eq!(img[0x12], 6, "no 3D device");
        assert_eq!(img[0x13], 1, "device 1 at the lowest resolution moves to 1");
        // Moving down past an unsupported resolution steps once.
        options.resolution_d90128 = 3;
        set_value(m, key::RESOLUTION, 2);
        options.commit(OptionScreen::Advanced, m, &mut img, &caps, &mut Vec::new());
        assert_eq!(img[0x13], 1);
        caps.resolution_fits[0] = false;
        set_value(m, key::TEXTURES, 1);
        options.commit(OptionScreen::Advanced, m, &mut img, &caps, &mut Vec::new());
        assert_eq!(img[0x14], 0);
    }

    #[test]
    fn sound_writes_the_volumes_and_cues_a_change() {
        let (mut set, mut options, mut img) = build();
        let m = set.menu_mut(options.id(OptionScreen::Sound));
        img[0xd] = 0x83;
        img[0xe] = 4;
        img[0xf] = 5;
        options.enter(OptionScreen::Sound, 0, m, &img, &caps());
        let mut effects = Vec::new();
        options.commit(OptionScreen::Sound, m, &mut img, &caps(), &mut effects);
        assert_eq!(
            effects,
            vec![OptionsEffect::Volumes {
                effects: 60,
                music: 100
            }]
        );
        set_value(m, key::EFFECTS_VOLUME, 1);
        effects.clear();
        options.commit(OptionScreen::Sound, m, &mut img, &caps(), &mut effects);
        assert!(effects.contains(&OptionsEffect::PreviewCue));
        assert_eq!(img[0xd], 0x81, "bit 7 kept");
    }

    #[test]
    fn difficulty_selects_writes_and_resets_the_championship() {
        let (mut set, mut options, mut img) = build();
        let id = options.id(OptionScreen::Difficulty);
        img[0x20] = 0x10 | 3;
        img[0x180] = 0xff;
        img[0x181] = 0x25;
        let m = set.menu_mut(id);
        options.enter(OptionScreen::Difficulty, 0, m, &img, &caps());
        assert_eq!(m.items[m.selected].key, key::EXPERT);
        assert!(!m.items[2].enabled, "expert locked");
        let item = m.items[0].clone();
        assert_eq!(
            options.activate(OptionScreen::Difficulty, &item, &mut img),
            None
        );
        assert_eq!((img[0x20], img[0x180], img[0x181]), (3, 3, 0));
        // The settings' control items choose the player.
        let settings = set.menu(options.id(OptionScreen::Settings)).clone();
        options.activate(OptionScreen::Settings, &settings.items[3], &mut img);
        assert_eq!(options.player, 1);
        let table = set.menu(options.id(OptionScreen::BestStage)).items[0].clone();
        assert_eq!(
            options.activate(OptionScreen::BestStage, &table, &mut img),
            Some(options.id(OptionScreen::HiScores))
        );
    }

    #[test]
    fn modern_mode_exposes_enhanced_screen_and_updates_modern_settings() {
        use crate::modern_settings::{
            AntiAliasingMode, AspectRatioMode, DrawDistanceScale, FpsLimit, ModernPreset,
            ModernResolution, ModernSettings, TextureFilterMode, UpscalingMode,
        };

        let (mut set, mut options, mut img) = build();
        let initial = ModernSettings::default();
        options.enable_modern_mode(&mut set, initial);
        assert!(options.modern_mode);

        // Graphics menu prioritizes PRESET at row 0, followed by RESOLUTION, ASPECT RATIO, FRAME RATE,
        // and submenus for ADVANCED, DETAIL, and HUD.
        let graphics_id = options.id(OptionScreen::Graphics);
        let graphics_menu = set.menu_mut(graphics_id);
        assert_eq!(graphics_menu.items[0].key, key::MODERN_PRESET);
        assert_eq!(graphics_menu.items[1].key, key::RESOLUTION);
        assert_eq!(graphics_menu.items[2].key, key::MODERN_ASPECT);
        assert_eq!(graphics_menu.items[3].key, key::MODERN_FPS);
        assert!(
            graphics_menu
                .items
                .iter()
                .any(|item| item.key == key::MODERN_ENHANCED),
            "Graphics menu must link to OptionScreen::Modern (HUD)"
        );

        options.enter(OptionScreen::Graphics, 0, graphics_menu, &img, &caps());
        assert_eq!(
            value(graphics_menu, key::MODERN_PRESET),
            ModernPreset::Enhanced1080p.index()
        );
        assert_eq!(
            value(graphics_menu, key::RESOLUTION),
            ModernResolution::Res1920x1080.index()
        );
        assert_eq!(
            value(graphics_menu, key::MODERN_ASPECT),
            AspectRatioMode::Widescreen16x9.index()
        );
        assert_eq!(
            value(graphics_menu, key::MODERN_FPS),
            FpsLimit::Fps120.index()
        );

        // Selecting Ultra4K preset on Graphics immediately updates Resolution on the same screen.
        set_value(
            graphics_menu,
            key::MODERN_PRESET,
            ModernPreset::Ultra4K.index(),
        );
        let mut effects = Vec::new();
        options.commit(
            OptionScreen::Graphics,
            graphics_menu,
            &mut img,
            &caps(),
            &mut effects,
        );
        assert_eq!(options.modern.preset, ModernPreset::Ultra4K);
        assert_eq!(options.modern.resolution, ModernResolution::Res3840x2160);
        assert_eq!(
            value(graphics_menu, key::RESOLUTION),
            ModernResolution::Res3840x2160.index()
        );
        assert!(effects.contains(&OptionsEffect::ModernChanged(options.modern)));

        // Advanced screen in modern mode houses all granular rendering options without legacy DEVICE/CHIPSET.
        let adv_id = options.id(OptionScreen::Advanced);
        let adv = set.menu_mut(adv_id);
        assert!(adv.find(key::DEVICE).is_none());
        assert_eq!(adv.items.len(), 7);
        assert_eq!(adv.items[6].kind, ItemKind::Back);
        options.enter(OptionScreen::Advanced, 0, adv, &img, &caps());
        set_value(adv, key::SMOOTHNESS, 0);
        set_value(adv, key::MODERN_MSAA, AntiAliasingMode::Msaa8x.index());
        set_value(
            adv,
            key::MODERN_UPSCALING,
            UpscalingMode::FsrUltraQuality.index(),
        );
        set_value(
            adv,
            key::TEXTURES,
            TextureFilterMode::Anisotropic16x.index(),
        );
        set_value(
            adv,
            key::MODERN_DRAW_DISTANCE,
            DrawDistanceScale::Ultra3x.index(),
        );
        set_value(adv, key::MODERN_HD_ART, 1);
        effects.clear();
        options.commit(OptionScreen::Advanced, adv, &mut img, &caps(), &mut effects);
        assert_eq!(options.modern.preset, ModernPreset::Custom);
        assert!(!options.modern.vsync);
        assert_eq!(options.modern.msaa, AntiAliasingMode::Msaa8x);
        assert_eq!(options.modern.upscaling, UpscalingMode::FsrUltraQuality);
        assert_eq!(
            options.modern.texture_filter,
            TextureFilterMode::Anisotropic16x
        );
        assert_eq!(options.modern.draw_distance, DrawDistanceScale::Ultra3x);
        assert!(options.modern.hd_art);
        assert!(effects.contains(&OptionsEffect::ModernChanged(options.modern)));

        // Returning to Graphics shows PRESET as Custom.
        let graphics_menu = set.menu_mut(graphics_id);
        options.enter(OptionScreen::Graphics, 1, graphics_menu, &img, &caps());
        assert_eq!(
            value(graphics_menu, key::MODERN_PRESET),
            ModernPreset::Custom.index()
        );

        // HUD screen (`OptionScreen::Modern`) updates HUD/wipes flags in `img[0x10]`.
        let hud_id = options.id(OptionScreen::Modern);
        let hud = set.menu_mut(hud_id);
        options.enter(OptionScreen::Modern, 0, hud, &img, &caps());
        set_value(hud, key::WIPES, 1);
        set_value(hud, key::GAME_DISPLAYS, 2);
        set_value(hud, key::ARROWS, 0);
        options.commit(
            OptionScreen::Modern,
            hud,
            &mut img,
            &caps(),
            &mut Vec::new(),
        );
        assert_eq!(img[0x10] & 0x3d, 0x20 | (2 << 3));
    }
}
