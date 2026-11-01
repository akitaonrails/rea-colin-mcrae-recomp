//! Modern graphics, display, frame-rate, and gamepad settings persisted
//! alongside `cmRally.cfg` in `save/modern.cfg`.
//!
//! While `cmRally.cfg` preserves the fixed `0x1e20`-byte 1998 binary layout
//! (`FUN_004b9e70` / `FUN_004ba5e0`) so save data and legacy records remain
//! 100% compatible with the preservation branch, [`ModernSettings`] stores the
//! modern engine enhancements (high resolutions up to 4K, widescreen aspect
//! ratios, high-FPS sub-tick interpolation targets, extended draw distance,
//! anisotropic texture filtering, MSAA, FSR/CAS upscaling, and modern gamepad
//! defaults + rumble) and synchronizes the relevant legacy bytes on load/commit.

use std::time::Duration;

use crate::config::LEGACY_CONFIG_SIZE;
use crate::display::DisplayExtent;

/// File name for modern settings inside `<user-data>/save/`.
pub const MODERN_SETTINGS_FILE_NAME: &str = "modern.cfg";

/// High-level quick preset for the in-game `ENHANCED` graphics screen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum ModernPreset {
    #[default]
    Enhanced1080p = 0,
    SteamDeck = 1,
    Ultra4K = 2,
    Original1998 = 3,
    Custom = 4,
}

impl ModernPreset {
    pub const COUNT: i32 = 5;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Enhanced1080p,
            1 => Self::SteamDeck,
            2 => Self::Ultra4K,
            3 => Self::Original1998,
            _ => Self::Custom,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Enhanced1080p => b"ENHANCED 1080P",
            Self::SteamDeck => b"STEAM DECK",
            Self::Ultra4K => b"ULTRA 4K",
            Self::Original1998 => b"1998 ORIGINAL",
            Self::Custom => b"CUSTOM",
        }
    }
}

/// Display resolution selectable from `OPTIONS -> GRAPHICS -> ADVANCED`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum ModernResolution {
    Res640x480 = 0,
    Res800x600 = 1,
    Res1280x720 = 2,
    Res1280x800 = 3,
    #[default]
    Res1920x1080 = 4,
    Res2560x1440 = 5,
    Res3840x2160 = 6,
}

impl ModernResolution {
    pub const COUNT: i32 = 7;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Res640x480,
            1 => Self::Res800x600,
            2 => Self::Res1280x720,
            3 => Self::Res1280x800,
            4 => Self::Res1920x1080,
            5 => Self::Res2560x1440,
            _ => Self::Res3840x2160,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Res640x480 => (640, 480),
            Self::Res800x600 => (800, 600),
            Self::Res1280x720 => (1280, 720),
            Self::Res1280x800 => (1280, 800),
            Self::Res1920x1080 => (1920, 1080),
            Self::Res2560x1440 => (2560, 1440),
            Self::Res3840x2160 => (3840, 2160),
        }
    }

    pub const fn extent(self) -> DisplayExtent {
        let (w, h) = self.dimensions();
        DisplayExtent {
            width: w as u16,
            height: h as u16,
        }
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Res640x480 => b"640X480",
            Self::Res800x600 => b"800X600",
            Self::Res1280x720 => b"1280X720",
            Self::Res1280x800 => b"1280X800",
            Self::Res1920x1080 => b"1920X1080",
            Self::Res2560x1440 => b"2560X1440",
            Self::Res3840x2160 => b"3840X2160",
        }
    }

    /// Maps the modern resolution back to the closest 1998 2-bit index (`0..=3`)
    /// in `img[0x13]`.
    pub const fn legacy_index(self) -> u8 {
        match self {
            Self::Res640x480 => 0,
            Self::Res800x600 => 1,
            Self::Res1280x720 | Self::Res1280x800 => 2,
            Self::Res1920x1080 | Self::Res2560x1440 | Self::Res3840x2160 => 3,
        }
    }
}

/// Camera and viewport aspect ratio mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum AspectRatioMode {
    Auto = 0,
    #[default]
    Widescreen16x9 = 1,
    Deck16x10 = 2,
    Ultrawide21x9 = 3,
    Original4x3 = 4,
}

impl AspectRatioMode {
    pub const COUNT: i32 = 5;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Auto,
            1 => Self::Widescreen16x9,
            2 => Self::Deck16x10,
            3 => Self::Ultrawide21x9,
            _ => Self::Original4x3,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Auto => b"AUTO",
            Self::Widescreen16x9 => b"16:9 WIDE",
            Self::Deck16x10 => b"16:10 DECK",
            Self::Ultrawide21x9 => b"21:9 ULTRA",
            Self::Original4x3 => b"4:3 ORIGINAL",
        }
    }

    /// Effective aspect ratio (`width / height`) for 3D projection.
    pub fn effective_aspect(self, window_width: u32, window_height: u32) -> f32 {
        match self {
            Self::Auto => {
                if window_width > 0 && window_height > 0 {
                    (window_width as f32 / window_height as f32).clamp(1.2, 3.6)
                } else {
                    16.0 / 9.0
                }
            }
            Self::Widescreen16x9 => 16.0 / 9.0,
            Self::Deck16x10 => 16.0 / 10.0,
            Self::Ultrawide21x9 => 21.0 / 9.0,
            Self::Original4x3 => 4.0 / 3.0,
        }
    }
}

/// Target presentation frame rate (decoupled from the 25 Hz physics tick via
/// continuous Q16 sub-tick pose interpolation).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum FpsLimit {
    Original25 = 0,
    Fps60 = 1,
    Fps75 = 2,
    #[default]
    Fps120 = 3,
    Fps144 = 4,
    Uncapped = 5,
}

impl FpsLimit {
    pub const COUNT: i32 = 6;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Original25,
            1 => Self::Fps60,
            2 => Self::Fps75,
            3 => Self::Fps120,
            4 => Self::Fps144,
            _ => Self::Uncapped,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Original25 => b"25 FPS",
            Self::Fps60 => b"60 FPS",
            Self::Fps75 => b"75 FPS",
            Self::Fps120 => b"120 FPS",
            Self::Fps144 => b"144 FPS",
            Self::Uncapped => b"UNCAPPED",
        }
    }

    pub const fn target_fps(self) -> Option<u32> {
        match self {
            Self::Original25 => Some(25),
            Self::Fps60 => Some(60),
            Self::Fps75 => Some(75),
            Self::Fps120 => Some(120),
            Self::Fps144 => Some(144),
            Self::Uncapped => None,
        }
    }

    pub const fn frame_interval(self) -> Option<Duration> {
        match self {
            Self::Original25 => Some(Duration::from_nanos(40_000_000)),
            Self::Fps60 => Some(Duration::from_nanos(16_666_667)),
            Self::Fps75 => Some(Duration::from_nanos(13_333_333)),
            Self::Fps120 => Some(Duration::from_nanos(8_333_333)),
            Self::Fps144 => Some(Duration::from_nanos(6_944_444)),
            Self::Uncapped => None,
        }
    }
}

/// Extended draw distance multiplier beyond the original 1998 horizon.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum DrawDistanceScale {
    Original1x = 0,
    Extended1_5x = 1,
    #[default]
    Far2x = 2,
    VeryFar2_5x = 3,
    Ultra3x = 4,
}

impl DrawDistanceScale {
    pub const COUNT: i32 = 5;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Original1x,
            1 => Self::Extended1_5x,
            2 => Self::Far2x,
            3 => Self::VeryFar2_5x,
            _ => Self::Ultra3x,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Original1x => b"1.0X",
            Self::Extended1_5x => b"1.5X",
            Self::Far2x => b"2.0X FAR",
            Self::VeryFar2_5x => b"2.5X V.FAR",
            Self::Ultra3x => b"3.0X ULTRA",
        }
    }

    pub const fn multiplier_f32(self) -> f32 {
        match self {
            Self::Original1x => 1.0,
            Self::Extended1_5x => 1.5,
            Self::Far2x => 2.0,
            Self::VeryFar2_5x => 2.5,
            Self::Ultra3x => 3.0,
        }
    }
}

/// Texture filtering and anisotropic sampling mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum TextureFilterMode {
    Nearest1998 = 0,
    Trilinear = 1,
    Anisotropic4x = 2,
    Anisotropic8x = 3,
    #[default]
    Anisotropic16x = 4,
}

impl TextureFilterMode {
    pub const COUNT: i32 = 5;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Nearest1998,
            1 => Self::Trilinear,
            2 => Self::Anisotropic4x,
            3 => Self::Anisotropic8x,
            _ => Self::Anisotropic16x,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Nearest1998 => b"NEAREST",
            Self::Trilinear => b"TRILINEAR",
            Self::Anisotropic4x => b"ANISO 4X",
            Self::Anisotropic8x => b"ANISO 8X",
            Self::Anisotropic16x => b"ANISO 16X",
        }
    }

    pub const fn anisotropy_clamp(self) -> u16 {
        match self {
            Self::Nearest1998 | Self::Trilinear => 1,
            Self::Anisotropic4x => 4,
            Self::Anisotropic8x => 8,
            Self::Anisotropic16x => 16,
        }
    }

    pub const fn uses_linear_filtering(self) -> bool {
        !matches!(self, Self::Nearest1998)
    }

    /// Filter level encoded into `clip_parameters.y` for `fragment_true_color`:
    /// `0.0` = Nearest, `1.0` = Bilinear/Trilinear, `4.0`..=`16.0` = Anisotropic.
    pub const fn shader_filter_level(self) -> f32 {
        match self {
            Self::Nearest1998 => 0.0,
            Self::Trilinear => 1.0,
            Self::Anisotropic4x => 4.0,
            Self::Anisotropic8x => 8.0,
            Self::Anisotropic16x => 16.0,
        }
    }
}

/// Multi-sample anti-aliasing (MSAA) mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum AntiAliasingMode {
    Off = 0,
    Msaa2x = 1,
    #[default]
    Msaa4x = 2,
    Msaa8x = 3,
}

impl AntiAliasingMode {
    pub const COUNT: i32 = 4;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Off,
            1 => Self::Msaa2x,
            2 => Self::Msaa4x,
            _ => Self::Msaa8x,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Off => b"OFF",
            Self::Msaa2x => b"MSAA 2X",
            Self::Msaa4x => b"MSAA 4X",
            Self::Msaa8x => b"MSAA 8X",
        }
    }

    pub const fn sample_count(self) -> u32 {
        match self {
            Self::Off => 1,
            Self::Msaa2x => 2,
            Self::Msaa4x => 4,
            Self::Msaa8x => 8,
        }
    }
}

/// Spatial upscaling and Contrast-Adaptive Sharpening (FSR / CAS) mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum UpscalingMode {
    Native = 0,
    FsrUltraQuality = 1,
    FsrQuality = 2,
    FsrBalanced = 3,
    #[default]
    CasSharpening = 4,
}

impl UpscalingMode {
    pub const COUNT: i32 = 5;

    pub const fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Native,
            1 => Self::FsrUltraQuality,
            2 => Self::FsrQuality,
            3 => Self::FsrBalanced,
            _ => Self::CasSharpening,
        }
    }

    pub const fn index(self) -> i32 {
        self as i32
    }

    pub const fn label_bytes(self) -> &'static [u8] {
        match self {
            Self::Native => b"NATIVE",
            Self::FsrUltraQuality => b"FSR U.QUAL",
            Self::FsrQuality => b"FSR QUALITY",
            Self::FsrBalanced => b"FSR BALANCED",
            Self::CasSharpening => b"CAS SHARPEN",
        }
    }

    pub const fn render_scale_f32(self) -> f32 {
        match self {
            Self::Native | Self::CasSharpening => 1.0,
            Self::FsrUltraQuality => 0.77,
            Self::FsrQuality => 0.67,
            Self::FsrBalanced => 0.59,
        }
    }

    pub const fn sharpness_f32(self) -> f32 {
        match self {
            Self::Native => 0.0,
            Self::FsrUltraQuality | Self::FsrQuality | Self::FsrBalanced => 0.65,
            Self::CasSharpening => 0.5,
        }
    }
}

/// Complete modern settings state persisted in `save/modern.cfg`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModernSettings {
    pub preset: ModernPreset,
    pub resolution: ModernResolution,
    pub aspect_ratio: AspectRatioMode,
    pub fps_limit: FpsLimit,
    pub vsync: bool,
    pub draw_distance: DrawDistanceScale,
    pub texture_filter: TextureFilterMode,
    pub msaa: AntiAliasingMode,
    pub upscaling: UpscalingMode,
    pub modern_gamepad: bool,
    pub rumble: bool,
}

impl Default for ModernSettings {
    fn default() -> Self {
        Self::from_preset(ModernPreset::Enhanced1080p)
    }
}

impl ModernSettings {
    /// Construct settings for one of the built-in presets.
    pub const fn from_preset(preset: ModernPreset) -> Self {
        match preset {
            ModernPreset::Enhanced1080p | ModernPreset::Custom => Self {
                preset,
                resolution: ModernResolution::Res1920x1080,
                aspect_ratio: AspectRatioMode::Widescreen16x9,
                fps_limit: FpsLimit::Fps120,
                vsync: true,
                draw_distance: DrawDistanceScale::Far2x,
                texture_filter: TextureFilterMode::Anisotropic16x,
                msaa: AntiAliasingMode::Msaa4x,
                upscaling: UpscalingMode::CasSharpening,
                modern_gamepad: true,
                rumble: true,
            },
            ModernPreset::SteamDeck => Self {
                preset: ModernPreset::SteamDeck,
                resolution: ModernResolution::Res1280x800,
                aspect_ratio: AspectRatioMode::Deck16x10,
                fps_limit: FpsLimit::Fps60,
                vsync: true,
                draw_distance: DrawDistanceScale::Far2x,
                texture_filter: TextureFilterMode::Anisotropic8x,
                msaa: AntiAliasingMode::Msaa2x,
                upscaling: UpscalingMode::CasSharpening,
                modern_gamepad: true,
                rumble: true,
            },
            ModernPreset::Ultra4K => Self {
                preset: ModernPreset::Ultra4K,
                resolution: ModernResolution::Res3840x2160,
                aspect_ratio: AspectRatioMode::Widescreen16x9,
                fps_limit: FpsLimit::Fps144,
                vsync: true,
                draw_distance: DrawDistanceScale::Ultra3x,
                texture_filter: TextureFilterMode::Anisotropic16x,
                msaa: AntiAliasingMode::Msaa4x,
                upscaling: UpscalingMode::CasSharpening,
                modern_gamepad: true,
                rumble: true,
            },
            ModernPreset::Original1998 => Self {
                preset: ModernPreset::Original1998,
                resolution: ModernResolution::Res640x480,
                aspect_ratio: AspectRatioMode::Original4x3,
                fps_limit: FpsLimit::Original25,
                vsync: true,
                draw_distance: DrawDistanceScale::Original1x,
                texture_filter: TextureFilterMode::Nearest1998,
                msaa: AntiAliasingMode::Off,
                upscaling: UpscalingMode::Native,
                modern_gamepad: true,
                rumble: false,
            },
        }
    }

    /// Apply a preset in place (leaving `Custom` untouched).
    pub fn apply_preset(&mut self, preset: ModernPreset) {
        if preset == ModernPreset::Custom {
            self.preset = ModernPreset::Custom;
            return;
        }
        let modern_gamepad = self.modern_gamepad;
        *self = Self::from_preset(preset);
        self.modern_gamepad = modern_gamepad;
    }

    /// Recompute whether the current fields match a known preset or `Custom`.
    pub fn refresh_preset_tag(&mut self) {
        for candidate in [
            ModernPreset::Enhanced1080p,
            ModernPreset::SteamDeck,
            ModernPreset::Ultra4K,
            ModernPreset::Original1998,
        ] {
            let mut reference = Self::from_preset(candidate);
            reference.modern_gamepad = self.modern_gamepad;
            reference.rumble = self.rumble;
            let mut current = *self;
            current.preset = candidate;
            if current == reference {
                self.preset = candidate;
                return;
            }
        }
        self.preset = ModernPreset::Custom;
    }

    /// Internal 3D rendering extent (`resolution * upscaling.render_scale_f32()`),
    /// clamped to `640..=3840` x `480..=2160` and aligned to even dimensions.
    #[must_use]
    pub fn internal_render_extent(&self) -> DisplayExtent {
        let (w, h) = self.resolution.dimensions();
        let scale = self.upscaling.render_scale_f32();
        let rw = (((w as f32 * scale).round() as u32).clamp(640, 3840) & !1).max(640);
        let rh = (((h as f32 * scale).round() as u32).clamp(480, 2160) & !1).max(480);
        DisplayExtent {
            width: rw as u16,
            height: rh as u16,
        }
    }

    /// Synchronize the legacy `0x1e20`-byte `cmRally.cfg` record so the
    /// built-in 1998 graphics detail and Player 1 gamepad defaults match
    /// modern expectations on startup.
    pub fn apply_to_legacy_record(&self, img: &mut [u8; LEGACY_CONFIG_SIZE], init_gamepad: bool) {
        // Sync legacy resolution index (`0..=3`) and high texture quality.
        img[0x13] = self.resolution.legacy_index();
        img[0x14] = u8::from(self.texture_filter.uses_linear_filtering());
        let vsync_i32 = i32::from(self.vsync);
        img[0x1c..0x20].copy_from_slice(&vsync_i32.to_le_bytes());
        if self.preset != ModernPreset::Original1998 {
            // Maximum object detail (1), track distance (3), object distance (3),
            // and high car resolution (2), plus weather (bit 6) and car effects (bit 7).
            img[0x15] = 1;
            img[0x16] = 3;
            img[0x17] = 3;
            img[0x18] = 2;
            img[0x10] |= 0xc0;
        }
        if init_gamepad && self.modern_gamepad {
            Self::apply_modern_gamepad_to_player(img, 0, self.rumble);
        }
    }

    /// Configure `player` (`0` = Player 1) in `cmRally.cfg` to default to
    /// `GAMEPAD 1` (`device = 2`), `MODERN GAMEPAD` (`device_type = 2`,
    /// analogue controller), with analogue Steering + Acceleration + Brake
    /// (`bits 0..=2 = 0b111`) and Rumble (`bit 3`), plus modern action button
    /// masks (`DEFAULT_MASKS`).
    pub fn apply_modern_gamepad_to_player(
        img: &mut [u8; LEGACY_CONFIG_SIZE],
        player: u8,
        rumble: bool,
    ) {
        let p = 0x44 + 0x9c * usize::from(player);
        if p + 0x54 > img.len() {
            return;
        }
        // Analogue bits: bit 2 = steering, bit 1 = accel, bit 0 = brake, bit 3 = force feedback/rumble.
        let ff_bit = u8::from(rumble) << 3;
        img[p] = (img[p] & !0x0f) | 0x07 | ff_bit;
        // Device 2 = Gamepad 1 (0 and 1 are Keyboard 1 and Keyboard 2).
        let device: u16 = 2 + u16::from(player);
        img[p + 2..p + 4].copy_from_slice(&device.to_le_bytes());
        // Device type 2 = Analogue Gamepad.
        let device_type: i32 = 2;
        img[p + 4..p + 8].copy_from_slice(&device_type.to_le_bytes());
        for (slot, mask) in crate::frontend_controls::DEFAULT_MASKS
            .into_iter()
            .enumerate()
        {
            img[p + 0x20 + 2 * slot..p + 0x22 + 2 * slot].copy_from_slice(&mask.to_le_bytes());
        }
        let profile = 0x1830 + usize::from(device) * 0x54;
        if profile + 0x54 <= img.len() {
            img.copy_within(p..p + 0x54, profile);
        }
    }

    /// Serialize to a human-readable key-value configuration file.
    pub fn to_config_string(&self) -> String {
        format!(
            "# Colin McRae Rally (1998) — Modern Branch Settings\n\
             preset={}\n\
             resolution={}\n\
             aspect_ratio={}\n\
             fps_limit={}\n\
             vsync={}\n\
             draw_distance={}\n\
             texture_filter={}\n\
             msaa={}\n\
             upscaling={}\n\
             modern_gamepad={}\n\
             rumble={}\n",
            self.preset.index(),
            self.resolution.index(),
            self.aspect_ratio.index(),
            self.fps_limit.index(),
            u8::from(self.vsync),
            self.draw_distance.index(),
            self.texture_filter.index(),
            self.msaa.index(),
            self.upscaling.index(),
            u8::from(self.modern_gamepad),
            u8::from(self.rumble),
        )
    }

    /// Parse from the key-value configuration file, falling back to defaults
    /// for any missing or unknown keys.
    pub fn from_config_str(input: &str) -> Self {
        let mut settings = Self::default();
        for raw_line in input.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let Ok(num) = value.trim().parse::<i32>() else {
                continue;
            };
            match key {
                "preset" => settings.preset = ModernPreset::from_index(num),
                "resolution" => settings.resolution = ModernResolution::from_index(num),
                "aspect_ratio" => settings.aspect_ratio = AspectRatioMode::from_index(num),
                "fps_limit" => settings.fps_limit = FpsLimit::from_index(num),
                "vsync" => settings.vsync = num != 0,
                "draw_distance" => settings.draw_distance = DrawDistanceScale::from_index(num),
                "texture_filter" => settings.texture_filter = TextureFilterMode::from_index(num),
                "msaa" => settings.msaa = AntiAliasingMode::from_index(num),
                "upscaling" => settings.upscaling = UpscalingMode::from_index(num),
                "modern_gamepad" => settings.modern_gamepad = num != 0,
                "rumble" => settings.rumble = num != 0,
                _ => {}
            }
        }
        settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serialization_preserves_every_field() {
        let mut s = ModernSettings::from_preset(ModernPreset::Ultra4K);
        s.fps_limit = FpsLimit::Fps120;
        s.refresh_preset_tag();
        assert_eq!(s.preset, ModernPreset::Custom);
        let text = s.to_config_string();
        let parsed = ModernSettings::from_config_str(&text);
        assert_eq!(parsed, s);
    }

    #[test]
    fn presets_populate_expected_resolutions_and_filters() {
        let deck = ModernSettings::from_preset(ModernPreset::SteamDeck);
        assert_eq!(deck.resolution.dimensions(), (1280, 800));
        assert_eq!(deck.aspect_ratio, AspectRatioMode::Deck16x10);
        assert_eq!(deck.fps_limit, FpsLimit::Fps60);

        let orig = ModernSettings::from_preset(ModernPreset::Original1998);
        assert_eq!(orig.resolution.dimensions(), (640, 480));
        assert_eq!(orig.aspect_ratio, AspectRatioMode::Original4x3);
        assert_eq!(orig.fps_limit, FpsLimit::Original25);
        assert!(!orig.texture_filter.uses_linear_filtering());
        assert_eq!(orig.texture_filter.shader_filter_level(), 0.0);

        let ultra = ModernSettings::from_preset(ModernPreset::Ultra4K);
        assert_eq!(ultra.resolution.dimensions(), (3840, 2160));
        assert_eq!(
            ultra.internal_render_extent(),
            crate::display::DisplayExtent {
                width: 3840,
                height: 2160
            }
        );
        assert_eq!(ultra.texture_filter.shader_filter_level(), 16.0);
        assert_eq!(ultra.msaa.sample_count(), 4);
    }

    #[test]
    fn apply_to_legacy_record_sets_high_detail_and_modern_gamepad() {
        let s = ModernSettings::default();
        let mut img = [0u8; LEGACY_CONFIG_SIZE];
        s.apply_to_legacy_record(&mut img, true);
        assert_eq!(img[0x13], 3);
        assert_eq!(img[0x14], 1);
        assert_eq!((img[0x15], img[0x16], img[0x17], img[0x18]), (1, 3, 3, 2));
        assert_eq!(img[0x44] & 0x0f, 0x0f);
        assert_eq!(u16::from_le_bytes([img[0x46], img[0x47]]), 2);
        assert_eq!(
            i32::from_le_bytes([img[0x48], img[0x49], img[0x4a], img[0x4b]]),
            2
        );
    }
}
