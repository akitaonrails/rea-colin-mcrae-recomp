//! The head of the integrator `FUN_0040a4a0`: the grounded grip assist
//! `FUN_0040b240`, the rollover trigger `FUN_0040bfe0`, and the contact force
//! and moment sum that follows them.
//!
//! Observations (pseudocode and listing level):
//!
//! - `FUN_0040a4a0` first calls `FUN_0040b240`, `FUN_0040bfe0` and
//!   `FUN_0040c980` (the last only acts with config `+0x24 & 0x40`), then sums
//!   forces from the base force `+0x51c`: nothing more while no probe touches
//!   (`+0x742 <= 0`); otherwise, for every touching probe in `+0x744..`, its
//!   support sum `+0x3fc + 12i` plus, for a wheel, its lane accumulator
//!   `+0x45c + 12i`; and then, for every wheel whose contact-distance flag
//!   `+0x68c + 4w` is set, its lane accumulator again. Each term adds
//!   `body(F) x (+0x5c + 12i)` to the moment, with `body` the row-dot
//!   transform `FUN_0048e100`.
//! - `FUN_0040b240`, while `+0x674 > 0` and the car is upright: damps the yaw
//!   rate, turns the forward row toward the steered wheel direction by at most
//!   `qmul(dt, 0x312)` per tick (scaled by speed, reversed when rolling
//!   backwards) and rebuilds the other rows, adds a sideways steer force to the
//!   two front lane accumulators, and slows the car when the ground normal
//!   changes along the direction of travel by more than `ctl +0x128`.
//! - `FUN_0040bfe0`, while some probe touches: on a slope steeper than `0x155`
//!   (from the arcsine of `n.y`) and with no car-to-car contact `+0x6dc`, it
//!   sends the car out of control and adds a tipping moment that grows with
//!   the counter `+0x734`; otherwise it clears the counter. While in control
//!   it compares the sideways speed with the four wheel surfaces' slide limits
//!   (`DAT_004f768c`), slews the roll value `+0x610` and rolls the car over
//!   once `|+0x610|` reaches `0xcccd`, with a roll moment from the sideways
//!   speed; `+0x60c` is the body-roll angle the render matrix uses.

use crate::drivetrain::reciprocal_or_minus_one as reciprocal;
use crate::fixed_point::{Fixed16, Vec3};
use crate::legacy_math::{SECOND_WHEEL_ARCTAN_TABLE, signed_arcsine_phase, sqrt_q16};
use crate::tumble::{cross, length, normalize, q, raw, vec};
use crate::vehicle_dynamics::{transform_by_matrix, transform_contact_force};
use cmr_formats::pe_tyre_tables::TYRE_SURFACE_COUNT;

fn basis_matrix(basis: [Vec3; 3]) -> [i32; 16] {
    let [r0, r1, r2] = basis.map(raw);
    [
        r0[0], r0[1], r0[2], 0, r1[0], r1[1], r1[2], 0, r2[0], r2[1], r2[2], 0, 0, 0, 0, 0x1_0000,
    ]
}

/// Inputs to `FUN_0040b240`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GripAssistInput {
    pub contact_count_674: i32,
    pub upright_404: bool,
    pub time_step: Fixed16,
    pub basis: [Vec3; 3],
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub speed_ec: i32,
    /// `ctl +0x3e0` steer angle and `ctl +0x318` steer position.
    pub steer_angle_3e0: i16,
    pub steer_position_318: i32,
    pub handbrake_30c: bool,
    /// Config `+0x25 & 0x10` reverses the steer force.
    pub steer_force_reversed: bool,
    /// Ground normal `+0x324` and the one saved before the last ground pass
    /// (`+0x330`).
    pub normal: Vec3,
    pub previous_normal_330: Vec3,
    /// `ctl +0x128/+0x12c`.
    pub crest_threshold_128: i32,
    pub crest_gain_12c: i32,
    /// The front lane accumulators `+0x45c/+0x468`.
    pub front_accumulators: [Vec3; 2],
    /// The published matrix `+0x2a0` that turns the wheel direction.
    pub published: [Vec3; 3],
}

/// What `FUN_0040b240` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GripAssistEffects {
    pub basis: [Vec3; 3],
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub front_accumulators: [Vec3; 2],
}

/// `FUN_0040b240`.
#[must_use]
pub fn grip_assist(input: &GripAssistInput, trig: &[i32]) -> GripAssistEffects {
    let mut out = GripAssistEffects {
        basis: input.basis,
        velocity: input.velocity,
        angular_velocity: input.angular_velocity,
        front_accumulators: input.front_accumulators,
    };
    if input.contact_count_674 <= 0 || !input.upright_404 {
        return out;
    }
    let dt = input.time_step.raw();
    let decay = q(dt, 0x624);
    let step = q(dt, 0x312);
    let scaled = q(out.angular_velocity.y.raw(), 0x3e8_0000);
    let damped = scaled.wrapping_sub(q(scaled, decay));
    out.angular_velocity.y = Fixed16::from_raw(q(damped, 0x41));

    let angle = i32::from(input.steer_angle_3e0);
    let wheel_local = vec([
        trig[((angle + 0x400) & 0xfff) as usize],
        0,
        trig[(angle & 0xfff) as usize].wrapping_neg(),
    ]);
    let wheel = transform_by_matrix(wheel_local, &basis_matrix(input.published));
    let [row0, _, row2] = out.basis;
    let speed = q(input.speed_ec, 0x2604);
    let reach = if row0.dot(out.velocity).raw() < 0 {
        speed.wrapping_neg()
    } else {
        speed
    };
    let target = normalize(row0.add(wheel.scale(Fixed16::from_raw(reach))));
    let delta = target.sub(row0);
    let distance = length(delta);
    let row0 = if distance < step {
        target
    } else {
        let inverse = reciprocal(distance);
        let unit = vec(raw(delta).map(|word| q(word, inverse)));
        normalize(row0.add(unit.scale(Fixed16::from_raw(step))))
    };
    let along = row2.dot(row0);
    let row2 = normalize(row2.sub(row0.scale(along)));
    let row1 = normalize(cross(row2, row0));
    out.basis = [row0, row1, row2];

    let up = out.basis[1];
    let sideways = out.velocity.sub(up.scale(up.dot(out.velocity)));
    if length(sideways) > 0 {
        let mut push = q(q(input.speed_ec, 0x1e_0000), input.steer_position_318);
        if input.handbrake_30c {
            push = push.wrapping_add(q(push, 0x2_0000));
        }
        if input.steer_force_reversed {
            push = push.wrapping_neg();
        }
        let force = out.basis[2].scale(Fixed16::from_raw(push));
        out.front_accumulators = out
            .front_accumulators
            .map(|accumulator| accumulator.add(force));
    }

    let [vx, _, vz] = raw(out.velocity);
    let horizontal = sqrt_q16(q(vx, vx).wrapping_add(q(vz, vz)) as u32) as i32;
    if horizontal > 0 {
        let inverse = reciprocal(horizontal);
        let (hx, hz) = (q(vx, inverse), q(vz, inverse));
        let [px, _, pz] = raw(input.previous_normal_330);
        let [nx, _, nz] = raw(input.normal);
        let change = q(pz, hz)
            .wrapping_add(q(px, hx))
            .wrapping_sub(q(nz, hz).wrapping_add(q(nx, hx)));
        if input.crest_threshold_128 < change {
            let loss = q(change, input.crest_gain_12c).min(0x1_0000);
            out.velocity = out
                .velocity
                .scale(Fixed16::from_raw(0x1_0000_i32.wrapping_sub(loss)));
        }
    }
    out
}

/// The rollover words of the car record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RolloverState {
    /// `+0x734`: ticks on a steep slope (capped at 200).
    pub slope_ticks_734: u16,
    /// `+0x610/+0x614`: the slewed roll value and its saturation ticks.
    pub roll_610: i32,
    pub roll_ticks_614: i16,
    /// `+0x60c`: the body-roll render angle.
    pub roll_angle_60c: i16,
}

/// Inputs to `FUN_0040bfe0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RolloverInput<'a> {
    pub contact_count_674: i32,
    pub count_742: u8,
    pub normal: Vec3,
    /// The live rows `+0x258..` (snapshot and sideways axis).
    pub basis: [Vec3; 3],
    /// The published matrix `+0x2a0` the body transforms use (the start-of-
    /// tick basis inside the integrator head).
    pub published: [Vec3; 3],
    pub velocity: Vec3,
    /// Base force `+0x51c` and inverse mass `+0x4c`.
    pub base_force: Vec3,
    pub inverse_mass_4c: Fixed16,
    /// `+0x6dc`, the car-to-car contact flag.
    pub car_contact_6dc: bool,
    /// The four wheel surface words `ctl +0x42a..`.
    pub wheel_surfaces: [i16; 4],
    /// `DAT_004f768c`; without it the sideways test cannot run.
    pub roll_limits: Option<&'a [[u8; 2]; TYRE_SURFACE_COUNT]>,
}

/// What `FUN_0040bfe0` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RolloverEffects {
    /// Set when this call sent the car out of control.
    pub out_of_control: bool,
    /// `n . rows` snapshotted when it did.
    pub axis_snapshot_bc: Option<Vec3>,
    /// Added to the prior moment `+0x3d8`.
    pub moment: Vec3,
}

/// A wheel surface word outside the roll-limit table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollSurfaceIndexError {
    pub wheel: usize,
    pub surface: i16,
}

fn surface_limits(
    surfaces: [i16; 4],
    limits: &[[u8; 2]; TYRE_SURFACE_COUNT],
    column: usize,
) -> Result<i32, RollSurfaceIndexError> {
    let mut sum = 0_i32;
    for (wheel, &surface) in surfaces.iter().enumerate() {
        let entry = usize::try_from(surface)
            .ok()
            .and_then(|index| limits.get(index))
            .ok_or(RollSurfaceIndexError { wheel, surface })?;
        sum = sum.wrapping_add(i32::from(entry[column]) << 16);
    }
    Ok(q(q(sum, 0x4000), 0x3d1))
}

/// `FUN_0040bfe0`.
///
/// # Errors
///
/// Rejects a wheel surface word outside the 120-entry roll-limit table when
/// the sideways test reads it.
pub fn rollover_trigger(
    state: &mut RolloverState,
    out_of_control_680: bool,
    input: &RolloverInput<'_>,
) -> Result<RolloverEffects, RollSurfaceIndexError> {
    let mut effects = RolloverEffects {
        out_of_control: false,
        axis_snapshot_bc: None,
        moment: Vec3::ZERO,
    };
    if input.contact_count_674 == 0 && input.count_742 == 0 {
        return Ok(effects);
    }
    let n = input.normal;
    let snapshot = |basis: [Vec3; 3]| Vec3::new(n.dot(basis[0]), n.dot(basis[1]), n.dot(basis[2]));
    let mut out_of_control = out_of_control_680;
    let ny = n.y.raw();
    let steep = ny < 0x1_0000
        && signed_arcsine_phase(ny).is_ok_and(|phase| 0x400 - phase > 0x155)
        && !input.car_contact_6dc;
    if steep {
        let rows = input.published;
        let tilt = transform_contact_force(rows, n.scale(Fixed16::from_raw(0x28_0000)));
        let ramp = q(i32::from(state.slope_ticks_734) << 16, 0xa3);
        let pull = transform_contact_force(rows, input.base_force.scale(Fixed16::from_raw(ramp)));
        let moment = cross(pull, tilt).scale(input.inverse_mass_4c);
        if !out_of_control {
            out_of_control = true;
            effects.out_of_control = true;
            effects.axis_snapshot_bc = Some(snapshot(input.basis));
        }
        effects.moment = moment;
        if state.slope_ticks_734 < 200 {
            state.slope_ticks_734 += 1;
        }
    } else {
        state.slope_ticks_734 = 0;
    }
    let Some(limits) = input.roll_limits else {
        return Ok(effects);
    };
    if out_of_control {
        return Ok(effects);
    }
    let threshold = surface_limits(input.wheel_surfaces, limits, 0)?;
    let side = input.basis[2];
    let side = normalize(side.sub(n.scale(n.dot(side))));
    let sideways = input.velocity.dot(side).raw();
    let ratio = if threshold < sideways.wrapping_abs() {
        let span = surface_limits(input.wheel_surfaces, limits, 1)?.wrapping_sub(threshold);
        let offset = if sideways > 0 {
            threshold.wrapping_neg()
        } else {
            threshold
        };
        let ratio = q(sideways.wrapping_add(offset), reciprocal(span));
        if ratio.wrapping_abs() > 0x1_0000 {
            if ratio < 1 { -0x1_0000 } else { 0x1_0000 }
        } else {
            ratio
        }
    } else {
        0
    };
    let target = q(ratio, 0x1_0000);
    let difference = target.wrapping_sub(state.roll_610);
    state.roll_610 = if difference.wrapping_abs() < 0x1999 {
        target
    } else if difference < 1 {
        state.roll_610.wrapping_sub(0x1999)
    } else {
        state.roll_610.wrapping_add(0x1999)
    };
    if state.roll_610.wrapping_abs() < 0xcccd {
        state.roll_ticks_614 = 0;
    } else {
        state.roll_ticks_614 = state.roll_ticks_614.wrapping_add(1);
    }
    state.roll_angle_60c = roll_angle(state.roll_610);
    if state.roll_ticks_614 > 0 {
        state.roll_angle_60c = 0;
        state.roll_ticks_614 = 0;
        effects.out_of_control = true;
        effects.axis_snapshot_bc = Some(snapshot(input.basis));
        effects.moment = Vec3::new(
            Fixed16::from_raw(q(sideways, -0xa_0000)),
            Fixed16::ZERO,
            Fixed16::ZERO,
        );
    }
    Ok(effects)
}

/// The `DAT_00833380` fold of `roll / 2.0` into the body-roll angle.
fn roll_angle(roll: i32) -> i16 {
    if roll == 0 {
        return 0;
    }
    let negative = roll < 0;
    let magnitude = roll.wrapping_abs();
    let (numerator, denominator, complement) = if magnitude > 0x2_0000 {
        (0x2_0000_i64, i64::from(magnitude), true)
    } else {
        (i64::from(magnitude), 0x2_0000_i64, false)
    };
    let index = (((numerator << 16) / denominator) >> 7) as usize;
    let mut angle = SECOND_WHEEL_ARCTAN_TABLE[index.min(SECOND_WHEEL_ARCTAN_TABLE.len() - 1)];
    if complement {
        angle = 0x400 - angle;
    }
    if negative {
        angle.wrapping_neg()
    } else {
        angle
    }
}

/// Inputs to the integrator's contact force and moment sum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContactSumInput {
    pub base_force: Vec3,
    pub support_3fc: [Vec3; 8],
    pub lane_accumulators: [Vec3; 4],
    pub hull_accumulators: [Vec3; 4],
    /// `+0x742` and the touching-probe list `+0x744..`.
    pub count_742: u8,
    pub touching_744: [u8; 8],
    pub contact_68c: [bool; 4],
    /// `+0x5c + 12i`.
    pub offsets: [Vec3; 8],
    /// The published matrix `+0x2a0` for the body transform.
    pub published: [Vec3; 3],
}

/// The summed force and the moment `body(F) x offset` over the native terms.
#[must_use]
pub fn contact_force_and_moment(input: &ContactSumInput) -> (Vec3, Vec3) {
    let mut force = input.base_force;
    let mut moment = Vec3::ZERO;
    if input.count_742 == 0 || input.count_742 > 8 {
        return (force, moment);
    }
    let mut add_moment = |term: Vec3, offset: Vec3| {
        moment = moment.add(cross(
            transform_contact_force(input.published, term),
            offset,
        ));
    };
    for position in (0..usize::from(input.count_742)).rev() {
        let probe = usize::from(input.touching_744[position]);
        let Some(&support) = input.support_3fc.get(probe) else {
            continue;
        };
        let term = if probe >= 4 {
            let hull = input.hull_accumulators[probe - 4];
            force = force.add(hull);
            support.add(hull)
        } else {
            support
        };
        add_moment(term, input.offsets[probe]);
    }
    for wheel in (0..4).rev() {
        if input.contact_68c[wheel] {
            let lane = input.lane_accumulators[wheel];
            force = force.add(lane);
            add_moment(lane, input.offsets[wheel]);
        }
    }
    (force, moment)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        vec([x, y, z])
    }

    fn identity() -> [Vec3; 3] {
        [v(0x1_0000, 0, 0), v(0, 0x1_0000, 0), v(0, 0, 0x1_0000)]
    }

    #[test]
    fn contact_sum_counts_touching_probes_then_flagged_wheels() {
        let input = ContactSumInput {
            base_force: v(0, -0x1_0000, 0),
            support_3fc: [v(0, 0x100, 0); 8],
            lane_accumulators: [v(0x10, 0, 0), v(0x20, 0, 0), v(0x40, 0, 0), v(0x80, 0, 0)],
            hull_accumulators: [v(0, 0, 0), v(0x08, 0, 0), v(0, 0, 0), v(0, 0, 0)],
            count_742: 2,
            touching_744: [0, 5, 0, 0, 0, 0, 0, 0],
            contact_68c: [true, false, false, true],
            offsets: [v(0, 0, 0x1_0000); 8],
            published: identity(),
        };
        let (force, moment) = contact_force_and_moment(&input);
        // Base, probe 5 hull accumulator (`+0x498`), and flagged wheels 0 and 3;
        // `support_3fc` contributes to moments only (`0x40a578..0x40a5bc`).
        assert_eq!(force, v(0x08 + 0x10 + 0x80, -0x1_0000, 0));
        assert_ne!(moment, Vec3::ZERO);
        let clear = ContactSumInput {
            count_742: 0,
            ..input
        };
        assert_eq!(
            contact_force_and_moment(&clear),
            (input.base_force, Vec3::ZERO)
        );
    }

    #[test]
    fn grip_assist_needs_contact_and_turns_the_heading_toward_the_wheels() {
        let trig = crate::fixed_point::build_sine_table();
        let base = GripAssistInput {
            contact_count_674: 4,
            upright_404: true,
            time_step: Fixed16::ONE,
            basis: identity(),
            velocity: v(0x8000, 0, 0),
            angular_velocity: v(0, 0x1000, 0),
            speed_ec: 0x8000,
            steer_angle_3e0: 0x100,
            steer_position_318: 0x8000,
            handbrake_30c: false,
            steer_force_reversed: false,
            normal: v(0, 0x1_0000, 0),
            previous_normal_330: v(0, 0x1_0000, 0),
            crest_threshold_128: 0x1000,
            crest_gain_12c: 0x1_0000,
            front_accumulators: [Vec3::ZERO; 2],
            published: identity(),
        };
        let airborne = grip_assist(
            &GripAssistInput {
                contact_count_674: 0,
                ..base
            },
            &trig,
        );
        assert_eq!(airborne.basis, identity());
        let left = grip_assist(&base, &trig);
        let right = grip_assist(
            &GripAssistInput {
                steer_angle_3e0: -0x100,
                ..base
            },
            &trig,
        );
        assert!(left.basis[0].z.raw() < 0 && right.basis[0].z.raw() > 0);
        assert!(left.angular_velocity.y.raw() < 0x1000);
        assert_eq!(left.front_accumulators[0], left.front_accumulators[1]);
        assert_ne!(left.front_accumulators[0], Vec3::ZERO);
        // A crest along the travel direction slows the car.
        let crest = grip_assist(
            &GripAssistInput {
                previous_normal_330: normalize(v(0x4000, 0xf000, 0)),
                ..base
            },
            &trig,
        );
        assert!(crest.velocity.x.raw() < left.velocity.x.raw());
    }

    #[test]
    fn steep_slopes_tip_the_car_and_level_ground_resets_the_counter() {
        let limits = [[4, 8]; TYRE_SURFACE_COUNT];
        let steep = normalize(v(0xc000, 0x8000, 0));
        let input = RolloverInput {
            contact_count_674: 4,
            count_742: 4,
            normal: steep,
            basis: identity(),
            published: identity(),
            velocity: Vec3::ZERO,
            base_force: v(0, -0x10_0000, 0),
            inverse_mass_4c: Fixed16::from_raw(0x100),
            car_contact_6dc: false,
            wheel_surfaces: [1; 4],
            roll_limits: Some(&limits),
        };
        let mut state = RolloverState {
            slope_ticks_734: 10,
            ..RolloverState::default()
        };
        let effects = rollover_trigger(&mut state, false, &input).unwrap();
        assert!(effects.out_of_control);
        assert!(effects.axis_snapshot_bc.is_some());
        assert_ne!(effects.moment, Vec3::ZERO);
        assert_eq!(state.slope_ticks_734, 11);
        let level = RolloverInput {
            normal: v(0, 0x1_0000, 0),
            ..input
        };
        let calm = rollover_trigger(&mut state, false, &level).unwrap();
        assert!(!calm.out_of_control);
        assert_eq!(state.slope_ticks_734, 0);
        let mut capped = RolloverState {
            slope_ticks_734: 200,
            ..RolloverState::default()
        };
        rollover_trigger(&mut capped, true, &input).unwrap();
        assert_eq!(capped.slope_ticks_734, 200);
    }

    #[test]
    fn a_fast_sideways_slide_saturates_the_roll_and_rolls_the_car() {
        let limits = [[1, 3]; TYRE_SURFACE_COUNT];
        let mut state = RolloverState::default();
        let input = RolloverInput {
            contact_count_674: 4,
            count_742: 4,
            normal: v(0, 0x1_0000, 0),
            basis: identity(),
            published: identity(),
            velocity: v(0, 0, 0x8000),
            base_force: Vec3::ZERO,
            inverse_mass_4c: Fixed16::ONE,
            car_contact_6dc: false,
            wheel_surfaces: [0; 4],
            roll_limits: Some(&limits),
        };
        let mut rolled = None;
        for tick in 0..20 {
            let effects = rollover_trigger(&mut state, false, &input).unwrap();
            if effects.out_of_control {
                rolled = Some((tick, effects));
                break;
            }
            assert!(state.roll_angle_60c > 0);
        }
        let (tick, effects) = rolled.expect("the slide rolls the car");
        assert!(tick >= 8, "the roll value slews by 0x1999 per tick");
        assert!(effects.moment.x.raw() < 0);
        assert_eq!((state.roll_ticks_614, state.roll_angle_60c), (0, 0));
        let bad = RolloverInput {
            wheel_surfaces: [0, 0, 120, 0],
            ..input
        };
        assert_eq!(
            rollover_trigger(&mut RolloverState::default(), false, &bad),
            Err(RollSurfaceIndexError {
                wheel: 2,
                surface: 120
            })
        );
    }
}
