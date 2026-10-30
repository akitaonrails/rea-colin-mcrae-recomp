//! Night headlights, rear/brake/reverse/tail lamp overlays, additive lamp flare
//! billboards, electrics flicker, night pod-lamp breakage, and the night road
//! light pool (`FUN_00449e60`, `FUN_0044a770`, `FUN_0044a950`, `FUN_0044add0`,
//! `FUN_0044b000`, `FUN_0044b110`, `FUN_0044b240`, `FUN_0044b600`,
//! `FUN_0044bbc0`, `FUN_0044c610`, `FUN_0044c8c0`, `FUN_004a8750`,
//! `FUN_004a8a60`, `FUN_004a9320`, `FUN_004a9440`, `FUN_004a9490`).
//!
//! All formulas and constants below are reconstructed from disassembly of the
//! official EU executable (`Rally.exe`), not original source.
//! Evidence: `rea.official-eu.car-headlights`, `.work/headlights-notes.md`.

use crate::engine_listener::camera_heading;
use crate::game_rng::GameLcg;
use crate::legacy_math::{cos_q16, signed_arcsine_phase, sin_q16, sqrt_q16};

/// Number of car models in the static lamp tables (`0..14`).
pub const CAR_LAMP_MODEL_COUNT: usize = 14;
/// Byte length of the contiguous `.data` span `0x4ef5d8..0x4f0d08`
/// (`car_lamp_tables` in the static-tables profile).
pub const CAR_LAMP_TABLES_LEN: usize = 0x1730;

/// Night stage ambient floor byte (`0x28`, RGB `0x282828`).
pub const NIGHT_AMBIENT_BYTE: u8 = 0x28;
/// Default night road light pool radius (`DAT_005028dc = 22.0` in Q16.16).
pub const POOL_RADIUS_Q16: i32 = 0x16_0000;
/// Local position of a car's road light pool node in the car root frame
/// (`0xb78188 = (21.0, 1.0, 0.0)` in Q16.16).
pub const POOL_LOCAL_OFFSET_Q16: [i32; 3] = [0x15_0000, 0x1_0000, 0];

/// Lit brake overlay RGBA tuple (`0x55ba68`).
pub const COLOUR_BRAKE_LIT: [u8; 4] = [0xff, 0x0a, 0x0a, 0x80];
/// Lit tail overlay RGBA tuple (`0x55bab4`).
pub const COLOUR_TAIL_LIT: [u8; 4] = [0xb4, 0x0a, 0x0a, 0x80];
/// Lit white (reverse / MK2 tail / mode-2 third) overlay RGBA tuple (`0x55bb80`).
pub const COLOUR_WHITE_LIT: [u8; 4] = [0xc8, 0xc8, 0xc8, 0x80];
/// Unlit red overlay RGBA tuple (`0x55bab8`, used while electrics flicker dims).
pub const COLOUR_RED_UNLIT: [u8; 4] = [0x80, 0x05, 0x05, 0x78];
/// Unlit white overlay RGBA tuple (`0x55af80`, used while electrics flicker dims).
pub const COLOUR_WHITE_UNLIT: [u8; 4] = [0x80, 0x80, 0x80, 0x78];

const ONE_Q16: i32 = 0x1_0000;
const HALF_Q16: i32 = 0x8000;
const QUARTER_Q16: i32 = 0x4000;

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

fn qdiv(a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    ((i64::from(a) << 16) / i64::from(b)) as i32
}

/// Truncate a Q16.16 value toward zero (`cdq; and edx,0xffff; add eax,edx; sar eax,16`).
fn rt(x: i32) -> i32 {
    x.wrapping_add(if x < 0 { 0xffff } else { 0 }) >> 16
}

fn add3(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
    ]
}

fn sub3(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [
        a[0].wrapping_sub(b[0]),
        a[1].wrapping_sub(b[1]),
        a[2].wrapping_sub(b[2]),
    ]
}

fn scale3(v: [i32; 3], s: i32) -> [i32; 3] {
    [qmul(v[0], s), qmul(v[1], s), qmul(v[2], s)]
}

/// Normalize a Q16.16 3-vector using `sqrt_q16` and integer reciprocal `2^32 / len`.
/// Zero length returns `[0, 0, 0]`.
#[must_use]
pub fn normalize_q16(v: [i32; 3]) -> [i32; 3] {
    let sum = qmul(v[0], v[0])
        .wrapping_add(qmul(v[1], v[1]))
        .wrapping_add(qmul(v[2], v[2]));
    let len = sqrt_q16(sum as u32) as i32;
    if len == 0 {
        return [0; 3];
    }
    let recip = (0x1_0000_0000_i64 / i64::from(len)) as i32;
    scale3(v, recip)
}

/// Row-vector multiply by the 3x3 rotational part of a 4x4 Q16 matrix (`FUN_0048dfe0`).
#[must_use]
pub fn row_mul_3x3(v: [i32; 3], m: &[i32; 16]) -> [i32; 3] {
    std::array::from_fn(|c| {
        qmul(v[0], m[c])
            .wrapping_add(qmul(v[1], m[4 + c]))
            .wrapping_add(qmul(v[2], m[8 + c]))
    })
}

/// Row-vector multiply by the transpose of the 3x3 rotational part (`FUN_0048e100`).
#[must_use]
pub fn row_mul_3x3_transpose(v: [i32; 3], m: &[i32; 16]) -> [i32; 3] {
    std::array::from_fn(|r| {
        let base = r * 4;
        qmul(v[0], m[base])
            .wrapping_add(qmul(v[1], m[base + 1]))
            .wrapping_add(qmul(v[2], m[base + 2]))
    })
}

/// Affine point transform by a 4x4 row-vector Q16 matrix (`v * R + T`).
#[must_use]
pub fn transform_point_q16(v: [i32; 3], m: &[i32; 16]) -> [i32; 3] {
    add3(row_mul_3x3(v, m), [m[12], m[13], m[14]])
}

/// Which flare texture a lamp sprite uses (`0x55ba6c` vs `0x55ba70`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LampFlareTexture {
    /// `images\headglow.bmp` (`DAT_0055ba6c`).
    Headglow,
    /// `images\bglow2.bmp` (`DAT_0055ba70`).
    Bglow2,
}

impl LampFlareTexture {
    /// Legacy relative asset path under `Game/`.
    #[must_use]
    pub const fn store_path(self) -> &'static str {
        match self {
            Self::Headglow => "Game/Images/HEADGLOW.BMP",
            Self::Bglow2 => "Game/Images/BGLOW2.BMP",
        }
    }
}

/// One car model's static lamp geometry and counts from `0x4ef5d8..0x4f0d08`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarModelLampSpec {
    /// `0x4ef5d8`: third-lamp face vertices (`4 × 12 B`).
    pub third_verts: [[i32; 3]; 4],
    /// `0x4ef878`: third-lamp vertex count.
    pub third_count: u8,
    /// `0x4ef888`: third-lamp mode (`0` none, `1` brake single, `2` tail mirrored pair).
    pub third_mode: u8,
    /// `0x4ef898`: brake-lamp face vertices (`8 × 12 B`).
    pub brake_verts: [[i32; 3]; 8],
    /// `0x4efdd8`: brake-lamp vertex count.
    pub brake_count: u8,
    /// `0x4efde8`: tail-lamp face vertices (`8 × 12 B`).
    pub tail_verts: [[i32; 3]; 8],
    /// `0x4f0328`: tail-lamp vertex count.
    pub tail_count: u8,
    /// `0x4f0338`: reverse-lamp face vertices (`6 × 12 B`).
    pub reverse_verts: [[i32; 3]; 6],
    /// `0x4f0728`: reverse-lamp vertex count.
    pub reverse_count: u8,
    /// `0x4f0738`: front lamp offsets (`0..base` main headlamps, `base..base+cnt` night pod lamps).
    pub front_offsets: [[i32; 3]; 8],
    /// `0x4f0c78`: `base` = main headlamp count.
    pub main_headlamp_count: u8,
    /// `0x4f0c88`: `cnt` = night pod lamp count (breakable).
    pub pod_lamp_count: u8,
    /// `0x4f0c98`: when non-zero, braking skips the tail overlay.
    pub skip_tail_when_braking: i32,
    /// `0x4f0cd0`: headlamp flare alpha scale in Q16.16.
    pub headlamp_alpha_scale_q16: i32,
}

impl Default for CarModelLampSpec {
    fn default() -> Self {
        Self {
            third_verts: [[0; 3]; 4],
            third_count: 0,
            third_mode: 0,
            brake_verts: [[0; 3]; 8],
            brake_count: 0,
            tail_verts: [[0; 3]; 8],
            tail_count: 0,
            reverse_verts: [[0; 3]; 6],
            reverse_count: 0,
            front_offsets: [[0; 3]; 8],
            main_headlamp_count: 0,
            pod_lamp_count: 0,
            skip_tail_when_braking: 0,
            headlamp_alpha_scale_q16: ONE_Q16,
        }
    }
}

impl CarModelLampSpec {
    /// The model's breakable night pod-lamp offsets in body space (`base..base+cnt`).
    #[must_use]
    pub fn pod_lamp_offsets(&self) -> Vec<[i32; 3]> {
        let base = usize::from(self.main_headlamp_count);
        let cnt = usize::from(self.pod_lamp_count);
        let end = base.saturating_add(cnt).min(self.front_offsets.len());
        if base >= end {
            return Vec::new();
        }
        self.front_offsets[base..end].to_vec()
    }
}

/// All 14 car models' lamp specs parsed from the `car_lamp_tables` static block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarLampTables {
    pub models: [CarModelLampSpec; CAR_LAMP_MODEL_COUNT],
}

impl CarLampTables {
    /// Parse the `0x1730`-byte `.data` span `0x4ef5d8..0x4f0d08`.
    #[must_use]
    pub fn from_block(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != CAR_LAMP_TABLES_LEN {
            return None;
        }
        let i32_at = |off: usize| -> i32 {
            i32::from_le_bytes(bytes[off..off + 4].try_into().expect("slice len 4"))
        };
        let vec3_at = |off: usize| -> [i32; 3] { [i32_at(off), i32_at(off + 4), i32_at(off + 8)] };
        let models = std::array::from_fn(|m| {
            let third_verts = std::array::from_fn(|k| vec3_at(m * 0x30 + k * 12));
            let third_count = bytes[0x02a0 + m];
            let third_mode = bytes[0x02b0 + m];
            let brake_verts = std::array::from_fn(|k| vec3_at(0x02c0 + m * 0x60 + k * 12));
            let brake_count = bytes[0x0800 + m];
            let tail_verts = std::array::from_fn(|k| vec3_at(0x0810 + m * 0x60 + k * 12));
            let tail_count = bytes[0x0d50 + m];
            let reverse_verts = std::array::from_fn(|k| vec3_at(0x0d60 + m * 0x48 + k * 12));
            let reverse_count = bytes[0x1150 + m];
            let front_offsets = std::array::from_fn(|k| vec3_at(0x1160 + m * 0x60 + k * 12));
            let main_headlamp_count = bytes[0x16a0 + m];
            let pod_lamp_count = bytes[0x16b0 + m];
            let skip_tail_when_braking = i32_at(0x16c0 + m * 4);
            let headlamp_alpha_scale_q16 = i32_at(0x16f8 + m * 4);
            CarModelLampSpec {
                third_verts,
                third_count,
                third_mode,
                brake_verts,
                brake_count,
                tail_verts,
                tail_count,
                reverse_verts,
                reverse_count,
                front_offsets,
                main_headlamp_count,
                pod_lamp_count,
                skip_tail_when_braking,
                headlamp_alpha_scale_q16,
            }
        });
        Some(Self { models })
    }

    /// Look up model `m` (`0..14`).
    #[must_use]
    pub fn model(&self, m: u8) -> Option<&CarModelLampSpec> {
        self.models.get(usize::from(m))
    }
}

/// Centroid helper in `FUN_00449e60`: wrapping Q16 sum of `verts` multiplied by
/// `n ? qdiv(1.0, n << 16) : -1` (preserving quirk Q10 where `n == 0` uses `-1`).
#[must_use]
pub fn centroid_449e60(verts: &[[i32; 3]]) -> [i32; 3] {
    let mut sum = [0i32; 3];
    for &v in verts {
        sum = add3(sum, v);
    }
    let n = verts.len();
    let inv = if n != 0 {
        qdiv(ONE_Q16, (n as i32) << 16)
    } else {
        -1
    };
    scale3(sum, inv)
}

/// Mirror a body-frame vector across the Z axis (`(x, y, -z)`).
#[must_use]
pub const fn mirror_z(v: [i32; 3]) -> [i32; 3] {
    [v[0], v[1], v[2].wrapping_neg()]
}

/// Per-slot rear sprite offsets (`0x55af88`, `0x55bc50`, `0x55bac0`, `0x55bb88`)
/// and colour/texture pointers (`B + 0xd8..0xfc`) initialised by `FUN_00449e60`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarLampSlotInit {
    pub model: u8,
    /// `0x55af88`: brake rear sprite offsets `[side 0, side 1]`.
    pub brake_offsets: [[i32; 3]; 2],
    /// `0x55bc50`: tail rear sprite offsets `[side 0, side 1]`.
    pub tail_offsets: [[i32; 3]; 2],
    /// `0x55bac0`: reverse rear sprite offsets `[side 0, side 1]`.
    pub reverse_offsets: [[i32; 3]; 2],
    /// `0x55bb88`: third rear sprite offsets `[0, 1]`.
    pub third_offsets: [[i32; 3]; 2],
    /// `B+0xd8..+0xec`: overlay RGBA colours `[d8, dc, e0, e4, e8, ec]`.
    pub colours_d8_ec: [[u8; 4]; 6],
    /// `B+0xf0..+0xfc`: flare textures `[f0 (brake), f4 (tail), f8 (reverse), fc (headlamp)]`.
    pub textures_f0_fc: [LampFlareTexture; 4],
    /// Corner half-extents in Q16: `[brake, tail, reverse, head_a, head_b]`.
    pub sprite_half_extents_q16: [i32; 5],
}

impl CarLampSlotInit {
    /// `FUN_00449e60` per-car initialization for model `m` (`0..14`).
    /// For `m == 0`, the brake centroid sums both `brake_verts[0]` and
    /// `tail_verts[0]`, and `tail_offsets` copies `brake_offsets`.
    #[must_use]
    pub fn from_tables(tables: &CarLampTables, m: u8, borrowers: bool) -> Option<Self> {
        let spec = tables.model(m)?;
        let n_brake = usize::from(spec.brake_count).min(spec.brake_verts.len());
        let n_tail = usize::from(spec.tail_count).min(spec.tail_verts.len());
        let n_rev = usize::from(spec.reverse_count).min(spec.reverse_verts.len());
        let n_third = usize::from(spec.third_count).min(spec.third_verts.len());

        let brake0 = if m == 0 {
            let mut combined = Vec::with_capacity(n_brake + n_tail);
            combined.extend_from_slice(&spec.brake_verts[..n_brake]);
            combined.extend_from_slice(&spec.tail_verts[..n_tail]);
            centroid_449e60(&combined)
        } else {
            centroid_449e60(&spec.brake_verts[..n_brake])
        };
        let brake_offsets = [brake0, mirror_z(brake0)];

        let reverse0 = centroid_449e60(&spec.reverse_verts[..n_rev]);
        let reverse_offsets = [reverse0, mirror_z(reverse0)];

        let tail_offsets = if m == 0 {
            brake_offsets
        } else {
            let tail0 = centroid_449e60(&spec.tail_verts[..n_tail]);
            [tail0, mirror_z(tail0)]
        };

        let mut third_offsets = [[0; 3]; 2];
        if spec.third_mode != 0 {
            let third0 = centroid_449e60(&spec.third_verts[..n_third]);
            third_offsets[0] = third0;
            if spec.third_mode == 2 {
                third_offsets[1] = mirror_z(third0);
            }
        }

        let (colours_d8_ec, textures_f0_fc) = if m != 5 {
            (
                [
                    COLOUR_BRAKE_LIT,
                    COLOUR_TAIL_LIT,
                    COLOUR_WHITE_LIT,
                    COLOUR_RED_UNLIT,
                    COLOUR_RED_UNLIT,
                    COLOUR_WHITE_UNLIT,
                ],
                [
                    LampFlareTexture::Bglow2,
                    LampFlareTexture::Bglow2,
                    LampFlareTexture::Headglow,
                    LampFlareTexture::Headglow,
                ],
            )
        } else {
            // Model 5 (MK2 Escort): white tail lit/unlit and headglow for tail.
            (
                [
                    COLOUR_BRAKE_LIT,
                    COLOUR_WHITE_LIT,
                    COLOUR_WHITE_LIT,
                    COLOUR_RED_UNLIT,
                    COLOUR_WHITE_UNLIT,
                    COLOUR_WHITE_UNLIT,
                ],
                [
                    LampFlareTexture::Bglow2,
                    LampFlareTexture::Headglow,
                    LampFlareTexture::Headglow,
                    LampFlareTexture::Headglow,
                ],
            )
        };

        let mut extents = [0x3333, 0x2666, 0x2666, 0x4ccc, 0];
        if borrowers {
            for e in &mut extents[..4] {
                *e = qmul(*e, HALF_Q16);
            }
        }
        extents[4] = qmul(extents[3], HALF_Q16);

        Some(Self {
            model: m,
            brake_offsets,
            tail_offsets,
            reverse_offsets,
            third_offsets,
            colours_d8_ec,
            textures_f0_fc,
            sprite_half_extents_q16: extents,
        })
    }

    /// Look up the body-frame offset for rear sprite `id` (`0..=7`).
    #[must_use]
    pub const fn rear_offset(&self, id: usize) -> [i32; 3] {
        match id {
            0 => self.brake_offsets[0],
            1 => self.brake_offsets[1],
            2 => self.tail_offsets[0],
            3 => self.tail_offsets[1],
            4 => self.reverse_offsets[0],
            5 => self.reverse_offsets[1],
            6 => self.third_offsets[0],
            7 => self.third_offsets[1],
            _ => [0; 3],
        }
    }
}

/// Fill the 23-entry random table `0x82ae84` as `FUN_0048bc00` does when weather
/// effects are enabled (`t_k = qdiv(rand() << 16, 0x7fff_0000)`).
#[must_use]
pub fn fill_flicker_rand_table_48bc00(rng: &mut GameLcg) -> [i32; 23] {
    std::array::from_fn(|_| {
        let r = (rng.next_raw() as i32) << 16;
        qdiv(r, 0x7fff_0000)
    })
}

/// Per-car electrics flicker state (`0x55aed0[slot]` countdown and `0x55bda4[slot]` dim flag).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ElectricsFlickerState {
    pub countdown_55aed0: i16,
    pub dim_flag_55bda4: bool,
}

impl ElectricsFlickerState {
    /// `FUN_0044b000(car)`: step the electrics flicker countdown for one viewport test.
    ///
    /// When `electrics_damage_810 == 0`, the lights remain full brightness (`dim_flag = false`).
    /// When `> 0`, counts down `> 0` with `dim_flag = false`, then dims for 5 calls
    /// (`0, -1, -2, -3`, and the reload call at `<= -4`), reloading from `rand_table_82ae84`.
    pub fn step_44b000(
        &mut self,
        electrics_damage_810: i32,
        rand_table_82ae84: &[i32; 23],
        rand_index_82a4d0: &mut usize,
    ) {
        self.dim_flag_55bda4 = false;
        if electrics_damage_810 == 0 {
            return;
        }
        let c = self.countdown_55aed0;
        if c > 0 {
            self.countdown_55aed0 = c - 1;
            return;
        }
        self.dim_flag_55bda4 = true;
        if c > -4 {
            self.countdown_55aed0 = c - 1;
            return;
        }
        let idx = *rand_index_82a4d0;
        let t = rand_table_82ae84.get(idx).copied().unwrap_or(0);
        *rand_index_82a4d0 = if idx + 1 < 23 { idx + 1 } else { 0 };
        let span = 0x1_3333_i32.wrapping_sub(electrics_damage_810);
        let reload = rt(qmul(qmul(0x01f4_0000, t), span));
        self.countdown_55aed0 = reload as i16;
    }
}

/// Output of `FUN_0044add0` for one viewport (`B.c0[view]`, `B.c8[view]`, `B.d0[view]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LampViewportVisibility {
    /// `B.c0[view]`: camera is behind the car (or at 90 degrees); rear lamps visible.
    pub rear_visible_c0: bool,
    /// `B.c8[view]`: camera is ahead of the car (`ang > 90.0`); front lamps visible.
    pub front_visible_c8: bool,
    /// `B.d0[view]`: angular intensity in Q16.16 (`w * 0x2d0`, or `w * 0x168` when dimmed).
    pub intensity_d0: i32,
}

/// `FUN_0044add0(car, view)`: step electrics flicker, then compute camera-side
/// visibility and lamp intensity from the camera yaw (`FUN_00434ab0`) and the
/// car's live forward row (`car + 0x258`).
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn side_test_44add0(
    flicker: &mut ElectricsFlickerState,
    electrics_damage_810: i32,
    rand_table_82ae84: &[i32; 23],
    rand_index_82a4d0: &mut usize,
    braking_6f4: bool,
    reverse_6fc: bool,
    lights_on_6f8: bool,
    camera_world_q16: &[i32; 16],
    car_forward_258: [i32; 3],
) -> LampViewportVisibility {
    flicker.step_44b000(electrics_damage_810, rand_table_82ae84, rand_index_82a4d0);
    if !(braking_6f4 || reverse_6fc || lights_on_6f8) {
        return LampViewportVisibility::default();
    }
    let a = (camera_heading(camera_world_q16[8], camera_world_q16[10]).unwrap_or(0) as u16) & 0xfff;
    let f = [sin_q16(a).raw(), 0, cos_q16(a).raw()];
    let d = qmul(f[0], car_forward_258[0])
        .wrapping_add(qmul(f[1], car_forward_258[1]))
        .wrapping_add(qmul(f[2], car_forward_258[2]));
    let ang = if d.unsigned_abs() >= ONE_Q16 as u32 {
        if d > 0 { 0 } else { 0x00b4_0000 }
    } else {
        (0x400 - i32::from(signed_arcsine_phase(d).unwrap_or(0))) * 0x1680
    };
    let ninety = 0x005a_0000;
    let (rear_visible_c0, front_visible_c8, w) = if ang > ninety {
        (false, true, ang - ninety)
    } else {
        (true, false, ninety - ang)
    };
    let scale = if flicker.dim_flag_55bda4 {
        0x168
    } else {
        0x2d0
    };
    LampViewportVisibility {
        rear_visible_c0,
        front_visible_c8,
        intensity_d0: qmul(w, scale),
    }
}

/// `FUN_0044b110`: transform one body-frame lamp offset into world space.
#[must_use]
pub fn transform_lamp_offset_44b110(
    offset: [i32; 3],
    body_matrix_94: &[i32; 16],
    container_matrix_94: &[i32; 16],
    base_world: [i32; 3],
    borrowers: bool,
    scale_550d50: i32,
    car_height_30: i32,
) -> [i32; 3] {
    let mut v = offset;
    if borrowers {
        let s = qmul(HALF_Q16, scale_550d50).wrapping_add(HALF_Q16);
        v = scale3(v, s);
        v[1] = v[1].wrapping_add(qmul(s.wrapping_sub(HALF_Q16), car_height_30));
    }
    let t = row_mul_3x3(v, body_matrix_94);
    add3(row_mul_3x3(t, container_matrix_94), base_world)
}

/// Inputs describing a car's pose and state for `FUN_0044a950` and `FUN_0044bbc0`.
#[derive(Clone, Copy, Debug)]
pub struct CarLampPoseInput {
    /// Body node (`car[+4]`, node id 5) local matrix (`+0x94`).
    pub body_local_94: [i32; 16],
    /// Body node (`car[+4]`) world matrix (`+0xd4`).
    pub body_world_d4: [i32; 16],
    /// Container node (`car[+8]`, node id 1) local/world matrix (`+0x94`).
    pub container_matrix_94: [i32; 16],
    /// Live forward row (`car + 0x258`).
    pub live_forward_258: [i32; 3],
    /// Live up row (`car + 0x264`), used for front lamps.
    pub live_up_264: [i32; 3],
    /// Frozen up row (`car + 0x228`), used for rear lamps (quirk Q1).
    pub frozen_up_228: [i32; 3],
    /// `car + 0x6f4`: brake light active (or forced by DELOREAN).
    pub braking_6f4: bool,
    /// `car + 0x6f8`: headlamps/tail lights on (night or bad weather).
    pub lights_on_6f8: bool,
    /// `car + 0x6fc`: reverse gear active (`gear_ratio < 0`).
    pub reverse_6fc: bool,
    /// `[L, R]` detached latches from `FUN_0041d710` (`S+0xbb8[S.ccc]`, `S+0xbb8[S.ccd]`).
    pub detached_lights_lr: [bool; 2],
    /// `S + 0xca8[0..6]`: broken night pod lamps (`FUN_0044a770`).
    pub broken_pod_lamps_ca8: [bool; 6],
    /// `S + 0x810`: electrics damage fraction in Q16.16.
    pub electrics_damage_810: i32,
    /// `FUN_0046b240() == 8`: night stage.
    pub night_stage: bool,
    /// `DAT_00828e28 != 0`: low-detail car models.
    pub low_detail_828e28: bool,
    /// `live[0x25] & 4 != 0`: BORROWERS small-car cheat.
    pub borrowers: bool,
    /// `live[0x26] & 1 != 0`: ALIENGOO cheat (suppresses `FUN_0044bbc0`).
    pub aliengoo: bool,
    /// `live[0x26] & 2 != 0`: DELOREAN cheat.
    pub delorean: bool,
    /// `car + 0x75c != 0`: out-of-bounds countdown active.
    pub out_of_bounds_75c: bool,
    /// `car + 0x760 != 0`: penalty reset active.
    pub penalty_760: bool,
    /// `DAT_00514478 != 0`: special/podium event.
    pub special_event_514478: bool,
}

/// Per-car runtime lamp block `B` (`0x55b268 + slot * 0x100`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarLampState {
    pub init: CarLampSlotInit,
    pub spec: CarModelLampSpec,
    /// `B + 0x00..0x60`: rear sprite world positions (`id in 0..8`).
    pub rear_world_positions: [[i32; 3]; 8],
    /// `B + 0x60..0xc0`: front lamp world positions (`r in 0..8`).
    pub front_world_positions: [[i32; 3]; 8],
    /// `B + 0xc0..0xd4`: per-viewport visibility and intensity.
    pub viewports: [LampViewportVisibility; 2],
    /// Flicker state (`0x55aed0[slot]`, `0x55bda4[slot]`).
    pub flicker: ElectricsFlickerState,
}

/// One additive billboard flare emitted by `FUN_0044b600` or `FUN_0044b240`
/// (kind 6 in `FUN_004a4330`/`FUN_004a4460`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LampFlareQuad {
    /// World-space centre, already pulled `0.25` (`0x4000`) toward the camera.
    pub centre_q16: [i32; 3],
    /// Corner half-extent in Q16 (`centre + camRow0 * (±half) + camRow1 * (±half)`).
    pub half_extent_q16: i32,
    /// Intensity byte `a` (`0xFF000000 | a<<16 | a<<8 | a`).
    pub alpha: u8,
    /// Grey byte (`0xff`).
    pub grey: u8,
    /// Flare texture (`headglow.bmp` or `bglow2.bmp`).
    pub texture: LampFlareTexture,
}

impl LampFlareQuad {
    /// Compute the 4 world-space corners `[v0, v1, v2, v3]` in Q16.16 from the
    /// camera's right (`cam_row0`) and up (`cam_row1`) basis rows (`FUN_004a4460`, kind 6).
    #[must_use]
    pub fn corners_q16(&self, cam_row0: [i32; 3], cam_row1: [i32; 3]) -> [[i32; 3]; 4] {
        let h = self.half_extent_q16;
        let r = scale3(cam_row0, h);
        let u = scale3(cam_row1, h);
        [
            sub3(sub3(self.centre_q16, r), u),
            add3(sub3(self.centre_q16, r), u),
            sub3(add3(self.centre_q16, r), u),
            add3(add3(self.centre_q16, r), u),
        ]
    }
}

/// One untextured rear-lamp overlay triangle in world space (`FUN_004a9920`/`FUN_004a9ad0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LampOverlayTriangle {
    /// World-space vertices in Q16.16 (already transformed by the body node's `+0xd4` matrix).
    pub vertices_q16: [[i32; 3]; 3],
    /// Packed `0xAARRGGBB` colour from the RGBA tuple `(R, G, B, A)`.
    pub argb: u32,
}

impl LampOverlayTriangle {
    /// Convert to floating-point world-space positions and opaque `0xFFRRGGBB`
    /// vertex colours for `blend_coloured_triangles` (reproducing the usual
    /// alpha-blend-off `LVERTEX` flush in `FUN_004a9ad0`).
    #[must_use]
    pub fn to_world_f32(&self) -> ([[f32; 3]; 3], [u32; 3]) {
        let pts = self
            .vertices_q16
            .map(|v| v.map(|c| (f64::from(c) / 65536.0) as f32));
        let opaque = self.argb | 0xff00_0000;
        (pts, [opaque; 3])
    }
}

/// Output of `FUN_0044bbc0` for one car and one viewport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarLampDrawPlan {
    pub overlays: Vec<LampOverlayTriangle>,
    pub flares: Vec<LampFlareQuad>,
}

impl CarLampState {
    /// Build a fresh per-car lamp block `B` (`FUN_00449e60`).
    #[must_use]
    pub fn new(tables: &CarLampTables, model: u8, borrowers: bool) -> Option<Self> {
        let init = CarLampSlotInit::from_tables(tables, model, borrowers)?;
        let spec = *tables.model(model)?;
        Some(Self {
            init,
            spec,
            rear_world_positions: [[0; 3]; 8],
            front_world_positions: [[0; 3]; 8],
            viewports: [LampViewportVisibility::default(); 2],
            flicker: ElectricsFlickerState::default(),
        })
    }

    /// `FUN_0044a950`: update viewport visibility, flicker, and world positions
    /// of active rear and front lamps.
    ///
    /// Preserves quirk Q1 (rear lamps use `frozen_up_228`, front lamps use
    /// `live_up_264`) and quirk Q3 (low detail at night skips updating pod lamp
    /// positions).
    pub fn update_positions_44a950(
        &mut self,
        pose: &CarLampPoseInput,
        camera_world_view0: &[i32; 16],
        split_camera_world_view1: Option<&[i32; 16]>,
        rand_table_82ae84: &[i32; 23],
        rand_index_82a4d0: &mut usize,
    ) {
        let braking = pose.braking_6f4 || pose.delorean;
        self.viewports[0] = side_test_44add0(
            &mut self.flicker,
            pose.electrics_damage_810,
            rand_table_82ae84,
            rand_index_82a4d0,
            braking,
            pose.reverse_6fc,
            pose.lights_on_6f8,
            camera_world_view0,
            pose.live_forward_258,
        );
        let rear = self.viewports[0].rear_visible_c0;
        let mut front = !rear && self.viewports[0].front_visible_c8;
        let mut force_rear_from_view1 = false;
        if let Some(cam1) = split_camera_world_view1 {
            self.viewports[1] = side_test_44add0(
                &mut self.flicker,
                pose.electrics_damage_810,
                rand_table_82ae84,
                rand_index_82a4d0,
                braking,
                pose.reverse_6fc,
                pose.lights_on_6f8,
                cam1,
                pose.live_forward_258,
            );
            if self.viewports[1].rear_visible_c0 {
                force_rear_from_view1 = true;
            } else if self.viewports[1].front_visible_c8 {
                front = true;
            }
        }

        let container_t = [
            pose.container_matrix_94[12],
            pose.container_matrix_94[13],
            pose.container_matrix_94[14],
        ];
        let body_ty = pose.body_local_94[13];

        if force_rear_from_view1 || rear || pose.delorean {
            let [left_detached, right_detached] = pose.detached_lights_lr;
            let rear_base = add3(container_t, scale3(pose.frozen_up_228, body_ty));
            let set_rear = |this: &mut Self, id: usize| {
                let off = this.init.rear_offset(id);
                this.rear_world_positions[id] = transform_lamp_offset_44b110(
                    off,
                    &pose.body_local_94,
                    &pose.container_matrix_94,
                    rear_base,
                    pose.borrowers,
                    ONE_Q16,
                    0,
                );
            };
            if braking {
                if !left_detached {
                    set_rear(self, 0);
                }
                if !right_detached {
                    set_rear(self, 1);
                }
                if self.spec.third_mode == 1 {
                    set_rear(self, 6);
                }
            }
            if pose.lights_on_6f8 {
                if !left_detached {
                    set_rear(self, 2);
                }
                if !right_detached {
                    set_rear(self, 3);
                }
                if self.spec.third_mode == 2 {
                    set_rear(self, 6);
                    set_rear(self, 7);
                }
            }
            if pose.reverse_6fc {
                if !left_detached {
                    set_rear(self, 4);
                }
                if !right_detached {
                    set_rear(self, 5);
                }
            }
        }

        if !front {
            return;
        }
        let front_base = add3(container_t, scale3(pose.live_up_264, body_ty));
        let base_m = usize::from(self.spec.main_headlamp_count);
        let cnt_m = usize::from(self.spec.pod_lamp_count);
        let n = if pose.night_stage && !pose.low_detail_828e28 {
            base_m.saturating_add(cnt_m)
        } else {
            base_m
        }
        .min(self.spec.front_offsets.len());

        for r in 0..n {
            self.front_world_positions[r] = transform_lamp_offset_44b110(
                self.spec.front_offsets[r],
                &pose.body_local_94,
                &pose.container_matrix_94,
                front_base,
                pose.borrowers,
                ONE_Q16,
                0,
            );
        }
    }

    /// `FUN_0044bbc0`: plan rear-lamp overlay triangles and front/rear flare
    /// billboard sprites for `view` (`0` or `1`).
    #[must_use]
    pub fn plan_draws_44bbc0(
        &self,
        pose: &CarLampPoseInput,
        model0_spec: Option<&CarModelLampSpec>,
        camera_world_q16: &[i32; 16],
        view: usize,
        flares_enabled_5fefcc: bool,
        render_mode_6: bool,
    ) -> CarLampDrawPlan {
        let mut plan = CarLampDrawPlan::default();
        if pose.out_of_bounds_75c || pose.aliengoo || pose.penalty_760 || pose.special_event_514478
        {
            return plan;
        }
        let cam_world = [
            camera_world_q16[12],
            camera_world_q16[13],
            camera_world_q16[14],
        ];
        let body_t = [
            pose.body_world_d4[12],
            pose.body_world_d4[13],
            pose.body_world_d4[14],
        ];
        let d = sub3(cam_world, body_t);
        let cam_body_55bdc8 = row_mul_3x3_transpose(d, &pose.body_world_d4);

        let [left_detached, right_detached] = pose.detached_lights_lr;
        let fl = self.flicker.dim_flag_55bda4;
        let m = self.init.model;
        let braking = pose.braking_6f4 || pose.delorean;
        let vp = self.viewports.get(view).copied().unwrap_or_default();

        let n_brake = usize::from(self.spec.brake_count).min(self.spec.brake_verts.len());
        let n_tail = usize::from(self.spec.tail_count).min(self.spec.tail_verts.len());
        let n_rev = usize::from(self.spec.reverse_count).min(self.spec.reverse_verts.len());
        let n_third = usize::from(self.spec.third_count).min(self.spec.third_verts.len());

        // BRAKE (`car+0x6f4 && render_mode != 6`)
        if braking && !render_mode_6 {
            let col = if fl {
                self.init.colours_d8_ec[3] // e4
            } else {
                self.init.colours_d8_ec[0] // d8
            };
            if !left_detached {
                build_overlay_triangles_44c8c0(
                    &mut plan.overlays,
                    &pose.body_world_d4,
                    &self.spec.brake_verts[..n_brake],
                    col,
                    false,
                    cam_body_55bdc8,
                    pose.borrowers,
                );
                // Quirk Q5: model 0 extra overlay uses tail_count[0].
                if m == 0
                    && let Some(m0) = model0_spec
                {
                    let n0 = usize::from(m0.tail_count).min(m0.tail_verts.len());
                    build_overlay_triangles_44c8c0(
                        &mut plan.overlays,
                        &pose.body_world_d4,
                        &m0.tail_verts[..n0],
                        col,
                        false,
                        cam_body_55bdc8,
                        pose.borrowers,
                    );
                }
                self.push_rear_sprite_44b240(
                    &mut plan.flares,
                    0,
                    cam_world,
                    vp,
                    flares_enabled_5fefcc,
                    pose.delorean,
                );
            }
            if !right_detached {
                build_overlay_triangles_44c8c0(
                    &mut plan.overlays,
                    &pose.body_world_d4,
                    &self.spec.brake_verts[..n_brake],
                    col,
                    true,
                    cam_body_55bdc8,
                    pose.borrowers,
                );
                if m == 0
                    && let Some(m0) = model0_spec
                {
                    let n0 = usize::from(m0.tail_count).min(m0.tail_verts.len());
                    build_overlay_triangles_44c8c0(
                        &mut plan.overlays,
                        &pose.body_world_d4,
                        &m0.tail_verts[..n0],
                        col,
                        true,
                        cam_body_55bdc8,
                        pose.borrowers,
                    );
                }
                self.push_rear_sprite_44b240(
                    &mut plan.flares,
                    1,
                    cam_world,
                    vp,
                    flares_enabled_5fefcc,
                    pose.delorean,
                );
            }
            if self.spec.third_mode == 1 {
                build_overlay_triangles_44c8c0(
                    &mut plan.overlays,
                    &pose.body_world_d4,
                    &self.spec.third_verts[..n_third],
                    col,
                    false,
                    cam_body_55bdc8,
                    pose.borrowers,
                );
                self.push_rear_sprite_44b240(
                    &mut plan.flares,
                    6,
                    cam_world,
                    vp,
                    flares_enabled_5fefcc,
                    pose.delorean,
                );
            }
        }

        // REVERSE (`car+0x6fc && render_mode != 6`)
        if pose.reverse_6fc && !render_mode_6 {
            let col = if fl {
                self.init.colours_d8_ec[5] // ec
            } else {
                self.init.colours_d8_ec[2] // e0
            };
            // MK2 (m == 5) has only the right reverse lamp.
            if m != 5 && !left_detached {
                build_overlay_triangles_44c8c0(
                    &mut plan.overlays,
                    &pose.body_world_d4,
                    &self.spec.reverse_verts[..n_rev],
                    col,
                    false,
                    cam_body_55bdc8,
                    pose.borrowers,
                );
                self.push_rear_sprite_44b240(
                    &mut plan.flares,
                    4,
                    cam_world,
                    vp,
                    flares_enabled_5fefcc,
                    pose.delorean,
                );
            }
            if !right_detached {
                build_overlay_triangles_44c8c0(
                    &mut plan.overlays,
                    &pose.body_world_d4,
                    &self.spec.reverse_verts[..n_rev],
                    col,
                    true,
                    cam_body_55bdc8,
                    pose.borrowers,
                );
                self.push_rear_sprite_44b240(
                    &mut plan.flares,
                    5,
                    cam_world,
                    vp,
                    flares_enabled_5fefcc,
                    pose.delorean,
                );
            }
        }

        // LIGHTS ON (`car+0x6f8`)
        if pose.lights_on_6f8 {
            if !(m == 5 && pose.reverse_6fc) {
                let col = if fl {
                    self.init.colours_d8_ec[4] // e8
                } else {
                    self.init.colours_d8_ec[1] // dc
                };
                let skip_tail_ov = self.spec.skip_tail_when_braking != 0 && braking;
                if !left_detached {
                    if !skip_tail_ov {
                        build_overlay_triangles_44c8c0(
                            &mut plan.overlays,
                            &pose.body_world_d4,
                            &self.spec.tail_verts[..n_tail],
                            col,
                            false,
                            cam_body_55bdc8,
                            pose.borrowers,
                        );
                        if m == 0
                            && let Some(m0) = model0_spec
                        {
                            let n0 = usize::from(m0.tail_count).min(m0.brake_verts.len());
                            build_overlay_triangles_44c8c0(
                                &mut plan.overlays,
                                &pose.body_world_d4,
                                &m0.brake_verts[..n0],
                                col,
                                false,
                                cam_body_55bdc8,
                                pose.borrowers,
                            );
                        }
                    }
                    self.push_rear_sprite_44b240(
                        &mut plan.flares,
                        2,
                        cam_world,
                        vp,
                        flares_enabled_5fefcc,
                        pose.delorean,
                    );
                }
                if !right_detached {
                    if !skip_tail_ov {
                        build_overlay_triangles_44c8c0(
                            &mut plan.overlays,
                            &pose.body_world_d4,
                            &self.spec.tail_verts[..n_tail],
                            col,
                            true,
                            cam_body_55bdc8,
                            pose.borrowers,
                        );
                        if m == 0
                            && let Some(m0) = model0_spec
                        {
                            let n0 = usize::from(m0.tail_count).min(m0.brake_verts.len());
                            build_overlay_triangles_44c8c0(
                                &mut plan.overlays,
                                &pose.body_world_d4,
                                &m0.brake_verts[..n0],
                                col,
                                true,
                                cam_body_55bdc8,
                                pose.borrowers,
                            );
                        }
                    }
                    self.push_rear_sprite_44b240(
                        &mut plan.flares,
                        3,
                        cam_world,
                        vp,
                        flares_enabled_5fefcc,
                        pose.delorean,
                    );
                }
                if self.spec.third_mode == 2 {
                    let col3 = if fl {
                        self.init.colours_d8_ec[5] // ec
                    } else {
                        self.init.colours_d8_ec[2] // e0
                    };
                    build_overlay_triangles_44c8c0(
                        &mut plan.overlays,
                        &pose.body_world_d4,
                        &self.spec.third_verts[..n_third],
                        col3,
                        false,
                        cam_body_55bdc8,
                        pose.borrowers,
                    );
                    build_overlay_triangles_44c8c0(
                        &mut plan.overlays,
                        &pose.body_world_d4,
                        &self.spec.third_verts[..n_third],
                        col3,
                        true,
                        cam_body_55bdc8,
                        pose.borrowers,
                    );
                    self.push_rear_sprite_44b240(
                        &mut plan.flares,
                        6,
                        cam_world,
                        vp,
                        flares_enabled_5fefcc,
                        pose.delorean,
                    );
                    self.push_rear_sprite_44b240(
                        &mut plan.flares,
                        7,
                        cam_world,
                        vp,
                        flares_enabled_5fefcc,
                        pose.delorean,
                    );
                }
            }
            self.push_headlamp_flares_44b600(
                &mut plan.flares,
                pose,
                cam_world,
                vp,
                flares_enabled_5fefcc,
            );
        }

        plan
    }

    /// `FUN_0044b600`: emit headlamp flare pairs (template A and half-size template B)
    /// for intact night pod lamps (when `night_stage`) and main headlamps.
    fn push_headlamp_flares_44b600(
        &self,
        out: &mut Vec<LampFlareQuad>,
        pose: &CarLampPoseInput,
        cam_world: [i32; 3],
        vp: LampViewportVisibility,
        flares_enabled_5fefcc: bool,
    ) {
        if !flares_enabled_5fefcc || !vp.front_visible_c8 {
            return;
        }
        let alpha = rt(qmul(self.spec.headlamp_alpha_scale_q16, vp.intensity_d0)) as u8;
        let tex = self.init.textures_f0_fc[3];
        let half_a = self.init.sprite_half_extents_q16[3];
        let half_b = self.init.sprite_half_extents_q16[4];
        let base_m = usize::from(self.spec.main_headlamp_count);
        let cnt_m = usize::from(self.spec.pod_lamp_count);

        let emit = |out: &mut Vec<LampFlareQuad>, r: usize| {
            let p = self.front_world_positions[r];
            let u = normalize_q16(sub3(cam_world, p));
            let c = add3(p, scale3(u, QUARTER_Q16));
            out.push(LampFlareQuad {
                centre_q16: c,
                half_extent_q16: half_a,
                alpha,
                grey: 0xff,
                texture: tex,
            });
            out.push(LampFlareQuad {
                centre_q16: c,
                half_extent_q16: half_b,
                alpha,
                grey: 0xff,
                texture: tex,
            });
        };

        if pose.night_stage {
            let end = base_m
                .saturating_add(cnt_m)
                .min(self.front_world_positions.len());
            for r in base_m..end {
                let k = r - base_m;
                if !pose.broken_pod_lamps_ca8.get(k).copied().unwrap_or(true) {
                    emit(out, r);
                }
            }
        }
        for r in 0..base_m.min(self.front_world_positions.len()) {
            emit(out, r);
        }
    }

    /// `FUN_0044b240`: emit one rear lamp sprite (`id in 0..=7`).
    fn push_rear_sprite_44b240(
        &self,
        out: &mut Vec<LampFlareQuad>,
        id: usize,
        cam_world: [i32; 3],
        vp: LampViewportVisibility,
        flares_enabled_5fefcc: bool,
        delorean: bool,
    ) {
        if !flares_enabled_5fefcc || !vp.rear_visible_c0 {
            return;
        }
        if delorean && matches!(id, 2 | 3) {
            return;
        }
        let brake_case = (
            self.init.sprite_half_extents_q16[0],
            self.init.textures_f0_fc[0],
            0x0064_0000, // 100.0
        );
        let reverse_case = (
            self.init.sprite_half_extents_q16[2],
            self.init.textures_f0_fc[2],
            0x0050_0000, // 80.0
        );
        let tail_case = (
            self.init.sprite_half_extents_q16[1],
            self.init.textures_f0_fc[1],
            0x0064_0000, // 100.0
        );
        let (half_extent_q16, texture, alpha_base) = match id {
            0 | 1 => brake_case,
            6 => {
                if self.spec.third_mode == 2 {
                    reverse_case
                } else {
                    brake_case
                }
            }
            4 | 5 | 7 => reverse_case,
            _ => tail_case,
        };
        let p = self.rear_world_positions.get(id).copied().unwrap_or([0; 3]);
        let u = normalize_q16(sub3(cam_world, p));
        let c = add3(p, scale3(u, QUARTER_Q16));
        let alpha = rt(qmul(alpha_base, vp.intensity_d0)) as u8;
        let quad = LampFlareQuad {
            centre_q16: c,
            half_extent_q16,
            alpha,
            grey: 0xff,
            texture,
        };
        out.push(quad);
        if delorean && matches!(id, 0 | 1) {
            out.push(quad);
        }
    }
}

/// `FUN_0044c610` + `FUN_0044c8c0` + `FUN_004a9920` + `FUN_004a9ad0`:
/// pull `verts` 1.0 unit toward `cam_body_55bdc8` in body space, fan into
/// triangles according to `verts.len()` (`4, 5, 6, 8`; `7` emits none per quirk Q7),
/// mirror winding/Z when `side` is set, and transform by `body_world_d4`.
pub fn build_overlay_triangles_44c8c0(
    out: &mut Vec<LampOverlayTriangle>,
    body_world_d4: &[i32; 16],
    verts: &[[i32; 3]],
    rgba: [u8; 4],
    side: bool,
    cam_body_55bdc8: [i32; 3],
    borrowers: bool,
) {
    let n = verts.len();
    if !(4..=8).contains(&n) || n == 7 {
        return;
    }
    let mut v = [[0i32; 3]; 8];
    for (slot, &src) in v.iter_mut().zip(verts.iter()) {
        let mut p = src;
        if side {
            p[2] = p[2].wrapping_neg();
        }
        let u = normalize_q16(sub3(cam_body_55bdc8, p));
        let pull = if borrowers {
            // With default scale_550d50 = 1.0, s0 = 1.0.
            u
        } else {
            u
        };
        p = add3(p, pull);
        if side {
            p[2] = p[2].wrapping_neg();
        }
        *slot = p;
    }

    let fan: &[[usize; 3]] = match n {
        4 => &[[0, 1, 2], [3, 0, 2]],
        5 => &[[0, 3, 4], [0, 1, 3], [1, 2, 3]],
        6 => &[[0, 1, 4], [5, 0, 4], [1, 2, 3], [4, 1, 3]],
        8 => &[
            [0, 1, 7],
            [1, 6, 7],
            [1, 2, 6],
            [2, 5, 6],
            [2, 3, 4],
            [2, 4, 5],
        ],
        _ => return,
    };

    let argb = (u32::from(rgba[3]) << 24)
        | (u32::from(rgba[0]) << 16)
        | (u32::from(rgba[1]) << 8)
        | u32::from(rgba[2]);

    for &[a, b, c] in fan {
        let local = if side {
            [mirror_z(v[a]), mirror_z(v[c]), mirror_z(v[b])]
        } else {
            [v[a], v[b], v[c]]
        };
        let vertices_q16 = local.map(|pt| transform_point_q16(pt, body_world_d4));
        out.push(LampOverlayTriangle { vertices_q16, argb });
    }
}

/// `FUN_004a9490`: compute the world position of a car's night road light pool
/// (`local_offset` in the car root's world matrix, normally `(21.0, 1.0, 0.0)`).
#[must_use]
pub fn pool_world_position_4a9490(car_root_world: &[i32; 16], local_offset: [i32; 3]) -> [i32; 3] {
    transform_point_q16(local_offset, car_root_world)
}

/// `FUN_004a8750`: compute the XZ road-pool grey byte (`0x28..=0xff`) at `vertex_pos`
/// from `light_pos` with radius `radius` (`DAT_005028dc`, `22.0` in Q16.16).
///
/// Preserves quirk Q9: at `d == 0`, `(0x10_0000 >> 12) & 0xff == 0`, which clamps
/// up to `0x28`.
#[must_use]
pub fn pool_grey_4a8750(light_pos: [i32; 3], vertex_pos: [i32; 3], radius: i32) -> u8 {
    let dx = vertex_pos[0].wrapping_sub(light_pos[0]);
    let dz = vertex_pos[2].wrapping_sub(light_pos[2]);
    if radius <= 0 || dx.wrapping_abs() >= radius || dz.wrapping_abs() >= radius {
        return NIGHT_AMBIENT_BYTE;
    }
    let d = sqrt_q16(qmul(dx, dx).wrapping_add(qmul(dz, dz)) as u32) as i32;
    if d >= radius {
        return NIGHT_AMBIENT_BYTE;
    }
    let step = qmul(d, qdiv(0x10_0000, radius));
    let g8 = (((0x10_0000_i32.wrapping_sub(step)) >> 12) & 0xff) as u8;
    g8.max(NIGHT_AMBIENT_BYTE)
}

/// `FUN_004a9440`: reset every vertex colour in `colours` to RGB `0x28`, keeping alpha
/// (`col = (col & 0xff282828) | 0x282828`).
pub fn reset_mesh_colours_4a9440(colours: &mut [u32]) {
    for col in colours {
        *col = (*col & 0xff28_2828) | 0x0028_2828;
    }
}

/// `FUN_004a9320`: relight mesh vertices from `light_pos` with `radius`.
/// Any vertex whose pool grey `g > 0x28` overwrites its colour with
/// `0xFF000000 | g<<16 | g<<8 | g` (last writer above `0x28` wins, quirk Q8).
pub fn relight_mesh_vertices_4a9320(
    vertices: &[[i32; 3]],
    colours: &mut [u32],
    light_pos: [i32; 3],
    radius: i32,
) {
    for (&v, col) in vertices.iter().zip(colours.iter_mut()) {
        let g = pool_grey_4a8750(light_pos, v, radius);
        if g > NIGHT_AMBIENT_BYTE {
            let g32 = u32::from(g);
            *col = 0xff00_0000 | (g32 << 16) | (g32 << 8) | g32;
        }
    }
}

/// Out-of-bounds road light pool state (`FUN_004853e0`, `0x829034`, `0x828fe8`,
/// `0x82900c`, `0x829190`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfBoundsPoolState {
    pub radius_5028dc: i32,
    pub light_local: [i32; 3],
    saved_radius_82900c: i32,
    saved_local_829190: [i32; 3],
    flag_829034: bool,
    flag_828fe8: bool,
}

impl Default for OutOfBoundsPoolState {
    fn default() -> Self {
        Self {
            radius_5028dc: POOL_RADIUS_Q16,
            light_local: POOL_LOCAL_OFFSET_Q16,
            saved_radius_82900c: POOL_RADIUS_Q16,
            saved_local_829190: POOL_LOCAL_OFFSET_Q16,
            flag_829034: false,
            flag_828fe8: false,
        }
    }
}

impl OutOfBoundsPoolState {
    /// `FUN_004853e0`: update the pool radius and local offset from the car's
    /// out-of-bounds countdown `car + 0x75c` (skipped in split screen).
    pub fn step_4853e0(&mut self, out_of_bounds_75c: i32, split_screen: bool) {
        if split_screen {
            return;
        }
        if out_of_bounds_75c > 0 && !self.flag_829034 {
            self.saved_radius_82900c = self.radius_5028dc;
            self.radius_5028dc = -0x000a_0000; // -10.0
            self.flag_829034 = true;
        } else if out_of_bounds_75c < 0 && !self.flag_828fe8 {
            self.saved_local_829190 = self.light_local;
            self.light_local = [0; 3];
            self.flag_828fe8 = true;
            self.radius_5028dc = self.saved_radius_82900c;
        } else if out_of_bounds_75c >= 0 && self.flag_828fe8 {
            self.light_local = self.saved_local_829190;
            self.flag_829034 = false;
            self.flag_828fe8 = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c3d_camera::MATRIX_IDENTITY_Q16;

    fn synthetic_lamp_block() -> Vec<u8> {
        let mut bytes = vec![0u8; CAR_LAMP_TABLES_LEN];
        let write_i32 = |buf: &mut [u8], off: usize, val: i32| {
            buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
        };
        // Model 0 (SUB): 4 brake verts, 4 tail verts, 4 reverse verts, 4 third verts (mode 1)
        // Front: 2 main headlamps, 4 night pod lamps, alpha scale 0x1_0000.
        for k in 0..4 {
            // third verts at 0x0000
            write_i32(&mut bytes, k * 12, -0x2_0000);
            write_i32(&mut bytes, k * 12 + 4, 0x8000 + (k as i32) * 0x1000);
            write_i32(&mut bytes, k * 12 + 8, (k as i32) * 0x1000);
            // brake verts at 0x02c0
            write_i32(&mut bytes, 0x02c0 + k * 12, -0x2_0000);
            write_i32(&mut bytes, 0x02c0 + k * 12 + 4, 0x4000);
            write_i32(&mut bytes, 0x02c0 + k * 12 + 8, 0x5000);
            // tail verts at 0x0810
            write_i32(&mut bytes, 0x0810 + k * 12, -0x2_0000);
            write_i32(&mut bytes, 0x0810 + k * 12 + 4, 0x6000);
            write_i32(&mut bytes, 0x0810 + k * 12 + 8, 0x5000);
            // reverse verts at 0x0d60
            write_i32(&mut bytes, 0x0d60 + k * 12, -0x2_0000);
            write_i32(&mut bytes, 0x0d60 + k * 12 + 4, 0x3000);
            write_i32(&mut bytes, 0x0d60 + k * 12 + 8, 0x4000);
        }
        bytes[0x02a0] = 4; // third_count[0]
        bytes[0x02b0] = 1; // third_mode[0]
        bytes[0x0800] = 4; // brake_count[0]
        bytes[0x0d50] = 4; // tail_count[0]
        bytes[0x1150] = 4; // reverse_count[0]
        for r in 0..6 {
            let off = 0x1160 + r * 12;
            write_i32(&mut bytes, off, 0x2_0000);
            write_i32(&mut bytes, off + 4, 0x4000);
            write_i32(&mut bytes, off + 8, (r as i32 - 2) * 0x2000);
        }
        bytes[0x16a0] = 2; // main_headlamp_count[0]
        bytes[0x16b0] = 4; // pod_lamp_count[0]
        write_i32(&mut bytes, 0x16c0, 0); // skip_tail_when_braking[0]
        write_i32(&mut bytes, 0x16f8, ONE_Q16); // headlamp_alpha_scale_q16[0]
        bytes
    }

    #[test]
    fn parses_static_lamp_tables_and_rejects_wrong_length() {
        assert!(CarLampTables::from_block(&[0u8; 16]).is_none());
        let block = synthetic_lamp_block();
        let tables = CarLampTables::from_block(&block).expect("valid block");
        let m0 = tables.model(0).expect("model 0");
        assert_eq!(m0.main_headlamp_count, 2);
        assert_eq!(m0.pod_lamp_count, 4);
        assert_eq!(m0.pod_lamp_offsets().len(), 4);
        assert_eq!(m0.headlamp_alpha_scale_q16, ONE_Q16);
    }

    #[test]
    fn model0_centroid_combines_brake_and_tail_and_mk2_uses_white_tail() {
        let block = synthetic_lamp_block();
        let tables = CarLampTables::from_block(&block).unwrap();
        let init0 = CarLampSlotInit::from_tables(&tables, 0, false).unwrap();
        // Combined average of y = 0x4000 (4x) and y = 0x6000 (4x) is 0x5000.
        assert_eq!(init0.brake_offsets[0][1], 0x5000);
        assert_eq!(init0.tail_offsets, init0.brake_offsets);
        assert_eq!(init0.brake_offsets[1][2], -0x5000);

        let init5 = CarLampSlotInit::from_tables(&tables, 5, true).unwrap();
        assert_eq!(init5.colours_d8_ec[1], COLOUR_WHITE_LIT);
        assert_eq!(init5.textures_f0_fc[1], LampFlareTexture::Headglow);
        // BORROWERS halves template extents.
        assert_eq!(init5.sprite_half_extents_q16[3], 0x2666);
        assert_eq!(init5.sprite_half_extents_q16[4], 0x1333);
    }

    #[test]
    fn electrics_flicker_dims_for_five_calls_then_reloads() {
        let mut rng = GameLcg::new(0x1234);
        let table = fill_flicker_rand_table_48bc00(&mut rng);
        let mut idx = 0usize;
        let mut flicker = ElectricsFlickerState::default();

        // No electrics damage -> never dims.
        flicker.step_44b000(0, &table, &mut idx);
        assert!(!flicker.dim_flag_55bda4);

        // With damage and initial countdown 0: dims at c = 0, -1, -2, -3, and reloads on the 5th call (c = -4).
        for step in 0..5 {
            flicker.step_44b000(0x4000, &table, &mut idx);
            assert!(flicker.dim_flag_55bda4, "dim on step {step}");
        }
        assert!(flicker.countdown_55aed0 > 0, "reloaded positive countdown");
        assert_eq!(idx, 1);

        // Next call while countdown > 0 clears dim_flag and decrements countdown.
        let before = flicker.countdown_55aed0;
        flicker.step_44b000(0x4000, &table, &mut idx);
        assert!(!flicker.dim_flag_55bda4);
        assert_eq!(flicker.countdown_55aed0, before - 1);
    }

    #[test]
    fn side_test_splits_front_and_rear_visibility_and_halves_when_dimmed() {
        let table = [ONE_Q16; 23];
        let mut idx = 0;
        let mut flicker = ElectricsFlickerState::default();
        // Identity camera looks along +Z (`camera_heading == 0`, forward = (0, 0, 1)).
        let cam = MATRIX_IDENTITY_Q16;

        // Car facing +Z (`d = +1.0` -> `ang = 0`): camera is behind car -> rear visible.
        let rear = side_test_44add0(
            &mut flicker,
            0,
            &table,
            &mut idx,
            true,
            false,
            true,
            &cam,
            [0, 0, ONE_Q16],
        );
        assert!(rear.rear_visible_c0);
        assert!(!rear.front_visible_c8);
        assert_eq!(rear.intensity_d0, qmul(0x005a_0000, 0x2d0));

        // Car facing -Z (`d = -1.0` -> `ang = 180`): camera is ahead of car -> front visible.
        let front = side_test_44add0(
            &mut flicker,
            0,
            &table,
            &mut idx,
            false,
            false,
            true,
            &cam,
            [0, 0, -ONE_Q16],
        );
        assert!(!front.rear_visible_c0);
        assert!(front.front_visible_c8);
        assert_eq!(front.intensity_d0, qmul(0x005a_0000, 0x2d0));

        // With electrics damage, intensity is halved (`0x168` vs `0x2d0`).
        let front_dim = side_test_44add0(
            &mut flicker,
            0x8000,
            &table,
            &mut idx,
            false,
            false,
            true,
            &cam,
            [0, 0, -ONE_Q16],
        );
        assert_eq!(front_dim.intensity_d0, qmul(0x005a_0000, 0x168));
    }

    #[test]
    fn plan_draws_emits_headlamp_flares_and_skips_broken_pods_and_7_vertex_overlays() {
        let block = synthetic_lamp_block();
        let tables = CarLampTables::from_block(&block).unwrap();
        let mut state = CarLampState::new(&tables, 0, false).unwrap();
        let mut cam = MATRIX_IDENTITY_Q16;
        cam[14] = -0x000a_0000; // camera at z = -10 looking +Z

        // Car facing -Z (toward camera) at night, with pod lamp 1 broken.
        let mut broken = [false; 6];
        broken[1] = true;
        let pose = CarLampPoseInput {
            body_local_94: MATRIX_IDENTITY_Q16,
            body_world_d4: MATRIX_IDENTITY_Q16,
            container_matrix_94: MATRIX_IDENTITY_Q16,
            live_forward_258: [0, 0, -ONE_Q16],
            live_up_264: [0, ONE_Q16, 0],
            frozen_up_228: [0, ONE_Q16, 0],
            braking_6f4: false,
            lights_on_6f8: true,
            reverse_6fc: false,
            detached_lights_lr: [false, false],
            broken_pod_lamps_ca8: broken,
            electrics_damage_810: 0,
            night_stage: true,
            low_detail_828e28: false,
            borrowers: false,
            aliengoo: false,
            delorean: false,
            out_of_bounds_75c: false,
            penalty_760: false,
            special_event_514478: false,
        };
        let table = [ONE_Q16; 23];
        let mut idx = 0;
        state.update_positions_44a950(&pose, &cam, None, &table, &mut idx);
        let plan = state.plan_draws_44bbc0(&pose, tables.model(0), &cam, 0, true, false);
        // 2 main headlamps + 3 intact pod lamps = 5 lamps * 2 concentric quads = 10 flares.
        assert_eq!(plan.flares.len(), 10);

        // 7-vertex overlay emits nothing (quirk Q7).
        let mut ov = Vec::new();
        build_overlay_triangles_44c8c0(
            &mut ov,
            &MATRIX_IDENTITY_Q16,
            &[[0; 3]; 7],
            COLOUR_BRAKE_LIT,
            false,
            [0, 0, -ONE_Q16],
            false,
        );
        assert!(ov.is_empty());
    }

    #[test]
    fn night_road_pool_falloff_and_quirks_match_4a8750_and_4a9320() {
        let lp = [0, 0, 0];
        // Outside radius 22.0 -> 0x28.
        assert_eq!(
            pool_grey_4a8750(lp, [0x17_0000, 0, 0], POOL_RADIUS_Q16),
            NIGHT_AMBIENT_BYTE
        );
        // Quirk Q9: d == 0 wraps low byte of 0x100 to 0 -> clamped to 0x28.
        assert_eq!(
            pool_grey_4a8750(lp, lp, POOL_RADIUS_Q16),
            NIGHT_AMBIENT_BYTE
        );
        // Near centre (d = 1.0) -> bright grey (> 240).
        let near = pool_grey_4a8750(lp, [ONE_Q16, 0, 0], POOL_RADIUS_Q16);
        assert!(near > 240, "expected bright pool near centre, got {near}");

        // Reset + relight mesh vertices.
        let mut cols = [0x80ff_ffff, 0xffff_ffff];
        reset_mesh_colours_4a9440(&mut cols);
        assert_eq!(cols[0], 0x8028_2828);
        assert_eq!(cols[1], 0xff28_2828);

        let verts = [[ONE_Q16, 0, 0], [0x20_0000, 0, 0]];
        relight_mesh_vertices_4a9320(&verts, &mut cols, lp, POOL_RADIUS_Q16);
        let expected = 0xff00_0000 | (u32::from(near) * 0x01_0101);
        assert_eq!(cols[0], expected);
        assert_eq!(cols[1], 0xff28_2828);
    }

    #[test]
    fn out_of_bounds_pool_state_disables_and_restores_pool() {
        let mut pool = OutOfBoundsPoolState::default();
        pool.step_4853e0(10, false);
        assert_eq!(pool.radius_5028dc, -0x000a_0000);
        pool.step_4853e0(-5, false);
        assert_eq!(pool.radius_5028dc, POOL_RADIUS_Q16);
        assert_eq!(pool.light_local, [0; 3]);
        pool.step_4853e0(0, false);
        assert_eq!(pool.light_local, POOL_LOCAL_OFFSET_Q16);
    }
}
