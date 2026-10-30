//! Car vs moving-object collision (`FUN_0043f790`, `FUN_0043fa10`,
//! `FUN_00440a10`, `FUN_00441b80`, `FUN_00440d90`, `FUN_0043faf0`,
//! `FUN_004439a0`, `FUN_004439b0`).
//!
//! Runs immediately after static scenery collision (`FUN_0043d5d0`) while the
//! car's upper hull points are still deformed by `FUN_00417f60`. Walks the
//! car's four active grid cells (`+0x750..+0x756`), tests non-inert moving
//! objects with a bounding-sphere check (`FUN_0043fa10`) and a two-body XZ
//! projected OBB narrow phase (`FUN_00440d90`), shares depenetration between
//! the object and car by mass class (`0x4f1500`: 1.0, 0.4, 0.0), applies
//! inelastic two-body impulse/torque response (`FUN_0043faf0`), marks hit
//! objects `state::HIT`, updates the rally-school recent-hit cache
//! (`FUN_004439b0`), and rotates the car's five-slot moving-object contact
//! history (`+0x71e..+0x733`).
//!
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.moving-objects`, `.work/moving-objects-notes.md`.

use crate::{
    fixed_point::{Fixed16, Vec3},
    live_static_collision_car_frame::{LiveCarProjectionCache, build_live_car_projection},
    moving_objects::{
        MASS, MovingObject, MovingObjects, add, corners, cross, dot, length, norm, q, qdiv,
        rows_dot, scale, state, sub,
    },
    static_collision_response::{
        DamageResult, ExceptionalResponseKind, ResponseBasis, StaticDamageOwner,
        StaticDamageRequest, StaticResponseEvent, StaticResponseHistory, evaluate_static_damage,
        static_damage_emits_contact_effect,
    },
    static_collision_response_input::{
        StaticResponseInputProducerError, StaticResponseInputProducerState,
        produce_static_response_input, static_response_canonical_axes,
    },
    static_world_collision::StaticCollisionNodeId,
};

/// `0x4f1500`: the object's share of depenetration by mass index (1.0, 0.4, 0.0).
pub const SHARE: [i32; 3] = [0x1_0000, 0x6666, 0];

const HALF: i32 = 0x8000;
const TWO: i32 = 0x2_0000;
const ROLLOVER_KICK_Y: i32 = -0x32_0000;
const ROLLOVER_X_SCALE: i32 = 0x64_0000;
const ROLLOVER_Z_SCALE: i32 = 0x32_0000;

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

/// Recent moving-object hit cache (`0x552f64`, `0x552f68`, `0x55901c[3]`).
///
/// `FUN_0043f790` increments the counter and clears `object_contact_552f68` at
/// the start of each pass (`FUN_004439a0`). Each hit calls `record_hit`
/// (`FUN_004439b0`), which expires the 3-node ring after `50 * (double_rate ? 2 : 1)`
/// passes and sets `object_contact_552f68 = true` when the hit node is not
/// already in the ring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MovingObjectHitCache {
    pub tick_counter_552f64: i32,
    pub object_contact_552f68: bool,
    pub recent_nodes_55901c: [Option<usize>; 3],
}

impl MovingObjectHitCache {
    /// Reset the cache (`FUN_004439a0` init).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Per-pass entry in `FUN_0043f790`: increments `0x552f64` and zeroes `0x552f68`.
    pub fn begin_pass(&mut self) {
        self.tick_counter_552f64 = self.tick_counter_552f64.wrapping_add(1);
        self.object_contact_552f68 = false;
    }

    /// `FUN_004439b0`: record a hit on `node`.
    pub fn record_hit(&mut self, node: usize, double_rate_551af0: bool) {
        let limit = if double_rate_551af0 { 100 } else { 50 };
        if self.tick_counter_552f64 > limit {
            self.recent_nodes_55901c = [None; 3];
            self.tick_counter_552f64 = 0;
        }
        if !self.recent_nodes_55901c.contains(&Some(node)) {
            self.object_contact_552f68 = true;
            self.recent_nodes_55901c[2] = self.recent_nodes_55901c[1];
            self.recent_nodes_55901c[1] = self.recent_nodes_55901c[0];
            self.recent_nodes_55901c[0] = Some(node);
        }
    }
}

/// `FUN_0043fa10`: bounding-sphere overlap between car (`+0xc8`, `+0x44`) and
/// object (`+0x04`, `+0x88`).
#[must_use]
pub fn sphere_overlap(
    car_position: [i32; 3],
    car_radius_44: i32,
    object_position: [i32; 3],
    object_radius: i32,
) -> bool {
    let d = sub(car_position, object_position);
    let r = car_radius_44.wrapping_add(object_radius);
    if abs_i32(d[0]) >= r || abs_i32(d[1]) >= r || abs_i32(d[2]) >= r {
        return false;
    }
    let d2 = dot(d, d);
    let r2 = q(r, r);
    if r2 < 0 { length(d) < r } else { d2 < r2 }
}

/// Live car state read and mutated by `FUN_0043f790`, `FUN_00440d90`, and
/// `FUN_0043faf0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovingObjectCarState {
    pub position_c8: [i32; 3],
    pub previous_xz_3e4: [i32; 2],
    pub velocity_d4: [i32; 3],
    pub velocity_normalizer_ec: i32,
    pub hull_points_fc: [[i32; 3]; 8],
    pub radius_44: i32,
    pub mass_48: i32,
    pub contact_limits_2c: [i32; 3],
    pub axis_258: [i32; 3],
    pub axis_264: [i32; 3],
    pub projected_axis_270: [i32; 3],
    pub basis_2a0: [[i32; 3]; 3],
    pub linear_correction_3cc: [i32; 3],
    pub angular_correction_3d8: [i32; 3],
    pub contact_3f0: [i32; 3],
    pub contact_count_674: i32,
    pub out_of_control_680: bool,
    pub out_of_control_688: bool,
    pub flag_743: u8,
    pub car_type_738: i8,
    pub car_subtype_739: i8,
    pub active_nodes_750: [Option<StaticCollisionNodeId>; 4],
    pub object_history: StaticResponseHistory,
}

/// Narrow-phase output passed from `FUN_00440d90` into `FUN_0043faf0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovingObjectNarrowHit {
    pub normal: [i32; 3],
    pub object_contact_552f50: [i32; 3],
    pub primary_limit_552f6c: i32,
    pub projected_limit_558fc4: i32,
}

/// One resolved car-vs-moving-object contact in a pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovingObjectResolvedContact {
    pub object_index: usize,
    pub node: usize,
    pub contact_id: i32,
    pub material: u8,
    pub normal: Vec3,
    pub world_contact: Vec3,
    pub repeated_from_previous_tick: bool,
    pub damage_severity: Option<Fixed16>,
    pub damage_sequence_invoked: bool,
    pub events: Vec<StaticResponseEvent>,
    pub debug_material_write: Option<u8>,
}

/// Aggregate result of `resolve_moving_object_collisions` (`FUN_0043f790`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovingObjectCollisionEffects {
    pub contacts: Vec<MovingObjectResolvedContact>,
    pub object_contact_552f68: bool,
    pub debug_material_558fb0: Option<u8>,
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

fn build_projection_raw(
    points: [[i32; 3]; 8],
    center: [i32; 3],
    row0: [i32; 3],
    row1: [i32; 3],
) -> LiveCarProjectionCache {
    build_live_car_projection(points.map(vec3), vec3(center), vec3(row0), vec3(row1))
}

/// `FUN_00440d90`: two-body XZ projected OBB narrow phase with mass-shared
/// depenetration and `FUN_0043c6f0` response threshold setup.
pub fn resolve_moving_object_narrow_phase(
    car: &mut MovingObjectCarState,
    car_proj: &mut LiveCarProjectionCache,
    object: &mut MovingObject,
) -> Result<Option<MovingObjectNarrowHit>, StaticResponseInputProducerError> {
    let mut obj_corners = corners(object.position, &object.rows, object.half);
    let obj_proj =
        build_projection_raw(obj_corners, object.position, object.rows[0], object.rows[1]);

    if car_proj.max_y.raw() < obj_proj.min_y.raw() || obj_proj.max_y.raw() < car_proj.min_y.raw() {
        return Ok(None);
    }

    let mut t1 = car_proj.projected_corners.map(raw);
    let mut t2 = obj_proj.projected_corners.map(raw);
    let a1 = raw(car_proj.axis);
    let p1 = raw(car_proj.perpendicular);
    let r1a = car_proj.axis_radius.raw();
    let r1p = car_proj.perpendicular_radius.raw();
    let a2 = raw(obj_proj.axis);
    let p2 = raw(obj_proj.perpendicular);
    let r2a = obj_proj.axis_radius.raw();
    let r2p = obj_proj.perpendicular_radius.raw();

    let u = norm([
        object.position[0].wrapping_sub(car.previous_xz_3e4[0]),
        0,
        object.position[2].wrapping_sub(car.previous_xz_3e4[1]),
    ]);

    let mut list_a = [0usize; 4];
    let mut len_a = 0usize;
    let mut list_b = [0usize; 4];
    let mut len_b = 0usize;
    let mut hit = false;
    let mut normal = [0; 3];

    // Pass 1: object projected corners T2 against car XZ box along u.
    let dir_car = [dot(a1, u), dot(p1, u)];
    let mut best1 = 0;
    for (k, corner) in t2.iter().copied().enumerate() {
        let rel = [
            corner[0].wrapping_sub(car.position_c8[0]),
            0,
            corner[2].wrapping_sub(car.position_c8[2]),
        ];
        let proj = [dot(a1, rel), dot(p1, rel)];
        if abs_i32(proj[0]) <= r1a && abs_i32(proj[1]) <= r1p {
            update_pass_max(&mut best1, proj, [r1a, r1p], dir_car);
            list_a[len_a] = k;
            len_a += 1;
            hit = true;
        }
    }
    if len_a > 0 {
        normal = u;
        if best1 > 0 {
            let s = SHARE[object.mass_index()];
            let k_obj = q(best1, s);
            let rem_car = best1.wrapping_sub(k_obj);
            let shift_obj = scale(u, k_obj);
            let shift_car = scale(u, rem_car.wrapping_neg());
            object.position = add(object.position, shift_obj);
            for c in &mut obj_corners {
                *c = add(*c, shift_obj);
            }
            for c in &mut t2 {
                *c = add(*c, shift_obj);
            }
            car.position_c8 = add(car.position_c8, shift_car);
            for c in &mut car.hull_points_fc {
                *c = add(*c, shift_car);
            }
            for c in &mut t1 {
                *c = add(*c, shift_car);
            }
        }
    }

    // Pass 2: car projected corners T1 against object XZ box along -u.
    let neg_u = scale(u, -0x1_0000);
    let dir_obj = [dot(a2, neg_u), dot(p2, neg_u)];
    let mut best2 = 0;
    for (k, corner) in t1.iter().copied().enumerate() {
        let rel = [
            corner[0].wrapping_sub(object.position[0]),
            0,
            corner[2].wrapping_sub(object.position[2]),
        ];
        let proj = [dot(a2, rel), dot(p2, rel)];
        if abs_i32(proj[0]) <= r2a && abs_i32(proj[1]) <= r2p {
            update_pass_max(&mut best2, proj, [r2a, r2p], dir_obj);
            list_b[len_b] = k;
            len_b += 1;
            hit = true;
        }
    }
    if len_b > 0 {
        normal = u;
        if best2 > 0 {
            let s = SHARE[object.mass_index()];
            let k_obj = q(best2, s);
            let rem_car = best2.wrapping_sub(k_obj);
            let shift_obj = scale(u, k_obj);
            let shift_car = scale(u, rem_car.wrapping_neg());
            object.position = add(object.position, shift_obj);
            for c in &mut obj_corners {
                *c = add(*c, shift_obj);
            }
            for c in &mut t2 {
                *c = add(*c, shift_obj);
            }
            car.position_c8 = add(car.position_c8, shift_car);
            for c in &mut car.hull_points_fc {
                *c = add(*c, shift_car);
            }
            for c in &mut t1 {
                *c = add(*c, shift_car);
            }
        }
    }

    car_proj.projected_corners = t1.map(vec3);

    if !hit {
        return Ok(None);
    }

    let mut d = sub(object.position, car.position_c8);
    if length(sub(car.velocity_d4, object.velocity)) > HALF {
        d[1] = d[1].wrapping_add(0x2_8000);
    }
    let object_contact_552f50 = rows_dot(&object.rows, d).map(i32::wrapping_neg);

    let avg = if len_b > 0 {
        let mut sx = 0i32;
        let mut sz = 0i32;
        for &idx in &list_b[..len_b] {
            sx = sx.wrapping_add(t1[idx][0].wrapping_sub(car.position_c8[0]));
            sz = sz.wrapping_add(t1[idx][2].wrapping_sub(car.position_c8[2]));
        }
        let inv = qdiv(0x1_0000, (len_b as i32) << 16);
        [q(sx, inv), 0, q(sz, inv)]
    } else if len_a > 0 {
        let mut sx = 0i32;
        let mut sz = 0i32;
        for &idx in &list_a[..len_a] {
            sx = sx.wrapping_add(t2[idx][0].wrapping_sub(car.position_c8[0]));
            sz = sz.wrapping_add(t2[idx][2].wrapping_sub(car.position_c8[2]));
        }
        let inv = qdiv(0x1_0000, (len_a as i32) << 16);
        [q(sx, inv), 0, q(sz, inv)]
    } else {
        [0; 3]
    };
    car.contact_3f0 = rows_dot(&car.basis_2a0, avg);

    let canonical = static_response_canonical_axes();
    let produced = produce_static_response_input(StaticResponseInputProducerState {
        existing_normal: vec3(normal),
        contact_3f0: vec3(car.contact_3f0),
        contact_limits_2c: vec3(car.contact_limits_2c),
        contact_count_674: car.contact_count_674,
        velocity_normalizer_ec: Fixed16::from_raw(car.velocity_normalizer_ec),
        velocity_d4: vec3(car.velocity_d4),
        basis_2a0: ResponseBasis {
            rows: car.basis_2a0.map(vec3),
        },
        canonical_axis_552f28: canonical.axis_552f28,
        canonical_axis_552f34: canonical.axis_552f34,
        fallback_axis_258: vec3(car.axis_258),
        fallback_axis_270: vec3(car.projected_axis_270),
    })?;

    Ok(Some(MovingObjectNarrowHit {
        normal: raw(produced.normal),
        object_contact_552f50,
        primary_limit_552f6c: produced.primary_limit.raw(),
        projected_limit_558fc4: produced.projected_limit.raw(),
    }))
}

/// `FUN_0043faf0`: two-body inelastic impulse/torque response, damage/effect
/// dispatch, and contact-history update.
pub fn apply_moving_object_response<D: StaticDamageOwner>(
    car: &mut MovingObjectCarState,
    object: &mut MovingObject,
    object_index: usize,
    hit: MovingObjectNarrowHit,
    damage_owner: &mut D,
) -> Result<MovingObjectResolvedContact, D::Error> {
    let mut events = Vec::new();
    let effects_enabled = if car.car_type_738 == 1 {
        events.push(StaticResponseEvent::QueryTypeOneEffectGate);
        !damage_owner.type_one_effect_gate_blocked()?
    } else {
        true
    };

    for axis in 0..3 {
        car.contact_3f0[axis] = clamp_axis(car.contact_3f0[axis], car.contact_limits_2c[axis]);
    }
    let mut objc = hit.object_contact_552f50;
    objc[0] = clamp_axis(objc[0], object.half[0]);
    objc[2] = clamp_axis(objc[2], object.half[2]);

    let n = hit.normal;
    let exceptional_candidate = if car.out_of_control_680 {
        None
    } else if abs_i32(dot(n, car.velocity_d4)) > hit.primary_limit_552f6c {
        Some(ExceptionalResponseKind::PrimaryNormalSpeed)
    } else {
        let projected = scale(
            car.projected_axis_270,
            dot(car.projected_axis_270, car.velocity_d4),
        );
        if abs_i32(dot(n, projected)) > hit.projected_limit_558fc4 {
            Some(ExceptionalResponseKind::ProjectedAxisSpeed)
        } else {
            None
        }
    };

    let vc = dot(n, car.velocity_d4);
    let vo = dot(n, object.velocity);
    let e = q(vc.wrapping_sub(vo), 0);
    let mo = MASS[object.mass_index()];
    let mc = car.mass_48;
    let num = q(vo, mo).wrapping_sub(q(e, mo)).wrapping_add(q(vc, mc));
    let den = mo.wrapping_add(mc);
    let vs = if den == 0 { -1 } else { qdiv(num, den) };

    let delta_c_scalar = vs.wrapping_sub(vc);
    let mut dc = scale(n, delta_c_scalar);
    let mut do_imp = scale(n, vs.wrapping_sub(vo).wrapping_add(e));

    if object.mass_index() == 0 {
        dc = [0; 3];
    }
    if car.flag_743 != 1 && dc[1] < 0 {
        do_imp[1] = do_imp[1].wrapping_sub(dc[1]);
        dc[1] = 0;
    }
    if !object.airborne() && do_imp[1] < 0 {
        dc[1] = dc[1].wrapping_sub(do_imp[1]);
        do_imp[1] = 0;
    }

    let old_car_tq = car.angular_correction_3d8;
    let _ = old_car_tq;
    let mc_body = rows_dot(&car.basis_2a0, dc);
    car.linear_correction_3cc = add(car.linear_correction_3cc, dc);

    let contact_id = i32::from(object.id_material >> 5);
    let repeated = car.object_history.contained_last_tick(contact_id);
    let mut damage_severity = None;
    let mut damage_sequence_invoked = false;
    let mut damage_result = DamageResult::default();
    if !repeated {
        let linear_vec = vec3(car.linear_correction_3cc);
        events.push(StaticResponseEvent::RequestDamage {
            linear_correction: linear_vec,
        });
        let evaluation = evaluate_static_damage(linear_vec);
        damage_severity = Some(evaluation.severity);
        damage_sequence_invoked = evaluation.damage_sequence_invoked;
        if evaluation.damage_sequence_invoked {
            damage_result = damage_owner.commit_contact(StaticDamageRequest {
                car_type_738: car.car_type_738,
                car_subtype_739: car.car_subtype_739,
                contact_3f0: vec3(car.contact_3f0),
                linear_correction_3cc: linear_vec,
                severity: evaluation.severity,
            })?;
        }
    }

    let basis = ResponseBasis {
        rows: car.basis_2a0.map(vec3),
    };
    let world_contact = basis
        .body_to_world(vec3(car.contact_3f0))
        .add(vec3(car.position_c8));
    let material = object.material();
    let debug_material_write = damage_owner.capture_debug_material().then_some(material);

    if effects_enabled && !repeated {
        let evaluation = evaluate_static_damage(vec3(car.linear_correction_3cc));
        if static_damage_emits_contact_effect(evaluation, damage_result.effect_handle) {
            events.push(StaticResponseEvent::ContactEffect {
                handle: damage_result.effect_handle,
                world_contact,
            });
        }
        let severity = evaluation.severity;
        if severity.raw() > 0 {
            events.push(StaticResponseEvent::MaterialImpact {
                severity,
                world_contact,
                material,
            });
            events.push(StaticResponseEvent::ScalarFeedback {
                car_type: car.car_type_738,
                strength: severity.mul(Fixed16::from_raw(0x1_8000)),
                threshold: Fixed16::from_raw(0x4ccc),
            });
            events.push(StaticResponseEvent::DirectionalFeedback {
                car_type: car.car_type_738,
                direction: vec3(dc),
                strength: severity.mul(Fixed16::from_raw(TWO)),
                limit: Fixed16::from_raw(TWO),
                trailing_zero: 0,
            });
        }
    }

    // Native `0x4402cf..0x440315`: only sets `+0x680/+0x688` when `|vs - vc|`
    // also exceeds the active threshold (so light cones never flip the car).
    if let Some(kind) = exceptional_candidate {
        let limit = match kind {
            ExceptionalResponseKind::PrimaryNormalSpeed => hit.primary_limit_552f6c,
            ExceptionalResponseKind::ProjectedAxisSpeed => hit.projected_limit_558fc4,
        };
        if abs_i32(delta_c_scalar) > limit {
            car.out_of_control_680 = true;
            car.out_of_control_688 = true;
        }
    }

    if car.out_of_control_680 {
        car.linear_correction_3cc = sub(car.linear_correction_3cc, dc);
        let half_dc = scale(dc, HALF);
        car.linear_correction_3cc = add(car.linear_correction_3cc, half_dc);
    }

    car.contact_3f0[1] = 0;
    if !car.out_of_control_680 {
        car.angular_correction_3d8 =
            add(car.angular_correction_3d8, cross(mc_body, car.contact_3f0));
    } else if let Some(kind) = exceptional_candidate {
        let tq = match kind {
            ExceptionalResponseKind::PrimaryNormalSpeed => {
                let kick = rows_dot(&car.basis_2a0, [0, ROLLOVER_KICK_Y, 0]);
                let raw_tq = cross(mc_body, add(car.contact_3f0, kick));
                car.velocity_d4[1] = car.velocity_d4[1].wrapping_add(HALF);
                [
                    q(raw_tq[0], ROLLOVER_X_SCALE),
                    raw_tq[1],
                    q(raw_tq[2], ROLLOVER_Z_SCALE),
                ]
            }
            ExceptionalResponseKind::ProjectedAxisSpeed => {
                let raw_tq = cross(mc_body, car.contact_3f0);
                [raw_tq[0], q(raw_tq[1], TWO), raw_tq[2]]
            }
        };
        car.angular_correction_3d8 = add(car.angular_correction_3d8, tq);
    }
    car.angular_correction_3d8[0] = clamp_range(car.angular_correction_3d8[0], -0x8_0000, 0x8_0000);
    car.angular_correction_3d8[1] = clamp_range(car.angular_correction_3d8[1], -0x4_0000, 0x4_0000);
    car.angular_correction_3d8[2] = clamp_range(car.angular_correction_3d8[2], -0x8_0000, 0x8_0000);

    // Object impulse, torque, and grounded hop (`0x4406df..0x4409ba`).
    let old_obj_tq = object.torque;
    let mo_body = rows_dot(&object.rows, do_imp);
    object.impulse = add(object.impulse, do_imp);
    object.torque = add(object.torque, cross(mo_body, objc));
    if dot(old_obj_tq, object.angular) < 0 {
        object.torque = old_obj_tq;
    }
    object.torque[0] = clamp_range(object.torque[0], -0x8_0000, 0x8_0000);
    object.torque[1] = clamp_range(object.torque[1], -0x4_0000, 0x4_0000);
    object.torque[2] = clamp_range(object.torque[2], -0x8_0000, 0x8_0000);

    if !object.airborne() {
        let rel_speed = length(sub(car.velocity_d4, object.velocity));
        if rel_speed > HALF {
            let scaled = q(rel_speed, 100 << 16);
            let hop = if mo == 0 { -1 } else { qdiv(scaled, mo) };
            object.impulse[1] = object.impulse[1].wrapping_add(hop);
        }
    }

    car.object_history.push(contact_id);

    Ok(MovingObjectResolvedContact {
        object_index,
        node: object.node,
        contact_id,
        material,
        normal: vec3(n),
        world_contact,
        repeated_from_previous_tick: repeated,
        damage_severity,
        damage_sequence_invoked,
        events,
        debug_material_write,
    })
}

/// Errors returned while running `resolve_moving_object_collisions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MovingObjectCollisionError<E> {
    Producer(StaticResponseInputProducerError),
    Damage(E),
}

/// `FUN_0043f790`: per-car moving-object collision pass across the four active
/// cells (`car+0x750..+0x756`).
pub fn resolve_moving_object_collisions<D: StaticDamageOwner>(
    objects: &mut MovingObjects,
    car: &mut MovingObjectCarState,
    hit_cache: &mut MovingObjectHitCache,
    double_rate_551af0: bool,
    damage_owner: &mut D,
) -> Result<MovingObjectCollisionEffects, MovingObjectCollisionError<D::Error>> {
    hit_cache.begin_pass();
    car.object_history = car.object_history.begin_tick();

    let mut car_proj: Option<LiveCarProjectionCache> = None;
    let mut contacts = Vec::new();
    let mut debug_material_558fb0 = None;

    for cell in car.active_nodes_750.into_iter().flatten() {
        let mut cursor = objects.in_cell(cell.index() as i32).next();
        while let Some(obj_idx) = cursor {
            let next_cursor = objects
                .in_cell(cell.index() as i32)
                .skip_while(|&i| i != obj_idx)
                .nth(1);
            cursor = next_cursor;

            if objects.objects[obj_idx].state() == state::INERT {
                continue;
            }
            if !sphere_overlap(
                car.position_c8,
                car.radius_44,
                objects.objects[obj_idx].position,
                objects.objects[obj_idx].radius,
            ) {
                continue;
            }

            let proj = car_proj.get_or_insert_with(|| {
                build_projection_raw(
                    car.hull_points_fc,
                    car.position_c8,
                    car.axis_258,
                    car.axis_264,
                )
            });

            let object = &mut objects.objects[obj_idx];
            let Some(narrow_hit) = resolve_moving_object_narrow_phase(car, proj, object)
                .map_err(MovingObjectCollisionError::Producer)?
            else {
                continue;
            };

            let resolved =
                apply_moving_object_response(car, object, obj_idx, narrow_hit, damage_owner)
                    .map_err(MovingObjectCollisionError::Damage)?;

            object.mark_hit();
            let node = object.node;
            hit_cache.record_hit(node, double_rate_551af0);
            if let Some(mat) = resolved.debug_material_write {
                debug_material_558fb0 = Some(mat);
            }
            contacts.push(resolved);
        }
    }

    Ok(MovingObjectCollisionEffects {
        contacts,
        object_contact_552f68: hit_cache.object_contact_552f68,
        debug_material_558fb0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        c3d_terrain_cull::CellGrid,
        moving_objects::{CONE_MATERIAL, FlatGround},
    };
    use std::convert::Infallible;

    struct NoopDamageOwner {
        capture_material: bool,
        commits: usize,
    }

    impl StaticDamageOwner for NoopDamageOwner {
        type Error = Infallible;

        fn capture_debug_material(&self) -> bool {
            self.capture_material
        }

        fn commit_contact(
            &mut self,
            _request: StaticDamageRequest,
        ) -> Result<DamageResult, Self::Error> {
            self.commits += 1;
            Ok(DamageResult { effect_handle: 0 })
        }
    }

    fn one() -> i32 {
        0x1_0000
    }

    fn test_grid() -> CellGrid {
        CellGrid {
            width: 4,
            height: 4,
            cell_count: 16,
            cell_size: 16 << 16,
            min_x: 0,
            half_cell: 8 << 16,
            max_z: 64 << 16,
        }
    }

    fn test_object(
        node: usize,
        id: u16,
        material: u8,
        rest_axes: u8,
        mass_index: u8,
        pos: [i32; 3],
    ) -> MovingObject {
        MovingObject {
            node,
            position: pos,
            half: [one() / 2, one() / 2, one() / 2],
            velocity: [0; 3],
            angular: [0; 3],
            torque: [0; 3],
            impulse: [0; 3],
            inverse_inertia: [one(); 3],
            rows: [[one(), 0, 0], [0, one(), 0], [0, 0, one()]],
            base: [pos[0], pos[1] - one() / 2, pos[2]],
            radius: one(),
            ground_height: 0,
            id_material: (id << 5) | u16::from(material),
            ground_handle: 0,
            cell: 0,
            flags: mass_index,
            rest_axes,
            row_scale: [one(); 3],
            ground_normal: [0, one(), 0],
            next: None,
            prev: None,
        }
    }

    fn sample_car(
        pos: [i32; 3],
        prev_xz: [i32; 2],
        vel: [i32; 3],
        cell: i32,
    ) -> MovingObjectCarState {
        let basis = [[one(), 0, 0], [0, one(), 0], [0, 0, one()]];
        let half = [0x1_0000, 0x8000, 0x2_0000];
        let hull = corners(pos, &basis, half);
        MovingObjectCarState {
            position_c8: pos,
            previous_xz_3e4: prev_xz,
            velocity_d4: vel,
            velocity_normalizer_ec: one(),
            hull_points_fc: hull,
            radius_44: 0x3_0000,
            mass_48: 1000 << 16,
            contact_limits_2c: half,
            axis_258: basis[0],
            axis_264: basis[1],
            projected_axis_270: basis[2],
            basis_2a0: basis,
            linear_correction_3cc: [0; 3],
            angular_correction_3d8: [0; 3],
            contact_3f0: [0; 3],
            contact_count_674: 0,
            out_of_control_680: false,
            out_of_control_688: false,
            flag_743: 0,
            car_type_738: 0,
            car_subtype_739: 0,
            active_nodes_750: [
                Some(StaticCollisionNodeId::new(cell).unwrap()),
                None,
                None,
                None,
            ],
            object_history: StaticResponseHistory::default(),
        }
    }

    #[test]
    fn hit_cache_deduplicates_three_nodes_and_expires_after_rate_limit() {
        let mut cache = MovingObjectHitCache::default();
        cache.begin_pass();
        cache.record_hit(7, false);
        assert!(cache.object_contact_552f68);
        assert_eq!(cache.recent_nodes_55901c, [Some(7), None, None]);

        // Same node within 50 ticks does not re-trigger object_contact_552f68.
        cache.begin_pass();
        cache.record_hit(7, false);
        assert!(!cache.object_contact_552f68);

        // After > 50 ticks at single rate, the cache expires and re-triggers.
        for _ in 0..50 {
            cache.begin_pass();
        }
        cache.record_hit(7, false);
        assert!(cache.object_contact_552f68);
        assert_eq!(cache.tick_counter_552f64, 0);
    }

    #[test]
    fn hitting_light_cone_pushes_cone_without_slowing_or_flipping_car() {
        let grid = test_grid();
        let cone = test_object(0, 0, CONE_MATERIAL, 0b010, 0, [4 << 16, one() / 2, 4 << 16]);
        let sign = test_object(1, 1, 1, 0b010, 1, [20 << 16, one() / 2, 4 << 16]);
        let mut objects = MovingObjects::from_objects_for_test(vec![cone, sign], &grid);
        assert!(objects.objects[0].is_cone());

        let cone_pos = objects.objects[0].position;
        let cell = objects.objects[0].cell;
        let car_pos = [cone_pos[0], cone_pos[1], cone_pos[2] - 0x1_0000];
        let mut car = sample_car(
            car_pos,
            [car_pos[0], car_pos[2] - 0x1_0000],
            [0, 0, 0x3_0000],
            cell,
        );
        let mut cache = MovingObjectHitCache::default();
        let mut owner = NoopDamageOwner {
            capture_material: true,
            commits: 0,
        };

        let effects =
            resolve_moving_object_collisions(&mut objects, &mut car, &mut cache, false, &mut owner)
                .unwrap();

        assert_eq!(effects.contacts.len(), 1);
        assert!(effects.object_contact_552f68);
        assert_eq!(effects.debug_material_558fb0, Some(CONE_MATERIAL));
        // Cone is mass_index 0: car linear_correction_3cc stays zero and car is not out of control!
        assert_eq!(car.linear_correction_3cc, [0, 0, 0]);
        assert!(!car.out_of_control_680);
        // Cone received full depenetration, forward impulse, upward hop, and HIT state.
        assert_eq!(objects.objects[0].state(), state::HIT);
        assert!(objects.objects[0].position[2] > cone_pos[2]);
        assert!(objects.objects[0].impulse[2] > 0);
        assert!(objects.objects[0].impulse[1] > 0);
        // Car position was not depenetrated by a mass_index 0 cone (SHARE[0] == 1.0).
        assert_eq!(car.position_c8, car_pos);

        // Next tick update moves the cone into MOVING state.
        let trig = crate::fixed_point::build_sine_table().to_vec();
        objects.update(&mut FlatGround(0), 0x1_0000, &trig);
        assert_eq!(objects.objects[0].state(), state::MOVING);
    }

    #[test]
    fn hitting_medium_sign_shares_depenetration_and_pushes_both_bodies() {
        let grid = test_grid();
        let cone = test_object(0, 0, CONE_MATERIAL, 0b010, 0, [4 << 16, one() / 2, 4 << 16]);
        let sign = test_object(1, 1, 1, 0b010, 1, [20 << 16, one() / 2, 4 << 16]);
        let mut objects = MovingObjects::from_objects_for_test(vec![cone, sign], &grid);
        let sign_pos = objects.objects[1].position;
        let cell = objects.objects[1].cell;
        let car_pos = [sign_pos[0], sign_pos[1], sign_pos[2] - 0x1_0000];
        let mut car = sample_car(
            car_pos,
            [car_pos[0], car_pos[2] - 0x1_0000],
            [0, 0, 0x1_0000],
            cell,
        );
        let mut cache = MovingObjectHitCache::default();
        let mut owner = NoopDamageOwner {
            capture_material: false,
            commits: 0,
        };

        let effects =
            resolve_moving_object_collisions(&mut objects, &mut car, &mut cache, false, &mut owner)
                .unwrap();

        assert_eq!(effects.contacts.len(), 1);
        assert_eq!(effects.contacts[0].object_index, 1);
        // SHARE[1] == 0.4: both sign (+z) and car (-z) are depenetrated!
        assert!(objects.objects[1].position[2] > sign_pos[2]);
        assert!(car.position_c8[2] < car_pos[2]);
        // Car receives a backward (-z) linear correction and sign receives a forward (+z) impulse.
        assert!(car.linear_correction_3cc[2] < 0);
        assert!(objects.objects[1].impulse[2] > 0);
    }
}
