//! Weather and environmental-effect behaviour (item E10).
//!
//! # Provenance
//!
//! Weather is not chosen from a top-level enum at the particle layer: it is
//! driven by the stage's country/surface set. The config byte
//! `DAT_00d3ca80 + 0x168` low nibble selects a country/surface index (0..=8),
//! and the surface-material ids laid down on the stage are run through
//! capability tables (`FUN_00424a40`) that accumulate per-effect enable
//! counters (snow, dust, spray, grass, leaves, gravel, ...) which gate the
//! particle texture loads. A scalar weather-type global `DAT_00592ef8`
//! (getter `FUN_0046b220`; `3`/`4` = rain/wet) plus an intensity drive the
//! sky/fog and the full-screen precipitation overlay; the lighting global
//! `DAT_00592f00` (getter `FUN_0046b240`; `8` = night) disables the shadows
//! and tyre marks and darkens the particles.
//!
//! The wheel particles and tyre marks are [`crate::car_particles`]; the car
//! shadow is [`crate::car_shadow`]. Fog is software distance-haze
//! (color/density/far setters), the engine-level analog of the C3D type-2
//! per-vertex fog ([`crate::c3d_type2_fog`]) - there is no DirectX fog
//! render-state path.
//!
//! ## What is reconstructed vs. left as game data
//!
//! This module reconstructs the state gates, the effect-enable aggregation
//! (capability tables supplied by the caller), the named tuning-constant
//! defaults and the fog constants. The per-surface capability tables
//! (`0x4ec0f8..0x4ec788`) and the exact weather/intensity -> fog selection
//! are game data and are not vendored.
//!
//! See evidence `rea.official-eu.weather-effects`.

/// Config byte offset selecting the country/surface index (`& 0xf`).
pub const COUNTRY_CONFIG_OFFSET: usize = 0x168;

/// The country index forced for the special/bonus stage (`DAT_00d3ca88 != 0`).
pub const SPECIAL_STAGE_COUNTRY: u8 = 7;

/// The country/surface index for a stage: the low nibble of the config byte,
/// or [`SPECIAL_STAGE_COUNTRY`] when the special-stage flag is set.
pub fn country_index(config_byte: u8, special_stage: bool) -> u8 {
    if special_stage {
        SPECIAL_STAGE_COUNTRY
    } else {
        config_byte & 0x0f
    }
}

/// Whether the weather type is a rain/wet type (`3` or `4`), which arms the
/// thunder ambience (H03), the rain/wet zone audio, and the fog/overlay.
pub fn is_wet_weather(weather_type: u32) -> bool {
    weather_type == 3 || weather_type == 4
}

/// Whether the stage's lighting (`FUN_0046b240`) is night (`8`): shadows
/// off, the particle alpha floored, no fog.
pub fn is_night(lighting: u32) -> bool {
    lighting == crate::stage_weather::NIGHT_LIGHTING
}

/// Accumulate one effect's enable counter over the surface ids present on a
/// stage, reproducing `FUN_00424a40`: for each distinct present surface id, add
/// that id's capability byte for the effect. A positive result enables the
/// effect (e.g. snow is enabled iff its counter is `> 0`). The capability table
/// is game data supplied by the caller.
pub fn accumulate_effect_enable(present_surface_ids: &[usize], capability: &[u8]) -> u32 {
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u32;
    for &id in present_surface_ids {
        if seen.insert(id)
            && let Some(&byte) = capability.get(id)
        {
            total += byte as u32;
        }
    }
    total
}

pub const SPLASH_EFFECT_COUNTER: usize = 0;
pub const DUST_EFFECT_COUNTER: usize = 2;
pub const SNOW_EFFECT_COUNTER: usize = 5;

/// Weather gates whose global counter identities are statically recovered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageWeatherEffects {
    pub splash: bool,
    pub dust: bool,
    pub snow: bool,
}

impl StageWeatherEffects {
    #[must_use]
    pub fn from_counters(counters: &[u32; crate::drivetrain::SURFACE_CAPABILITY_COUNT]) -> Self {
        Self {
            splash: counters[SPLASH_EFFECT_COUNTER] > 0,
            dust: counters[DUST_EFFECT_COUNTER] > 0,
            snow: counters[SNOW_EFFECT_COUNTER] > 0,
        }
    }
}

/// Named defaults from the particle/track tuning block (`0x4ebe00`, stride
/// `0x28`, name + value; a text tuning file overrides these at load). These are
/// engine behavior constants, encoded like the drivetrain/grip constants.
pub mod tuning {
    use crate::fixed_point::Fixed16;

    /// `Initial_dust_size` (Q16.16 ~0.2).
    pub const INITIAL_DUST_SIZE: Fixed16 = Fixed16::from_raw(0x3333);
    /// `Initial_follow_size` (Q16.16 ~0.2), the spawn size addend.
    pub const INITIAL_FOLLOW_SIZE: Fixed16 = Fixed16::from_raw(0x3333);
    /// `Follow_size_growth` (Q16.16), per-frame size growth.
    pub const FOLLOW_SIZE_GROWTH: Fixed16 = Fixed16::from_raw(0x51e);
    /// `Max_dust_size` (2.0 Q16.16), used directly by the native clamp.
    pub const MAX_DUST_SIZE: Fixed16 = Fixed16::from_raw(0x2_0000);
    /// `Draw_dirt_my_viewport`: particle budget for the own-car viewport.
    pub const DRAW_DIRT_MY_VIEWPORT: u32 = 100;
    /// `Draw_dirt_other_viewport`: particle budget for the other viewport.
    pub const DRAW_DIRT_OTHER_VIEWPORT: u32 = 30;
    /// `Gas_alpha_delta`: per-frame alpha decrement for the gas/dust class.
    pub const GAS_ALPHA_DELTA: u32 = 3;
    /// `Smoke_alpha_delta`.
    pub const SMOKE_ALPHA_DELTA: u32 = 2;
    /// `Dry_snow_alpha_delta`.
    pub const DRY_SNOW_ALPHA_DELTA: u32 = 2;
    /// `Loose_sand_alpha_delta`.
    pub const LOOSE_SAND_ALPHA_DELTA: u32 = 3;
    /// `Spray_wetness`.
    pub const SPRAY_WETNESS: u32 = 8;
    /// `Splash_wetness`.
    pub const SPLASH_WETNESS: u32 = 0x28;
    /// `Track_Decay_Life`: tyre-mark lifetime in frames.
    pub const TRACK_DECAY_LIFE: u32 = 0x4d8;
    /// `Track_Decay_RATE`: per-frame tyre-mark decay step.
    pub const TRACK_DECAY_RATE: u32 = 0x7c;
}

/// Sky / horizon / fog setup (`FUN_0048b960`, `FUN_00480d80`).
pub mod sky {
    use crate::fixed_point::Fixed16;

    /// Config bit (`DAT_00d3ca80 + 0x25 & 0x80`) that suppresses the `.hzn`
    /// horizon model when set.
    pub const HORIZON_SKIP_BIT: u8 = 0x80;

    /// The recovered fog density values (f32), rising with weather intensity.
    pub const FOG_DENSITIES: [f32; 4] = [0.2, 0.4, 0.6, 0.8];

    /// The recovered fog far-distance values (Q16.16): 20, 40, 60, 70, 80.
    pub const FOG_FAR_DISTANCES: [Fixed16; 5] = [
        Fixed16::from_raw(0x14_0000),
        Fixed16::from_raw(0x28_0000),
        Fixed16::from_raw(0x3c_0000),
        Fixed16::from_raw(0x46_0000),
        Fixed16::from_raw(0x50_0000),
    ];

    /// The recovered packed fog colors (ARGB): grey, bluish, and white.
    pub const FOG_COLOR_GREY: u32 = 0x7864_6464;
    /// Bluish fog color.
    pub const FOG_COLOR_BLUISH: u32 = 0x7864_492a;
    /// White fog color.
    pub const FOG_COLOR_WHITE: u32 = 0xffff_ffff;

    /// Complete inputs to the software fog setters.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct FogSettings {
        pub color: u32,
        pub density: f32,
        pub far: Fixed16,
        pub source_base: u32,
    }

    /// Select fog from weather type, intensity, country and the lighting.
    /// Weather 5 always uses density 0.8/far 70; weather 3 uses intensity rows
    /// 0..2, weather 4 rows 1..3. Other cases or night disable fog.
    #[must_use]
    pub fn fog_settings(
        weather_type: u32,
        intensity: u32,
        country: u8,
        night: bool,
    ) -> Option<FogSettings> {
        let color = match country {
            3 | 6 => FOG_COLOR_BLUISH,
            2 | 4 if weather_type == 4 => FOG_COLOR_WHITE,
            _ if weather_type == 4 => FOG_COLOR_WHITE,
            _ => FOG_COLOR_GREY,
        };
        if weather_type == 5 {
            return Some(FogSettings {
                color,
                density: FOG_DENSITIES[3],
                far: FOG_FAR_DISTANCES[3],
                source_base: 0x50_0000,
            });
        }
        if night || !(1..=3).contains(&intensity) {
            return None;
        }
        let row = usize::try_from(intensity - 1).ok()?;
        match weather_type {
            3 => Some(FogSettings {
                color,
                density: FOG_DENSITIES[row],
                far: FOG_FAR_DISTANCES[row],
                source_base: 0x8c_0000,
            }),
            4 => Some(FogSettings {
                color,
                density: FOG_DENSITIES[row + 1],
                far: FOG_FAR_DISTANCES[row + 1],
                source_base: 0x50_0000,
            }),
            _ => None,
        }
    }

    /// The full-screen precipitation overlay tint for wet weather
    /// (`IMAGES\SNOW.BMP`), RGBA `(100,100,100,255)`.
    pub const PRECIP_OVERLAY_RGBA: (u8, u8, u8, u8) = (100, 100, 100, 255);

    /// Whether the `.hzn` horizon model is loaded for a given config byte (it is
    /// suppressed when [`HORIZON_SKIP_BIT`] is set).
    pub fn loads_horizon(config_byte: u8) -> bool {
        config_byte & HORIZON_SKIP_BIT == 0
    }
}

/// Precipitation particle pool capacity for single-player (`0x48bca9`: `0xc8`).
pub const PRECIPITATION_POOL_SIZE: usize = 200;
/// Spawn box half-extent around camera anchor in Q16.16 meters (`0x48bd9e`: `6.0`).
pub const PRECIPITATION_HALF_EXTENT_Q16: i32 = 0x6_0000;
/// Full wrap span of the precipitation box in Q16.16 meters (`12.0`).
pub const PRECIPITATION_BOX_SPAN_Q16: i32 = PRECIPITATION_HALF_EXTENT_Q16 * 2;
/// Forward offset of the precipitation box center from the camera (`0x48bd35`: `10.0`).
pub const PRECIPITATION_FORWARD_OFFSET_Q16: i32 = 0xa_0000;
/// Snowflake billboard half-size in Q16.16 meters (`0x48bee3`: `0x0ccc` ≈ 0.05 m).
pub const SNOWFLAKE_HALF_SIZE_Q16: i32 = 0x0ccc;

/// Active precipitation type (`DAT_0082a374`, `FUN_0048b960` / `FUN_0048c620`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrecipitationKind {
    None,
    /// `weather_type == 3`: falling rain streaks (`0x48cd68..0x48ce13`).
    Rain,
    /// `weather_type == 4`: falling `IMAGES\SNOW.BMP` billboards (`0x48ce16..0x48cea9`).
    Snow,
}

impl PrecipitationKind {
    #[must_use]
    pub const fn from_weather_type(weather_type: u32) -> Self {
        match weather_type {
            3 => Self::Rain,
            4 => Self::Snow,
            _ => Self::None,
        }
    }
}

/// One camera-facing rain streak or snowflake world-space quad ready for the
/// stage sprite renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrecipitationQuad {
    /// Four world-space corners in meters `[tl, tr, br, bl]`.
    pub corners: [[f32; 3]; 4],
    /// Opacity `0..=255`.
    pub alpha: u8,
    /// Uniform greyscale tint `0..=255` (`0x64` = 100 for rain, `0xff` for snow).
    pub grey: u8,
    /// `true` for `IMAGES\SNOW.BMP`, `false` for rain streak.
    pub is_snow: bool,
}

/// Live 3D precipitation particle system reconstructing `FUN_0048b960`,
/// `FUN_0048bc00`, `FUN_0048bc60`, `FUN_0048c080`, `FUN_0048c230`, and
/// `FUN_0048c620`.
#[derive(Clone, Debug)]
pub struct PrecipitationSystem {
    kind: PrecipitationKind,
    detail: u32,
    night: bool,
    base_count: usize,
    active_count: usize,
    target_count: usize,
    intensity_timer: i32,
    wind_speed_q16: i32,
    wind_target_speed_q16: i32,
    wind_angle: i16,
    wind_target_angle: i16,
    wind_timer: i32,
    gravity_y_q16: i32,
    velocity_q16: [i32; 3],
    streak_delta_q16: [i32; 3],
    prev_camera_q16: [i32; 3],
    initialized: bool,
    positions_q16: [[i32; 3]; PRECIPITATION_POOL_SIZE],
    alphas: [u8; PRECIPITATION_POOL_SIZE],
}

impl PrecipitationSystem {
    /// Create a precipitation system for the stage's weather (`FUN_0048b960` +
    /// `FUN_0048bc60`).
    #[must_use]
    pub fn new(weather_type: u32, detail: u32, night: bool) -> Self {
        let kind = PrecipitationKind::from_weather_type(weather_type);
        // `0x48bc6d..0x48bc87`: detail 3 -> 200, detail 2 -> 100, detail 1 -> 50.
        let base_count = match kind {
            PrecipitationKind::None => 0,
            PrecipitationKind::Rain | PrecipitationKind::Snow => match detail {
                3.. => 200,
                2 => 100,
                _ => 50,
            },
        };
        // `0x48bf0c..0x48bf6b`: gravity fall speed per 25 Hz tick in Q16.16.
        let gravity_y_q16 = match kind {
            PrecipitationKind::Snow => -0x2800, // ~-0.16 m/tick (~4 m/s)
            PrecipitationKind::Rain => -0xb000, // ~-0.69 m/tick (~17 m/s)
            PrecipitationKind::None => 0,
        };
        Self {
            kind,
            detail,
            night,
            base_count,
            active_count: base_count,
            target_count: base_count,
            intensity_timer: 200,
            wind_speed_q16: 0,
            wind_target_speed_q16: 0,
            wind_angle: 0,
            wind_target_angle: 0,
            wind_timer: 0,
            gravity_y_q16,
            velocity_q16: [0, gravity_y_q16, 0],
            streak_delta_q16: [0, gravity_y_q16, 0],
            prev_camera_q16: [0; 3],
            initialized: false,
            positions_q16: [[0; 3]; PRECIPITATION_POOL_SIZE],
            alphas: [0xfe; PRECIPITATION_POOL_SIZE],
        }
    }

    /// Active precipitation kind.
    #[must_use]
    pub const fn kind(&self) -> PrecipitationKind {
        self.kind
    }

    /// Current active particle count (`0..=200`).
    #[must_use]
    pub const fn active_count(&self) -> usize {
        self.active_count
    }

    fn box_center(camera_pos: [i32; 3], camera_fwd: [i32; 3]) -> [i32; 3] {
        let mut center = [0i32; 3];
        for axis in 0..3 {
            let offset = ((i64::from(camera_fwd[axis])
                * i64::from(PRECIPITATION_FORWARD_OFFSET_Q16))
                >> 16) as i32;
            center[axis] = camera_pos[axis].wrapping_add(offset);
        }
        center
    }

    /// Advance one 25 Hz tick (`FUN_0048c080` + `FUN_0048c230` + `FUN_0048c620`).
    pub fn tick(
        &mut self,
        camera_pos: [i32; 3],
        camera_fwd: [i32; 3],
        rng: &mut crate::game_rng::GameLcg,
        trig: &[i32; crate::fixed_point::TRIG_TABLE_LEN],
    ) {
        if self.kind == PrecipitationKind::None {
            return;
        }
        let center = Self::box_center(camera_pos, camera_fwd);
        if !self.initialized {
            // `FUN_0048bc60`: scatter all 200 particles uniformly in the 12m box.
            for pos in &mut self.positions_q16 {
                for axis in 0..3 {
                    let r = i64::from(rng.next_raw() & 0x7fff);
                    let span = ((r * i64::from(PRECIPITATION_BOX_SPAN_Q16)) / 0x7fff) as i32;
                    pos[axis] = center[axis]
                        .wrapping_sub(PRECIPITATION_HALF_EXTENT_Q16)
                        .wrapping_add(span);
                }
            }
            self.prev_camera_q16 = camera_pos;
            self.initialized = true;
        }

        // `FUN_0048c080`: periodic intensity variation (every 200..1000 ticks).
        if self.intensity_timer <= 0 {
            let r = i64::from(rng.next_raw() & 0x7fff);
            let next_timer = ((r * 1000) / 0x7fff).max(200) as i32;
            self.intensity_timer = next_timer;
            if self.detail <= 1 && (rng.next_raw() & 0x7fff) < 0x1999 {
                // Light weather 20% lull (`0x48c0f9`: `< 0x3333` in Q16.16).
                self.target_count = 0;
                self.intensity_timer = (next_timer * 2).max(1000);
            } else {
                let delta = (((i64::from(rng.next_raw() & 0x7fff)) * 25) / 0x7fff) as i32;
                let signed = if (rng.next_raw() & 0x7fff) > 0x4000 {
                    delta
                } else {
                    -delta
                };
                self.target_count = (self.base_count as i32 + signed)
                    .clamp(5, PRECIPITATION_POOL_SIZE as i32)
                    as usize;
            }
        } else {
            self.intensity_timer -= 1;
        }
        if self.active_count < self.target_count {
            self.active_count += 1;
        } else if self.active_count > self.target_count {
            self.active_count -= 1;
        }

        // `FUN_0048c230`: wind speed/angle slew (`0x147` speed step, `±11` angle step).
        if self.wind_timer <= 0 {
            self.wind_timer = 150 + ((rng.next_raw() & 0x7fff) as i32 % 250);
            let max_wind = match self.kind {
                PrecipitationKind::Snow => 0x1800,
                PrecipitationKind::Rain => 0x2000,
                PrecipitationKind::None => 0,
            };
            self.wind_target_speed_q16 =
                ((i64::from(rng.next_raw() & 0x7fff) * i64::from(max_wind)) / 0x7fff) as i32;
            self.wind_target_angle = (rng.next_raw() & 0x0fff) as i16;
        } else {
            self.wind_timer -= 1;
        }
        let speed_diff = self.wind_target_speed_q16 - self.wind_speed_q16;
        if speed_diff.abs() < 0x147 {
            self.wind_speed_q16 = self.wind_target_speed_q16;
        } else if speed_diff > 0 {
            self.wind_speed_q16 += 0x147;
        } else {
            self.wind_speed_q16 -= 0x147;
        }
        let angle_diff = self.wind_target_angle.wrapping_sub(self.wind_angle);
        if angle_diff.abs() < 11 {
            self.wind_angle = self.wind_target_angle;
        } else if angle_diff > 0 {
            self.wind_angle = self.wind_angle.wrapping_add(11) & 0x0fff;
        } else {
            self.wind_angle = self.wind_angle.wrapping_sub(11) & 0x0fff;
        }
        let sin_a = trig[(self.wind_angle as usize) & 0x0fff];
        let cos_a = trig[((self.wind_angle as usize) + 0x400) & 0x0fff];
        let wx = ((i64::from(cos_a) * i64::from(self.wind_speed_q16)) >> 16) as i32;
        let wz = ((i64::from(sin_a) * i64::from(self.wind_speed_q16)) >> 16) as i32;
        self.velocity_q16 = [wx, self.gravity_y_q16, wz];

        // Rain streak vector (`0x48c7e3..0x48c7f4`): particle velocity minus camera motion.
        let cam_dx = camera_pos[0].wrapping_sub(self.prev_camera_q16[0]);
        let cam_dy = camera_pos[1].wrapping_sub(self.prev_camera_q16[1]);
        let cam_dz = camera_pos[2].wrapping_sub(self.prev_camera_q16[2]);
        self.prev_camera_q16 = camera_pos;
        // Clamp camera jump (e.g. reset/replay wrap) so streaks don't span the whole stage.
        let cam_delta = if cam_dx.abs() < 0x8_0000 && cam_dz.abs() < 0x8_0000 {
            [cam_dx, cam_dy, cam_dz]
        } else {
            [0, 0, 0]
        };
        self.streak_delta_q16 = [
            self.velocity_q16[0].wrapping_sub(cam_delta[0] / 2),
            self.velocity_q16[1].wrapping_sub(cam_delta[1] / 2),
            self.velocity_q16[2].wrapping_sub(cam_delta[2] / 2),
        ];

        // `FUN_0048c620`: advect particles and wrap within `±6.0` m of `center`.
        for i in 0..self.active_count {
            let pos = &mut self.positions_q16[i];
            for axis in 0..3 {
                pos[axis] = pos[axis].wrapping_add(self.velocity_q16[axis]);
                let mut rel = pos[axis].wrapping_sub(center[axis]);
                while rel > PRECIPITATION_HALF_EXTENT_Q16 {
                    rel -= PRECIPITATION_BOX_SPAN_Q16;
                }
                while rel < -PRECIPITATION_HALF_EXTENT_Q16 {
                    rel += PRECIPITATION_BOX_SPAN_Q16;
                }
                pos[axis] = center[axis].wrapping_add(rel);
            }
            // Night attenuation (`0x48cc42..0x48cc55`): dimmer outside headlight cone.
            self.alphas[i] = if self.night {
                match self.kind {
                    PrecipitationKind::Snow => 0x80,
                    PrecipitationKind::Rain => 0x70,
                    PrecipitationKind::None => 0,
                }
            } else {
                match self.kind {
                    PrecipitationKind::Snow => 0xfe,
                    PrecipitationKind::Rain => 0xb0,
                    PrecipitationKind::None => 0,
                }
            };
        }
    }

    /// Build world-space quads for all active rain/snow particles
    /// (`0x48cd60..0x48ceb0`).
    #[must_use]
    pub fn billboards(
        &self,
        right: [i32; 3],
        up: [i32; 3],
        forward: [i32; 3],
    ) -> Vec<PrecipitationQuad> {
        if self.kind == PrecipitationKind::None || !self.initialized || self.active_count == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(self.active_count);
        let to_m = |v: i32| (f64::from(v) / 65536.0) as f32;
        let rx = to_m(right[0]);
        let ry = to_m(right[1]);
        let rz = to_m(right[2]);
        let ux = to_m(up[0]);
        let uy = to_m(up[1]);
        let uz = to_m(up[2]);

        match self.kind {
            PrecipitationKind::Snow => {
                let s = to_m(SNOWFLAKE_HALF_SIZE_Q16);
                let hr = [rx * s, ry * s, rz * s];
                let hu = [ux * s, uy * s, uz * s];
                for i in 0..self.active_count {
                    let p = self.positions_q16[i].map(to_m);
                    let tl = [
                        p[0] - hr[0] + hu[0],
                        p[1] - hr[1] + hu[1],
                        p[2] - hr[2] + hu[2],
                    ];
                    let tr = [
                        p[0] + hr[0] + hu[0],
                        p[1] + hr[1] + hu[1],
                        p[2] + hr[2] + hu[2],
                    ];
                    let br = [
                        p[0] + hr[0] - hu[0],
                        p[1] + hr[1] - hu[1],
                        p[2] + hr[2] - hu[2],
                    ];
                    let bl = [
                        p[0] - hr[0] - hu[0],
                        p[1] - hr[1] - hu[1],
                        p[2] - hr[2] - hu[2],
                    ];
                    out.push(PrecipitationQuad {
                        corners: [tl, tr, br, bl],
                        alpha: self.alphas[i],
                        grey: 0xff,
                        is_snow: true,
                    });
                }
            }
            PrecipitationKind::Rain => {
                let dx = to_m(self.streak_delta_q16[0]);
                let dy = to_m(self.streak_delta_q16[1]);
                let dz = to_m(self.streak_delta_q16[2]);
                let fx = to_m(forward[0]);
                let fy = to_m(forward[1]);
                let fz = to_m(forward[2]);
                // Side vector perpendicular to rain streak and camera forward.
                let sx = dy * fz - dz * fy;
                let sy = dz * fx - dx * fz;
                let sz = dx * fy - dy * fx;
                let slen = (sx * sx + sy * sy + sz * sz).sqrt();
                let half_w = 0.012f32;
                let side = if slen > 1e-4 {
                    [sx / slen * half_w, sy / slen * half_w, sz / slen * half_w]
                } else {
                    [rx * half_w, ry * half_w, rz * half_w]
                };
                for i in 0..self.active_count {
                    let p0 = self.positions_q16[i].map(to_m);
                    let p1 = [p0[0] + dx, p0[1] + dy, p0[2] + dz];
                    let tl = [p0[0] - side[0], p0[1] - side[1], p0[2] - side[2]];
                    let tr = [p0[0] + side[0], p0[1] + side[1], p0[2] + side[2]];
                    let br = [p1[0] + side[0], p1[1] + side[1], p1[2] + side[2]];
                    let bl = [p1[0] - side[0], p1[1] - side[1], p1[2] - side[2]];
                    out.push(PrecipitationQuad {
                        corners: [tl, tr, br, bl],
                        alpha: self.alphas[i],
                        grey: 0x64,
                        is_snow: false,
                    });
                }
            }
            PrecipitationKind::None => {}
        }
        out
    }
}

/// Number of procedural night-sky stars (`FUN_00484130` / `FUN_00484eb0`).
pub const SKY_STAR_COUNT: usize = 60;
/// Number of numbered lens-flare ring bitmaps (`images\lensflr\1.bmp..7.bmp`,
/// `0x482d57..0x482d95`).
pub const LENS_FLARE_RING_COUNT: usize = 7;

/// Sky overlay sprite identity (`FUN_00482d10` / `FUN_00484130`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SkyOverlaySpriteKind {
    Sun,
    SunHalo,
    /// `0..7` corresponding to `images/lensflr/1.bmp..7.bmp`.
    LensFlare(u8),
    Moon,
    Star,
}

/// One projected 2D screen-space sky sprite (`FUN_00483c00` / `FUN_00483cc0` /
/// `FUN_00483db0` / `FUN_00484a70` / `FUN_00484eb0` / `FUN_00485100`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkyOverlaySprite {
    pub kind: SkyOverlaySpriteKind,
    pub dst_x: i32,
    pub dst_y: i32,
    pub dst_w: i32,
    pub dst_h: i32,
    pub alpha: u8,
}

/// Per-stage sky sun, lens flare, moon, and starfield state (`FUN_00482d10`,
/// `FUN_00483060`, `FUN_00484130`, `FUN_00484eb0`).
#[derive(Clone, Debug)]
pub struct SkyOverlayState {
    clear_sky: bool,
    night: bool,
    sun_dir: [f32; 3],
    stars: [(f32, f32, f32); SKY_STAR_COUNT],
}

impl SkyOverlayState {
    /// Initialize sky celestial bodies from the stage's weather and primary
    /// directional light (`FUN_0048b960` -> `FUN_00482d10` + `FUN_00484130`).
    #[must_use]
    pub fn new(
        weather_type: u32,
        lighting: u32,
        primary_light_dir: Option<[i32; 3]>,
        rng: &mut crate::game_rng::GameLcg,
        trig: &[i32; crate::fixed_point::TRIG_TABLE_LEN],
    ) -> Self {
        let clear_sky = weather_type == 1;
        let night = is_night(lighting);
        let raw_dir = primary_light_dir.unwrap_or([0x6000, 0x8000, 0x8000]);
        let mut dx = f64::from(raw_dir[0]) / 65536.0;
        let mut dy = f64::from(raw_dir[1]) / 65536.0;
        let mut dz = f64::from(raw_dir[2]) / 65536.0;
        // Keep the sun/moon above the horizon.
        if dy < 0.22 {
            dy = 0.28;
        }
        let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-4);
        dx /= len;
        dy /= len;
        dz /= len;

        // `0x484ed2..0x484f9d`: distribute 60 stars on the upper dome:
        // horizontal radius = 70*(1 - elev), height = 30 + 90*elev.
        let mut stars = [(0.0f32, 1.0f32, 0.0f32); SKY_STAR_COUNT];
        for star in &mut stars {
            let az = (rng.next_raw() & 0x0fff) as usize;
            let elev = f64::from(rng.next_raw() & 0x7fff) / 32767.0;
            let s = f64::from(trig[az & 0x0fff]) / 65536.0;
            let c = f64::from(trig[(az + 0x400) & 0x0fff]) / 65536.0;
            let horiz = 70.0 * (1.0 - elev);
            let sx = s * horiz;
            let sy = 30.0 + 90.0 * elev;
            let sz = c * horiz;
            let slen = (sx * sx + sy * sy + sz * sz).sqrt().max(1e-4);
            *star = ((sx / slen) as f32, (sy / slen) as f32, (sz / slen) as f32);
        }

        Self {
            clear_sky,
            night,
            sun_dir: [dx as f32, dy as f32, dz as f32],
            stars,
        }
    }

    fn project_dir(
        dir: [f32; 3],
        camera_world: &[i32; 16],
        vw: f32,
        vh: f32,
    ) -> Option<(f32, f32, f32)> {
        let to_f = |v: i32| (f64::from(v) / 65536.0) as f32;
        let rx = to_f(camera_world[0]);
        let ry = to_f(camera_world[1]);
        let rz = to_f(camera_world[2]);
        let ux = to_f(camera_world[4]);
        let uy = to_f(camera_world[5]);
        let uz = to_f(camera_world[6]);
        let fx = to_f(camera_world[8]);
        let fy = to_f(camera_world[9]);
        let fz = to_f(camera_world[10]);

        let cam_z = dir[0] * fx + dir[1] * fy + dir[2] * fz;
        if cam_z <= 0.15 {
            return None;
        }
        let cam_x = dir[0] * rx + dir[1] * ry + dir[2] * rz;
        let cam_y = dir[0] * ux + dir[1] * uy + dir[2] * uz;
        let focal_y = vh * 0.85;
        let focal_x = focal_y;
        let sx = vw * 0.5 + (cam_x / cam_z) * focal_x;
        let sy = vh * 0.5 - (cam_y / cam_z) * focal_y;
        Some((sx, sy, cam_z))
    }

    /// Plan screen-space sky sprites for the current camera view (`0x47aa35..0x47aa7f`,
    /// dispatching `FUN_00483a40` for clear day and `FUN_00484eb0` for clear night).
    #[must_use]
    pub fn plan(
        &self,
        camera_world: &[i32; 16],
        viewport_w: u16,
        viewport_h: u16,
        enabled: bool,
        twinkle_phase: u32,
    ) -> Vec<SkyOverlaySprite> {
        if !enabled || !self.clear_sky || viewport_w == 0 || viewport_h == 0 {
            return Vec::new();
        }
        let vw = f32::from(viewport_w);
        let vh = f32::from(viewport_h);
        let scale = vh / 480.0;
        let mut out = Vec::new();

        if !self.night {
            // Clear day/dusk: sun disc + sun halo + 7 lens-flare rings (`0x483300..0x483f60`).
            if let Some((sx, sy, _)) = Self::project_dir(self.sun_dir, camera_world, vw, vh) {
                let margin = 96.0 * scale;
                if sx >= -margin && sx <= vw + margin && sy >= -margin && sy <= vh + margin {
                    let cx = vw * 0.5;
                    let cy = vh * 0.5;
                    let nx = ((sx - cx) / (vw * 0.65)).abs();
                    let ny = ((sy - cy) / (vh * 0.65)).abs();
                    let vis = (1.0 - nx.max(ny).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                    let sun_size = (64.0 * scale).round() as i32;
                    let halo_size = (160.0 * scale).round() as i32;
                    out.push(SkyOverlaySprite {
                        kind: SkyOverlaySpriteKind::SunHalo,
                        dst_x: sx.round() as i32 - halo_size / 2,
                        dst_y: sy.round() as i32 - halo_size / 2,
                        dst_w: halo_size,
                        dst_h: halo_size,
                        alpha: (190.0 * (0.4 + 0.6 * vis)).round() as u8,
                    });
                    out.push(SkyOverlaySprite {
                        kind: SkyOverlaySpriteKind::Sun,
                        dst_x: sx.round() as i32 - sun_size / 2,
                        dst_y: sy.round() as i32 - sun_size / 2,
                        dst_w: sun_size,
                        dst_h: sun_size,
                        alpha: 255,
                    });
                    if vis > 0.05 {
                        // 7 flare elements along the sun -> screen-center axis (`0x483db0`).
                        const FLARE_SPEC: [(f32, f32, f32); LENS_FLARE_RING_COUNT] = [
                            (-0.35, 36.0, 0.55),
                            (0.25, 28.0, 0.50),
                            (0.55, 48.0, 0.45),
                            (0.80, 24.0, 0.55),
                            (1.05, 64.0, 0.40),
                            (1.30, 32.0, 0.50),
                            (1.55, 56.0, 0.35),
                        ];
                        let dx = cx - sx;
                        let dy = cy - sy;
                        for (idx, &(t, base_px, weight)) in FLARE_SPEC.iter().enumerate() {
                            let fx = sx + dx * t;
                            let fy = sy + dy * t;
                            let sz = (base_px * scale).round().max(4.0) as i32;
                            let alpha = (220.0 * weight * vis).round().clamp(0.0, 255.0) as u8;
                            if alpha > 4 {
                                out.push(SkyOverlaySprite {
                                    kind: SkyOverlaySpriteKind::LensFlare(idx as u8),
                                    dst_x: fx.round() as i32 - sz / 2,
                                    dst_y: fy.round() as i32 - sz / 2,
                                    dst_w: sz,
                                    dst_h: sz,
                                    alpha,
                                });
                            }
                        }
                    }
                }
            }
        } else {
            // Clear night: twinkling stars (`STAR.BMP`) + moon (`MOON.BMP`) (`0x484eb0`).
            let star_sz = (8.0 * scale).round().max(2.0) as i32;
            for (i, &(sx_d, sy_d, sz_d)) in self.stars.iter().enumerate() {
                if let Some((px, py, _)) =
                    Self::project_dir([sx_d, sy_d, sz_d], camera_world, vw, vh)
                    && px >= -10.0
                    && px <= vw + 10.0
                    && py >= -10.0
                    && py <= vh * 0.65
                {
                    let hash = (i as u32)
                        .wrapping_mul(2654435761)
                        .wrapping_add(twinkle_phase >> 2);
                    let alpha = 140 + ((hash & 0x7f) as u8 / 2);
                    out.push(SkyOverlaySprite {
                        kind: SkyOverlaySpriteKind::Star,
                        dst_x: px.round() as i32 - star_sz / 2,
                        dst_y: py.round() as i32 - star_sz / 2,
                        dst_w: star_sz,
                        dst_h: star_sz,
                        alpha,
                    });
                }
            }
            if let Some((mx, my, _)) = Self::project_dir(self.sun_dir, camera_world, vw, vh) {
                let moon_sz = (56.0 * scale).round().max(8.0) as i32;
                if mx >= -moon_sz as f32
                    && mx <= vw + moon_sz as f32
                    && my >= -moon_sz as f32
                    && my <= vh + moon_sz as f32
                {
                    out.push(SkyOverlaySprite {
                        kind: SkyOverlaySpriteKind::Moon,
                        dst_x: mx.round() as i32 - moon_sz / 2,
                        dst_y: my.round() as i32 - moon_sz / 2,
                        dst_w: moon_sz,
                        dst_h: moon_sz,
                        alpha: 240,
                    });
                }
            }
        }
        out
    }
}

/// Two 6-tick storm lightning flash alpha envelopes initialized by
/// `FUN_0048cec0` (`0x48cec0..0x48cf1a` -> `0x82a334..0x82a33f`):
/// - Pattern 0: double-flash `[60, 100, 140, 180, 0, 180]`
/// - Pattern 1: single-flash `[60, 100, 140, 180, 0, 0]`
pub const LIGHTNING_FLASH_PATTERNS: [[u8; 6]; 2] = [
    [0x3c, 0x64, 0x8c, 0xb4, 0x00, 0xb4],
    [0x3c, 0x64, 0x8c, 0xb4, 0x00, 0x00],
];

/// Heavy-storm lightning flash controller reconstructing `FUN_0048cec0` and
/// `FUN_0048cf20` (`weather_detail == 3` in rain/snow).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightningFlashState {
    enabled: bool,
    flashing: bool,
    pattern: usize,
    timer: i16,
    current_alpha: u8,
}

impl LightningFlashState {
    /// Initialize storm lightning (`FUN_0048b960` at `0x48b9f8` -> `FUN_0048cec0`).
    #[must_use]
    pub const fn new(weather_type: u32, detail: u32) -> Self {
        let enabled = (weather_type == 3 || weather_type == 4) && detail >= 3;
        Self {
            enabled,
            flashing: false,
            pattern: 0,
            // Start with an initial wait (~120 ticks = ~5 s) before the first strike.
            timer: 120,
            current_alpha: 0,
        }
    }

    /// Current white additive flash alpha (`0..=0xb4`).
    #[must_use]
    pub const fn current_alpha(&self) -> u8 {
        self.current_alpha
    }

    /// Advance one 25 Hz tick (`FUN_0048cf20`) and return the active flash alpha.
    pub fn tick(&mut self, rng: &mut crate::game_rng::GameLcg) -> u8 {
        if !self.enabled {
            self.current_alpha = 0;
            return 0;
        }
        if self.flashing {
            let idx = (self.timer.clamp(0, 5)) as usize;
            self.current_alpha = LIGHTNING_FLASH_PATTERNS[self.pattern][idx];
            self.timer -= 1;
            if self.timer < 0 {
                self.flashing = false;
                // `0x48cf77..0x48cf9c`: wait `500..=625` ticks (`0x1f4 + rand * 0x7d`).
                let extra = ((i64::from(rng.next_raw() & 0x7fff) * 0x7d) / 0x7fff) as i16;
                self.timer = 0x1f4 + extra;
            }
        } else {
            self.current_alpha = 0;
            self.timer -= 1;
            if self.timer < 0 {
                self.flashing = true;
                // `0x48d00a..0x48d019`: select single- or double-flash pattern.
                self.pattern = usize::from((rng.next_raw() & 0x7fff) >= 0x4000);
                // `0x48d02f`: 6-tick envelope (`5..=0`).
                self.timer = 5;
            }
        }
        self.current_alpha
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_index_masks_or_forces_special() {
        assert_eq!(country_index(0x05, false), 5);
        assert_eq!(country_index(0xf8, false), 8);
        assert_eq!(country_index(0x05, true), SPECIAL_STAGE_COUNTRY);
    }

    #[test]
    fn weather_and_detail_gates() {
        assert!(is_wet_weather(3));
        assert!(is_wet_weather(4));
        assert!(!is_wet_weather(0));
        assert!(!is_wet_weather(5));
        assert!(is_night(8));
        assert!(!is_night(2));
    }

    #[test]
    fn effect_enable_accumulates_distinct_surfaces() {
        // capability[id] = per-surface enable weight for this effect.
        let capability = [0u8, 1, 0, 2, 0, 3];
        // Surfaces 1, 3, 5 present (1 is listed twice - counted once).
        let present = [1usize, 3, 5, 1];
        assert_eq!(accumulate_effect_enable(&present, &capability), 1 + 2 + 3);
        // No enabling surfaces -> 0 (effect disabled).
        assert_eq!(accumulate_effect_enable(&[0, 2, 4], &capability), 0);
        // Out-of-range ids are ignored.
        assert_eq!(accumulate_effect_enable(&[99], &capability), 0);
    }

    #[test]
    fn stage_weather_gates_use_the_recovered_counter_indices() {
        let mut counters = [0; crate::drivetrain::SURFACE_CAPABILITY_COUNT];
        assert_eq!(
            StageWeatherEffects::from_counters(&counters),
            StageWeatherEffects {
                splash: false,
                dust: false,
                snow: false,
            }
        );
        counters[SPLASH_EFFECT_COUNTER] = 1;
        counters[DUST_EFFECT_COUNTER] = 2;
        counters[SNOW_EFFECT_COUNTER] = 3;
        counters[1] = 99;
        let effects = StageWeatherEffects::from_counters(&counters);
        assert!(effects.splash && effects.dust && effects.snow);
    }

    #[test]
    fn tuning_defaults_are_byte_exact() {
        assert_eq!(tuning::TRACK_DECAY_LIFE, 0x4d8);
        assert_eq!(tuning::TRACK_DECAY_RATE, 0x7c);
        assert_eq!(tuning::DRAW_DIRT_MY_VIEWPORT, 100);
        assert_eq!(tuning::DRAW_DIRT_OTHER_VIEWPORT, 30);
        assert_eq!(tuning::INITIAL_FOLLOW_SIZE.raw(), 0x3333);
        assert_eq!(tuning::FOLLOW_SIZE_GROWTH.raw(), 0x51e);
    }

    #[test]
    fn fog_selection_covers_weather_intensity_country_and_detail() {
        use sky::*;
        let wet = fog_settings(3, 2, 3, false).unwrap();
        assert_eq!(wet.color, FOG_COLOR_BLUISH);
        assert_eq!(wet.density, 0.4);
        assert_eq!(wet.far, FOG_FAR_DISTANCES[1]);
        assert_eq!(wet.source_base, 0x8c_0000);

        let rain = fog_settings(4, 3, 2, false).unwrap();
        assert_eq!(rain.color, FOG_COLOR_WHITE);
        assert_eq!(rain.density, 0.8);
        assert_eq!(rain.far, FOG_FAR_DISTANCES[3]);
        assert_eq!(rain.source_base, 0x50_0000);

        let override_settings = fog_settings(5, 0, 6, true).unwrap();
        assert_eq!(override_settings.color, FOG_COLOR_BLUISH);
        assert_eq!(override_settings.density, 0.8);
        assert_eq!(override_settings.far, FOG_FAR_DISTANCES[3]);
        assert!(fog_settings(4, 0, 0, false).is_none());
        assert!(fog_settings(3, 2, 0, true).is_none());
        assert!(fog_settings(2, 2, 0, false).is_none());
        assert_eq!(tuning::MAX_DUST_SIZE.raw(), 0x2_0000);
    }

    #[test]
    fn sky_horizon_gate_and_fog_constants() {
        use sky::{FOG_COLOR_GREY, FOG_FAR_DISTANCES, loads_horizon};
        assert!(loads_horizon(0x00));
        assert!(loads_horizon(0x7f));
        assert!(!loads_horizon(0x80));
        assert_eq!(FOG_COLOR_GREY, 0x7864_6464);
        // Far distances are 20.0 .. 80.0 in Q16.16.
        assert_eq!(FOG_FAR_DISTANCES[0].raw(), 0x14_0000);
        assert_eq!(FOG_FAR_DISTANCES[4].raw(), 0x50_0000);
    }

    #[test]
    fn precipitation_counts_and_kinds_match_48bc60() {
        assert_eq!(
            PrecipitationSystem::new(0, 3, false).kind(),
            PrecipitationKind::None
        );
        assert_eq!(PrecipitationSystem::new(0, 3, false).active_count(), 0);
        let light_rain = PrecipitationSystem::new(3, 1, false);
        assert_eq!(light_rain.kind(), PrecipitationKind::Rain);
        assert_eq!(light_rain.active_count(), 50);
        let med_snow = PrecipitationSystem::new(4, 2, false);
        assert_eq!(med_snow.kind(), PrecipitationKind::Snow);
        assert_eq!(med_snow.active_count(), 100);
        let heavy_snow = PrecipitationSystem::new(4, 3, true);
        assert_eq!(heavy_snow.active_count(), 200);
    }

    #[test]
    fn precipitation_ticks_wrap_particles_around_moving_camera_and_emit_quads() {
        let mut rng = crate::game_rng::GameLcg::new(42);
        let trig = crate::fixed_point::build_sine_table();
        let right = [0x1_0000, 0, 0];
        let up = [0, 0x1_0000, 0];
        let fwd = [0, 0, 0x1_0000];

        let mut snow = PrecipitationSystem::new(4, 2, false);
        snow.tick([0, 0, 0], fwd, &mut rng, &trig);
        let q0 = snow.billboards(right, up, fwd);
        assert_eq!(q0.len(), 100);
        assert!(q0.iter().all(|q| q.is_snow && q.alpha == 0xfe));

        // Move camera 50m forward and tick again: all particles wrap into [center - 6, center + 6].
        snow.tick([0, 0, 50 * 0x1_0000], fwd, &mut rng, &trig);
        let q1 = snow.billboards(right, up, fwd);
        assert_eq!(q1.len(), 100);
        for q in &q1 {
            let z = q.corners[0][2];
            assert!(
                (53.5..=66.5).contains(&z),
                "wrapped snowflake z={z} should stay near camera+10m (60m ± 6m)"
            );
        }

        let mut rain = PrecipitationSystem::new(3, 1, false);
        rain.tick([0, 0, 0], fwd, &mut rng, &trig);
        let rq = rain.billboards(right, up, fwd);
        assert_eq!(rq.len(), 50);
        assert!(rq.iter().all(|q| !q.is_snow && q.grey == 0x64));
    }

    #[test]
    fn sky_overlay_emits_sun_and_flares_on_clear_day_and_moon_stars_on_clear_night() {
        let mut rng = crate::game_rng::GameLcg::new(7);
        let trig = crate::fixed_point::build_sine_table();
        let identity_cam = [
            0x1_0000, 0, 0, 0, // right = +X
            0, 0x1_0000, 0, 0, // up    = +Y
            0, 0, 0x1_0000, 0, // fwd   = +Z
            0, 0, 0, 0x1_0000,
        ];
        // Clear day with sun in front (+Z, +Y).
        let day = SkyOverlayState::new(1, 2, Some([0, 0x6000, 0x1_0000]), &mut rng, &trig);
        let sprites = day.plan(&identity_cam, 640, 480, true, 0);
        assert!(sprites.iter().any(|s| s.kind == SkyOverlaySpriteKind::Sun));
        assert!(
            sprites
                .iter()
                .any(|s| s.kind == SkyOverlaySpriteKind::SunHalo)
        );
        let flare_count = sprites
            .iter()
            .filter(|s| matches!(s.kind, SkyOverlaySpriteKind::LensFlare(_)))
            .count();
        assert_eq!(flare_count, LENS_FLARE_RING_COUNT);
        // Option disabled -> empty.
        assert!(day.plan(&identity_cam, 640, 480, false, 0).is_empty());

        // Sun behind camera (-Z) -> empty.
        let behind = SkyOverlayState::new(1, 2, Some([0, 0x6000, -0x1_0000]), &mut rng, &trig);
        assert!(behind.plan(&identity_cam, 640, 480, true, 0).is_empty());

        // Clear night -> moon + stars.
        let night = SkyOverlayState::new(1, 8, Some([0, 0x6000, 0x1_0000]), &mut rng, &trig);
        let night_sprites = night.plan(&identity_cam, 640, 480, true, 0);
        assert!(
            night_sprites
                .iter()
                .any(|s| s.kind == SkyOverlaySpriteKind::Moon)
        );
        assert!(
            night_sprites
                .iter()
                .any(|s| s.kind == SkyOverlaySpriteKind::Star)
        );

        // Rainy weather (weather_type == 3) -> no sun/moon/stars.
        let rainy = SkyOverlayState::new(3, 2, Some([0, 0x6000, 0x1_0000]), &mut rng, &trig);
        assert!(rainy.plan(&identity_cam, 640, 480, true, 0).is_empty());
    }

    #[test]
    fn lightning_flash_fires_6_tick_envelope_only_in_heavy_storms() {
        let mut rng = crate::game_rng::GameLcg::new(11);
        let mut dry = LightningFlashState::new(1, 3);
        for _ in 0..300 {
            assert_eq!(dry.tick(&mut rng), 0);
        }
        let mut light_rain = LightningFlashState::new(3, 2);
        for _ in 0..300 {
            assert_eq!(light_rain.tick(&mut rng), 0);
        }
        let mut storm = LightningFlashState::new(3, 3);
        let mut saw_peak = false;
        for _ in 0..200 {
            if storm.tick(&mut rng) == 0xb4 {
                saw_peak = true;
            }
        }
        assert!(
            saw_peak,
            "heavy storm must trigger 0xb4 lightning flash peak"
        );
    }
}
