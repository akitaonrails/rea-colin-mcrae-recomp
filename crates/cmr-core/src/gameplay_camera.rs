//! Reconstruction of the in-race chase/follow camera placement (the core of the
//! recovered gameplay-camera subsystem).
//!
//! The shipped game drives a per-player camera scene-entity whose world matrix
//! the renderer inverts into the D3D VIEW transform (that inverse is already
//! [`crate::c3d_camera::view_matrix_words`] / `FUN_0048f460`). This module
//! reconstructs how the chase camera's **world** matrix is built each frame from
//! the followed car's pose, for the main chase modes `5`/`6`/`10`:
//!
//! - The per-mode offset table `DAT_004eae98` (`{lateral, up, longitudinal}`
//!   Q16.16 triples): the camera sits `longitudinal` behind and `up` above the
//!   car, expressed in the car's frame (`FUN_0041a3e0`), so
//!   `camera_pos = car_pos + car_basis * offset`.
//! - The camera orientation tracks the car basis; the recovered per-frame
//!   smoothing filter (`FUN_00435400`, invoked from `FUN_00419e50`) eases and
//!   Gram-Schmidt re-orthonormalizes the orientation toward the car's
//!   orientation at the rate `0x1C9210000 / ((1 + pal) * 0x190000)` (`0x190000`
//!   = 25.0 Q16.16, the 25 fps base; the PAL flag halves it), holding once every
//!   basis element is within the snap threshold `0x51E`.
//! - After easing the camera basis, `FUN_0041a3e0` (`0x41a55b..0x41a5c5`) rotates
//!   the per-mode offset `DAT_004eae98` by the **eased camera basis** and adds
//!   the car position (`camera_pos = car_pos + camera_basis * offset`), keeping
//!   the car centered at `-offset` in camera view space so the car rotates around
//!   its own center while the camera orbits smoothly behind it.
//!
//! The car pose here is the race sim's `(position, heading)` (`heading` a 16-bit
//! BAM yaw on the x/z ground plane, forward `= (cos h, 0, sin h)`, up `= +Y`),
//! which is the reconstruction's stand-in for the engine's per-car world matrix
//! `DAT_00550f68 + car*0x30`.
//!
//! Scope: this reconstructs the steady chase placement + the orientation ease.
//! The full camera subsystem additionally has a mode state machine
//! (`FUN_004343a0`/`FUN_00434060`), the A/B cosine mode-change crossfade
//! (`FUN_00436930`), the post-reset look-ahead transient, the speed-projected
//! bumper-cam lead, and the trackside/cinematic mode `0xb` - later recovery.
//! See evidence `rea.official-eu.gameplay-camera`.

use crate::fixed_point::{Fixed16, TRIG_TABLE_LEN, Vec3, build_sine_table, cos_bam, sin_bam};

/// The recovered per-mode camera offset table `DAT_004eae98`: seven Q16.16
/// `{lateral, up, longitudinal}` triples. The longitudinal term is negative
/// (behind the car). Rows `0`/`3`/`6` are the single-player presets for chase
/// modes `5`/`6`/`10`; rows `1`/`2`/`4`/`5` are the split-screen base rows
/// selected when more than one viewport is active (`DAT_00550c08 != 1`).
pub const CHASE_OFFSETS: [[i32; 3]; 7] = [
    [0, 0x0001_8000, -0x0006_0000], // 0: mode 5 near  - 1.5 up, 6.0 behind
    [0, 0x0001_4ccc, -0x0008_0000], // 1: split base   - 1.3 up, 8.0 behind
    [0, 0x0001_8000, -0x0006_0000], // 2: split base   - 1.5 up, 6.0 behind
    [0, 0x0002_0000, -0x000a_0000], // 3: mode 6 far   - 2.0 up, 10.0 behind
    [0, 0x0001_e666, -0x000d_0000], // 4: split base   - 1.9 up, 13.0 behind
    [0, 0x0002_0000, -0x000a_0000], // 5: split base   - 2.0 up, 10.0 behind
    [0, 0x0001_0000, -0x0006_0000], // 6: mode 10 third- 1.0 up, 6.0 behind
];

/// A single-player chase camera preset, mapping the recovered chase modes
/// (`5`/`6`/`10`) to their [`CHASE_OFFSETS`] row via the variant arg `0`/`3`/`6`
/// that `FUN_004362a0` passes to the chase evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChasePreset {
    /// Mode `5`, variant `0`: the default near chase (row `0`).
    Near,
    /// Mode `6`, variant `3`: the far chase (row `3`).
    Far,
    /// Mode `10`, variant `6`: the third preset (row `6`).
    Third,
}

impl ChasePreset {
    /// The `[lateral, up, longitudinal]` Q16.16 offset for this preset.
    pub const fn offset(self) -> [i32; 3] {
        match self {
            ChasePreset::Near => CHASE_OFFSETS[0],
            ChasePreset::Far => CHASE_OFFSETS[3],
            ChasePreset::Third => CHASE_OFFSETS[6],
        }
    }
}

/// The orientation snap/hold threshold (`0x51E` Q16.16, ~0.0199): once every
/// basis element is within this of the target, the ease holds at the target
/// (`FUN_00435400`).
pub const EASE_SNAP_THRESHOLD_Q16: i32 = 0x51e;

/// Numerator of the recovered orientation ease rate (`0x1C9210000`); an `i64`
/// because it exceeds 32 bits before the divide.
pub const EASE_RATE_NUMERATOR: i64 = 0x1_c921_0000;

/// The 25 fps base divisor factor `0x190000` (= 25.0 Q16.16).
pub const EASE_RATE_FPS_BASE_Q16: i64 = 0x19_0000;

/// The recovered per-frame orientation ease fraction (Q16.16):
/// `0x1C9210000 / ((1 + pal) * 0x190000)`. `pal` doubles the divisor (the PAL
/// framerate flag `DAT_00551af0`), halving the rate. For NTSC this is `0x1234`
/// (~0.071); for PAL ~`0x91a` (~0.0356).
#[must_use]
pub fn ease_rate_q16(pal: bool) -> i32 {
    let divisor = (1 + i64::from(pal)) * EASE_RATE_FPS_BASE_Q16;
    (EASE_RATE_NUMERATOR / divisor) as i32
}

/// Whether any of the nine 3x3 basis elements of `cur` differs from `target` by
/// more than [`EASE_SNAP_THRESHOLD_Q16`] (so the ease still runs this frame).
#[must_use]
fn basis_needs_ease(cur: &[i32; 16], target: &[i32; 16]) -> bool {
    (0..3).any(|j| {
        (0..3).any(|i| {
            let idx = 4 * j + i;
            (i64::from(cur[idx]) - i64::from(target[idx])).abs()
                > i64::from(EASE_SNAP_THRESHOLD_Q16)
        })
    })
}

/// Build the car's world matrix from the sim pose, column-major Q16.16: right
/// `= (sin h, 0, -cos h)` (col 0), up `= +Y` (col 1), forward `= (cos h, 0,
/// sin h)` (col 2), translation `= position` (col 3). The basis is orthonormal
/// and matches the sim's forward `(cos h, 0, sin h)`.
#[must_use]
pub fn car_world_q16(trig: &[i32; TRIG_TABLE_LEN], position: Vec3, heading: u16) -> [i32; 16] {
    let cos = cos_bam(trig, heading).raw();
    let sin = sin_bam(trig, heading).raw();
    let one = Fixed16::ONE.raw();
    [
        sin,
        0,
        -cos,
        0, // col 0: right
        0,
        one,
        0,
        0, // col 1: up
        cos,
        0,
        sin,
        0, // col 2: forward
        position.x.raw(),
        position.y.raw(),
        position.z.raw(),
        one, // col 3: translation
    ]
}

/// The chase camera's **target** world matrix for a car pose and offset: the
/// orientation is the car basis (the chase camera faces the car's forward), and
/// the translation is `car_pos + car_basis * offset` (`FUN_0041a3e0`: the offset
/// is rotated into the car frame, then added to the car position).
#[must_use]
pub fn chase_target_world(car_world: &[i32; 16], offset: [i32; 3]) -> [i32; 16] {
    let mut target = *car_world; // orientation (cols 0..2) copied verbatim
    // translation = car_pos + sum_j basis_col_j * offset_j, per axis.
    for i in 0..3 {
        let rotated = (i64::from(car_world[i]) * i64::from(offset[0])
            + i64::from(car_world[4 + i]) * i64::from(offset[1])
            + i64::from(car_world[8 + i]) * i64::from(offset[2]))
            >> 16;
        target[12 + i] = car_world[12 + i] + rotated as i32;
    }
    target
}

/// The stateful in-race chase camera: holds the current (eased) world matrix and
/// a sine table, advancing one frame per [`Self::update`].
#[derive(Clone)]
pub struct GameplayCamera {
    trig: [i32; TRIG_TABLE_LEN],
    world: [i32; 16],
    initialized: bool,
}

impl Default for GameplayCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl GameplayCamera {
    /// A fresh camera; the first [`Self::update`] snaps to the chase target
    /// (mirroring the setup-time snap in `FUN_0041a330`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            trig: build_sine_table(),
            world: IDENTITY_Q16,
            initialized: false,
        }
    }

    /// Advance one frame: ease and Gram-Schmidt re-orthonormalize the camera's
    /// 3x3 basis toward the car's basis (`FUN_00419e50` -> `FUN_00435400`, or
    /// hold when already within [`EASE_SNAP_THRESHOLD_Q16`]), then place the
    /// camera at `car_pos + eased_camera_basis * offset` (`FUN_0041a3e0` at
    /// `0x41a55b..0x41a5c5`). Returns the camera world matrix to feed
    /// [`crate::c3d_camera::view_matrix_words`].
    pub fn update(
        &mut self,
        position: Vec3,
        heading: u16,
        preset: ChasePreset,
        pal: bool,
    ) -> &[i32; 16] {
        let car = car_world_q16(&self.trig, position, heading);

        if !self.initialized {
            self.world = chase_target_world(&car, preset.offset());
            self.initialized = true;
            return &self.world;
        }

        if basis_needs_ease(&self.world, &car) {
            let k = ease_rate_q16(pal);
            let blended = crate::camera_crossfade::blend_pose(&self.world, &car, k);
            self.world[0..12].copy_from_slice(&blended[0..12]);
        } else {
            // Within the snap threshold on all nine basis elements: hold at target.
            self.world[0..12].copy_from_slice(&car[0..12]);
        }

        // Rotate the chase offset by the EASED camera basis and add car_pos
        // (`FUN_0041a3e0`: `cam_pos = car_pos + cam_basis * offset`), keeping the
        // car centered at `-offset` in camera view space during turns.
        self.world[12..15].copy_from_slice(&car[12..15]);
        self.world = chase_target_world(&self.world, preset.offset());
        &self.world
    }

    /// The current camera world matrix (column-major Q16.16).
    #[must_use]
    pub fn world(&self) -> &[i32; 16] {
        &self.world
    }
}

/// Duration of the start-line camera orbit (Phase 2 of State 5, `FUN_00439950` /
/// `FUN_00467630`): 75 ticks at 25 Hz (3.0 seconds).
pub const START_ORBIT_TICKS: i32 = 75;

/// Preset 0 (`0x4f3d70`) of the Mode 8 start-line orbit camera (`FUN_004678c0`):
/// starts at `height = 0.0`, `distance = -4.0`, `yaw = 0x6ccc` (~153 degrees,
/// front-left of the car), with cosine ease cutoff `0xb333` (0.7).
const ORBIT_PRESET0_HEIGHT_START_Q16: i32 = 0;
const ORBIT_PRESET0_DIST_START_Q16: i32 = -0x0004_0000;
const ORBIT_PRESET0_YAW_START_Q16: i32 = 0x0000_6ccc;
const ORBIT_PRESET0_CUTOFF_Q16: i32 = 0x0000_b333;

/// Reconstructs `FUN_00467c20`: eases a normalized orbit countdown `t_q16` in
/// `0..=0x10000` against `cutoff_q16` using the double cosine S-curve over
/// `DAT_0082d380`.
#[must_use]
pub fn ease_orbit_param_467c20(
    t_q16: i32,
    cutoff_q16: i32,
    trig: &[i32; TRIG_TABLE_LEN],
) -> i32 {
    if t_q16 <= 0 {
        return 0;
    }
    if t_q16 > cutoff_q16 {
        return Fixed16::ONE.raw();
    }
    if cutoff_q16 == Fixed16::ONE.raw() {
        return ((i64::from(t_q16) * i64::from(t_q16)) >> 16) as i32;
    }
    let mut u = if cutoff_q16 == 0 {
        -1
    } else {
        ((i64::from(t_q16) << 16) / i64::from(cutoff_q16)) as i32
    };
    for _ in 0..2 {
        let inv = Fixed16::ONE.raw().wrapping_sub(u);
        let bam = ((i64::from(inv) * 0x800) >> 16) as i32;
        let idx = ((0x400 - bam) & 0xfff) as usize;
        let cos_val = trig[idx].wrapping_add(Fixed16::ONE.raw());
        u = ((i64::from(cos_val) * 0x8000) >> 16) as i32;
    }
    u
}

/// Reconstructs the Mode 8 start-line orbit camera world matrix (`FUN_004677a0`
/// and `FUN_004678c0`, Preset 0 at `0x4f3d70`) for `remaining_ticks` in
/// `0..=START_ORBIT_TICKS`.
///
/// At `remaining_ticks == START_ORBIT_TICKS` (start of Phase 2), the camera sits
/// in front-left of the car looking back at it; as `remaining_ticks` counts down
/// to `0`, the camera orbits around the side of the car and lands at
/// `chase_target_world(car_world, ChasePreset::Far.offset())` (`(0, +2.0, -10.0)`
/// behind the car), ready for the 25-tick crossfade into `ChasePreset::Near`.
#[must_use]
pub fn start_orbit_camera(
    trig: &[i32; TRIG_TABLE_LEN],
    car_world: &[i32; 16],
    remaining_ticks: i32,
) -> [i32; 16] {
    let clamped = remaining_ticks.clamp(0, START_ORBIT_TICKS);
    let t_q16 = ((i64::from(clamped) << 16) / i64::from(START_ORBIT_TICKS)) as i32;
    let eased = ease_orbit_param_467c20(t_q16, ORBIT_PRESET0_CUTOFF_Q16, trig);

    let y_local = 0x0002_0000
        + (((i64::from(ORBIT_PRESET0_HEIGHT_START_Q16 - 0x0002_0000)) * i64::from(eased)) >> 16)
            as i32;
    let z_local = -0x000a_0000
        + (((i64::from(ORBIT_PRESET0_DIST_START_Q16 + 0x000a_0000)) * i64::from(eased)) >> 16)
            as i32;
    let yaw_bam = (((i64::from(ORBIT_PRESET0_YAW_START_Q16) * i64::from(eased) * 4096) >> 16)
        >> 16) as u16
        & 0xfff;

    let sin_y = sin_bam(trig, yaw_bam).raw();
    let cos_y = cos_bam(trig, yaw_bam).raw();
    let one = Fixed16::ONE.raw();

    // Local orbit basis in car space (`FUN_004678c0`, pitch == 0 for Preset 0):
    // right = (cos_y, 0, sin_y), up = (0, 1, 0), forward = (-sin_y, 0, cos_y).
    let local_right = [cos_y, 0, sin_y];
    let local_up = [0, one, 0];
    let local_fwd = [-sin_y, 0, cos_y];

    // Local translation `T_local(0, y_local, z_local) * R_orbit`:
    // pos_local = z_local * local_fwd + y_local * local_up.
    let local_pos = [
        ((i64::from(z_local) * i64::from(local_fwd[0])) >> 16) as i32,
        y_local,
        ((i64::from(z_local) * i64::from(local_fwd[2])) >> 16) as i32,
    ];

    // Multiply `car_world * M_orbit` (`0x467bd2: call 0x48ecb0`).
    let transform_vec = |v: [i32; 3]| -> [i32; 3] {
        let mut out = [0i32; 3];
        for i in 0..3 {
            out[i] = ((i64::from(car_world[i]) * i64::from(v[0])
                + i64::from(car_world[4 + i]) * i64::from(v[1])
                + i64::from(car_world[8 + i]) * i64::from(v[2]))
                >> 16) as i32;
        }
        out
    };
    let world_right = transform_vec(local_right);
    let world_up = transform_vec(local_up);
    let world_fwd = transform_vec(local_fwd);
    let world_offset = transform_vec(local_pos);

    [
        world_right[0],
        world_right[1],
        world_right[2],
        0,
        world_up[0],
        world_up[1],
        world_up[2],
        0,
        world_fwd[0],
        world_fwd[1],
        world_fwd[2],
        0,
        car_world[12].wrapping_add(world_offset[0]),
        car_world[13].wrapping_add(world_offset[1]),
        car_world[14].wrapping_add(world_offset[2]),
        one,
    ]
}

/// Column-major Q16.16 identity.
const IDENTITY_Q16: [i32; 16] = {
    let one = 0x0001_0000;
    [one, 0, 0, 0, 0, one, 0, 0, 0, 0, one, 0, 0, 0, 0, one]
};

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i32 = 0x0001_0000;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3::new(
            Fixed16::from_raw(x),
            Fixed16::from_raw(y),
            Fixed16::from_raw(z),
        )
    }

    #[test]
    fn presets_map_to_the_recovered_offset_rows() {
        assert_eq!(ChasePreset::Near.offset(), CHASE_OFFSETS[0]);
        assert_eq!(ChasePreset::Far.offset(), CHASE_OFFSETS[3]);
        assert_eq!(ChasePreset::Third.offset(), CHASE_OFFSETS[6]);
        // The near preset is 1.5 up, 6.0 behind, centred.
        assert_eq!(ChasePreset::Near.offset(), [0, 0x0001_8000, -0x0006_0000]);
    }

    #[test]
    fn ease_rate_matches_the_recovered_divide() {
        // NTSC: 0x1C9210000 / 0x190000 = 4681 exactly (the recovery's ~0x1234 is
        // an approximation of this integer divide).
        assert_eq!(ease_rate_q16(false), 4681);
        assert_eq!(
            ease_rate_q16(false),
            (EASE_RATE_NUMERATOR / EASE_RATE_FPS_BASE_Q16) as i32
        );
        // PAL doubles the divisor, halving the rate (integer floor).
        assert_eq!(ease_rate_q16(true), 2340);
    }

    #[test]
    fn chase_places_the_camera_behind_and_above_a_car_facing_plus_x() {
        // heading 0 -> forward = (cos0, 0, sin0) = (+1, 0, 0) (+x).
        let trig = build_sine_table();
        let car = car_world_q16(&trig, v(10 * ONE, 0, 0), 0);
        // forward col (col 2) is +x; up col (col 1) is +y.
        assert_eq!([car[8], car[9], car[10]], [ONE, 0, 0]);
        assert_eq!([car[4], car[5], car[6]], [0, ONE, 0]);
        // right col (col 0) = (sin, 0, -cos) = (0, 0, -1).
        assert_eq!([car[0], car[1], car[2]], [0, 0, -ONE]);

        let target = chase_target_world(&car, ChasePreset::Near.offset());
        // camera = car_pos + forward*(-6) + up*(1.5) = (10-6, 1.5, 0) = (4, 1.5, 0).
        assert_eq!(target[12], 4 * ONE);
        assert_eq!(target[13], 0x0001_8000); // 1.5
        assert_eq!(target[14], 0);
        // orientation copied from the car basis (camera faces car forward).
        assert_eq!([target[8], target[9], target[10]], [ONE, 0, 0]);
    }

    #[test]
    fn non_cardinal_heading_uses_exact_startup_trig_values() {
        let trig = build_sine_table();
        let car = car_world_q16(&trig, Vec3::ZERO, 1);
        assert_eq!([car[8], car[9], car[10]], [65_535, 0, 100]);
        assert_eq!([car[0], car[1], car[2]], [100, 0, -65_535]);
    }

    #[test]
    fn first_update_snaps_then_translation_tracks_rigidly() {
        let mut cam = GameplayCamera::new();
        let first = *cam.update(v(0, 0, 0), 0, ChasePreset::Near, false);
        // first frame snaps to the target: camera at (-6, 1.5, 0).
        assert_eq!(first[12], -6 * ONE);
        assert_eq!(first[13], 0x0001_8000);

        // Move the car to +x = 20 with the same heading: translation snaps to the
        // new behind point (14, 1.5, 0) immediately (no position lag), and the
        // orientation is unchanged (same heading) so it holds.
        let second = *cam.update(v(20 * ONE, 0, 0), 0, ChasePreset::Near, false);
        assert_eq!(second[12], 14 * ONE);
        assert_eq!([second[8], second[9], second[10]], [ONE, 0, 0]);
    }

    #[test]
    fn orientation_eases_toward_a_turned_car_then_holds() {
        let mut cam = GameplayCamera::new();
        cam.update(v(0, 0, 0), 0, ChasePreset::Near, false);
        // Snap target basis for heading 0.
        let start_fwd = [cam.world()[8], cam.world()[9], cam.world()[10]];
        assert_eq!(start_fwd, [ONE, 0, 0]);

        // Turn the car 90 degrees (BAM quarter turn = 0x400, full turn 0x1000):
        // forward rotates toward the recovered sine-table target basis.
        let quarter = crate::fixed_point::BAM_QUARTER_TURN;
        let trig = build_sine_table();
        let target_car = car_world_q16(&trig, v(0, 0, 0), quarter);
        let target_fwd = [target_car[8], target_car[9], target_car[10]];
        assert_ne!(target_fwd, start_fwd); // the target genuinely differs

        // One eased step moves the basis toward the target but not all the way.
        cam.update(v(0, 0, 0), quarter, ChasePreset::Near, false);
        let after_one = [cam.world()[8], cam.world()[9], cam.world()[10]];
        assert_ne!(after_one, start_fwd); // moved
        assert_ne!(after_one, target_fwd); // not yet arrived

        // Running many frames at the same heading converges and then holds exactly
        // at the recovered target basis (the snap step lands it precisely).
        for _ in 0..512 {
            cam.update(v(0, 0, 0), quarter, ChasePreset::Near, false);
        }
        let converged = [cam.world()[8], cam.world()[9], cam.world()[10]];
        assert_eq!(converged, target_fwd);
    }

    #[test]
    fn turning_car_stays_centered_in_camera_view_space_and_basis_stays_orthonormal() {
        use crate::c3d_camera::{apply_affine_q16, invert_affine_q16};

        let mut cam = GameplayCamera::new();
        let car_pos = v(12 * ONE, 2 * ONE, -8 * ONE);
        cam.update(car_pos, 0, ChasePreset::Near, false);

        // Turn the car 90 degrees and step through the ease transition.
        // At every intermediate frame:
        // 1. The camera basis remains orthonormal (Gram-Schmidt via FUN_00435400).
        // 2. The car's position in camera view space remains at -offset = [0, -1.5, +6.0]
        //    (no lateral "windshield wiper" swing).
        let quarter = crate::fixed_point::BAM_QUARTER_TURN;
        let offset = ChasePreset::Near.offset();
        let expected_view_car = [-offset[0], -offset[1], -offset[2]];

        for step in 0..32 {
            let world = *cam.update(car_pos, quarter, ChasePreset::Near, false);
            for col in [0usize, 4, 8] {
                let len_sq = ((i64::from(world[col]) * i64::from(world[col]))
                    + (i64::from(world[col + 1]) * i64::from(world[col + 1]))
                    + (i64::from(world[col + 2]) * i64::from(world[col + 2])))
                    >> 16;
                assert!(
                    (len_sq - i64::from(ONE)).abs() <= 8,
                    "step {step} col {col} not unit length: {len_sq:#x}"
                );
            }
            let view = invert_affine_q16(&world);
            let car_in_view = apply_affine_q16(
                &view,
                [car_pos.x.raw(), car_pos.y.raw(), car_pos.z.raw()],
            );
            assert!(
                (car_in_view[0] - expected_view_car[0]).abs() <= 16,
                "step {step} lateral view swing: got {}, expected {}",
                car_in_view[0],
                expected_view_car[0]
            );
            assert!(
                (car_in_view[1] - expected_view_car[1]).abs() <= 16,
                "step {step} vertical view drift: got {}, expected {}",
                car_in_view[1],
                expected_view_car[1]
            );
            assert!(
                (car_in_view[2] - expected_view_car[2]).abs() <= 16,
                "step {step} longitudinal view drift: got {}, expected {}",
                car_in_view[2],
                expected_view_car[2]
            );
        }
    }

    #[test]
    fn snap_threshold_holds_within_tolerance() {
        let mut a = IDENTITY_Q16;
        let b = IDENTITY_Q16;
        // Equal matrices never need easing.
        assert!(!basis_needs_ease(&a, &b));
        // A basis element off by exactly the threshold does not trip it; one more does.
        a[0] = ONE + EASE_SNAP_THRESHOLD_Q16;
        assert!(!basis_needs_ease(&a, &b));
        a[0] = ONE + EASE_SNAP_THRESHOLD_Q16 + 1;
        assert!(basis_needs_ease(&a, &b));
    }

    #[test]
    fn start_orbit_camera_sweeps_from_front_left_to_far_chase_target() {
        let trig = build_sine_table();
        let car_pos = v(5 * ONE, ONE, -3 * ONE);
        let car = car_world_q16(&trig, car_pos, 0x200);

        let start = start_orbit_camera(&trig, &car, START_ORBIT_TICKS);
        let end = start_orbit_camera(&trig, &car, 0);
        let far_target = chase_target_world(&car, ChasePreset::Far.offset());

        // At remaining_ticks == 0, the orbit lands directly on ChasePreset::Far.
        assert_eq!(end, far_target);
        assert_ne!(start, end);

        for remaining in (0..=START_ORBIT_TICKS).rev() {
            let world = start_orbit_camera(&trig, &car, remaining);
            for col in [0usize, 4, 8] {
                let len_sq = ((i64::from(world[col]) * i64::from(world[col]))
                    + (i64::from(world[col + 1]) * i64::from(world[col + 1]))
                    + (i64::from(world[col + 2]) * i64::from(world[col + 2])))
                    >> 16;
                assert!(
                    (len_sq - i64::from(ONE)).abs() <= 16,
                    "remaining {remaining} col {col} not unit length: {len_sq:#x}"
                );
            }
        }
    }
}
