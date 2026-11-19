//! The wheel particles: the dust, sand and snow clouds, the clods, gravel
//! and dry mud, the grass and leaves and the water spray each car throws
//! up (`FUN_00428320` spawn and `FUN_00427d00` update per sim tick,
//! `FUN_00423ec0` render snapshot, `FUN_00428bc0` billboard list).
//!
//! Each car owns a ring of 100 particles (`DAT_005144d0`, `0x50`-byte
//! records), written at a cursor that wraps without searching for a free
//! slot. Per wheel the touched surface feeds emission accumulators through
//! fifteen capability tables (`0x4ec0f8..`) plus the runtime "solid" table;
//! the emitters draw from the shared game `rand()` in the native order.
//!
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.car-particles`.

use crate::fixed_point::TRIG_TABLE_LEN;
use crate::game_rng::GameLcg;
use crate::legacy_math::sqrt_q16;

/// The ring length per car.
pub const RING: usize = 100;
/// The particle tick count cap (`+0x3c > 5000`).
const TICK_CAP: i32 = 5000;
/// `DAT_004ebe20`: the age clock's step (0.8).
pub const AGE_STEP: i32 = 0xcccc;
/// `Follow_size_growth` (`DAT_004ebe98`, 0.02).
pub const FOLLOW_SIZE_GROWTH: i32 = 0x51e;
/// `Gas_alpha_delta` and `Dry_snow_alpha_delta`.
const GAS_LIFE_STEP: i16 = 3;
const SNOW_LIFE_STEP: i16 = 2;
/// `Air_fric_solid` (`DAT_004ebdd0`, 0.01) and the terminal velocity
/// `DAT_004ecbd0` (0, -9.5, 0).
const AIR_FRICTION: i32 = 0x28f;
const TERMINAL_Y: i32 = -0x9_8000;
/// `Min_dirt_vertical` (0.15) and `Spray_wetness` (8).
const MIN_DIRT_VERTICAL: i32 = 0x2666;
const SPRAY_WETNESS: i32 = 8;
/// The sideways offset of the wheel points along the axle vectors (0.155).
const AXLE_OFFSET: i32 = 0x27ae;
/// The wheel point blend `DAT_0054f800`/`DAT_0054b7ec` (default 0.78/0.22).
pub const DEFAULT_BLEND: (i32, i32) = (0x3851, 0xc7ae);

/// The particle types (`+0x4a`).
pub mod kind {
    pub const DUST: u8 = 1;
    pub const CLOD: u8 = 2;
    pub const SAND: u8 = 6;
    pub const SNOW: u8 = 7;
    pub const SNOW_CLOUD: u8 = 8;
    pub const SPLASH: u8 = 0xb;
    pub const FOLIAGE: u8 = 0xc;
    pub const GRASS_A: u8 = 0xd;
    pub const GRASS_B: u8 = 0xe;
    pub const YELLOW_GRASS: u8 = 0xf;
    pub const LEAVES: u8 = 0x10;
    pub const MULCH: u8 = 0x11;
    pub const GRAVEL: u8 = 0x15;
    pub const DUST_16: u8 = 0x16;
    pub const RED_GRAVEL: u8 = 0x17;
    pub const DUST_18: u8 = 0x18;
    pub const BIG_GRAVEL: u8 = 0x19;
    pub const DRY_MUD: u8 = 0x1a;
}

/// The sprite table a particle draws from (`+0x40`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sprites {
    #[default]
    None,
    /// `0x52e6e0`: `images\CC\Dust\Dstcors%1d.bmp` (fallback `images\Dust`).
    Dust,
    /// `0x524138`: `images\NewSand\IndSand%d.bmp`.
    Sand,
    /// `0x52dfa8`: `images\NewSnow\Snow%02d.bmp`.
    Snow,
    /// `0x54b768`: `images\Snow2\Sncld%02d.bmp`.
    SnowCloud,
    /// `0x54b7cc`: `images\Dust\Dstcors%1d.bmp`.
    Dust16,
    /// `0x52e490`: `images\CC\Dust\Dstcors%1d.bmp`.
    Dust18,
    /// `0x54ba88`/`0x52c2f0`: `images\clota%1d.bmp`, `clotb`.
    ClotA,
    ClotB,
    /// `0x54bd7c`: `images\Wsplsh9\%02d.bmp`.
    Splash,
    /// `0x54beec`, `0x52c244`: `images\Grass1\Grass%02d.bmp`.
    GrassA,
    GrassB,
    /// `0x52c774`: `images\YellowGrass\Grass%02d.bmp`.
    YellowGrass,
    /// `0x52fe18`: `images\Leaves\Leaves%02d.bmp`.
    Leaves,
    /// `0x52c5a0`: `images\CC\MlchScrb\Mulch%02d.bmp`.
    Mulch,
    /// `0x52c224`: `images\CC\Grav\CCGrav%02d.bmp`.
    Gravel,
    /// `0x54bad4`: `images\NZ\Grav\NZGrav%02d.bmp` or the Australian gravel.
    RedGravel,
    /// `0x54b788`: `images\CC\DryMud\dmud%02d.bmp`.
    DryMud,
}

/// One pool record (`DAT_005144d0 + (car * 100 + i) * 0x50`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Particle {
    pub pos: [i32; 3],
    pub prev: [i32; 3],
    pub origin: [i32; 3],
    pub vel: [i32; 3],
    pub word_30: i32,
    /// `+0x34`: the size, or the dust family's size growth.
    pub size: i32,
    /// `+0x38`: the Q16 age clock.
    pub age: i32,
    /// `+0x3c`: ticks lived.
    pub ticks: i32,
    pub sprites: Sprites,
    /// `+0x44`: the variant (clod table, splash side `0x21`/`0x22`).
    pub variant: u8,
    /// `+0x46`/`+0x48`: the life (also the dust's alpha) and its step.
    pub life: i16,
    pub life_step: i16,
    pub kind: u8,
    /// `+0x4b`: the grey level.
    pub grey: u8,
    /// `+0x4c`: bit 0 active, bit 1 the first wheel of an emitting pair.
    pub flags: u8,
}

impl Particle {
    pub const fn active(&self) -> bool {
        self.flags & 1 != 0
    }

    /// `FUN_00425240`: the kill.
    fn kill(&mut self) {
        self.word_30 = 0;
        self.age = 0;
        self.flags &= !1;
        self.kind = 0;
        self.pos[1] = -0x64_0000;
    }
}

/// A wheel's emission accumulators (`FUN_00425e30`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Accumulators {
    pub mud: i32,
    pub dust: i32,
    pub dust_16: i32,
    pub dust_18: i32,
    pub sand: i32,
    pub snow: i32,
    pub solid: i32,
    pub wet: i32,
    pub grass_a: i32,
    pub grass_b: i32,
    pub yellow_grass: i32,
    pub leaves: i32,
    pub mulch: i32,
    pub dry_mud: i32,
    pub gravel_15: i32,
    pub gravel_17: i32,
}

/// One car's particles and per-wheel emitter state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarParticles {
    pub ring: Vec<Particle>,
    /// `DAT_0052e640[car]`: the next slot.
    pub cursor: usize,
    pub acc: [Accumulators; 4],
    /// `0x524280`/`0x52c420`: this tick's and last tick's wheel points.
    pub wheels: [[i32; 3]; 4],
    pub wheels_prev: [[i32; 3]; 4],
    /// `DAT_005240b8`: consecutive contact ticks, `DAT_0054baa8` wheels
    /// down, `DAT_0052e4c0` last surfaces, `DAT_0052fc18` last contacts.
    pub contact_ticks: [u32; 4],
    pub wheels_down: u32,
    pub last_surface: [i16; 4],
    pub last_contact: [bool; 4],
    /// `DAT_0052c39c`: set to `0x1e` when the solid accumulator passes 40.
    pub solid_marks: [u8; 4],
    /// `DAT_0052f9e8`: a snow-bank hit at a probe (`FUN_00427920`).
    pub snow_puff: Option<usize>,
    /// The tyre marks: 80 per wheel (`DAT_0052ff60`), their heads
    /// (`DAT_0052e06c`) and marking flags (`DAT_0052fb98`).
    pub marks: Vec<[TyreMark; MARKS]>,
    pub mark_heads: [usize; 4],
    pub marking: [bool; 4],
}

/// The tyre marks per wheel.
pub const MARKS: usize = 80;
/// `Track_Decay_Life` (1240 ticks) and `Track_Decay_RATE` (`0x7c`).
pub const MARK_LIFE: i32 = 0x4d8;
pub const MARK_RATE: i32 = 0x7c;
/// `ABIT` (`DAT_004ebec0`): each wheel's mark sits `(w + 4) * ABIT` lower.
const ABIT: i32 = 0x83;

/// One tyre-mark segment end (`0x2c` bytes): its two edge points, age,
/// alpha, the tick it was laid (`+0x24`) and its colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TyreMark {
    pub a: [i32; 3],
    pub b: [i32; 3],
    pub age: i32,
    pub alpha: u8,
    pub valid: bool,
    pub seq: u32,
    pub rgb: [u8; 3],
}

impl Default for CarParticles {
    fn default() -> Self {
        Self {
            ring: vec![Particle::default(); RING],
            cursor: 0,
            acc: [Accumulators::default(); 4],
            wheels: [[0; 3]; 4],
            wheels_prev: [[0; 3]; 4],
            contact_ticks: [0; 4],
            wheels_down: 0,
            last_surface: [0; 4],
            last_contact: [false; 4],
            solid_marks: [0x20; 4],
            snow_puff: None,
            marks: vec![[TyreMark::default(); MARKS]; 4],
            mark_heads: [0; 4],
            marking: [false; 4],
        }
    }
}

/// The per-tick car and control words the spawners read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WheelCar {
    /// `car +0xd4`, `+0xec`.
    pub velocity: [i32; 3],
    pub speed: i32,
    /// `car +0xfc..+0x158`: the eight probe points.
    pub probes: [[i32; 3]; 8],
    /// `car +0x264`: the up row.
    pub up: [i32; 3],
    /// `car +0x68c`: the wheels touching.
    pub contact: [bool; 4],
    /// `car +0x73a`: the per-wheel bytes.
    pub flags_73a: [u8; 4],
    /// `ctl +0x2a8`: the raw wheel speeds.
    pub wheel_speed: [i32; 4],
    /// `ctl +0x380/+0x38c/+0x398/+0x3a4`.
    pub head: [[i32; 3]; 4],
    /// `ctl +0x3b0` (longitudinal slip) and `+0x3c0` (lateral dot).
    pub longitudinal: [i32; 4],
    pub lateral: [i32; 4],
    /// `ctl +0x42a`: the wheel surfaces.
    pub surfaces: [i16; 4],
    /// `ctl +0x43a`: the gear (7 reverse).
    pub gear: i8,
    /// `ctl +0x404`: upright, and `car +0x33c + 12w`: each wheel's ground
    /// normal (the tyre marks).
    pub upright: bool,
    pub normals: [[i32; 3]; 4],
}

/// The stage facts the spawners read.
#[derive(Clone, Copy, Debug)]
pub struct ParticleStage<'a> {
    /// The fifteen capability tables (`0x4ec0f8 + k * 0x78`) and the
    /// runtime solid table (`0x54f824`).
    pub tables: &'a [[u8; 120]],
    pub solid: &'a [u8; 120],
    /// `FUN_0046b240() == 8`, `FUN_0046b220()`, the country (7 on the
    /// special stage).
    pub night: bool,
    pub weather_type: u32,
    pub country: u8,
    /// `FUN_004a96e0`: the last lit vertex colour, which the spawn grey
    /// averages (backend policy: the stage's lit colour).
    pub lit: [u8; 3],
    /// `DAT_0054f89c`/`DAT_0054b7c8`: the fixed grey latch of
    /// `FUN_00425be0`.
    pub fixed_grey: Option<u8>,
    /// `DAT_0054f800`/`DAT_0054b7ec`: the wheel point blend `(a, b)`.
    pub blend: (i32, i32),
    /// `0x4ec800`: each surface's mark enable and colour (`FUN_00424c60`
    /// sets surface `0x2c`'s by country), and `cfg[0x25] & 4` (narrow).
    pub mark_surfaces: &'a [[u8; 8]],
    pub narrow_marks: bool,
}

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

fn qdiv(a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    ((i64::from(a) << 16) / i64::from(b)) as i32
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| a[i].wrapping_add(b[i]))
}

fn sub(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| a[i].wrapping_sub(b[i]))
}

fn scale(a: [i32; 3], k: i32) -> [i32; 3] {
    a.map(|c| qmul(c, k))
}

fn length(v: [i32; 3]) -> i32 {
    let sq = (0..3).fold(0i32, |s, i| s.wrapping_add(qmul(v[i], v[i])));
    sqrt_q16(sq as u32) as i32
}

fn rand(rng: &mut GameLcg) -> i32 {
    rng.next_raw() as i32
}

/// `rand() % 4000 / 4000` in Q16 (the `__ftol(n * 65536.0)` pattern).
fn unit(rng: &mut GameLcg) -> i32 {
    let n = rand(rng) % 4000;
    ((i64::from(n * 65536) << 16) / 0xfa0_0000) as i32
}

/// The table value for a surface word (sign-extended index).
fn table(stage: &ParticleStage, k: usize, surface: i16) -> i32 {
    let Some(t) = stage.tables.get(k) else {
        return 0;
    };
    usize::try_from(surface)
        .ok()
        .and_then(|s| t.get(s))
        .map_or(0, |&v| i32::from(v))
}

/// The world's particle state (`DAT_0054f8b0` and the counters).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleSystem {
    pub cars: Vec<CarParticles>,
    /// `DAT_0054f8b0`: particles enabled (the options' effects toggle).
    pub enabled: bool,
    /// `DAT_0052c398` and `DAT_0054f8ac`.
    pub ticks: u32,
    pub spawn_ticks: u32,
}

impl ParticleSystem {
    /// The race-init clear (`FUN_00424160`'s tail) for `cars` cars.
    pub fn new(cars: usize, enabled: bool) -> Self {
        Self {
            cars: vec![CarParticles::default(); cars],
            enabled,
            ticks: 0,
            spawn_ticks: 0,
        }
    }

    /// `FUN_00428320`: one tick's spawning for each car.
    pub fn spawn(
        &mut self,
        cars: &[WheelCar],
        stage: &ParticleStage,
        rng: &mut GameLcg,
        trig: &[i32; TRIG_TABLE_LEN],
    ) {
        if !self.enabled {
            return;
        }
        self.ticks = self.ticks.wrapping_add(1);
        for (state, car) in self.cars.iter_mut().zip(cars) {
            state.wheel_points(car, stage.blend);
            state.spawn_car(car, stage, rng, trig, &mut self.spawn_ticks);
        }
    }

    /// `FUN_00427d00`: one tick's integration for each car.
    pub fn update(&mut self, rng: &mut GameLcg) {
        for car in &mut self.cars {
            car.integrate(rng);
        }
    }
}

impl CarParticles {
    /// `FUN_00428360`: the wheel points, blended between the probe pairs and
    /// offset along the axle vectors.
    fn wheel_points(&mut self, car: &WheelCar, (a, b): (i32, i32)) {
        let p = |i: usize| car.probes[i];
        let mix = |x: [i32; 3], kx: i32, y: [i32; 3], ky: i32| add(scale(x, kx), scale(y, ky));
        let off_b = scale(car.head[2], AXLE_OFFSET);
        let off_d = scale(car.head[3], AXLE_OFFSET);
        self.wheels = [
            sub(mix(p(2), a, p(0), b), off_b),
            add(mix(p(3), a, p(1), b), off_b),
            sub(mix(p(2), b, p(0), a), off_d),
            add(mix(p(1), a, p(3), b), off_d),
        ];
    }

    /// The next ring slot (`FUN_00424f40`/`FUN_00425be0` advance it `% 100`).
    fn next(&mut self) -> &mut Particle {
        let i = self.cursor;
        self.cursor = (self.cursor + 1) % RING;
        &mut self.ring[i]
    }

    fn grey(stage: &ParticleStage) -> u8 {
        if stage.night {
            0x32
        } else {
            ((u32::from(stage.lit[0]) + u32::from(stage.lit[1]) + u32::from(stage.lit[2])) / 3)
                as u8
        }
    }

    /// `FUN_00424f40`: a dust-family particle.
    #[allow(clippy::too_many_arguments)]
    fn spawn_dust(
        &mut self,
        stage: &ParticleStage,
        pos: [i32; 3],
        vel: [i32; 3],
        life: i32,
        age: i32,
        growth: i32,
        kind: u8,
        first: bool,
    ) {
        let grey = Self::grey(stage);
        let (sprites, step) = match kind {
            kind::SAND => (Sprites::Sand, SNOW_LIFE_STEP),
            kind::SNOW => (Sprites::Snow, SNOW_LIFE_STEP),
            kind::SNOW_CLOUD => (Sprites::SnowCloud, SNOW_LIFE_STEP),
            9 | 0x1b => (Sprites::None, SNOW_LIFE_STEP),
            kind::DUST_16 => (Sprites::Dust16, GAS_LIFE_STEP),
            kind::DUST_18 => (Sprites::Dust18, GAS_LIFE_STEP),
            _ => (Sprites::Dust, GAS_LIFE_STEP),
        };
        *self.next() = Particle {
            pos,
            prev: pos,
            origin: pos,
            vel,
            word_30: 0,
            size: growth,
            age,
            ticks: 0,
            sprites,
            variant: 0,
            life: life as i16,
            life_step: step,
            kind,
            grey,
            flags: 1 | (u8::from(first) << 1),
        };
    }

    /// `FUN_00425be0`: a debris particle (clods, gravel, mud, foliage).
    fn spawn_debris(
        &mut self,
        stage: &ParticleStage,
        rng: &mut GameLcg,
        pos: [i32; 3],
        vel: [i32; 3],
        age: i32,
        kind: u8,
    ) {
        let grey = stage.fixed_grey.unwrap_or_else(|| Self::grey(stage));
        let mut kind = kind;
        let size = match kind {
            kind::MULCH => 0x35c2 + rand(rng) % 0xa3d,
            kind::GRAVEL => 0x1999 + rand(rng) % 0x1999,
            kind::BIG_GRAVEL => {
                kind = kind::GRAVEL;
                0x2666 + rand(rng) % 0x2666
            }
            kind::DRY_MUD => 0x2e14 + rand(rng) % 0x7ae,
            _ => 0x451e + rand(rng) % 0xccc,
        };
        let variant = (rand(rng) & 1) as u8;
        let (sprites, kind, age) = match kind {
            kind::GRASS_A => (Sprites::GrassA, kind::FOLIAGE, age + (rand(rng) & 0x1ffff)),
            kind::GRASS_B => (Sprites::GrassB, kind::FOLIAGE, age + (rand(rng) & 0x1ffff)),
            kind::YELLOW_GRASS => (
                Sprites::YellowGrass,
                kind::FOLIAGE,
                age + (rand(rng) & 0x1ffff),
            ),
            kind::LEAVES => (Sprites::Leaves, kind::FOLIAGE, age + (rand(rng) & 0x1ffff)),
            kind::MULCH => (Sprites::Mulch, kind::FOLIAGE, age + (rand(rng) & 0x1ffff)),
            kind::CLOD => (
                if variant == 0 {
                    Sprites::ClotA
                } else {
                    Sprites::ClotB
                },
                kind,
                age,
            ),
            kind::GRAVEL => (Sprites::Gravel, kind, age),
            kind::RED_GRAVEL => (Sprites::RedGravel, kind, age),
            kind::DRY_MUD => (Sprites::DryMud, kind, age),
            _ => (Sprites::None, kind, age),
        };
        *self.next() = Particle {
            pos,
            prev: pos,
            origin: pos,
            vel,
            word_30: 0,
            size,
            age,
            ticks: 0,
            sprites,
            variant,
            life: 0,
            life_step: 0,
            kind,
            grey,
            flags: 1,
        };
    }

    /// `FUN_00425e30`: a wheel's surface into its accumulators.
    fn accumulate(&mut self, car: &WheelCar, stage: &ParticleStage, w: usize) {
        let s = car.surfaces[w];
        let a = &mut self.acc[w];
        if car.contact[w] {
            a.mud += table(stage, 0, s);
            a.dust += table(stage, 2, s);
            a.dust_16 += table(stage, 13, s);
            a.dust_18 += table(stage, 14, s);
            a.sand += table(stage, 3, s);
            a.snow += table(stage, 4, s);
            a.solid += usize::try_from(s)
                .ok()
                .and_then(|i| stage.solid.get(i))
                .map_or(0, |&v| i32::from(v));
            a.wet = table(stage, 1, s);
            a.grass_a = table(stage, 5, s);
            a.grass_b = table(stage, 6, s);
            a.yellow_grass = table(stage, 7, s);
            a.leaves = table(stage, 8, s);
            a.mulch = table(stage, 9, s);
            a.dry_mud = table(stage, 10, s);
            a.gravel_15 = table(stage, 11, s);
            a.gravel_17 = table(stage, 12, s);
        } else {
            let mud = a.mud;
            *a = Accumulators {
                mud,
                ..Accumulators::default()
            };
        }
        a.mud = a.mud * 4 / 5;
        for x in [
            &mut a.dust,
            &mut a.dust_16,
            &mut a.dust_18,
            &mut a.snow,
            &mut a.sand,
            &mut a.solid,
        ] {
            *x = *x * 45 / 50;
        }
        a.wet = a.wet * 20 / 50;
        let c = stage.country;
        if stage.weather_type == 4
            || stage.weather_type == 3
            || a.wet != 0
            || matches!(c, 7 | 6 | 5 | 4 | 2)
            || (c == 0 && 0x13 < s && s < 0x1e)
        {
            a.dust = 0;
            a.dust_16 = 0;
            a.dust_18 = 0;
            a.sand = 0;
        }
    }

    /// `FUN_00428320`'s per-car body `FUN_00425280`.
    fn spawn_car(
        &mut self,
        car: &WheelCar,
        stage: &ParticleStage,
        rng: &mut GameLcg,
        trig: &[i32; TRIG_TABLE_LEN],
        seq: &mut u32,
    ) {
        for w in 0..4 {
            self.accumulate(car, stage, w);
            // `FUN_00425450`: clods from every wheel.
            let mud = self.acc[w].mud;
            self.debris(car, stage, rng, trig, mud, w, kind::CLOD);
            // `FUN_004267d0`: gravel.
            let a = self.acc[w];
            let (lat, long) = (car.lateral[w].abs(), car.longitudinal[w].abs());
            if (a.gravel_15 != 0 || a.gravel_17 != 0)
                && (lat >= 0x2_0000 || long >= 0x1_3333 || a.gravel_15 > 0x17)
            {
                let (amount, k) = if a.gravel_17 < a.gravel_15 {
                    (a.gravel_15, kind::GRAVEL)
                } else {
                    (a.gravel_17, kind::RED_GRAVEL)
                };
                self.debris(car, stage, rng, trig, amount, w, k);
            }
            // `FUN_00426860`: dry mud.
            if a.dry_mud != 0 && (lat >= 0x2_0000 || long >= 0x1_3333) {
                self.debris(car, stage, rng, trig, a.dry_mud, w, kind::DRY_MUD);
            }
            // `FUN_004268d0`.
            if a.solid > 0x28 {
                self.solid_marks[w] = 0x1e;
            }
            // `FUN_00426c80`: the foliage types, in table order.
            for (amount, k) in [
                (a.grass_a, kind::GRASS_A),
                (a.grass_b, kind::GRASS_B),
                (a.yellow_grass, kind::YELLOW_GRASS),
                (a.leaves, kind::LEAVES),
                (a.mulch, kind::MULCH),
            ] {
                if amount > 0 {
                    self.foliage(car, stage, rng, w, amount, k);
                }
            }
        }
        // `DAT_0054f8ac`: the tick the marks are laid in.
        *seq = seq.wrapping_add(1);
        // `FUN_00426900`: the spray.
        for w in 0..4 {
            let wet = self.acc[w].wet;
            // `wet > 0 && wet >= Spray_wetness`.
            if wet >= SPRAY_WETNESS {
                self.splash(car, stage, rng, w);
            }
        }
        // `FUN_004261c0`: the tyre marks, not at night.
        if !stage.night {
            self.lay_marks(car, stage, *seq);
        }
        let pair = if car.gear == 7 { (0, 1) } else { (2, 3) };
        // `FUN_00427040`, `FUN_004277b0`.
        let a = self.acc;
        self.dust_pair(
            car,
            stage,
            rng,
            pair,
            (a[pair.0].dust, a[pair.1].dust),
            kind::DUST,
            0x1_0000,
        );
        self.dust_pair(
            car,
            stage,
            rng,
            pair,
            (a[pair.0].dust_16, a[pair.1].dust_16),
            kind::DUST_16,
            0x1_0000,
        );
        self.dust_pair(
            car,
            stage,
            rng,
            pair,
            (a[pair.0].dust_18, a[pair.1].dust_18),
            kind::DUST_18,
            0x1_0000,
        );
        // `FUN_00427950`: snow from both pairs, one type per tick.
        let snow = if rand(rng) % 100 > 49 {
            kind::SNOW_CLOUD
        } else {
            kind::SNOW
        };
        for pair in [(0, 1), (2, 3)] {
            self.dust_pair(
                car,
                stage,
                rng,
                pair,
                (a[pair.0].snow, a[pair.1].snow),
                snow,
                0x2_0000,
            );
        }
        if let Some(probe) = self.snow_puff.take() {
            let p = car.probes[probe % 8];
            let q = car.probes[(probe + 4) % 8];
            let d = sub(q, p);
            let k = rand(rng) % 8000 + 0x3d2c;
            let pos = add(p, scale(d, k));
            self.spawn_dust(stage, pos, [0; 3], 200, 0, FOLLOW_SIZE_GROWTH, snow, true);
        }
        // `FUN_004278c0`: sand.
        self.dust_pair(
            car,
            stage,
            rng,
            pair,
            (a[pair.0].sand, a[pair.1].sand),
            kind::SAND,
            0x1_0000,
        );
        for w in 0..4 {
            self.last_surface[w] = car.surfaces[w];
            self.wheels_prev[w] = self.wheels[w];
            self.last_contact[w] = car.contact[w];
            if car.contact[w] && car.flags_73a[w] != 1 {
                self.contact_ticks[w] += 1;
                self.wheels_down += 1;
            } else {
                self.contact_ticks[w] = 0;
            }
        }
    }

    /// `FUN_004261c0`: each sliding wheel lays a mark across its contact
    /// point, `0.12` either side of its travel, darker the harder it slides.
    fn lay_marks(&mut self, car: &WheelCar, stage: &ParticleStage, seq: u32) {
        let surface = |s: i16| {
            usize::try_from(s)
                .ok()
                .and_then(|i| stage.mark_surfaces.get(i))
                .copied()
                .unwrap_or([0; 8])
        };
        for w in 0..4 {
            for mark in self.marks[w].iter_mut().filter(|m| m.valid) {
                mark.age += 1;
            }
            let now = surface(car.surfaces[w]);
            let before = surface(self.last_surface[w]);
            if !(car.contact[w]
                && self.contact_ticks[w] > 10
                && self.wheels_down > 1
                && car.upright
                && car.flags_73a[w] != 1
                && now[0] == 1
                && before[0] == 1)
            {
                continue;
            }
            let slide = car.lateral[w].abs() - 0x9999;
            let spin = car.longitudinal[w].abs() - 0x9999;
            if slide <= 0 && spin <= 0 {
                self.marking[w] = false;
                continue;
            }
            let alpha = ((i64::from((slide / 2).max(spin)) * 0xff_0000) >> 32).min(255) as i32;
            if alpha < 0x51 {
                continue;
            }
            self.mark_heads[w] = (self.mark_heads[w] + 1) % MARKS;
            let p = self.wheels[w];
            let d = sub(self.wheels_prev[w], p);
            let n = car.normals[w];
            let cross = [
                qmul(d[1], n[2]) - qmul(d[2], n[1]),
                qmul(d[2], n[0]) - qmul(d[0], n[2]),
                qmul(d[0], n[1]) - qmul(d[1], n[0]),
            ];
            let len = length(cross);
            let mut h = if len == 0 {
                [0; 3]
            } else {
                scale(cross.map(|c| qdiv(c, len)), 0x1eb8)
            };
            if stage.narrow_marks {
                h = scale(h, 0x8000);
            }
            let drop = (w as i32 + 4) * ABIT;
            let mut a = add(p, h);
            let mut b = sub(p, h);
            a[1] -= drop;
            b[1] -= drop;
            let colour = [before[4], before[5], before[6]];
            let rgb: [u8; 3] = std::array::from_fn(|k| {
                ((u32::from(stage.lit[k]) * u32::from(colour[k])) / 255) as u8
            });
            let head = self.mark_heads[w];
            self.marks[w][head] = TyreMark {
                a,
                b,
                age: 0,
                alpha: alpha as u8,
                valid: true,
                seq,
                rgb,
            };
            self.marking[w] = true;
            self.marks[w][(head + 1) % MARKS].alpha = 0;
        }
    }

    /// `FUN_00429980`: the marks as coloured triangles (two per joined
    /// pair of consecutive ticks), fading over `Life / RATE` ticks past
    /// their life; `ambient` is the stage's sixth light colour
    /// (`DAT_00592ef4`).
    pub fn mark_triangles(&self, ambient: [u8; 3]) -> Vec<([[i32; 3]; 3], [u32; 3])> {
        let mut out = Vec::new();
        for w in 0..4 {
            let ring = &self.marks[w];
            let head = self.mark_heads[w];
            for k in 0..MARKS {
                let i = (head + MARKS - k) % MARKS;
                let j = (i + MARKS - 1) % MARKS;
                let (ni, nj) = (ring[i], ring[j]);
                if !(ni.valid && nj.valid && ni.seq == nj.seq.wrapping_add(1)) {
                    continue;
                }
                let fade = |m: &TyreMark| {
                    let a = i32::from(m.alpha);
                    let a = if nj.age > MARK_LIFE {
                        a - (m.age - MARK_LIFE) * a * MARK_RATE / MARK_LIFE
                    } else {
                        a
                    };
                    a.clamp(0, 255)
                };
                let (ai, aj) = (fade(&ni), fade(&nj));
                if ai <= 4 {
                    continue;
                }
                let colour = |m: &TyreMark, a: i32| {
                    let c: [u32; 3] =
                        std::array::from_fn(|k| u32::from(m.rgb[k]) * u32::from(ambient[k]) / 200);
                    ((a as u32) << 24)
                        | (c[0].min(255) << 16)
                        | (c[1].min(255) << 8)
                        | c[2].min(255)
                };
                let (ci, cj) = (colour(&ni, ai), colour(&nj, aj));
                out.push(([nj.a, ni.b, ni.a], [cj, ci, ci]));
                out.push(([nj.b, ni.b, nj.a], [cj, ci, cj]));
            }
        }
        out
    }

    /// `FUN_004270f0`: a cloud from a wheel pair, spread between the two
    /// wheels and back along this tick's travel.
    #[allow(clippy::too_many_arguments)]
    fn dust_pair(
        &mut self,
        car: &WheelCar,
        stage: &ParticleStage,
        rng: &mut GameLcg,
        (wa, wb): (usize, usize),
        amounts: (i32, i32),
        kind: u8,
        spread: i32,
    ) {
        let a = self.wheels[wa];
        let ap = self.wheels_prev[wa];
        let b = self.wheels[wb];
        let v = car.velocity;
        let speed = length(v);
        let mut amounts = [amounts.0, amounts.1];
        for (k, amount) in amounts.iter_mut().enumerate() {
            let w = if k == 0 { wa } else { wb };
            let long = car.longitudinal[w];
            let s = if speed < 0x2_0000 {
                speed + long.abs()
            } else {
                speed
            };
            let r = rand(rng) % 100;
            let n = (r * (qdiv(s, 0x2666) >> 16) * *amount / 5000 / 3).min(3);
            let vel = add(scale(scale(car.head[1], long), 0x1999), v);
            let dl = car.lateral[w].abs() - 0x3333;
            let dg = long.abs() - 0x3333;
            if (dl > 0 || dg > 0) && qdiv(dl, 0x2_0000) + qdiv(dg, 0x1_0000) > 0x8000 {
                *amount = *amount * 3 / 2;
            }
            for _ in 0..n.max(0) {
                let u = unit(rng);
                let r1 = rand(rng) % 6000;
                let r2 = rand(rng) % 8000 + 8000;
                let mut t = qdiv(qdiv(r1 << 16, spread), r2 << 16);
                if k == 1 {
                    t = 0x1_0000 - t;
                }
                let pos = sub(
                    add(add(a, scale(sub(ap, a), u)), scale(sub(b, a), t)),
                    scale(vel, u),
                );
                self.spawn_dust(
                    stage,
                    pos,
                    vel,
                    *amount,
                    u,
                    FOLLOW_SIZE_GROWTH,
                    kind,
                    k == 0,
                );
            }
        }
    }

    /// `FUN_00425470`: debris thrown from one wheel.
    #[allow(clippy::too_many_arguments)]
    fn debris(
        &mut self,
        car: &WheelCar,
        stage: &ParticleStage,
        rng: &mut GameLcg,
        trig: &[i32; TRIG_TABLE_LEN],
        amount: i32,
        w: usize,
        kind: u8,
    ) {
        let n = if amount < 1 {
            0
        } else if car.longitudinal[w].abs() < 0x4ccc {
            1
        } else {
            let r = rand(rng) % amount;
            if r > 15 {
                4
            } else if r > 12 {
                2
            } else {
                1
            }
        };
        let v = car.velocity;
        let sin = |a: i32| trig[(a & 0xfff) as usize];
        let cos = |a: i32| trig[((a + 0x400) & 0xfff) as usize];
        for _ in 0..n {
            let u = unit(rng);
            let pos0 = add(
                self.wheels[w],
                scale(sub(self.wheels_prev[w], self.wheels[w]), u),
            );
            let ws = (-car.wheel_speed[w]).clamp(-0x1_6666, 0x1_6666);
            let la = car.lateral[w].clamp(-0x1_6666, 0x1_6666);
            let d = scale(add(scale(car.head[0], ws), scale(car.head[2], la)), 0x3333);
            let angle = (i32::from((rand(rng) & 0xfff) as i16) - 0x800) / 0x12;
            let (sa, ca) = (sin(angle), cos(angle));
            let z = qmul(d[2], ca) - qmul(d[0], sa);
            let x = qmul(d[0], ca) + qmul(d[2], sa);
            let e_num = i32::from((rand(rng) & 0xfff) as i16);
            let e = e_num / (rand(rng) % 10 + 10);
            let (se, ce) = (sin(e), cos(e));
            let hx = x / 2 + v[0];
            let hz = z / 2 + v[2];
            let vy = qmul(x.abs() + z.abs(), se) + v[1] + qmul((hx.abs() + hz.abs()) / 2, se);
            let vel = [qmul(hx, ce), vy, qmul(hz, ce)];
            let mut o = scale(if w < 2 { car.head[0] } else { car.head[1] }, ce);
            if car.wheel_speed[w] > 0 {
                o = o.map(i32::wrapping_neg);
            }
            let pos = add(pos0, scale(add(o, scale(car.up, se)), 0x4000));
            let age = qmul(u, 0xcccc);
            if kind == kind::GRAVEL && amount == 0x1c {
                self.spawn_debris(stage, rng, pos, vel, age, kind::BIG_GRAVEL);
            } else if (amount == 0x18 && kind == kind::GRAVEL) || vel[1] > MIN_DIRT_VERTICAL {
                self.spawn_debris(stage, rng, pos, vel, age, kind);
            }
        }
    }

    /// `FUN_00426d00`: grass and leaves kicked sideways.
    fn foliage(
        &mut self,
        car: &WheelCar,
        stage: &ParticleStage,
        rng: &mut GameLcg,
        w: usize,
        amount: i32,
        kind: u8,
    ) {
        let n = if car.longitudinal[w].abs() >= 0x1_3333 {
            let r = rand(rng) % amount;
            if r > 15 {
                2
            } else if r > 12 {
                1
            } else {
                0
            }
        } else {
            0
        };
        for _ in 0..n {
            let u = unit(rng);
            let pos = add(
                self.wheels[w],
                scale(sub(self.wheels_prev[w], self.wheels[w]), u),
            );
            let r = if w < 2 { car.head[2] } else { car.head[3] };
            let t = scale(r, car.lateral[w] / 4);
            let v = scale(car.velocity, 0xcccc);
            let vel = [t[0] + v[0], v[1], t[2] + v[2]];
            self.spawn_debris(stage, rng, pos, vel, 0, kind);
        }
    }

    /// `FUN_00426940`: the spray from a wet wheel (front wheels going
    /// forward, rear ones in reverse).
    fn splash(&mut self, car: &WheelCar, stage: &ParticleStage, rng: &mut GameLcg, w: usize) {
        let rear = w == 2 || w == 3;
        for i in 0..3 {
            let half = i * 0x8000;
            if (car.gear == 7) != rear {
                continue;
            }
            let pos = add(
                self.wheels[w],
                scale(sub(self.wheels_prev[w], self.wheels[w]), half),
            );
            let spd = car.speed;
            let mut lat = ((rand(rng) % 12 + 0x3f) * spd) / 240;
            let mut sz = qdiv(qmul(qmul(spd, 0x4ccc), 0x43_1168), 0x2d_0000);
            let r1 = rand(rng);
            let r2 = rand(rng);
            sz += (r1.wrapping_mul(sz) % 10) / (((r2 & 7) + 4) * 100);
            sz = sz.min(0x6666);
            let side = if w == 1 || w == 3 {
                lat = -lat;
                0x22
            } else {
                0x21
            };
            let noise = (rand(rng) - rand(rng)) / 4 + 0x1_0000;
            lat = qmul(lat, noise);
            let vel = add(car.velocity, scale(car.head[3], lat));
            let grey = Self::grey(stage);
            *self.next() = Particle {
                pos,
                prev: pos,
                origin: pos,
                vel,
                word_30: 0,
                size: sz,
                age: half,
                ticks: 0,
                sprites: Sprites::Splash,
                variant: side,
                life: 0,
                life_step: 0,
                kind: kind::SPLASH,
                grey,
                flags: 1,
            };
        }
    }

    /// `FUN_00427d20`: one tick of every live particle.
    fn integrate(&mut self, rng: &mut GameLcg) {
        for p in &mut self.ring {
            if !p.active() {
                continue;
            }
            p.prev = p.pos;
            match p.kind {
                kind::CLOD | kind::GRAVEL | kind::DRY_MUD | kind::RED_GRAVEL => {
                    let t = p.age;
                    let e = decay(qmul(AIR_FRICTION, t));
                    let f = qdiv(0x1_0000 - e, AIR_FRICTION);
                    let vinf = [0, TERMINAL_Y, 0];
                    p.pos = add(add(p.origin, scale(vinf, t)), scale(sub(p.vel, vinf), f));
                    if p.pos[1] - p.origin[1] < 0 {
                        p.vel = scale(p.vel, 0xcccc);
                        p.pos[1] = p.origin[1];
                        // `DAT_0054ba74` has no writer: the bounce never runs.
                        p.kill();
                    }
                }
                kind::SPLASH => {
                    p.vel = scale(p.vel, 0xcccc);
                    if p.age > 0x6_0000 {
                        let r1 = rand(rng);
                        let r2 = rand(rng);
                        p.size = qmul(p.size, (r1 - r2) / 40 + 0x1_0000);
                        let r3 = rand(rng);
                        let r4 = rand(rng);
                        p.size += (r3 - r4) / 40;
                    }
                    p.pos = add(p.pos, p.vel);
                    if p.pos[1] - p.origin[1] < -0x14_0000 {
                        p.kill();
                    }
                }
                kind::DUST
                | kind::DUST_16
                | kind::DUST_18
                | kind::SNOW
                | kind::SNOW_CLOUD
                | kind::SAND
                | 5
                | kind::FOLIAGE
                | 9 => {
                    p.pos = add(p.pos, p.vel);
                    p.vel = scale(p.vel, 0xf333);
                    if matches!(
                        p.kind,
                        kind::DUST | kind::DUST_16 | kind::DUST_18 | kind::SAND
                    ) {
                        p.vel[1] -= 0x34;
                    }
                    if p.pos[1] - p.origin[1] < -0x14_0000 {
                        p.kill();
                    }
                    if matches!(
                        p.kind,
                        kind::DUST
                            | kind::DUST_16
                            | kind::DUST_18
                            | kind::SAND
                            | kind::SNOW_CLOUD
                            | kind::SNOW
                    ) {
                        p.life = p.life.wrapping_sub(p.life_step);
                        if p.life < 0 {
                            p.kill();
                        }
                    }
                }
                _ => {}
            }
            p.age = p.age.wrapping_add(AGE_STEP);
            p.ticks += 1;
            if p.ticks > TICK_CAP {
                p.kill();
            }
        }
    }
}

/// `FUN_004281f0`: `1 - x + x^2/2 - x^3/6 + x^4/24 - x^5/120`.
fn decay(x: i32) -> i32 {
    let x2 = qmul(x, x);
    let x3 = qmul(x2, x);
    let x4 = qmul(x3, x);
    let x5 = qmul(x4, x);
    0x1_0000 - x + x2 / 2 - x3 / 6 + x4 / 24 - x5 / 120
}

/// The texture country codes (`FUN_00424160`).
pub const TEXTURE_COUNTRIES: [&str; 9] = ["NZ", "GR", "MC", "AU", "SW", "CO", "IN", "UK", "SP"];

/// The texture country: 7 on the special stage, 8 on a secret stage, else
/// `cfg[0x168] & 0xf`.
pub const fn texture_country(special: bool, secret: bool, country: u8) -> u8 {
    if special {
        7
    } else if secret {
        8
    } else {
        country & 0xf
    }
}

/// `FUN_00424160`: each frame's bitmap under `Game`, with the fallback the
/// loader tries when the first is missing.
pub fn texture_paths(sprites: Sprites, country: u8) -> Vec<Vec<String>> {
    let cc = TEXTURE_COUNTRIES
        .get(usize::from(country))
        .copied()
        .unwrap_or("NZ");
    let frames = |n: usize, from: usize, f: &dyn Fn(usize) -> Vec<String>| {
        (from..from + n).map(f).collect::<Vec<_>>()
    };
    match sprites {
        Sprites::None => Vec::new(),
        Sprites::Dust | Sprites::Dust18 => frames(8, 0, &|i| {
            let mut v = vec![format!("images\\{cc}\\Dust\\Dstcors{i}.bmp")];
            if sprites == Sprites::Dust {
                v.push(format!("images\\Dust\\Dstcors{i}.bmp"));
            }
            v
        }),
        Sprites::Dust16 => frames(8, 0, &|i| vec![format!("images\\Dust\\Dstcors{i}.bmp")]),
        Sprites::Sand => frames(8, 0, &|i| vec![format!("images\\NewSand\\IndSand{i}.bmp")]),
        Sprites::Snow => frames(16, 0, &|i| vec![format!("images\\NewSnow\\Snow{i:02}.bmp")]),
        Sprites::SnowCloud => frames(8, 0, &|i| vec![format!("images\\Snow2\\Sncld{i:02}.bmp")]),
        Sprites::ClotA => frames(8, 1, &|i| vec![format!("images\\clota{i}.bmp")]),
        Sprites::ClotB => frames(8, 1, &|i| vec![format!("images\\clotb{i}.bmp")]),
        Sprites::Splash => frames(32, 0, &|i| vec![format!("images\\Wsplsh9\\{i:02}.bmp")]),
        Sprites::GrassA | Sprites::GrassB => {
            frames(16, 0, &|i| vec![format!("images\\Grass1\\Grass{i:02}.bmp")])
        }
        Sprites::YellowGrass => frames(16, 0, &|i| {
            vec![format!("images\\YellowGrass\\Grass{i:02}.bmp")]
        }),
        Sprites::Leaves => frames(16, 0, &|i| {
            vec![format!("images\\Leaves\\Leaves{i:02}.bmp")]
        }),
        Sprites::Mulch => frames(16, 0, &|i| {
            vec![format!("images\\{cc}\\MlchScrb\\Mulch{i:02}.bmp")]
        }),
        Sprites::Gravel => frames(8, 1, &|i| {
            vec![
                format!("images\\{cc}\\Grav\\{cc}Grav{i:02}.bmp"),
                format!("images\\NzGravel\\NzGrav{i:02}.bmp"),
            ]
        }),
        Sprites::RedGravel => frames(8, 1, &|i| {
            let first = if country == 0 {
                format!("images\\NZ\\Grav\\NZGrav{i:02}.bmp")
            } else {
                format!("images\\AusGravel\\AusGrv{i:02}.bmp")
            };
            vec![first, format!("images\\Gr\\RedGrav\\GrRedGrav{i:02}.bmp")]
        }),
        Sprites::DryMud => frames(16, 0, &|i| {
            vec![format!("images\\{cc}\\DryMud\\dmud{i:02}.bmp")]
        }),
    }
}

/// Every sprite table [`texture_paths`] loads.
pub const SPRITE_TABLES: [Sprites; 18] = [
    Sprites::Dust,
    Sprites::Sand,
    Sprites::Snow,
    Sprites::SnowCloud,
    Sprites::Dust16,
    Sprites::Dust18,
    Sprites::ClotA,
    Sprites::ClotB,
    Sprites::Splash,
    Sprites::GrassA,
    Sprites::GrassB,
    Sprites::YellowGrass,
    Sprites::Leaves,
    Sprites::Mulch,
    Sprites::Gravel,
    Sprites::RedGravel,
    Sprites::DryMud,
    Sprites::None,
];

/// `FUN_00429c90`'s fixed grey latch for `FUN_00425be0`: night, or a storm
/// on three particular stages.
pub fn fixed_grey(
    night: bool,
    weather_type: u32,
    detail: u32,
    special: bool,
    country: u8,
    stage: u8,
) -> Option<u8> {
    if night {
        return Some(0x32);
    }
    if weather_type == 3 && detail > 1 {
        if special {
            return Some(0x6e);
        }
        match (country, stage) {
            (6, 2) => return Some(0x78),
            (7, 6) => return Some(0x32),
            _ => {}
        }
    }
    None
}

/// The camera-facing corners of a billboard (`FUN_004a4460` kind 2):
/// `centre + right * a + up * b`, v0 `(a0, b0)`, v1 `(a0, b1)`, v2
/// `(a1, b0)`, v3 `(a1, b1)`.
pub fn billboard_corners(b: &Billboard, right: [i32; 3], up: [i32; 3]) -> [[i32; 3]; 4] {
    let at = |a: i32, v: i32| add(add(b.centre, scale(right, a)), scale(up, v));
    [
        at(b.a.0, b.b.0),
        at(b.a.0, b.b.1),
        at(b.a.1, b.b.0),
        at(b.a.1, b.b.1),
    ]
}

/// `Draw_dirt_my_viewport` (100) and `Draw_dirt_other_viewport` (30).
pub const OWN_BUDGET: usize = 100;
pub const OTHER_BUDGET: usize = 30;
/// The dust family's size cap (`DAT_004ebfd8`, 2.0) and size floor
/// (`DAT_004ebe70`, 0.2).
const DUST_SIZE_CAP: i32 = 0x2_0000;
const DUST_SIZE_FLOOR: i32 = 0x3333;

/// One camera-facing sprite (`FUN_004a4330`'s packet): the centre, the
/// extents along the camera's right (`a`) and up (`b`) rows, the vertex
/// alpha and grey, the texture and whether U is mirrored; `distance` is the
/// sort key (drawn far to near).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Billboard {
    pub centre: [i32; 3],
    pub a: (i32, i32),
    pub b: (i32, i32),
    pub alpha: u8,
    pub grey: u8,
    pub sprites: Sprites,
    pub frame: usize,
    pub flip: bool,
    pub distance: i32,
}

/// What the billboard pass reads besides the particles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BillboardView {
    /// The owning car's camera position (`0x550498 + car * 0x60`, `+0x30`).
    pub camera: [i32; 3],
    /// The viewer is this car, and its own car is hidden (`DAT_00559e10`).
    pub own: bool,
    pub own_hidden: bool,
    /// `car +0xec`: the car's speed (the walk order).
    pub speed: i32,
    /// `cfg[0xb] != 0`: alpha multiplier 1, else 3.
    pub single_alpha: bool,
    pub night: bool,
    /// The splash side test (`FUN_00434ab0` yaw and the car's velocity):
    /// `true` when the camera sees the spray's far side.
    pub splash_side: bool,
}

impl CarParticles {
    /// `FUN_00423ec0`: the positions this frame draws, `t` (Q16) of the way
    /// from the last tick's position to this one's (a particle older than
    /// one tick).
    pub fn snapshot(&self, t: i32) -> Vec<Particle> {
        self.ring
            .iter()
            .map(|p| {
                let mut s = *p;
                if p.ticks > 1 {
                    s.pos = add(p.prev, scale(sub(p.pos, p.prev), t));
                }
                s
            })
            .collect()
    }

    /// `FUN_00428bc0`: the sprites of one car for one viewer, far to near;
    /// a particle whose frame runs past its table is killed (in `live`).
    pub fn billboards(&mut self, snapshot: &[Particle], view: &BillboardView) -> Vec<Billboard> {
        if view.own && view.own_hidden {
            return Vec::new();
        }
        let budget = if view.own { OWN_BUDGET } else { OTHER_BUDGET };
        let multiplier = if view.single_alpha { 1 } else { 3 };
        let head = self.cursor;
        let mut out = Vec::new();
        for k in 0..budget {
            let idx = if view.speed < 0x1_0000 {
                (head + 199 - k) % RING
            } else {
                (head + 199 + k - budget) % RING
            };
            let s = snapshot[idx];
            if !s.active() {
                continue;
            }
            let d = sub(view.camera, s.pos);
            let sq = (0..3).fold(0i32, |acc, i| acc.wrapping_add(qmul(d[i], d[i])));
            let distance = sqrt_q16(sq as u32) as i32;
            let near = || distance > 0x3_0000 && distance < 0x64_0000;
            let mut centre = s.pos;
            let ticks = s.ticks;
            let mut kill = false;
            let sprite = match s.kind {
                kind::DUST | kind::DUST_16 | kind::DUST_18 | kind::SAND | 9 => {
                    let h = (qmul(s.size, s.age) + DUST_SIZE_FLOOR)
                        .abs()
                        .min(DUST_SIZE_CAP);
                    centre[1] += h;
                    let mut alpha = (i32::from(s.life as u8) * multiplier).min(255);
                    if view.night {
                        alpha = alpha.max(0x50);
                    }
                    let frame = match s.kind {
                        9 => 0,
                        kind::SAND => (ticks * 8).min(7),
                        _ => ticks.min(7),
                    } as usize;
                    (alpha > 4).then_some(((-h, h), (-h, h), alpha as u8, frame, false))
                }
                kind::FOLIAGE => {
                    let h = s.size;
                    let k = if s.sprites == Sprites::Leaves {
                        0x1_0000
                    } else {
                        0xb333
                    };
                    let frame = ((i64::from(s.age) * i64::from(k)) >> 32) as i32;
                    if frame >= 16 {
                        kill = true;
                        None
                    } else {
                        near().then_some(((-h, h), (0, 2 * h), 0xff, frame as usize, false))
                    }
                }
                kind::SNOW | kind::SNOW_CLOUD => {
                    let h = qmul(s.size, s.age) + DUST_SIZE_FLOOR;
                    let frame = (s.age / 2) >> 16;
                    let limit = if s.kind == kind::SNOW { 16 } else { 8 };
                    if frame >= limit {
                        kill = true;
                        None
                    } else {
                        near().then_some((
                            (-h, h),
                            (0, 2 * h),
                            0x3c,
                            frame as usize,
                            s.flags & 2 != 0,
                        ))
                    }
                }
                kind::CLOD => {
                    let h = s.size;
                    if ticks / 3 >= 8 {
                        kill = true;
                        None
                    } else {
                        near().then_some(((-h, h), (-h, h), 0xff, (ticks / 3) as usize, false))
                    }
                }
                kind::GRAVEL | kind::RED_GRAVEL | kind::DRY_MUD => {
                    let h = s.size * 3 / 2;
                    let (frame, limit) = if s.kind == kind::DRY_MUD {
                        (ticks, 16)
                    } else {
                        (ticks >> 3, 8)
                    };
                    if frame >= limit {
                        kill = true;
                        None
                    } else {
                        near().then_some(((-h, h), (-h, h), 0xff, frame as usize, false))
                    }
                }
                kind::SPLASH => {
                    let h = s.size;
                    let frame = qmul(s.age, 0x2_8000) >> 16;
                    if frame >= 32 {
                        kill = true;
                        None
                    } else {
                        let marked = s.variant == 0x21;
                        let a = match (view.splash_side, marked) {
                            (false, true) => (-h, 3 * h),
                            (false, false) => (-3 * h, h),
                            (true, false) => (0, 4 * h),
                            (true, true) => (-4 * h, 0),
                        };
                        let flip = if view.splash_side { !marked } else { marked };
                        Some((a, (0, 6 * h), 0x6e, frame as usize, flip))
                    }
                }
                _ => None,
            };
            if kill {
                self.ring[idx].kill();
                continue;
            }
            if let Some((a, b, alpha, frame, flip)) = sprite
                && s.sprites != Sprites::None
                && out.len() < RING
            {
                out.push(Billboard {
                    centre,
                    a,
                    b,
                    alpha,
                    grey: s.grey,
                    sprites: s.sprites,
                    frame,
                    flip,
                    distance,
                });
            }
        }
        // `FUN_00427c30`: far to near.
        out.sort_by_key(|b| std::cmp::Reverse(b.distance));
        out
    }
}

/// Maximum concurrent collision sparks (`0x41a993`: `0x64` = 100).
pub const MAX_COLLISION_SPARKS: usize = 100;
/// Initial collision spark lifetime in 25 Hz ticks (`0x41af1b`: `0x08`).
pub const COLLISION_SPARK_TTL: u8 = 8;
/// Collision spark colour RGBA `(0xff, 0xf0, 0x96, 0xff)` (`0x41a925..0x41a93a`).
pub const COLLISION_SPARK_RGBA: [u8; 4] = [0xff, 0xf0, 0x96, 0xff];

/// One active 3D collision spark streak quad (`FUN_0041a960` / `0x41afb1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SparkQuad {
    pub corners: [[f32; 3]; 4],
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct CollisionSpark {
    pos: [i32; 3],
    vel: [i32; 3],
    ttl: u8,
}

/// 100-slot collision spark pool reconstructing `FUN_0041a900` (init),
/// `FUN_0041ab00` (emit burst on barrier/scenery impact), and `FUN_0041a960`
/// (per-tick integration & 3D streak emission).
#[derive(Clone, Debug, Default)]
pub struct CollisionSparkSystem {
    sparks: Vec<CollisionSpark>,
}

impl CollisionSparkSystem {
    /// Create an empty spark pool (`FUN_0041a900`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            sparks: Vec::with_capacity(MAX_COLLISION_SPARKS),
        }
    }

    /// Number of currently active sparks (`0..=MAX_COLLISION_SPARKS`).
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.sparks.len()
    }

    /// Emit a burst of sparks at `origin_q16` (`FUN_0041ab00`), clamping
    /// impact intensity to `[0x4000, 0x1_0000]` (`0x41ab43..0x41ab59`) and
    /// scaling spark spray velocity by `0x4ccc` (~0.3, `0x41ae7d`).
    pub fn emit(
        &mut self,
        origin_q16: [i32; 3],
        car_vel_q16: [i32; 3],
        severity: i32,
        rng: &mut GameLcg,
    ) {
        let clamped = severity.abs().clamp(0x4000, 0x1_0000);
        let room = MAX_COLLISION_SPARKS.saturating_sub(self.sparks.len());
        if room == 0 {
            return;
        }
        let burst = (6 + ((clamped >> 12) as usize)).min(room);
        let speed_scale = ((i64::from(clamped) * 0x4ccc) >> 16) as i32;
        for _ in 0..burst {
            let rx = ((rng.next_raw() & 0x7fff) as i32) - 0x4000;
            let ry = ((rng.next_raw() & 0x3fff) as i32) + 0x1000;
            let rz = ((rng.next_raw() & 0x7fff) as i32) - 0x4000;
            let vx = car_vel_q16[0]
                .wrapping_add(((i64::from(rx) * i64::from(speed_scale)) >> 14) as i32);
            let vy = car_vel_q16[1]
                .wrapping_add(((i64::from(ry) * i64::from(speed_scale)) >> 14) as i32);
            let vz = car_vel_q16[2]
                .wrapping_add(((i64::from(rz) * i64::from(speed_scale)) >> 14) as i32);
            self.sparks.push(CollisionSpark {
                pos: origin_q16,
                vel: [vx, vy, vz],
                ttl: COLLISION_SPARK_TTL,
            });
        }
    }

    /// Advance all active sparks by one 25 Hz tick (`FUN_0041a960`).
    pub fn tick(&mut self) {
        for spark in &mut self.sparks {
            spark.pos[0] = spark.pos[0].wrapping_add(spark.vel[0]);
            spark.pos[1] = spark.pos[1].wrapping_add(spark.vel[1]);
            spark.pos[2] = spark.pos[2].wrapping_add(spark.vel[2]);
            // Gravity pull on sparks.
            spark.vel[1] = spark.vel[1].wrapping_sub(0x0a00);
            spark.ttl = spark.ttl.saturating_sub(1);
        }
        self.sparks.retain(|s| s.ttl > 0);
    }

    /// Build 3D streak quads for all active sparks (`FUN_0041a960` / `0x41afb1`).
    #[must_use]
    pub fn streaks(&self, right: [i32; 3], forward: [i32; 3]) -> Vec<SparkQuad> {
        if self.sparks.is_empty() {
            return Vec::new();
        }
        let to_m = |v: i32| (f64::from(v) / 65536.0) as f32;
        let rx = to_m(right[0]);
        let ry = to_m(right[1]);
        let rz = to_m(right[2]);
        let fx = to_m(forward[0]);
        let fy = to_m(forward[1]);
        let fz = to_m(forward[2]);
        let half_w = 0.018f32;
        let mut out = Vec::with_capacity(self.sparks.len());
        for spark in &self.sparks {
            let p0 = spark.pos.map(to_m);
            let dx = to_m(spark.vel[0]);
            let dy = to_m(spark.vel[1]);
            let dz = to_m(spark.vel[2]);
            let p1 = [p0[0] + dx, p0[1] + dy, p0[2] + dz];
            let sx = dy * fz - dz * fy;
            let sy = dz * fx - dx * fz;
            let sz = dx * fy - dy * fx;
            let slen = (sx * sx + sy * sy + sz * sz).sqrt();
            let side = if slen > 1e-4 {
                [sx / slen * half_w, sy / slen * half_w, sz / slen * half_w]
            } else {
                [rx * half_w, ry * half_w, rz * half_w]
            };
            let alpha = ((u32::from(spark.ttl) * 255) / u32::from(COLLISION_SPARK_TTL)) as u8;
            out.push(SparkQuad {
                corners: [
                    [p0[0] - side[0], p0[1] - side[1], p0[2] - side[2]],
                    [p0[0] + side[0], p0[1] + side[1], p0[2] + side[2]],
                    [p1[0] + side[0], p1[1] + side[1], p1[2] + side[2]],
                    [p1[0] - side[0], p1[1] - side[1], p1[2] - side[2]],
                ],
                alpha,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trig() -> [i32; TRIG_TABLE_LEN] {
        crate::fixed_point::build_sine_table()
    }

    fn tables(dust: u8) -> Vec<[u8; 120]> {
        let mut t = vec![[0u8; 120]; 16];
        t[2][5] = dust;
        t
    }

    fn stage(tables: &[[u8; 120]]) -> ParticleStage<'_> {
        static SOLID: [u8; 120] = [0; 120];
        ParticleStage {
            tables,
            solid: &SOLID,
            night: false,
            weather_type: 0,
            country: 3,
            lit: [0xc0, 0xc0, 0xc0],
            fixed_grey: None,
            blend: DEFAULT_BLEND,
            mark_surfaces: &MARK_SURFACES,
            narrow_marks: false,
        }
    }

    /// Surface 5 marks, in a mid grey.
    static MARK_SURFACES: [[u8; 8]; 120] = {
        let mut t = [[0u8; 8]; 120];
        t[5] = [1, 0, 0, 0, 0x80, 0x80, 0x80, 0];
        t
    };

    fn car() -> WheelCar {
        WheelCar {
            velocity: [0, 0, 0x8_0000],
            speed: 0x8_0000,
            contact: [true; 4],
            surfaces: [5; 4],
            head: [
                [0x1_0000, 0, 0],
                [0, 0, 0x1_0000],
                [0x1_0000, 0, 0],
                [0, 0, 0x1_0000],
            ],
            gear: 2,
            ..WheelCar::default()
        }
    }

    #[test]
    fn the_dust_accumulators_converge_and_wet_clears_them() {
        let t = tables(10);
        let st = stage(&t);
        let mut c = CarParticles::default();
        for _ in 0..200 {
            c.accumulate(&car(), &st, 0);
        }
        // `x = (x + 10) * 45 / 50` settles at 81 in integers.
        assert_eq!(c.acc[0].dust, 81);
        let wet = ParticleStage {
            weather_type: 3,
            ..st
        };
        c.accumulate(&car(), &wet, 0);
        assert_eq!(c.acc[0].dust, 0, "no dry dust in the rain");
        let mut lifted = car();
        lifted.contact[0] = false;
        c.acc[0].mud = 50;
        c.accumulate(&lifted, &st, 0);
        assert_eq!(
            (c.acc[0].mud, c.acc[0].dust),
            (40, 0),
            "the mud decays in the air"
        );
    }

    #[test]
    fn a_dusty_surface_puts_dust_behind_the_rear_wheels() {
        let t = tables(40);
        let st = stage(&t);
        let mut system = ParticleSystem::new(1, true);
        let mut rng = GameLcg::new(7);
        let trig = trig();
        for _ in 0..30 {
            system.spawn(&[car()], &st, &mut rng, &trig);
            system.update(&mut rng);
        }
        let live: Vec<&Particle> = system.cars[0].ring.iter().filter(|p| p.active()).collect();
        assert!(!live.is_empty());
        assert!(
            live.iter()
                .all(|p| p.kind == kind::DUST && p.sprites == Sprites::Dust)
        );
        assert!(live.iter().any(|p| p.flags & 2 != 0) && live.iter().any(|p| p.flags & 2 == 0));
        // The dust drifts with the car and sinks slowly.
        assert!(live.iter().all(|p| p.vel[2] > 0));
    }

    #[test]
    fn disabled_particles_spawn_nothing_and_leave_the_generator() {
        let t = tables(40);
        let st = stage(&t);
        let mut system = ParticleSystem::new(1, false);
        let mut rng = GameLcg::new(7);
        system.spawn(&[car()], &st, &mut rng, &trig());
        assert_eq!(rng, GameLcg::new(7));
        assert!(system.cars[0].ring.iter().all(|p| !p.active()));
    }

    #[test]
    fn the_ring_overwrites_its_oldest_slot() {
        let t = tables(0);
        let st = stage(&t);
        let mut c = CarParticles::default();
        for i in 0..(RING + 3) {
            c.spawn_dust(&st, [i as i32, 0, 0], [0; 3], 10, 0, 0, kind::DUST, true);
        }
        assert_eq!(c.cursor, 3);
        assert_eq!(c.ring[2].pos[0], (RING + 2) as i32);
        assert_eq!(c.ring[3].pos[0], 3);
    }

    #[test]
    fn integration_drifts_dust_and_drops_clods() {
        let t = tables(0);
        let st = stage(&t);
        let mut c = CarParticles::default();
        let mut rng = GameLcg::new(1);
        c.spawn_dust(&st, [0; 3], [0x1_0000, 0, 0], 9, 0, 0, kind::DUST, true);
        c.integrate(&mut rng);
        let p = c.ring[0];
        assert_eq!(p.pos[0], 0x1_0000);
        assert_eq!(p.vel[0], 0xf333, "the velocity damps by 0.95");
        assert_eq!(p.vel[1], -0x34, "dust sinks");
        assert_eq!((p.life, p.age, p.ticks), (6, AGE_STEP, 1));
        for _ in 0..3 {
            c.integrate(&mut rng);
        }
        assert!(!c.ring[0].active(), "its life ran out");
        // A clod thrown upward falls back to its origin height and dies.
        let mut c = CarParticles::default();
        c.spawn_debris(&st, &mut rng, [0; 3], [0, 0x2_0000, 0], 0, kind::CLOD);
        let mut ticks = 0;
        while c.ring[0].active() && ticks < 100 {
            c.integrate(&mut rng);
            ticks += 1;
        }
        assert!(ticks > 2 && ticks < 100);
    }

    #[test]
    fn a_sliding_wheel_lays_joined_marks_that_fade_after_their_life() {
        let t = tables(0);
        let st = stage(&t);
        let mut system = ParticleSystem::new(1, true);
        let mut rng = GameLcg::new(3);
        let trig = trig();
        let mut sliding = car();
        sliding.upright = true;
        sliding.normals = [[0, 0x1_0000, 0]; 4];
        sliding.lateral = [0x2_0000; 4];
        for tick in 0..20 {
            for p in &mut sliding.probes {
                p[2] = tick * 0x8_0000;
            }
            system.spawn(&[sliding], &st, &mut rng, &trig);
        }
        let c = &system.cars[0];
        assert!(c.marking.iter().all(|&m| m), "every wheel is marking");
        let tris = c.mark_triangles([200, 200, 200]);
        assert!(!tris.is_empty());
        // The first marks need ten ticks of contact; later ones join up.
        assert_eq!(c.marks[0].iter().filter(|m| m.valid).count(), 9);
        let ([a, b, _], colours) = tris[0];
        assert_ne!(a, b);
        // A 2.0 slide: (2.0 - 0.6) / 2 of full alpha.
        assert_eq!(colours[0] >> 24, 178);
        // Night lays none.
        let night = ParticleStage { night: true, ..st };
        let mut dark = ParticleSystem::new(1, true);
        for _ in 0..20 {
            dark.spawn(&[sliding], &night, &mut rng, &trig);
        }
        assert!(
            dark.cars[0]
                .marks
                .iter()
                .all(|r| r.iter().all(|m| !m.valid))
        );
        // Past their life the marks fade out.
        let mut old = c.clone();
        for ring in &mut old.marks {
            for m in ring.iter_mut() {
                m.age += MARK_LIFE + MARK_LIFE / MARK_RATE + 1;
            }
        }
        assert!(old.mark_triangles([200, 200, 200]).is_empty());
    }

    #[test]
    fn billboards_follow_the_type_rules_and_sort_far_to_near() {
        let t = tables(0);
        let st = stage(&t);
        let mut c = CarParticles::default();
        c.spawn_dust(
            &st,
            [0, 0, 0x10_0000],
            [0; 3],
            100,
            0x1_0000,
            FOLLOW_SIZE_GROWTH,
            kind::DUST,
            true,
        );
        c.spawn_dust(
            &st,
            [0, 0, 0x40_0000],
            [0; 3],
            1,
            0x1_0000,
            FOLLOW_SIZE_GROWTH,
            kind::DUST,
            true,
        );
        c.spawn_dust(
            &st,
            [0, 0, 0x20_0000],
            [0; 3],
            50,
            0x1_0000,
            FOLLOW_SIZE_GROWTH,
            kind::DUST,
            true,
        );
        let view = BillboardView {
            own: true,
            ..BillboardView::default()
        };
        let snap = c.snapshot(0x1_0000);
        let list = c.billboards(&snap, &view);
        assert_eq!(list.len(), 2, "a life of 1 times 3 is too faint");
        assert!(list[0].distance > list[1].distance);
        assert_eq!(list[0].alpha, 150);
        let h = qmul(FOLLOW_SIZE_GROWTH, 0x1_0000) + DUST_SIZE_FLOOR;
        assert_eq!((list[1].a, list[1].centre[1]), ((-h, h), h));
        // The own car hidden (in-car view): nothing.
        let hidden = BillboardView {
            own_hidden: true,
            ..view
        };
        assert!(c.billboards(&snap, &hidden).is_empty());
    }

    #[test]
    fn collision_sparks_cap_at_100_and_decay_over_8_ticks() {
        let mut rng = GameLcg::new(19);
        let mut sparks = CollisionSparkSystem::new();
        for _ in 0..20 {
            sparks.emit([0, 0x1_0000, 0], [0x4_0000, 0, 0], 0x1_0000, &mut rng);
        }
        assert_eq!(sparks.active_count(), MAX_COLLISION_SPARKS);
        let quads = sparks.streaks([0x1_0000, 0, 0], [0, 0, 0x1_0000]);
        assert_eq!(quads.len(), MAX_COLLISION_SPARKS);
        assert!(quads.iter().all(|q| q.alpha == 255));
        for _ in 0..COLLISION_SPARK_TTL {
            sparks.tick();
        }
        assert_eq!(sparks.active_count(), 0);
        assert!(
            sparks
                .streaks([0x1_0000, 0, 0], [0, 0, 0x1_0000])
                .is_empty()
        );
    }
}
