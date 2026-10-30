//! Car-vs-car collision (`FUN_00442740`, `FUN_004438a0`, `FUN_0043f250`,
//! `FUN_00443500`, `FUN_0043f600`, `FUN_00441b80`, `FUN_00442860`,
//! `FUN_00441f30`, `FUN_00485bb0`, `FUN_00401060`).
//!
//! Runs in `FUN_0043afd0` (`0x43b2cc..0x43b319`) after both cars have completed
//! their static-scenery (`FUN_0043d5d0`) and moving-object (`FUN_0043f790`)
//! passes, while both cars' upper hull points (`4..7`) are still deformed by
//! `FUN_00417f60`:
//!
//! 1. **Mode gate (`0x43b2cc..0x43b30e`)**: requires `DAT_00550e90 == 2`
//!    (two active simulated cars), `DAT_00d3ca88 == 0` (not Rally School /
//!    replay), and either `FUN_00485bb0() != 0` (network / super-special stage)
//!    or `FUN_00401060() != 0` (two-player split-screen / head-to-head).
//! 2. **Shared-cell broadphase (`FUN_00442740`, `0x442767..0x4427d0`)**: tests
//!    whether any of `car_a`'s four active cell words (`+0x750..+0x756`) equals
//!    any of `car_b`'s four active cell words (including the `-1` / `0xffff`
//!    sentinel).
//! 3. **Bounding-sphere check (`FUN_004438a0`)**: rejects when any axis
//!    difference exceeds `r_a + r_b` (`>`) or `d·d >= (r_a + r_b)^2`.
//! 4. **Two-body XZ OBB narrow phase (`FUN_00442860`)**: projects both cars'
//!    8-point deformed hulls onto XZ (`FUN_0043f250`/`FUN_0043f600` and
//!    `FUN_00443500`/`FUN_00441b80`), checks Y-span overlap, runs Pass 1
//!    (`car_b` corners in `car_a` OBB along `+u`) and Pass 2 (`car_a` corners
//!    in `car_b` OBB along `-u`) with 50/50 depenetration (`HALF = 0x8000`),
//!    publishes contact normal `u`, and sets each car's local lever arm `+0x3f0`
//!    from the post-depenetration center delta (`y -= 5.0` when relative speed
//!    exceeds `0.5`).
//! 5. **Two-body impulse, torque, damage, sound, and FFB (`FUN_00441f30`)**:
//!    clamps X and Z of `+0x3f0` (leaving Y unclamped), solves the inelastic
//!    normal velocity `v*`, transfers downward Y impulse off grounded cars
//!    (`+0x743 != 1`), accumulates `+0x3cc` and clamped `+0x3d8` (`±8/±4/±8`)
//!    for both cars, runs `FUN_0041f630` crash damage for both cars, doubles
//!    and saturates both severities to `1.0`, and—when `DAT_00552f5c == 0`
//!    (not already in contact on the previous tick)—plays material `5` impact
//!    sound and dispatches scalar + directional FFB for both cars.
//!
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.car-vs-car-collision`, `.work/car-damage-contact-notes.md`.

use crate::{
    fixed_point::{Fixed16, Vec3},
    live_static_collision_car_frame::{LiveCarProjectionCache, build_live_car_projection},
    moving_objects::{add, cross, dot, length, norm, q, qdiv, rows_dot, scale, sub},
    static_collision_response::{
        DamageResult, ResponseBasis, StaticDamageOwner, StaticDamageRequest, StaticResponseEvent,
        evaluate_static_damage, static_damage_emits_contact_effect,
    },
};

const HALF: i32 = 0x8000;
const ONE: i32 = 0x1_0000;
const ONE_AND_HALF: i32 = 0x1_8000;
const TWO: i32 = 0x2_0000;
const FIVE: i32 = 0x5_0000;
const TORQUE_LIMIT_XZ: i32 = 0x8_0000;
const TORQUE_LIMIT_Y: i32 = 0x4_0000;
const FFB_SCALAR_DURATION_4CCC: i32 = 0x4ccc;
const CAR_BODY_MATERIAL: u8 = 5;
const OPPONENT_HIT_TIMER_TICKS: i32 = 0x3c;

const fn vec3(raw: [i32; 3]) -> Vec3 {
    Vec3::new(
        Fixed16::from_raw(raw[0]),
        Fixed16::from_raw(raw[1]),
        Fixed16::from_raw(raw[2]),
    )
}

const fn raw(v: Vec3) -> [i32; 3] {
    [v.x.raw(), v.y.raw(), v.z.raw()]
}

const fn abs_i32(v: i32) -> i32 {
    if v < 0 { v.wrapping_neg() } else { v }
}

const fn clamp_axis(value: i32, limit: i32) -> i32 {
    if abs_i32(value) > limit {
        if value < 0 {
            limit.wrapping_neg()
        } else {
            limit
        }
    } else {
        value
    }
}

const fn clamp_range(value: i32, low: i32, high: i32) -> i32 {
    if value > high {
        high
    } else if value < low {
        low
    } else {
        value
    }
}

fn exit_distance(projected: i32, extent: i32, direction: i32) -> Option<i32> {
    if direction > 0x41 {
        Some(qdiv(extent.wrapping_sub(projected), direction))
    } else if direction < -0x41 {
        Some(qdiv(
            abs_i32(projected.wrapping_add(extent)),
            direction.wrapping_neg(),
        ))
    } else {
        None
    }
}

fn update_pass_max(best: &mut i32, projected: [i32; 2], extents: [i32; 2], direction: [i32; 2]) {
    let ta = exit_distance(projected[0], extents[0], direction[0]);
    let tp = exit_distance(projected[1], extents[1], direction[1]);
    let mut t = 0x7d00_0000;
    if let Some(v) = ta
        && v < t
    {
        t = v;
    }
    if let Some(v) = tp
        && v < t
    {
        t = v;
    }
    if (ta.is_some() || tp.is_some()) && t > *best {
        *best = t;
    }
}

/// `FUN_00485bb0`: returns `true` when the active configuration enables
/// network or super-special two-car mode (`cfg[0x166]` and `cfg[0x168]`).
#[must_use]
pub const fn is_network_or_super_special_485bb0(cfg_166: u8, cfg_168: u8) -> bool {
    if (cfg_166 & 0x40) != 0 {
        (cfg_168 & 0xf0) != 0x20
    } else if (cfg_166 & 0x20) != 0 {
        true
    } else {
        (cfg_168 & 0xf0) == 0x60
    }
}

/// `FUN_00401060`: returns `true` when the active configuration selects
/// two-player split-screen / head-to-head mode (`(cfg[0x166] & 0x1c) == 0x0c`)
/// and `DAT_00d3ca88 == 0`.
#[must_use]
pub const fn is_two_player_mode_401060(cfg_166: u8, mode_d3ca88: i32) -> bool {
    (cfg_166 & 0x1c) == 0x0c && mode_d3ca88 == 0
}

/// Complete `FUN_0043afd0` (`0x43b2cc..0x43b30e`) gate for `FUN_00442740`.
#[must_use]
pub const fn should_run_car_vs_car_43b2cc(
    active_cars_550e90: u32,
    mode_d3ca88: i32,
    cfg_166: u8,
    cfg_168: u8,
) -> bool {
    active_cars_550e90 == 2
        && mode_d3ca88 == 0
        && (is_network_or_super_special_485bb0(cfg_166, cfg_168)
            || is_two_player_mode_401060(cfg_166, mode_d3ca88))
}

/// `FUN_00442740` (`0x442767..0x4427d0`): 4×4 shared-cell check over the raw
/// `u16`/`i16` cell slots `+0x750..+0x756` (note: `-1` / `0xffff` slots are
/// compared without filtering).
#[must_use]
pub fn cars_share_cell_442767(cells_a_750: [i16; 4], cells_b_750: [i16; 4]) -> bool {
    for &cb in &cells_b_750 {
        for &ca in &cells_a_750 {
            if ca == cb {
                return true;
            }
        }
    }
    false
}

/// `FUN_004438a0`: bounding-sphere overlap between `car_a` and `car_b`.
///
/// Uses strict `>` on each axis (`abs(d[i]) > r`) and `<` on squared distance
/// (`dot(d, d) < q(r, r)`).
#[must_use]
pub fn car_sphere_overlap_4438a0(
    pos_a_c8: [i32; 3],
    radius_a_44: i32,
    pos_b_c8: [i32; 3],
    radius_b_44: i32,
) -> bool {
    let r = radius_b_44.wrapping_add(radius_a_44);
    let d = sub(pos_a_c8, pos_b_c8);
    if abs_i32(d[0]) > r || abs_i32(d[1]) > r || abs_i32(d[2]) > r {
        return false;
    }
    let d2 = dot(d, d);
    let r2 = q(r, r);
    d2 < r2
}

/// Per-car state read and mutated by `FUN_00442740`, `FUN_00442860`, and
/// `FUN_00441f30`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarVsCarState {
    pub position_c8: [i32; 3],
    pub previous_xz_3e4: [i32; 2],
    pub velocity_d4: [i32; 3],
    pub hull_points_fc: [[i32; 3]; 8],
    pub radius_44: i32,
    pub mass_48: i32,
    pub contact_limits_2c: [i32; 3],
    pub axis_258: [i32; 3],
    pub axis_264: [i32; 3],
    pub basis_2a0: [[i32; 3]; 3],
    pub linear_correction_3cc: [i32; 3],
    pub angular_correction_3d8: [i32; 3],
    pub contact_3f0: [i32; 3],
    pub opponent_hit_6e4: bool,
    pub car_type_738: i8,
    pub car_subtype_739: i8,
    pub flag_743: u8,
    pub active_cells_750: [i16; 4],
}

/// Persistent session state across ticks for `FUN_00442740` (`DAT_00552f5c`,
/// `DAT_00550014`, `DAT_00514458`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarVsCarSessionState {
    /// `DAT_00552f5c`: whether `FUN_00442860` reported a hit on the previous tick.
    pub prev_tick_hit_552f5c: bool,
    /// `DAT_00550014`: latched when `DAT_00550c08 == 1` and cars collide.
    pub opponent_contact_latch_550014: bool,
    /// `DAT_00514458`: set to `0x3c` (60 ticks) when `DAT_00550c08 == 1` and cars collide.
    pub opponent_contact_timer_514458: i32,
}

/// Narrow-phase hit returned by `FUN_00442860`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarVsCarNarrowHit {
    /// Published contact normal `DAT_00558fd8..00558fe0` (unit XZ vector from
    /// `car_a.previous_xz_3e4` toward `car_b.position_c8`).
    pub normal_558fd8: [i32; 3],
}

/// Resolved impulse, damage, sound, and FFB output of `FUN_00441f30`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarVsCarResolvedContact {
    pub normal: Vec3,
    pub delta_a: Vec3,
    pub delta_b: Vec3,
    pub world_contact_a: Vec3,
    pub sustained_contact: bool,
    pub raw_severity_a: Fixed16,
    pub raw_severity_b: Fixed16,
    pub doubled_severity_a: Fixed16,
    pub doubled_severity_b: Fixed16,
    pub damage_invoked_a: bool,
    pub damage_invoked_b: bool,
    pub events: Vec<StaticResponseEvent>,
}

fn build_car_projection(car: &CarVsCarState) -> LiveCarProjectionCache {
    build_live_car_projection(
        car.hull_points_fc.map(vec3),
        vec3(car.position_c8),
        vec3(car.axis_258),
        vec3(car.axis_264),
    )
}

/// `FUN_00442860`: two-body XZ OBB narrow phase, 50/50 depenetration split, and
/// local contact-arm (`+0x3f0`) setup for `car_a` and `car_b`.
#[must_use]
pub fn resolve_car_vs_car_narrow_phase_442860(
    car_a: &mut CarVsCarState,
    car_b: &mut CarVsCarState,
) -> Option<CarVsCarNarrowHit> {
    let proj_a = build_car_projection(car_a);
    let proj_b = build_car_projection(car_b);

    // Y-span overlap check (0x442866..0x442898)
    if proj_a.max_y.raw() < proj_b.min_y.raw() || proj_b.max_y.raw() < proj_a.min_y.raw() {
        return None;
    }

    let mut t1 = proj_a.projected_corners.map(raw);
    let mut t2 = proj_b.projected_corners.map(raw);
    let a1 = raw(proj_a.axis);
    let p1 = raw(proj_a.perpendicular);
    let r1a = proj_a.axis_radius.raw();
    let r1p = proj_a.perpendicular_radius.raw();
    let a2 = raw(proj_b.axis);
    let p2 = raw(proj_b.perpendicular);
    let r2a = proj_b.axis_radius.raw();
    let r2p = proj_b.perpendicular_radius.raw();

    // Unit horizontal separation vector u from car_a.previous_xz_3e4 to car_b.position_c8
    let u = norm([
        car_b.position_c8[0].wrapping_sub(car_a.previous_xz_3e4[0]),
        0,
        car_b.position_c8[2].wrapping_sub(car_a.previous_xz_3e4[1]),
    ]);

    let mut hit = false;
    let mut normal = [0; 3];

    // Pass 1 (0x4429b9..0x442e47): car_b corners T2 inside car_a OBB along +u.
    let dir_a = [dot(a1, u), dot(p1, u)];
    let mut best1 = 0;
    let mut pass1_hit = false;
    for corner in t2.iter().copied() {
        let rel = [
            corner[0].wrapping_sub(car_a.position_c8[0]),
            0,
            corner[2].wrapping_sub(car_a.position_c8[2]),
        ];
        let proj = [dot(a1, rel), dot(p1, rel)];
        if abs_i32(proj[0]) <= r1a && abs_i32(proj[1]) <= r1p {
            update_pass_max(&mut best1, proj, [r1a, r1p], dir_a);
            pass1_hit = true;
            hit = true;
        }
    }
    if pass1_hit {
        normal = u;
        if best1 > 0 {
            let k_b = q(best1, HALF);
            let rem_a = best1.wrapping_sub(k_b);
            let shift_b = scale(u, k_b);
            let shift_a = scale(u, rem_a.wrapping_neg());
            car_b.position_c8 = add(car_b.position_c8, shift_b);
            for c in &mut car_b.hull_points_fc {
                *c = add(*c, shift_b);
            }
            for c in &mut t2 {
                *c = add(*c, shift_b);
            }
            car_a.position_c8 = add(car_a.position_c8, shift_a);
            for c in &mut car_a.hull_points_fc {
                *c = add(*c, shift_a);
            }
            for c in &mut t1 {
                *c = add(*c, shift_a);
            }
        }
    }

    // Pass 2 (0x442e48..0x443316): updated car_a corners T1 inside car_b OBB along -u.
    let neg_u = scale(u, -ONE);
    let dir_b = [dot(a2, neg_u), dot(p2, neg_u)];
    let mut best2 = 0;
    let mut pass2_hit = false;
    for corner in t1.iter().copied() {
        let rel = [
            corner[0].wrapping_sub(car_b.position_c8[0]),
            0,
            corner[2].wrapping_sub(car_b.position_c8[2]),
        ];
        let proj = [dot(a2, rel), dot(p2, rel)];
        if abs_i32(proj[0]) <= r2a && abs_i32(proj[1]) <= r2p {
            update_pass_max(&mut best2, proj, [r2a, r2p], dir_b);
            pass2_hit = true;
            hit = true;
        }
    }
    if pass2_hit {
        normal = u;
        if best2 > 0 {
            let k_b = q(best2, HALF);
            let rem_a = best2.wrapping_sub(k_b);
            let shift_b = scale(u, k_b);
            let shift_a = scale(u, rem_a.wrapping_neg());
            car_b.position_c8 = add(car_b.position_c8, shift_b);
            for c in &mut car_b.hull_points_fc {
                *c = add(*c, shift_b);
            }
            for c in &mut t2 {
                *c = add(*c, shift_b);
            }
            car_a.position_c8 = add(car_a.position_c8, shift_a);
            for c in &mut car_a.hull_points_fc {
                *c = add(*c, shift_a);
            }
            for c in &mut t1 {
                *c = add(*c, shift_a);
            }
        }
    }

    if !hit {
        return None;
    }

    // Local contact lever arms (0x443322..0x4434e5)
    let mut d_ab = sub(car_b.position_c8, car_a.position_c8);
    let mut d_ba = scale(d_ab, -ONE);
    if length(sub(car_a.velocity_d4, car_b.velocity_d4)) > HALF {
        d_ab[1] = d_ab[1].wrapping_sub(FIVE);
        d_ba[1] = d_ba[1].wrapping_sub(FIVE);
    }
    car_b.contact_3f0 = rows_dot(&car_b.basis_2a0, d_ba);
    car_a.contact_3f0 = rows_dot(&car_a.basis_2a0, d_ab);

    Some(CarVsCarNarrowHit {
        normal_558fd8: normal,
    })
}

fn double_and_saturate_severity(severity: i32) -> i32 {
    if severity >= HALF {
        ONE
    } else {
        q(severity, TWO)
    }
}

/// `FUN_00441f30`: two-body inelastic normal impulse, grounded Y-transfer,
/// linear/angular correction accumulation, crash-damage evaluation for both
/// cars, and (when `!prev_tick_hit_552f5c`) impact sound + FFB dispatch.
pub fn apply_car_vs_car_response_441f30<D: StaticDamageOwner>(
    car_a: &mut CarVsCarState,
    car_b: &mut CarVsCarState,
    hit: CarVsCarNarrowHit,
    prev_tick_hit_552f5c: bool,
    damage_owner: &mut D,
) -> Result<CarVsCarResolvedContact, D::Error> {
    let mut events = Vec::new();
    let effects_enabled = if car_a.car_type_738 == 1 {
        events.push(StaticResponseEvent::QueryTypeOneEffectGate);
        !damage_owner.type_one_effect_gate_blocked()?
    } else {
        true
    };

    // Entry clamps on X (+0x3f0) and Z (+0x3f8) only; Y (+0x3f4) is left unclamped (0x441f5f..0x441ff0).
    car_a.contact_3f0[0] = clamp_axis(car_a.contact_3f0[0], car_a.contact_limits_2c[0]);
    car_a.contact_3f0[2] = clamp_axis(car_a.contact_3f0[2], car_a.contact_limits_2c[2]);
    car_b.contact_3f0[0] = clamp_axis(car_b.contact_3f0[0], car_b.contact_limits_2c[0]);
    car_b.contact_3f0[2] = clamp_axis(car_b.contact_3f0[2], car_b.contact_limits_2c[2]);

    // Normal impulse & grounded Y-transfer (0x441ff0..0x4421c2)
    let n = hit.normal_558fd8;
    let v_a = dot(n, car_a.velocity_d4);
    let v_b = dot(n, car_b.velocity_d4);
    let e = q(v_a.wrapping_sub(v_b), 0);
    let m_a = car_a.mass_48;
    let m_b = car_b.mass_48;
    let num = q(v_b, m_b).wrapping_sub(q(e, m_b)).wrapping_add(q(v_a, m_a));
    let den = m_b.wrapping_add(m_a);
    let v_star = if den == 0 { -1 } else { qdiv(num, den) };

    let mut delta_a = scale(n, v_star.wrapping_sub(v_a));
    let mut delta_b = scale(n, v_star.wrapping_sub(v_b).wrapping_add(e));

    if car_a.flag_743 != 1 && delta_a[1] < 0 {
        delta_b[1] = delta_b[1].wrapping_sub(delta_a[1]);
        delta_a[1] = 0;
    }
    if car_b.flag_743 != 1 && delta_b[1] < 0 {
        delta_a[1] = delta_a[1].wrapping_sub(delta_b[1]);
        delta_b[1] = 0;
    }

    // Car A linear & angular correction accumulation (0x4421c2..0x44231f)
    let m_a_body = rows_dot(&car_a.basis_2a0, delta_a);
    car_a.linear_correction_3cc = add(car_a.linear_correction_3cc, delta_a);
    let tq_a = cross(m_a_body, car_a.contact_3f0);
    car_a.angular_correction_3d8 = add(car_a.angular_correction_3d8, tq_a);
    car_a.angular_correction_3d8[0] = clamp_range(
        car_a.angular_correction_3d8[0],
        -TORQUE_LIMIT_XZ,
        TORQUE_LIMIT_XZ,
    );
    car_a.angular_correction_3d8[1] = clamp_range(
        car_a.angular_correction_3d8[1],
        -TORQUE_LIMIT_Y,
        TORQUE_LIMIT_Y,
    );
    car_a.angular_correction_3d8[2] = clamp_range(
        car_a.angular_correction_3d8[2],
        -TORQUE_LIMIT_XZ,
        TORQUE_LIMIT_XZ,
    );

    // Car B linear & angular correction accumulation (0x44231f..0x442480)
    let m_b_body = rows_dot(&car_b.basis_2a0, delta_b);
    car_b.linear_correction_3cc = add(car_b.linear_correction_3cc, delta_b);
    let tq_b = cross(m_b_body, car_b.contact_3f0);
    car_b.angular_correction_3d8 = add(car_b.angular_correction_3d8, tq_b);
    car_b.angular_correction_3d8[0] = clamp_range(
        car_b.angular_correction_3d8[0],
        -TORQUE_LIMIT_XZ,
        TORQUE_LIMIT_XZ,
    );
    car_b.angular_correction_3d8[1] = clamp_range(
        car_b.angular_correction_3d8[1],
        -TORQUE_LIMIT_Y,
        TORQUE_LIMIT_Y,
    );
    car_b.angular_correction_3d8[2] = clamp_range(
        car_b.angular_correction_3d8[2],
        -TORQUE_LIMIT_XZ,
        TORQUE_LIMIT_XZ,
    );

    // Damage calls for both cars (0x442480..0x4424e0)
    let linear_vec_a = vec3(car_a.linear_correction_3cc);
    events.push(StaticResponseEvent::RequestDamage {
        linear_correction: linear_vec_a,
    });
    let eval_a = evaluate_static_damage(linear_vec_a);
    let mut damage_result_a = DamageResult::default();
    if eval_a.damage_sequence_invoked {
        damage_result_a = damage_owner.commit_contact(StaticDamageRequest {
            car_type_738: car_a.car_type_738,
            car_subtype_739: car_a.car_subtype_739,
            contact_3f0: vec3(car_a.contact_3f0),
            linear_correction_3cc: linear_vec_a,
            severity: eval_a.severity,
        })?;
    }

    let linear_vec_b = vec3(car_b.linear_correction_3cc);
    events.push(StaticResponseEvent::RequestDamage {
        linear_correction: linear_vec_b,
    });
    let eval_b = evaluate_static_damage(linear_vec_b);
    if eval_b.damage_sequence_invoked {
        let _ = damage_owner.commit_contact(StaticDamageRequest {
            car_type_738: car_b.car_type_738,
            car_subtype_739: car_b.car_subtype_739,
            contact_3f0: vec3(car_b.contact_3f0),
            linear_correction_3cc: linear_vec_b,
            severity: eval_b.severity,
        })?;
    }

    let doubled_a = Fixed16::from_raw(double_and_saturate_severity(eval_a.severity.raw()));
    let doubled_b = Fixed16::from_raw(double_and_saturate_severity(eval_b.severity.raw()));

    let basis_a = ResponseBasis {
        rows: car_a.basis_2a0.map(vec3),
    };
    let world_contact_a = basis_a
        .body_to_world(vec3(car_a.contact_3f0))
        .add(vec3(car_a.position_c8));

    if effects_enabled {
        if static_damage_emits_contact_effect(eval_a, damage_result_a.effect_handle) {
            events.push(StaticResponseEvent::ContactEffect {
                handle: damage_result_a.effect_handle,
                world_contact: world_contact_a,
            });
        }
        if !prev_tick_hit_552f5c && doubled_a.raw() > 0 {
            events.push(StaticResponseEvent::MaterialImpact {
                severity: doubled_a,
                world_contact: world_contact_a,
                material: CAR_BODY_MATERIAL,
            });
            let ffb_scalar_gain = Fixed16::from_raw(q(doubled_a.raw(), ONE_AND_HALF));
            let ffb_dir_gain = Fixed16::from_raw(q(doubled_a.raw(), TWO));
            events.push(StaticResponseEvent::ScalarFeedback {
                car_type: car_a.car_type_738,
                strength: ffb_scalar_gain,
                threshold: Fixed16::from_raw(FFB_SCALAR_DURATION_4CCC),
            });
            events.push(StaticResponseEvent::ScalarFeedback {
                car_type: car_b.car_type_738,
                strength: ffb_scalar_gain,
                threshold: Fixed16::from_raw(FFB_SCALAR_DURATION_4CCC),
            });
            events.push(StaticResponseEvent::DirectionalFeedback {
                car_type: car_a.car_type_738,
                direction: vec3(delta_a),
                strength: ffb_dir_gain,
                limit: Fixed16::from_raw(TWO),
                trailing_zero: 0,
            });
            events.push(StaticResponseEvent::DirectionalFeedback {
                car_type: car_b.car_type_738,
                direction: vec3(delta_b),
                strength: ffb_dir_gain,
                limit: Fixed16::from_raw(TWO),
                trailing_zero: 0,
            });
        }
    }

    Ok(CarVsCarResolvedContact {
        normal: vec3(n),
        delta_a: vec3(delta_a),
        delta_b: vec3(delta_b),
        world_contact_a,
        sustained_contact: prev_tick_hit_552f5c,
        raw_severity_a: eval_a.severity,
        raw_severity_b: eval_b.severity,
        doubled_severity_a: doubled_a,
        doubled_severity_b: doubled_b,
        damage_invoked_a: eval_a.damage_sequence_invoked,
        damage_invoked_b: eval_b.damage_sequence_invoked,
        events,
    })
}

/// `FUN_00442740`: full per-tick pair dispatcher combining the mode gate,
/// shared-cell broadphase, bounding-sphere check (`FUN_004438a0`), OBB narrow
/// phase (`FUN_00442860`), impulse/damage response (`FUN_00441f30`), and
/// session flag updates (`DAT_00552f5c`, `DAT_00550014`, `DAT_00514458`).
pub fn resolve_car_vs_car_collision_442740<D: StaticDamageOwner>(
    car_a: &mut CarVsCarState,
    car_b: &mut CarVsCarState,
    session: &mut CarVsCarSessionState,
    mode_d3ca88: i32,
    replay_car_count_550c08: i32,
    damage_owner: &mut D,
) -> Result<Option<CarVsCarResolvedContact>, D::Error> {
    if mode_d3ca88 != 0 {
        return Ok(None);
    }
    if !cars_share_cell_442767(car_a.active_cells_750, car_b.active_cells_750) {
        session.prev_tick_hit_552f5c = false;
        return Ok(None);
    }
    if !car_sphere_overlap_4438a0(
        car_a.position_c8,
        car_a.radius_44,
        car_b.position_c8,
        car_b.radius_44,
    ) {
        session.prev_tick_hit_552f5c = false;
        return Ok(None);
    }
    let Some(hit) = resolve_car_vs_car_narrow_phase_442860(car_a, car_b) else {
        session.prev_tick_hit_552f5c = false;
        return Ok(None);
    };
    let resolved = apply_car_vs_car_response_441f30(
        car_a,
        car_b,
        hit,
        session.prev_tick_hit_552f5c,
        damage_owner,
    )?;
    if replay_car_count_550c08 == 1 {
        car_b.opponent_hit_6e4 = true;
        session.opponent_contact_latch_550014 = true;
        session.opponent_contact_timer_514458 = OPPONENT_HIT_TIMER_TICKS;
    }
    session.prev_tick_hit_552f5c = true;
    Ok(Some(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moving_objects::corners;
    use std::convert::Infallible;

    #[derive(Default)]
    struct RecordingDamageOwner {
        requests: Vec<StaticDamageRequest>,
    }

    impl StaticDamageOwner for RecordingDamageOwner {
        type Error = Infallible;

        fn type_one_effect_gate_blocked(&mut self) -> Result<bool, Self::Error> {
            Ok(false)
        }

        fn commit_contact(
            &mut self,
            request: StaticDamageRequest,
        ) -> Result<DamageResult, Self::Error> {
            self.requests.push(request);
            Ok(DamageResult { effect_handle: 7 })
        }

        fn capture_debug_material(&self) -> bool {
            false
        }
    }

    fn sample_car(pos_x: i32, vel_x: i32, car_type: i8) -> CarVsCarState {
        let pos = [pos_x, 0, 0];
        let rows = [[ONE, 0, 0], [0, ONE, 0], [0, 0, ONE]];
        let half = [0x2_0000, 0x0_b000, 0x0_e000];
        CarVsCarState {
            position_c8: pos,
            previous_xz_3e4: [pos_x.wrapping_sub(vel_x), 0],
            velocity_d4: [vel_x, 0, 0],
            hull_points_fc: corners(pos, &rows, half),
            radius_44: 0x2_8000,
            mass_48: ONE,
            contact_limits_2c: half,
            axis_258: rows[0],
            axis_264: rows[1],
            basis_2a0: rows,
            linear_correction_3cc: [0; 3],
            angular_correction_3d8: [0; 3],
            contact_3f0: [0; 3],
            opponent_hit_6e4: false,
            car_type_738: car_type,
            car_subtype_739: 0,
            flag_743: 0,
            active_cells_750: [12, -1, -1, -1],
        }
    }

    #[test]
    fn mode_gates_match_fun_00485bb0_and_fun_00401060() {
        // Super-special stage (cfg_168 & 0xf0 == 0x60) enables collision when 2 cars and mode_d3ca88 == 0
        assert!(should_run_car_vs_car_43b2cc(2, 0, 0x00, 0x60));
        // Split-screen mode (cfg_166 & 0x1c == 0x0c) enables collision
        assert!(should_run_car_vs_car_43b2cc(2, 0, 0x0c, 0x00));
        // Single car never runs
        assert!(!should_run_car_vs_car_43b2cc(1, 0, 0x0c, 0x60));
        // Non-zero DAT_00d3ca88 (school / replay) suppresses collision
        assert!(!should_run_car_vs_car_43b2cc(2, 1, 0x0c, 0x60));
        // cfg_166 & 0x40 with cfg_168 & 0xf0 == 0x20 disables network branch
        assert!(!is_network_or_super_special_485bb0(0x40, 0x20));
        assert!(is_network_or_super_special_485bb0(0x40, 0x10));
    }

    #[test]
    fn sphere_overlap_uses_strict_axis_greater_and_distance_less() {
        let a = [0, 0, 0];
        let r = ONE;
        // Exact boundary abs(dx) == 2.0 passes axis precheck, fails d2 < r2
        assert!(!car_sphere_overlap_4438a0(a, r, [2 * ONE, 0, 0], r));
        // Slightly inside 2.0 overlaps
        assert!(car_sphere_overlap_4438a0(a, r, [2 * ONE - 0x100, 0, 0], r));
    }

    #[test]
    fn narrow_phase_splits_depenetration_50_50_and_lowers_contact_y_when_fast() {
        let mut car_a = sample_car(0, 0x1_0000, 0);
        let mut car_b = sample_car(0x3_0000, -0x1_0000, 1);

        let hit = resolve_car_vs_car_narrow_phase_442860(&mut car_a, &mut car_b)
            .expect("overlapping cars should collide");
        assert_eq!(hit.normal_558fd8, norm([0x4_0000, 0, 0]));
        assert!(hit.normal_558fd8[0] > 0);
        // Equal and opposite X shifts applied to car_a and car_b (within 1 LSB of Q16 floor shift)
        assert!(car_a.position_c8[0] < 0);
        assert!(car_b.position_c8[0] > 0x3_0000);
        assert!(
            (car_a.position_c8[0].wrapping_add(car_b.position_c8[0]) - 0x3_0000).abs() <= 1
        );
        // Relative speed is 2.0 (> 0.5), so both local contact Y values drop by 5.0 (-0x5_0000)
        assert_eq!(car_a.contact_3f0[1], -FIVE);
        assert_eq!(car_b.contact_3f0[1], -FIVE);
    }

    #[test]
    fn grounded_y_transfer_moves_downward_impulse_to_other_car() {
        let mut car_a = sample_car(0, 0, 0);
        let mut car_b = sample_car(0x2_0000, 0, 1);
        car_a.velocity_d4 = [0, 0x2_0000, 0];
        car_b.velocity_d4 = [0, -0x2_0000, 0];
        car_a.flag_743 = 0; // grounded (not 1)
        car_b.flag_743 = 1; // airborne

        let mut owner = RecordingDamageOwner::default();
        let resolved = apply_car_vs_car_response_441f30(
            &mut car_a,
            &mut car_b,
            CarVsCarNarrowHit {
                normal_558fd8: [0, ONE, 0],
            },
            false,
            &mut owner,
        )
        .unwrap();

        // car_a would receive negative delta_a.y (-2.0), which transfers into delta_b.y
        assert_eq!(resolved.delta_a.y.raw(), 0);
        assert_eq!(resolved.delta_b.y.raw(), 0x4_0000);
        assert_eq!(car_a.linear_correction_3cc[1], 0);
        assert_eq!(car_b.linear_correction_3cc[1], 0x4_0000);
    }

    #[test]
    fn response_leaves_contact_y_unclamped_and_gates_sound_on_prev_tick_hit() {
        let mut car_a = sample_car(0, 0x1_0000, 0);
        let mut car_b = sample_car(0x3_0000, -0x1_0000, 0);
        let mut session = CarVsCarSessionState::default();
        let mut owner = RecordingDamageOwner::default();

        // First tick of contact: prev_tick_hit_552f5c is false -> emits MaterialImpact(5) + 4 FFB events
        let first = resolve_car_vs_car_collision_442740(
            &mut car_a,
            &mut car_b,
            &mut session,
            0,
            1,
            &mut owner,
        )
        .unwrap()
        .expect("contact on tick 1");
        assert!(!first.sustained_contact);
        assert!(session.prev_tick_hit_552f5c);
        assert!(car_b.opponent_hit_6e4);
        assert!(session.opponent_contact_latch_550014);
        assert_eq!(
            session.opponent_contact_timer_514458,
            OPPONENT_HIT_TIMER_TICKS
        );
        // X is clamped to +0x2_0000 / -0x2_0000, while Y remains unclamped at -5.0 (-0x5_0000)
        assert_eq!(car_a.contact_3f0[0], 0x2_0000);
        assert_eq!(car_a.contact_3f0[1], -FIVE);
        assert_eq!(owner.requests.len(), 2);
        assert!(first.events.iter().any(|e| matches!(
            e,
            StaticResponseEvent::MaterialImpact {
                material: CAR_BODY_MATERIAL,
                ..
            }
        )));

        // Second consecutive tick of contact: prev_tick_hit_552f5c is true -> damage still runs, but no MaterialImpact/FFB
        let mut car_a2 = sample_car(0, 0x1_0000, 0);
        let mut car_b2 = sample_car(0x3_0000, -0x1_0000, 0);
        let second = resolve_car_vs_car_collision_442740(
            &mut car_a2,
            &mut car_b2,
            &mut session,
            0,
            1,
            &mut owner,
        )
        .unwrap()
        .expect("contact on tick 2");
        assert!(second.sustained_contact);
        assert_eq!(owner.requests.len(), 4);
        assert!(
            !second
                .events
                .iter()
                .any(|e| matches!(e, StaticResponseEvent::MaterialImpact { .. }))
        );
    }
}
