//! The wheel and suspension animation the renderer draws: the suspension and
//! surface-vibration words of `FUN_00410350`, the wheel wobble of
//! `FUN_00417e20`, the render spin integrated in the mechanics routine
//! (`0x43b3a4..0x43b4c8`), and the wheel node pose of the render
//! interpolation `FUN_00439ee0` (`FUN_00492810`/`FUN_00493900`).
//!
//! None of these words feed the simulation (`rea.official-eu.car-animation-words`).
//!
//! Observations (pseudocode and instruction level):
//!
//! - `FUN_00410350` (end of the ground pass), for a normal car (config
//!   `+0x26 & 2` clear): with `c = n . row1 < 0xcccd` each compression word
//!   `+0x55c[i]` decays toward zero by `ctl +0x124` and `+0x594[i]` clears.
//!   Otherwise each wheel's height `d_i = -qmul(probe_y - lane_word, n.y)`
//!   gives `t_i = qmul(d_i, 2^32 / c)`; `+0x55c[i]` moves toward `t_i` by at
//!   most `qmul(ctl +0x124, c)` and is clamped to `+-ctl +0x120`, and
//!   `+0x594[i] = qmul(t_i - sum(t)/4, ctl +0x11c)`, rescaled when the largest
//!   exceeds `1.0`. While in control each eased word `+0x56c[i]` follows
//!   `d_i - +0x55c[i]` (clamped to `+-0x28f`) by at most `0x41` per tick, and
//!   on rough surfaces (by the type word `ctl +0x42a[i]`) a touching wheel of
//!   an upright car advances its vibration phase `+0x57c[i]` by
//!   `ftol(qmul(min(+0xec - n . cv_i, 1.0), gain) / 5760)`; an idle phase runs
//!   on by `0x238` to a full turn and stops. A non-zero phase sets the bounce
//!   `+0x584[i] = qmul(sin(phase), amplitude)`.
//! - `FUN_00417e20`: each wheel's wobble phase `+0x604[i]` turns by
//!   `ftol(ctl +0x2b8[i] * K)` (`K = -1/(32 pi)` at `0x4e6300`), or `+-0x28b`
//!   beyond `+-1.0`; with a positive setup word `+0x800[i]` the offsets
//!   `+0x5e4[i]` are `(sin, -cos)` of the phase times `qmul(word, 0xa3d)`.
//! - The render spin `DAT_00828de0[car][i]` turns by `ftol(rate * K)`, where
//!   the rate is the smoothed speed `ctl +0x2b8[i]` unless the wheel is held
//!   (handbrake on a rear wheel, the brake by the drive split `ctl +0x170`, or
//!   the raw speed `ctl +0x2a8[i]` disagreeing in sign); `DAT_00550ea0[car][i]`
//!   is `|rate| > 0x8000` for wheels 0 and 2 and `<= 0x8000` for 1 and 3.
//! - The pose rotates the wheel node's rest rows by the angles
//!   `(camber, yaw, spin)` (`FUN_00493900`: about row 1 by the yaw, then row 2
//!   by the spin, then row 0 by the camber, each an axis-angle rotation with
//!   re-normalization), mirrored to `(-camber, yaw + 0x800, -spin)` when the
//!   flag is set; the yaw is the steer angle `ctl +0x3e0` (or the target
//!   `+0x3e2` for analog steering) on the front wheels. The node translation is
//!   the rest position plus the wobble, compression, eased and bounce words and
//!   the ride offset `wheel radius - +0x30`.

use crate::fixed_point::Vec3;
use crate::tumble::{cross, length, normalize, q, raw, vec};
use crate::vehicle_dynamics::{axis_angle_matrix, transform_by_matrix};

/// `K` at `0x4e6300`: `-1 / (32 pi)`.
pub const SPIN_RATE_SCALE: f64 = f64::from_bits(0xbf84_5f30_6dc9_c883);
/// `1 / 5760` at `0x4e62f8`.
pub const ANGLE_SCALE: f64 = f64::from_bits(0x3f26_c16c_16c1_6c17);

/// Identity 3x3 rotation rows in Q16.16.
pub const IDENTITY_ROWS: [[i32; 3]; 3] = [[0x1_0000, 0, 0], [0, 0x1_0000, 0], [0, 0, 0x1_0000]];

/// The animation words of one car.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelAnimationState {
    /// `+0x55c[4]`: suspension compression.
    pub compression_55c: [i32; 4],
    /// `+0x56c[4]`: the eased height.
    pub eased_56c: [i32; 4],
    /// `+0x57c[4]`: the surface vibration phase.
    pub vibration_57c: [u16; 4],
    /// `+0x584[4]`: the vibration bounce.
    pub bounce_584: [i32; 4],
    /// `+0x594[4]`: the normalized relative travel.
    pub travel_594: [i32; 4],
    /// `ctl +0x88[3]`: dynamic body roll/heave/pitch integrator (`FUN_00416760`).
    pub roll_state_88: [i32; 3],
    /// `ctl +0xc8[4]`: per-corner dynamic roll/pitch height (`FUN_00416760`).
    pub corner_roll_c8: [i32; 4],
    /// `ctl +0x130[3]`: visual body roll/heave/pitch integrator (`FUN_00416760` `0x4171ee..0x417ca7`).
    pub visual_roll_state_130: [i32; 3],
    /// `[ctl +0x00]` (`[car +0x04]`, Node 5) local 3x3 rotation rows (`FUN_00416760`).
    pub visual_roll_rows: [[i32; 3]; 3],
    /// `[ctl +0x00]` (`[car +0x04]`, Node 5) vertical heave displacement (`FUN_00416760`).
    pub visual_heave_y: i32,
    /// `[ctl +0x04]` (`[car +0x08]`) lagged visual chassis world basis (`FUN_00415df0`).
    pub visual_chassis_basis: [[i32; 3]; 3],
    /// `[ctl +0x04]` ground-clearance corner lift along car up (`FUN_00415df0`).
    pub visual_chassis_lift_y: i32,
    /// Whether `visual_chassis_basis` has been seeded from `car +0x2a0`.
    pub visual_chassis_initialized: bool,
    /// `+0x654[4]`: the per-wheel normal load (`FUN_00417cf0`).
    pub wheel_load_654: [i32; 4],
    /// `+0x604[4]`: the wobble phase.
    pub wobble_phase_604: [u16; 4],
    /// `+0x5e4[4]`: the wobble offsets.
    pub wobble_5e4: [[i32; 2]; 4],
    /// `DAT_00828de0[car]`: the render spin.
    pub spin: [u16; 4],
    /// `DAT_00550ea0[car]`: draw the wheel turned about.
    pub flipped: [bool; 4],
}

impl Default for WheelAnimationState {
    fn default() -> Self {
        Self::REST
    }
}

impl WheelAnimationState {
    /// Every word zero (and 3x3 visual rotation bases at identity), as the
    /// initialized car record starts.
    pub const REST: Self = Self {
        compression_55c: [0; 4],
        eased_56c: [0; 4],
        vibration_57c: [0; 4],
        bounce_584: [0; 4],
        travel_594: [0; 4],
        roll_state_88: [0; 3],
        corner_roll_c8: [0; 4],
        visual_roll_state_130: [0; 3],
        visual_roll_rows: IDENTITY_ROWS,
        visual_heave_y: 0,
        visual_chassis_basis: IDENTITY_ROWS,
        visual_chassis_lift_y: 0,
        visual_chassis_initialized: false,
        wheel_load_654: [0; 4],
        wobble_phase_604: [0; 4],
        wobble_5e4: [[0; 2]; 4],
        spin: [0; 4],
        flipped: [false; 4],
    };
}

fn ftol(value: f64) -> i16 {
    value as i64 as i16
}

fn sine(trig: &[i32], angle: u16) -> i32 {
    trig[usize::from(angle & 0xfff)]
}

/// The suspension constants `ctl +0xd8..+0x13c` patched by `FUN_00407270`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SuspensionConstants {
    pub limit_x_d8: i32,
    pub heave_limit_dc: i32,
    pub limit_z_e0: i32,
    pub heave_scale_e4: i32,
    pub up_e8: i32,
    pub min_corner_y_ec: i32,
    pub gravity_f0: i32,
    pub max_step_f4: i32,
    pub damping_f8: i32,
    pub up_deadzone_fc: i32,
    pub up_blend_slope_100: i32,
    pub roll_limit_104: i32,
    pub roll_gain_108: i32,
    pub up_blend_base_10c: i32,
    pub up_blend_range_110: i32,
    pub load_transfer_118: i32,
    pub travel_gain_11c: i32,
    pub limit_120: i32,
    pub rate_124: i32,
    pub visual_damping_13c: i32,
}

/// What `FUN_00416760` reads during the live force pass (`0x407f59`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyRollInput {
    pub velocity_d4: Vec3,
    pub prior_velocity_e0: Vec3,
    pub start_basis_27c: [Vec3; 3],
    pub half_x_2c: i32,
    pub half_z_34: i32,
    pub mass_48: i32,
    pub count_674: i32,
    pub scale_52c: i32,
    pub upright_404: bool,
    pub resting_700: bool,
    pub constants: SuspensionConstants,
}

/// `FUN_00416760` (`0x416760..0x417ce7`): steps the dynamic body roll/pitch/heave
/// vector `ctl +0x88..+0x90`, computes the four corner roll displacements
/// `ctl +0xc8..+0xd4`, and updates `car +0x654..+0x660` (`wheel_load_654`) via
/// `FUN_00417cf0`.
pub fn step_body_roll_416760(state: &mut WheelAnimationState, input: &BodyRollInput) {
    let ctl = input.constants;
    let r0 = raw(input.start_basis_27c[0]);
    let r1 = raw(input.start_basis_27c[1]);
    let r2 = raw(input.start_basis_27c[2]);
    let transform_into_basis = |v: [i32; 3]| -> [i32; 3] {
        [
            q(v[0], r0[0])
                .wrapping_add(q(v[1], r0[1]))
                .wrapping_add(q(v[2], r0[2])),
            q(v[0], r1[0])
                .wrapping_add(q(v[1], r1[1]))
                .wrapping_add(q(v[2], r1[2])),
            q(v[0], r2[0])
                .wrapping_add(q(v[1], r2[1]))
                .wrapping_add(q(v[2], r2[2])),
        ]
    };

    let local_delta = if input.count_674 != 0 {
        if input.upright_404 {
            let delta_world = [
                input
                    .prior_velocity_e0
                    .x
                    .raw()
                    .wrapping_sub(input.velocity_d4.x.raw()),
                input
                    .prior_velocity_e0
                    .y
                    .raw()
                    .wrapping_sub(input.velocity_d4.y.raw()),
                input
                    .prior_velocity_e0
                    .z
                    .raw()
                    .wrapping_sub(input.velocity_d4.z.raw()),
            ];
            let local = transform_into_basis(delta_world);
            if length(vec(local)) == 0 || input.resting_700 {
                [0, 0, 0]
            } else {
                local
            }
        } else {
            [0, 0, 0]
        }
    } else {
        let grav_local = transform_into_basis([0, ctl.gravity_f0, 0]);
        [0, grav_local[1].max(0), 0]
    };

    let len_88 = length(vec(state.roll_state_88));
    let damping_vec = if len_88 > 0 {
        let scale = -q(len_88, ctl.damping_f8);
        let inv_len = (0x1_0000_0000_i64 / i64::from(len_88)) as i32;
        [
            q(q(state.roll_state_88[0], inv_len), scale),
            q(q(state.roll_state_88[1], inv_len), scale),
            q(q(state.roll_state_88[2], inv_len), scale),
        ]
    } else {
        [0, 0, 0]
    };

    let raw_step = [
        local_delta[0].wrapping_add(damping_vec[0]),
        local_delta[1].wrapping_add(damping_vec[1]),
        local_delta[2].wrapping_add(damping_vec[2]),
    ];
    let dt_scale = q(0xa3d, 0x1_3333);
    let mut step = [
        q(raw_step[0], dt_scale),
        q(raw_step[1], dt_scale),
        q(raw_step[2], dt_scale),
    ];
    let horiz_len = length(vec([step[0], 0, step[2]]));
    if ctl.max_step_f4 > 0 && horiz_len > ctl.max_step_f4 {
        let inv_hlen = (0x1_0000_0000_i64 / i64::from(horiz_len)) as i32;
        step[0] = q(q(step[0], inv_hlen), ctl.max_step_f4);
        step[2] = q(q(step[2], inv_hlen), ctl.max_step_f4);
    }
    for (dst, &delta) in state.roll_state_88.iter_mut().zip(&step) {
        *dst = dst.wrapping_add(delta);
    }

    let v = [
        clamp_step(state.roll_state_88[0], ctl.limit_x_d8),
        ctl.up_e8,
        clamp_step(state.roll_state_88[2], ctl.limit_z_e0),
    ];
    let row1 = raw(normalize(vec(v)));
    let row0 = raw(normalize(vec([
        0x1_0000_i32.wrapping_sub(q(row1[0], row1[0])),
        -q(row1[1], row1[0]),
        -q(row1[2], row1[0]),
    ])));
    let row2 = raw(normalize(cross(vec(row0), vec(row1))));

    let corners = [
        [input.half_x_2c, input.half_z_34],
        [input.half_x_2c, -input.half_z_34],
        [-input.half_x_2c, input.half_z_34],
        [-input.half_x_2c, -input.half_z_34],
    ];
    let mut corner_roll = [0_i32; 4];
    let mut max_abs = 0_i32;
    for i in 0..4 {
        let c8 = q(corners[i][0], row0[1]).wrapping_add(q(corners[i][1], row2[1]));
        corner_roll[i] = c8;
        max_abs = max_abs.max(c8.wrapping_abs());
    }
    if ctl.roll_limit_104 > 0 && max_abs > ctl.roll_limit_104 {
        let scale = ((i64::from(ctl.roll_limit_104) << 16) / i64::from(max_abs)) as i32;
        for c8 in &mut corner_roll {
            *c8 = q(*c8, scale);
        }
    }
    state.corner_roll_c8 = corner_roll;

    // `0x4171ee..0x417ca7`: visual body roll/heave/pitch integrator `ctl +0x130`
    // and `[ctl +0x00]` (`[car +0x04]`, Node 5) local rotation/heave.
    let local_delta_130 = if input.count_674 != 0 {
        if input.upright_404 {
            let scaled = [
                local_delta[0],
                local_delta[1],
                q(local_delta[2], 0x2_0000),
            ];
            if length(vec(scaled)) == 0 || input.resting_700 {
                [0, 0, 0]
            } else {
                scaled
            }
        } else {
            [0, 0, 0]
        }
    } else {
        local_delta
    };
    let visual_damping = if ctl.visual_damping_13c != 0 {
        ctl.visual_damping_13c
    } else {
        ctl.damping_f8
    };
    let len_130 = length(vec(state.visual_roll_state_130));
    let damping_130 = if len_130 > 0 {
        let scale = -q(len_130, visual_damping);
        let inv_len = (0x1_0000_0000_i64 / i64::from(len_130)) as i32;
        [
            q(q(state.visual_roll_state_130[0], inv_len), scale),
            q(q(state.visual_roll_state_130[1], inv_len), scale),
            q(q(state.visual_roll_state_130[2], inv_len), scale),
        ]
    } else {
        [0, 0, 0]
    };
    let raw_step_130 = [
        local_delta_130[0].wrapping_add(damping_130[0]),
        local_delta_130[1].wrapping_add(damping_130[1]),
        local_delta_130[2].wrapping_add(damping_130[2]),
    ];
    let mut step_130 = [
        q(raw_step_130[0], dt_scale),
        q(raw_step_130[1], dt_scale),
        q(raw_step_130[2], dt_scale),
    ];
    let horiz_len_130 = length(vec([step_130[0], 0, step_130[2]]));
    if ctl.max_step_f4 > 0 && horiz_len_130 > ctl.max_step_f4 {
        let inv_hlen = (0x1_0000_0000_i64 / i64::from(horiz_len_130)) as i32;
        step_130[0] = q(q(step_130[0], inv_hlen), ctl.max_step_f4);
        step_130[2] = q(q(step_130[2], inv_hlen), ctl.max_step_f4);
    }
    for (dst, &delta) in state.visual_roll_state_130.iter_mut().zip(&step_130) {
        *dst = dst.wrapping_add(delta);
    }
    let v_130 = [
        clamp_step(state.visual_roll_state_130[0], ctl.limit_x_d8),
        ctl.up_e8,
        clamp_step(state.visual_roll_state_130[2], ctl.limit_z_e0),
    ];
    if ctl.up_e8 != 0 {
        let vrow1 = raw(normalize(vec(v_130)));
        let vrow0 = raw(normalize(vec([
            0x1_0000_i32.wrapping_sub(q(vrow1[0], vrow1[0])),
            -q(vrow1[1], vrow1[0]),
            -q(vrow1[2], vrow1[0]),
        ])));
        let vrow2 = raw(normalize(cross(vec(vrow0), vec(vrow1))));
        state.visual_roll_rows = [vrow0, vrow1, vrow2];
    }
    state.visual_heave_y = clamp_step(
        q(state.visual_roll_state_130[1], ctl.heave_scale_e4),
        ctl.heave_limit_dc,
    );

    state.wheel_load_654 = compute_wheel_loads_417cf0(
        input.mass_48,
        input.count_674,
        input.scale_52c,
        state.travel_594,
        state.corner_roll_c8,
        ctl,
    );
}

/// `FUN_00415df0` (`0x415df0..0x416754`): steps the visual chassis follower
/// `[ctl +0x04]` (`[car +0x08]`) that lags behind `car +0x2a0` over bumps/crests
/// when touching and upright, and computes the body-corner ground-clearance lift
/// (`ctl.min_corner_y_ec`, `-0x4ccc`) along `car.motion.basis[1]`.
pub fn step_visual_chassis_415df0(
    state: &mut WheelAnimationState,
    basis_2a0: [Vec3; 3],
    half_words: [i32; 3],
    count_674: i32,
    upright_404: bool,
    ctl: SuspensionConstants,
) {
    let car_rows = basis_2a0.map(raw);
    if !state.visual_chassis_initialized {
        state.visual_chassis_basis = car_rows;
        state.visual_chassis_lift_y = 0;
        state.visual_chassis_initialized = true;
        return;
    }
    if count_674 > 0 && upright_404 && ctl.up_blend_base_10c > 0 {
        let old_up = state.visual_chassis_basis[1];
        let car_up = car_rows[1];
        let diff_up = [
            car_up[0].wrapping_sub(old_up[0]),
            car_up[1].wrapping_sub(old_up[1]),
            car_up[2].wrapping_sub(old_up[2]),
        ];
        let dist_up = length(vec(diff_up));
        let over = dist_up.wrapping_sub(ctl.up_deadzone_fc);
        let t_up = if over <= 0 {
            ctl.up_blend_base_10c
        } else if ctl.up_blend_slope_100 <= 0 || over >= ctl.up_blend_slope_100 {
            ctl.up_blend_base_10c.wrapping_add(ctl.up_blend_range_110)
        } else {
            let frac = ((i64::from(over) << 16) / i64::from(ctl.up_blend_slope_100)) as i32;
            ctl.up_blend_base_10c
                .wrapping_add(q(frac, ctl.up_blend_range_110))
        };
        let inv_t_up = 0x1_0000_i32.wrapping_sub(t_up);
        let blended_up = std::array::from_fn(|k| {
            q(car_up[k], t_up).wrapping_add(q(old_up[k], inv_t_up))
        });
        let new_up = raw(normalize(vec(blended_up)));

        let old_r2 = state.visual_chassis_basis[2];
        let car_r2 = car_rows[2];
        let blended_r2 =
            std::array::from_fn(|k| q(car_r2[k], 0xcccc).wrapping_add(q(old_r2[k], 0x3333)));
        let new_r2_init = raw(normalize(vec(blended_r2)));
        let new_r0 = raw(normalize(cross(vec(new_up), vec(new_r2_init))));
        let new_r2 = raw(normalize(cross(vec(new_r0), vec(new_up))));
        state.visual_chassis_basis = [new_r0, new_up, new_r2];
    } else {
        state.visual_chassis_basis = car_rows;
    }

    let [r0, r1, r2] = state.visual_chassis_basis;
    let car_up = car_rows[1];
    let hx = half_words[0];
    let hy = -half_words[1];
    let hz = half_words[2];
    let mut min_y = 0x64_0000_i32;
    for (sx, sz) in [(hx, hz), (hx, -hz), (-hx, hz), (-hx, -hz)] {
        let world_corner: [i32; 3] = std::array::from_fn(|k| {
            q(sx, r0[k])
                .wrapping_add(q(hy, r1[k]))
                .wrapping_add(q(sz, r2[k]))
        });
        let local_y = q(world_corner[0], car_up[0])
            .wrapping_add(q(world_corner[1], car_up[1]))
            .wrapping_add(q(world_corner[2], car_up[2]));
        if local_y < min_y {
            min_y = local_y;
        }
    }
    state.visual_chassis_lift_y = if ctl.min_corner_y_ec != 0 && min_y < ctl.min_corner_y_ec {
        ctl.min_corner_y_ec.wrapping_sub(min_y)
    } else {
        0
    };
}

/// `FUN_00417cf0` and the `0x417ca7..0x417ce0` tail of `FUN_00416760`:
/// computes the per-wheel normal loads `car +0x654..+0x660` consumed by
/// `FUN_004095e0` (`apply_tyre_load_pass`).
#[must_use]
pub fn compute_wheel_loads_417cf0(
    mass_48: i32,
    count_674: i32,
    scale_52c: i32,
    travel_594: [i32; 4],
    mut corner_roll_c8: [i32; 4],
    constants: SuspensionConstants,
) -> [i32; 4] {
    if count_674 != 4 {
        return [scale_52c; 4];
    }
    let quarter_mass = mass_48 >> 2;
    let mut max_abs = 0_i32;
    for &c8 in &corner_roll_c8 {
        max_abs = max_abs.max(c8.wrapping_abs());
    }
    if constants.roll_limit_104 > 0 && max_abs > constants.roll_limit_104 {
        let scale = ((i64::from(constants.roll_limit_104) << 16) / i64::from(max_abs)) as i32;
        for c8 in &mut corner_roll_c8 {
            *c8 = q(*c8, scale);
        }
    }
    std::array::from_fn(|i| {
        let combined = q(corner_roll_c8[i], constants.roll_gain_108).wrapping_add(travel_594[i]);
        let transfer = q(constants.load_transfer_118, combined);
        quarter_mass.wrapping_sub(transfer)
    })
}

/// What `FUN_00410350` reads after the ground pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuspensionInput {
    pub normal: Vec3,
    pub row1: Vec3,
    pub probe_y: [i32; 4],
    pub lane_words: [i32; 4],
    pub constants: SuspensionConstants,
    pub out_of_control_680: bool,
    /// `ctl +0x42a[i]`.
    pub surface_types: [u16; 4],
    /// `+0x73a[i]`: the probe is clear of the ground.
    pub probe_clear: [bool; 4],
    pub upright_404: bool,
    pub speed_ec: i32,
    pub contact_vectors: [Vec3; 4],
}

/// The vibration gain and amplitude for a surface type, or `None` for a
/// smooth surface.
#[must_use]
pub fn surface_vibration(surface: u16) -> Option<(i32, i32)> {
    let s = surface as i16;
    if (10..=13).contains(&s) || (16..=17).contains(&s) {
        return Some((0xc8_0000, 0x3d7));
    }
    if s == 0x2d
        || s == 0x62
        || (14..=15).contains(&s)
        || (20..=29).contains(&s)
        || (40..=42).contains(&s)
        || (71..=72).contains(&s)
        || (99..=101).contains(&s)
        || (80..=83).contains(&s)
    {
        return Some((0xc8_0000, 0x666));
    }
    if (91..=93).contains(&s) || s == 0x2b || s == 0x5a {
        return Some((0x12c_0000, 0xb85));
    }
    None
}

fn clamp_step(value: i32, limit: i32) -> i32 {
    if value.wrapping_abs() > limit {
        if value < 1 { -limit } else { limit }
    } else {
        value
    }
}

/// `FUN_00410350` for a normal car.
pub fn suspension_step(state: &mut WheelAnimationState, input: &SuspensionInput, trig: &[i32]) {
    let n = raw(input.normal);
    let r1 = raw(input.row1);
    let c = q(n[2], r1[2])
        .wrapping_add(q(n[0], r1[0]))
        .wrapping_add(q(n[1], r1[1]));
    let constants = input.constants;
    let mut heights = [0_i32; 4];
    if c < 0xcccd {
        for i in 0..4 {
            let v = state.compression_55c[i];
            state.compression_55c[i] = if v.wrapping_abs() < constants.rate_124 {
                0
            } else if v < 1 {
                constants.rate_124.wrapping_add(v)
            } else {
                v.wrapping_sub(constants.rate_124)
            };
            state.travel_594[i] = 0;
        }
    } else {
        let step = q(constants.rate_124, c);
        let inverse = if c == 0 {
            -1
        } else {
            (0x1_0000_0000_i64 / i64::from(c)) as i32
        };
        let mut targets = [0_i32; 4];
        let mut sum = 0_i32;
        for i in 0..4 {
            heights[i] = q(input.probe_y[i].wrapping_sub(input.lane_words[i]), n[1]).wrapping_neg();
            targets[i] = q(heights[i], inverse);
            sum = sum.wrapping_add(targets[i]);
            let delta = clamp_step(targets[i].wrapping_sub(state.compression_55c[i]), step);
            state.compression_55c[i] = state.compression_55c[i].wrapping_add(delta);
            let limit = constants.limit_120;
            if state.compression_55c[i].wrapping_abs() > limit {
                state.compression_55c[i] = if state.compression_55c[i] < 1 {
                    -limit
                } else {
                    limit
                };
            }
        }
        let quarter = sum.wrapping_add((sum >> 31) & 3) >> 2;
        let mut largest = 0_i32;
        for (travel, target) in state.travel_594.iter_mut().zip(targets) {
            *travel = q(target.wrapping_sub(quarter), constants.travel_gain_11c);
            largest = largest.max(travel.wrapping_abs());
        }
        if largest > 0x1_0000 {
            let inverse = (0x1_0000_0000_i64 / i64::from(largest)) as i32;
            for travel in &mut state.travel_594 {
                *travel = q(*travel, inverse);
            }
        }
    }
    if input.out_of_control_680 {
        return;
    }
    // `local_10` starts at 0x3d7 and keeps the last rough wheel's amplitude
    // from one wheel to the next. On a tilted car the native eases toward
    // stack words the tilted branch never wrote; this uses zero there.
    let mut amplitude = 0x3d7;
    // One wheel index walks several parallel native arrays.
    #[allow(clippy::needless_range_loop)]
    for i in 0..4 {
        let toward = clamp_step(heights[i].wrapping_sub(state.compression_55c[i]), 0x28f);
        let step = clamp_step(toward.wrapping_sub(state.eased_56c[i]), 0x41);
        state.eased_56c[i] = state.eased_56c[i].wrapping_add(step);
        let rough = surface_vibration(input.surface_types[i]);
        if let Some((_, rough_amplitude)) = rough {
            amplitude = rough_amplitude;
        }
        match rough {
            Some((gain, _)) if !input.probe_clear[i] && input.upright_404 => {
                let along = input.normal.dot(input.contact_vectors[i]).raw();
                let speed = input.speed_ec.wrapping_sub(along).min(0x1_0000);
                let turn = ftol(f64::from(q(speed, gain)) * ANGLE_SCALE);
                let mut phase = state.vibration_57c[i].wrapping_add(turn as u16) as i16;
                if phase >= 0x1000 {
                    phase = phase.wrapping_sub(0x1000);
                }
                state.vibration_57c[i] = phase as u16;
            }
            _ => {
                let phase = state.vibration_57c[i] as i16;
                if phase != 0 {
                    let next = phase.wrapping_add(0x238);
                    state.vibration_57c[i] = if next > 0x1000 { 0 } else { next as u16 };
                }
            }
        }
        if state.vibration_57c[i] != 0 {
            state.bounce_584[i] = q(sine(trig, state.vibration_57c[i]), amplitude);
        }
    }
}

/// `FUN_00417e20`.
pub fn wobble_step(
    state: &mut WheelAnimationState,
    smoothed_2b8: [i32; 4],
    setup_800: [i32; 4],
    trig: &[i32],
) {
    for i in 0..4 {
        let rate = smoothed_2b8[i];
        let turn = if rate.wrapping_abs() <= 0x1_0000 {
            ftol(f64::from(rate) * SPIN_RATE_SCALE) as u16
        } else if rate > 0 {
            0xfd75
        } else {
            0x28b
        };
        state.wobble_phase_604[i] = state.wobble_phase_604[i].wrapping_add(turn);
        if setup_800[i] < 1 {
            state.wobble_5e4[i] = [0, 0];
        } else {
            let amplitude = q(setup_800[i], 0xa3d);
            let phase = state.wobble_phase_604[i];
            state.wobble_5e4[i] = [
                q(sine(trig, phase), amplitude),
                q(
                    sine(trig, phase.wrapping_add(0x400)).wrapping_neg(),
                    amplitude,
                ),
            ];
        }
    }
}

/// The control words the render spin reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpinInput {
    /// `ctl +0x2a8[i]`.
    pub raw_2a8: [i32; 4],
    /// `ctl +0x2b8[i]`.
    pub smoothed_2b8: [i32; 4],
    /// `ctl +0x30c`.
    pub handbrake_30c: i32,
    /// `ctl +0x308`.
    pub brake_308: i32,
    /// `ctl +0x170`.
    pub split_170: i32,
}

/// The mechanics routine's render-spin block (`0x43b3a4..0x43b4c8`).
pub fn spin_step(state: &mut WheelAnimationState, input: &SpinInput) {
    for i in 0..4 {
        let rear = i > 1;
        let check_sign = if input.handbrake_30c != 0 && rear {
            true
        } else if input.brake_308 == 0 {
            false
        } else if input.split_170 == 0 {
            i < 2
        } else if input.split_170 == 0x1_0000 {
            i > 1
        } else {
            true
        };
        let raw = input.raw_2a8[i];
        let smoothed = input.smoothed_2b8[i];
        let rate = if !check_sign || (raw > 0 && smoothed > 0) || (raw < 0 && smoothed < 0) {
            smoothed
        } else {
            0
        };
        state.spin[i] = state.spin[i].wrapping_add(ftol(f64::from(rate) * SPIN_RATE_SCALE) as u16);
        let fast = rate.wrapping_abs() > 0x8000;
        state.flipped[i] = if i == 0 || i == 2 { fast } else { !fast };
    }
}

/// `FUN_00493900`: rotate a node's rows by `(angle0, angle1, angle2)` in the
/// native order and re-normalize.
#[must_use]
pub fn rotate_node_rows(rows: [Vec3; 3], angles: [i16; 3], trig: &[i32]) -> Option<[Vec3; 3]> {
    let [mut r0, mut r1, mut r2] = rows;
    let turn = |axis: Vec3, angle: i16, row: Vec3| -> Option<Vec3> {
        let matrix = axis_angle_matrix(axis, angle, trig)?;
        Some(normalize(transform_by_matrix(row, &matrix)))
    };
    if angles[1] != 0 {
        r0 = turn(r1, angles[1], r0)?;
        r2 = turn(r1, angles[1], r2)?;
    }
    if angles[2] != 0 {
        r0 = turn(r2, angles[2], r0)?;
        r1 = turn(r2, angles[2], r1)?;
    }
    if angles[0] != 0 {
        r1 = turn(r0, angles[0], r1)?;
        r2 = turn(r0, angles[0], r2)?;
    }
    Some([r0, r1, r2])
}

/// What the wheel pose reads besides the animation words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelPoseInput {
    /// The wheel nodes' rest rows and translations (`+0x5a4[w]`).
    pub rest_rows: [[Vec3; 3]; 4],
    pub rest_translation: [[i32; 3]; 4],
    /// `DAT_005ffc64`: the model's wheel radius.
    pub wheel_radius: i32,
    /// `+0x30`.
    pub half_height_30: i32,
    /// `ctl +0x3e0`, or `+0x3e2` with analog steering (`ctl +0x410`).
    pub steer_angle: i16,
    /// `setup +0x800[w]`.
    pub setup_800: [i32; 4],
    /// Config `+0x25 & 0x10`: the rear wheels steer.
    pub rear_steer: bool,
    /// Config `+0x25 & 4`: drop the ride offset.
    pub no_ride_offset: bool,
}

/// One wheel node's local matrix (rows, translation at `[12..15]`), as
/// `FUN_00439ee0` poses it.
#[must_use]
pub fn wheel_pose(
    state: &WheelAnimationState,
    input: &WheelPoseInput,
    wheel: usize,
    trig: &[i32],
) -> Option<[i32; 16]> {
    let camber = ftol(f64::from(q(input.setup_800[wheel], 0x5_0000)) * ANGLE_SCALE);
    let steered = if input.rear_steer {
        wheel >= 2
    } else {
        wheel < 2
    };
    let yaw = if steered { input.steer_angle } else { 0 };
    let spin = state.spin[wheel] as i16;
    let angles = if state.flipped[wheel] {
        [
            camber.wrapping_neg(),
            yaw.wrapping_add(0x800),
            spin.wrapping_neg(),
        ]
    } else {
        [camber, yaw, spin]
    };
    let rows = rotate_node_rows(input.rest_rows[wheel], angles, trig)?;
    let ride = if input.no_ride_offset {
        0
    } else {
        input.wheel_radius.wrapping_sub(input.half_height_30)
    };
    let rest = input.rest_translation[wheel];
    let wobble = state.wobble_5e4[wheel];
    let x = rest[0].wrapping_add(wobble[0]);
    let y = rest[1]
        .wrapping_add(state.eased_56c[wheel])
        .wrapping_add(state.compression_55c[wheel])
        .wrapping_add(ride)
        .wrapping_add(state.bounce_584[wheel])
        .wrapping_add(wobble[1]);
    let z = rest[2];
    let [r0, r1, r2] = rows.map(raw);
    Some([
        r0[0], r0[1], r0[2], 0, r1[0], r1[1], r1[2], 0, r2[0], r2[1], r2[2], 0, x, y, z, 0x1_0000,
    ])
}

/// A matrix's rows and translation, for building the pose input.
#[must_use]
pub fn matrix_rows(matrix: &[i32; 16]) -> ([Vec3; 3], [i32; 3]) {
    (
        [
            vec([matrix[0], matrix[1], matrix[2]]),
            vec([matrix[4], matrix[5], matrix[6]]),
            vec([matrix[8], matrix[9], matrix[10]]),
        ],
        [matrix[12], matrix[13], matrix[14]],
    )
}

/// Node 5 (`BODY_NODE_ID = 5`, `[car + 0x04]`) local matrix relative to
/// `car_world` (`[car + 0x2a0]`), combining the visual chassis follower
/// `[ctl + 0x04]` (`FUN_00415df0`) and the dynamic body roll/pitch/heave
/// transform `[ctl + 0x00]` (`FUN_00416760`).
#[must_use]
pub fn body_pose(
    state: &WheelAnimationState,
    car_basis: [Vec3; 3],
    rest_translation: [i32; 3],
    wheel_radius: i32,
    half_height_30: i32,
    no_ride_offset: bool,
) -> [i32; 16] {
    let car_rows = car_basis.map(raw);
    let chassis_world = if state.visual_chassis_initialized {
        state.visual_chassis_basis
    } else {
        car_rows
    };
    let chassis_local: [[i32; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            q(chassis_world[i][0], car_rows[j][0])
                .wrapping_add(q(chassis_world[i][1], car_rows[j][1]))
                .wrapping_add(q(chassis_world[i][2], car_rows[j][2]))
        })
    });
    let mul_chassis_local = |v: [i32; 3]| -> [i32; 3] {
        std::array::from_fn(|j| {
            q(v[0], chassis_local[0][j])
                .wrapping_add(q(v[1], chassis_local[1][j]))
                .wrapping_add(q(v[2], chassis_local[2][j]))
        })
    };
    let r0 = raw(normalize(vec(mul_chassis_local(state.visual_roll_rows[0]))));
    let r1 = raw(normalize(vec(mul_chassis_local(state.visual_roll_rows[1]))));
    let r2 = raw(normalize(vec(mul_chassis_local(state.visual_roll_rows[2]))));
    let ride = if no_ride_offset {
        0
    } else {
        wheel_radius.wrapping_sub(half_height_30)
    };
    let local_t = [
        rest_translation[0],
        rest_translation[1]
            .wrapping_add(state.visual_heave_y)
            .wrapping_add(ride),
        rest_translation[2],
    ];
    let t = mul_chassis_local(local_t);
    let x = t[0];
    let y = t[1].wrapping_add(state.visual_chassis_lift_y);
    let z = t[2];
    [
        r0[0], r0[1], r0[2], 0, r1[0], r1[1], r1[2], 0, r2[0], r2[1], r2[2], 0, x, y, z, 0x1_0000,
    ]
}

fn lerp_q16(a: i32, b: i32, alpha_q16: i32) -> i32 {
    a.wrapping_add(q(b.wrapping_sub(a), alpha_q16))
}

/// Interpolate a car's wheel animation state between two 25 Hz ticks at
/// `alpha_q16` (`0..=0x1_0000`) for sub-tick `wheel_pose` evaluation
/// (`FUN_00439ee0`).
#[must_use]
pub fn interpolate_wheel_animation_state(
    prev: &WheelAnimationState,
    next: &WheelAnimationState,
    alpha_q16: i32,
) -> WheelAnimationState {
    if alpha_q16 <= 0 {
        return *prev;
    }
    if alpha_q16 >= 0x1_0000 {
        return *next;
    }
    let mut out = *next;
    for w in 0..4 {
        out.compression_55c[w] = lerp_q16(prev.compression_55c[w], next.compression_55c[w], alpha_q16);
        out.eased_56c[w] = lerp_q16(prev.eased_56c[w], next.eased_56c[w], alpha_q16);
        out.bounce_584[w] = lerp_q16(prev.bounce_584[w], next.bounce_584[w], alpha_q16);
        out.wobble_5e4[w] = [
            lerp_q16(prev.wobble_5e4[w][0], next.wobble_5e4[w][0], alpha_q16),
            lerp_q16(prev.wobble_5e4[w][1], next.wobble_5e4[w][1], alpha_q16),
        ];
        let delta_spin = next.spin[w].wrapping_sub(prev.spin[w]) as i16;
        out.spin[w] = prev.spin[w].wrapping_add(q(i32::from(delta_spin), alpha_q16) as u16);
    }
    out
}

/// `FUN_00435400` (`0x435400..0x435b94`): interpolates two Q16.16 affine
/// transforms (`prev` at `+0x54`, `next` at `+0x94`) by `alpha_q16` (`0..=0x1_0000`),
/// linearly interpolating translation (`[12..15]`) and rows 1 and 2 (`[4..7]`,
/// `[8..11]`) and orthonormalizing via `r0 = normalize(r1 x r2)` and
/// `r2 = normalize(r0 x r1)`.
#[must_use]
pub fn interpolate_matrix_435400(
    prev: &[i32; 16],
    next: &[i32; 16],
    alpha_q16: i32,
) -> [i32; 16] {
    if alpha_q16 <= 0 {
        return *prev;
    }
    if alpha_q16 >= 0x1_0000 {
        return *next;
    }
    let r1_lerp = [
        lerp_q16(prev[4], next[4], alpha_q16),
        lerp_q16(prev[5], next[5], alpha_q16),
        lerp_q16(prev[6], next[6], alpha_q16),
    ];
    let r2_lerp = [
        lerp_q16(prev[8], next[8], alpha_q16),
        lerp_q16(prev[9], next[9], alpha_q16),
        lerp_q16(prev[10], next[10], alpha_q16),
    ];
    let r1 = raw(normalize(vec(r1_lerp)));
    let r2_init = raw(normalize(vec(r2_lerp)));
    let r0 = raw(normalize(cross(vec(r1), vec(r2_init))));
    let r2 = raw(normalize(cross(vec(r0), vec(r1))));
    let [r0, r1, r2] = if r0 == [0; 3] || r1 == [0; 3] || r2 == [0; 3] {
        [
            [next[0], next[1], next[2]],
            [next[4], next[5], next[6]],
            [next[8], next[9], next[10]],
        ]
    } else {
        [r0, r1, r2]
    };
    [
        r0[0],
        r0[1],
        r0[2],
        0,
        r1[0],
        r1[1],
        r1[2],
        0,
        r2[0],
        r2[1],
        r2[2],
        0,
        lerp_q16(prev[12], next[12], alpha_q16),
        lerp_q16(prev[13], next[13], alpha_q16),
        lerp_q16(prev[14], next[14], alpha_q16),
        0x1_0000,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed_point::build_sine_table;

    const ONE: i32 = 0x1_0000;

    fn up() -> Vec3 {
        vec([0, ONE, 0])
    }

    fn input() -> SuspensionInput {
        SuspensionInput {
            normal: up(),
            row1: up(),
            probe_y: [ONE; 4],
            lane_words: [0; 4],
            constants: SuspensionConstants {
                roll_limit_104: 0x4000,
                roll_gain_108: 0x4_0000,
                load_transfer_118: 0,
                travel_gain_11c: ONE,
                limit_120: 0x8000,
                rate_124: 0x1000,
                ..SuspensionConstants::default()
            },
            out_of_control_680: false,
            surface_types: [0; 4],
            probe_clear: [false; 4],
            upright_404: true,
            speed_ec: 0,
            contact_vectors: [Vec3::ZERO; 4],
        }
    }

    #[test]
    fn the_constants_are_the_shipped_doubles() {
        assert!((SPIN_RATE_SCALE + 1.0 / (32.0 * std::f64::consts::PI)).abs() < 1e-12);
        assert!((ANGLE_SCALE - 1.0 / 5760.0).abs() < 1e-15);
    }

    #[test]
    fn compression_steps_toward_the_wheel_height_and_clamps() {
        let trig = build_sine_table();
        let mut state = WheelAnimationState::default();
        let mut wheels = input();
        // Wheel 0 sits one unit above its lane: d = -1.0.
        wheels.probe_y = [ONE, 0, 0, 0];
        suspension_step(&mut state, &wheels, &trig);
        // The rate step is qmul(0x1000, c = 1.0).
        assert_eq!(state.compression_55c, [-0x1000, 0, 0, 0]);
        for _ in 0..20 {
            suspension_step(&mut state, &wheels, &trig);
        }
        assert_eq!(state.compression_55c[0], -0x8000);
        // The eased word follows d - compression by at most 0x41 a tick.
        assert!(state.eased_56c[0] < 0 && state.eased_56c[0] >= -21 * 0x41);
        // Travel is relative to the four-wheel mean.
        assert!(state.travel_594[0] < 0 && state.travel_594[1] > 0);
    }

    #[test]
    fn a_tilted_car_decays_its_compression() {
        let trig = build_sine_table();
        let mut state = WheelAnimationState {
            compression_55c: [0x3000, -0x3000, 0x800, 0],
            travel_594: [1, 2, 3, 4],
            ..WheelAnimationState::default()
        };
        let mut tilted = input();
        tilted.row1 = vec([ONE, 0, 0]);
        suspension_step(&mut state, &tilted, &trig);
        assert_eq!(state.compression_55c, [0x2000, -0x2000, 0, 0]);
        assert_eq!(state.travel_594, [0; 4]);
    }

    #[test]
    fn a_rough_surface_vibrates_a_touching_wheel_and_the_phase_runs_out() {
        let trig = build_sine_table();
        let mut state = WheelAnimationState::default();
        let mut rough = input();
        rough.probe_y = [0; 4];
        rough.surface_types = [11, 0, 0, 0];
        rough.speed_ec = ONE;
        suspension_step(&mut state, &rough, &trig);
        let turn = ftol(f64::from(q(ONE, 0xc8_0000)) * ANGLE_SCALE) as u16;
        assert_eq!(state.vibration_57c[0], turn);
        assert_eq!(state.bounce_584[0], q(sine(&trig, turn), 0x3d7));
        assert_eq!(state.vibration_57c[1], 0);
        // Off the rough surface the phase runs on to a full turn and stops.
        rough.surface_types = [0; 4];
        let mut ticks = 0;
        while state.vibration_57c[0] != 0 && ticks < 20 {
            suspension_step(&mut state, &rough, &trig);
            ticks += 1;
        }
        assert_eq!(state.vibration_57c[0], 0);
        assert!(ticks > 1);
    }

    #[test]
    fn the_surface_table_groups_match_the_branches() {
        assert_eq!(surface_vibration(12), Some((0xc8_0000, 0x3d7)));
        assert_eq!(surface_vibration(0x62), Some((0xc8_0000, 0x666)));
        assert_eq!(surface_vibration(92), Some((0x12c_0000, 0xb85)));
        assert_eq!(surface_vibration(0), None);
        assert_eq!(surface_vibration(30), None);
    }

    #[test]
    fn wobble_turns_with_the_wheel_and_needs_a_setup_word() {
        let trig = build_sine_table();
        let mut state = WheelAnimationState::default();
        wobble_step(
            &mut state,
            [0x8000, 0x2_0000, -0x2_0000, 0],
            [0, ONE, 0, 0],
            &trig,
        );
        assert_eq!(
            state.wobble_phase_604[0],
            ftol(f64::from(0x8000) * SPIN_RATE_SCALE) as u16
        );
        assert_eq!(state.wobble_phase_604[1], 0xfd75);
        assert_eq!(state.wobble_phase_604[2], 0x28b);
        assert_eq!(state.wobble_5e4[0], [0, 0]);
        let amplitude = q(ONE, 0xa3d);
        assert_eq!(
            state.wobble_5e4[1],
            [
                q(sine(&trig, 0xfd75), amplitude),
                q(
                    sine(&trig, 0xfd75u16.wrapping_add(0x400)).wrapping_neg(),
                    amplitude
                )
            ]
        );
    }

    #[test]
    fn held_wheels_stop_spinning_and_the_flags_follow_the_rate() {
        let mut state = WheelAnimationState::default();
        let free = SpinInput {
            raw_2a8: [ONE; 4],
            smoothed_2b8: [ONE; 4],
            handbrake_30c: 0,
            brake_308: 0,
            split_170: 0x8000,
        };
        spin_step(&mut state, &free);
        let turn = ftol(f64::from(ONE) * SPIN_RATE_SCALE) as u16;
        assert_eq!(state.spin, [turn; 4]);
        assert_eq!(state.flipped, [true, false, true, false]);
        // The handbrake holds a rear wheel whose raw speed disagrees.
        let mut held = free;
        held.handbrake_30c = 1;
        held.raw_2a8 = [ONE, ONE, 0, -ONE];
        spin_step(&mut state, &held);
        assert_eq!(
            state.spin,
            [turn.wrapping_mul(2), turn.wrapping_mul(2), turn, turn]
        );
        assert_eq!(state.flipped, [true, false, false, true]);
    }

    #[test]
    fn the_pose_rotates_the_front_wheels_by_the_steer_and_adds_the_travel() {
        let trig = build_sine_table();
        let identity = [vec([ONE, 0, 0]), up(), vec([0, 0, ONE])];
        let pose_input = WheelPoseInput {
            rest_rows: [identity; 4],
            rest_translation: [[ONE, 0, 2 * ONE]; 4],
            wheel_radius: 0x5000,
            half_height_30: 0x3000,
            steer_angle: 0x100,
            setup_800: [0; 4],
            rear_steer: false,
            no_ride_offset: false,
        };
        let state = WheelAnimationState {
            compression_55c: [0x100; 4],
            eased_56c: [0x10; 4],
            ..WheelAnimationState::default()
        };
        let front = wheel_pose(&state, &pose_input, 0, &trig).unwrap();
        let rear = wheel_pose(&state, &pose_input, 2, &trig).unwrap();
        assert_eq!(&front[12..15], &[ONE, 0x110 + 0x2000, 2 * ONE]);
        // The rear wheel keeps its rest rows; the front one turned about y.
        assert_eq!(rear[0], ONE);
        assert_ne!(front[2], 0);
        assert_eq!(front[5], ONE);
        // Rotating by nothing keeps the rows.
        assert_eq!(rotate_node_rows(identity, [0; 3], &trig), Some(identity));
    }

    #[test]
    fn wheel_loads_417cf0_split_quarter_mass_and_apply_travel_transfer() {
        let constants = SuspensionConstants {
            roll_limit_104: 0x4000,
            roll_gain_108: 0x4_0000,
            load_transfer_118: 0x046d_c000 >> 3,
            travel_gain_11c: 0x3_2000,
            limit_120: 0x28f5,
            rate_124: 0x47a,
            ..SuspensionConstants::default()
        };
        let mass_48 = 0x046d_c000;
        let rest_loads =
            compute_wheel_loads_417cf0(mass_48, 4, 0x011b_7000, [0; 4], [0; 4], constants);
        assert_eq!(rest_loads, [mass_48 >> 2; 4]);
        let transferred = compute_wheel_loads_417cf0(
            mass_48,
            4,
            0x011b_7000,
            [0x4000, -0x4000, 0, 0],
            [0; 4],
            constants,
        );
        assert!(transferred[0] < rest_loads[0]);
        assert!(transferred[1] > rest_loads[1]);
        let non_four =
            compute_wheel_loads_417cf0(mass_48, 2, 0x0236_e000, [0; 4], [0; 4], constants);
        assert_eq!(non_four, [0x0236_e000; 4]);
    }

    #[test]
    fn body_roll_416760_shifts_load_forward_under_braking_deceleration() {
        let constants = SuspensionConstants {
            limit_x_d8: 0x1e_0000,
            heave_limit_dc: 0x1333,
            limit_z_e0: 0x28_0000,
            heave_scale_e4: 0x3333,
            up_e8: 0x38_0000,
            min_corner_y_ec: -0x4ccc,
            gravity_f0: 0x191,
            max_step_f4: 0x8_0000,
            damping_f8: 0x1999,
            up_deadzone_fc: 0x1999,
            up_blend_slope_100: 0x189,
            roll_limit_104: 0x4000,
            roll_gain_108: 0x4_0000,
            up_blend_base_10c: 0x3333,
            up_blend_range_110: 0x9999,
            load_transfer_118: 0x046d_c000 >> 3,
            travel_gain_11c: 0x3_2000,
            limit_120: 0x28f5,
            rate_124: 0x47a,
            visual_damping_13c: 0xcccc,
        };
        let mut state = WheelAnimationState::default();
        let identity = [vec([ONE, 0, 0]), up(), vec([0, 0, ONE])];
        let input = BodyRollInput {
            velocity_d4: vec([0x10_0000, 0, 0x01_0000]),
            prior_velocity_e0: vec([0x12_0000, 0x01_0000, 0]),
            start_basis_27c: identity,
            half_x_2c: 0x1_6000,
            half_z_34: 0x0_b000,
            mass_48: 0x046d_c000,
            count_674: 4,
            scale_52c: 0x011b_7000,
            upright_404: true,
            resting_700: false,
            constants,
        };
        step_body_roll_416760(&mut state, &input);
        assert!(state.roll_state_88[0] > 0);
        assert!(state.corner_roll_c8[0] < 0 && state.corner_roll_c8[1] < 0);
        assert!(state.corner_roll_c8[2] > 0 && state.corner_roll_c8[3] > 0);
        assert!(state.wheel_load_654[0] > state.wheel_load_654[2]);
        // Visual roll state 130 integrates 2x lateral impulse and builds a tilted Node 5 basis.
        assert_eq!(state.visual_roll_state_130[0], state.roll_state_88[0]);
        assert!(state.visual_roll_state_130[2].abs() > state.roll_state_88[2].abs());
        assert!(state.visual_roll_rows[0][1] < 0);
        assert!(state.visual_heave_y > 0);
    }

    #[test]
    fn visual_chassis_415df0_lags_car_basis_and_lifts_low_corner() {
        let ctl = SuspensionConstants {
            min_corner_y_ec: -0x4ccc,
            up_deadzone_fc: 0x1999,
            up_blend_slope_100: 0x189,
            up_blend_base_10c: 0x3333,
            up_blend_range_110: 0x9999,
            ..SuspensionConstants::default()
        };
        let mut state = WheelAnimationState::default();
        let flat = [vec([ONE, 0, 0]), up(), vec([0, 0, ONE])];
        let half = [0x1_6000, 0x4ccc, 0x0_b000];
        step_visual_chassis_415df0(&mut state, flat, half, 4, true, ctl);
        assert!(state.visual_chassis_initialized);
        assert_eq!(state.visual_chassis_lift_y, 0);

        // Sudden pitch change on the physics basis: visual_chassis_basis blends
        // partially and lifts the chassis so the lowest corner stays >= min_corner_y_ec.
        let pitched_up = normalize(vec([0x2000, ONE, 0]));
        let pitched_r0 = normalize(cross(pitched_up, vec([0, 0, ONE])));
        let pitched = [pitched_r0, pitched_up, vec([0, 0, ONE])];
        step_visual_chassis_415df0(&mut state, pitched, half, 4, true, ctl);
        assert!(state.visual_chassis_basis[1][0] > 0);
        assert!(state.visual_chassis_basis[1][0] < raw(pitched_up)[0]);
        assert!(state.visual_chassis_lift_y > 0);

        let pose = body_pose(&state, pitched, [0, 0x1000, 0], 0x5200, 0x4ccc, false);
        assert!(pose[13] > 0x1000 + (0x5200 - 0x4ccc));
    }

    #[test]
    fn interpolate_matrix_435400_blends_and_orthonormalizes() {
        let prev = [
            ONE, 0, 0, 0, 0, ONE, 0, 0, 0, 0, ONE, 0, 0, 0x1_0000, 0, ONE,
        ];
        let next = [
            0, 0, -ONE, 0, 0, ONE, 0, 0, ONE, 0, 0, 0, 0x2_0000, 0x3_0000, 0x4_0000, ONE,
        ];
        assert_eq!(interpolate_matrix_435400(&prev, &next, 0), prev);
        assert_eq!(interpolate_matrix_435400(&prev, &next, ONE), next);
        let mid = interpolate_matrix_435400(&prev, &next, ONE / 2);
        assert_eq!(&mid[12..15], &[0x1_0000, 0x2_0000, 0x2_0000]);
        // At 45 degrees about Y, row 0 and row 2 are unit length (~0.7071).
        let r0_len = length(vec([mid[0], mid[1], mid[2]]));
        let r2_len = length(vec([mid[8], mid[9], mid[10]]));
        assert!((r0_len - ONE).abs() <= 64);
        assert!((r2_len - ONE).abs() <= 64);
    }
}
