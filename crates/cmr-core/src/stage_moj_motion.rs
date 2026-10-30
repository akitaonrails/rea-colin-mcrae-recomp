//! Runtime binding, per-tick motion stepping, and render interpolation for
//! stage `.moj` scripted moving/animated objects (`FUN_0045bde0`,
//! `FUN_0045be90`, `FUN_0045c160`, `FUN_0045c1b0`, `FUN_0045c3b0`,
//! `FUN_0045c4b0`, `FUN_0045c5b0`, `FUN_0045c690`, `FUN_0045c770`,
//! `FUN_0045c890`, and `FUN_0045c9c0`).

use cmr_formats::stage::MojRecord;

use crate::c3d_camera::matmul_q16;
use crate::camera_crossfade::blend_pose;
use crate::legacy_math::{cos_q16, sin_q16};
use crate::moving_objects::{add, q, qdiv, scale, sub};

/// Fixed-point `1.0` (`0x1_0000`).
pub const ONE_Q16: i32 = 0x1_0000;

/// Fixed-point `2.0` (`0x2_0000`).
pub const TWO_Q16: i32 = 0x2_0000;

/// Fixed-point `0.5` (`0x8000`).
pub const HALF_Q16: i32 = 0x8000;

/// Identity 4x4 Q16.16 matrix (`FUN_0048ed80`).
pub const IDENTITY_4X4_Q16: [i32; 16] = [
    ONE_Q16, 0, 0, 0, //
    0, ONE_Q16, 0, 0, //
    0, 0, ONE_Q16, 0, //
    0, 0, 0, ONE_Q16,
];

/// One bound runtime `.moj` instance slot (`DAT_00573a88 + i * 0xf8` paired with
/// `DAT_00579b68 + i * 0x30`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MojRuntimeSlot {
    /// Active `.moj` record (`DAT_00579b68 + i * 0x30`).
    pub record: MojRecord,
    /// Bound scene-node index (`+0x04` pointer in `DAT_00573a8c`).
    pub scene_node_index: usize,
    /// `+0x00`: mode-3 one-shot started flag.
    pub mode3_started_00: u32,
    /// `+0x08..+0x48`: initial/base 4x4 transform copied from `[node + 0x94]`.
    pub base_tf_08: [i32; 16],
    /// `+0x48..+0x88`: current tick 4x4 transform.
    pub curr_tf_48: [i32; 16],
    /// `+0x88..+0xc8`: previous tick 4x4 transform.
    pub prev_tf_88: [i32; 16],
    /// `+0xc8..+0xd4`: orbit center / anchor position `(X, Y, Z)` in Q16.16.
    pub anchor_pos_c8: [i32; 3],
    /// `+0xd4..+0xe4`: tick counters `[translation, rot_a, rot_b, rot_c]`.
    pub tick_counters_d4: [i32; 4],
    /// `+0xe4..+0xf4`: period in ticks `[translation, rot_a, rot_b, rot_c]`.
    pub period_ticks_e4: [i32; 4],
}

/// Bind a `.moj` record to a matching scene node (`FUN_0045be90` at
/// `0x45be90..0x45c145`).
///
/// Matches `node_tf` translation `(X, Z)` (`node_tf[12]`, `node_tf[14]`) against
/// `record.position_or_target_q16[0]` and `record.position_or_target_q16[2]`.
#[must_use]
pub fn bind_moj_scene_node_45be90(
    record: MojRecord,
    scene_node_index: usize,
    node_tf: &[i32; 16],
    double_rate_551af0: bool,
) -> Option<MojRuntimeSlot> {
    if !record.is_active() {
        return None;
    }
    let node_pos = [node_tf[12], node_tf[13], node_tf[14]];
    if node_pos[0] != record.position_or_target_q16[0]
        || node_pos[2] != record.position_or_target_q16[2]
    {
        return None;
    }

    let rate = if double_rate_551af0 { 50 } else { 25 };
    let period_ticks_e4 = [
        record.translation_period_seconds.wrapping_mul(rate),
        record.rotation_period_seconds[0].wrapping_mul(rate),
        record.rotation_period_seconds[1].wrapping_mul(rate),
        record.rotation_period_seconds[2].wrapping_mul(rate),
    ];

    let angle = record.initial_angle_bam & 0xfff;
    let dir = [sin_q16(angle).raw(), 0, cos_q16(angle).raw()];
    let dist = match record.motion_mode {
        1 | 2 => qdiv(record.orbit_diameter_q16, TWO_Q16),
        3 | 4 => record.orbit_diameter_q16,
        _ => 0,
    };
    let anchor_pos_c8 = add(node_pos, scale(dir, dist));

    Some(MojRuntimeSlot {
        record,
        scene_node_index,
        mode3_started_00: 0,
        base_tf_08: *node_tf,
        curr_tf_48: *node_tf,
        prev_tf_88: *node_tf,
        anchor_pos_c8,
        tick_counters_d4: [0; 4],
        period_ticks_e4,
    })
}

/// Bind all active `.moj` records against a stage's scene-node local transforms
/// (`FUN_0045be90` + `FUN_0045c160`).
///
/// `FUN_0045c160` resets `DAT_0057ae28 = 0` and returns `0` if any active `.moj`
/// record failed to bind to a scene node.
#[must_use]
pub fn bind_all_moj_records_45c160(
    active_records: &[MojRecord],
    scene_node_locals: &[[i32; 16]],
    double_rate_551af0: bool,
) -> Vec<MojRuntimeSlot> {
    let mut bound = Vec::with_capacity(active_records.len());
    for &record in active_records {
        if !record.is_active() {
            continue;
        }
        let mut matched = None;
        for (node_idx, node_tf) in scene_node_locals.iter().enumerate() {
            if let Some(slot) =
                bind_moj_scene_node_45be90(record, node_idx, node_tf, double_rate_551af0)
            {
                matched = Some(slot);
                break;
            }
        }
        let Some(slot) = matched else {
            return Vec::new();
        };
        bound.push(slot);
    }
    bound
}

/// Post-multiply `curr_tf` by an X-axis rotation (`FUN_0045c770` at
/// `0x45c770..0x45c88e`): `curr_tf = rot_x * curr_tf`.
pub fn apply_moj_rot_x_45c770(curr_tf: &mut [i32; 16], phase_q16: i32) {
    let angle = (((i64::from(phase_q16) * 360) / 5760) as u16) & 0xfff;
    let s = sin_q16(angle).raw();
    let c = cos_q16(angle).raw();
    let mut rot = IDENTITY_4X4_Q16;
    rot[4] = 0;
    rot[5] = c;
    rot[6] = s;
    rot[8] = 0;
    rot[9] = s.wrapping_neg();
    rot[10] = c;
    *curr_tf = matmul_q16(&rot, curr_tf);
}

/// Post-multiply `curr_tf` by a Y-axis rotation (`FUN_0045c890` at
/// `0x45c890..0x45c9b0`): `curr_tf = rot_y * curr_tf`.
pub fn apply_moj_rot_y_45c890(curr_tf: &mut [i32; 16], phase_q16: i32) {
    let angle = (((i64::from(phase_q16) * 360) / 5760) as u16) & 0xfff;
    let neg_angle = angle.wrapping_neg() & 0xfff;
    let s_neg = sin_q16(neg_angle).raw();
    let c = cos_q16(angle).raw();
    let mut rot = IDENTITY_4X4_Q16;
    rot[0] = c;
    rot[1] = 0;
    rot[2] = s_neg;
    rot[8] = s_neg.wrapping_neg();
    rot[9] = 0;
    rot[10] = c;
    *curr_tf = matmul_q16(&rot, curr_tf);
}

fn phase_from_counter(counter: i32, period_ticks: i32) -> i32 {
    if period_ticks == 0 {
        return 0;
    }
    ((i64::from(counter) * i64::from(ONE_Q16)) / i64::from(period_ticks)) as i32
}

/// Step one bound `.moj` slot by one fixed physics tick (`FUN_0045c1b0` at
/// `0x45c1b0..0x45c39b`).
pub fn step_moj_slot_45c1b0(slot: &mut MojRuntimeSlot) {
    slot.prev_tf_88 = slot.curr_tf_48;
    slot.curr_tf_48 = slot.base_tf_08;
    slot.curr_tf_48[12] = 0;
    slot.curr_tf_48[13] = 0;
    slot.curr_tf_48[14] = 0;

    // Rotation channel A (+0xd8 / +0xe8) -> FUN_0045c770.
    if slot.period_ticks_e4[1] != 0 {
        let phase = phase_from_counter(slot.tick_counters_d4[1], slot.period_ticks_e4[1]);
        apply_moj_rot_x_45c770(&mut slot.curr_tf_48, phase);
        slot.tick_counters_d4[1] = slot.tick_counters_d4[1].wrapping_add(1);
        if slot.tick_counters_d4[1] == slot.period_ticks_e4[1].wrapping_abs() {
            slot.tick_counters_d4[1] = 0;
        }
    }

    // Rotation channel B (+0xdc / +0xec) -> FUN_0045c890.
    if slot.period_ticks_e4[2] != 0 {
        let phase = phase_from_counter(slot.tick_counters_d4[2], slot.period_ticks_e4[2]);
        apply_moj_rot_y_45c890(&mut slot.curr_tf_48, phase);
        slot.tick_counters_d4[2] = slot.tick_counters_d4[2].wrapping_add(1);
        if slot.tick_counters_d4[2] == slot.period_ticks_e4[2].wrapping_abs() {
            slot.tick_counters_d4[2] = 0;
        }
    }

    // Rotation channel C (+0xe0 / +0xf0) -> FUN_0045c770.
    if slot.period_ticks_e4[3] != 0 {
        let phase = phase_from_counter(slot.tick_counters_d4[3], slot.period_ticks_e4[3]);
        apply_moj_rot_x_45c770(&mut slot.curr_tf_48, phase);
        slot.tick_counters_d4[3] = slot.tick_counters_d4[3].wrapping_add(1);
        if slot.tick_counters_d4[3] == slot.period_ticks_e4[3].wrapping_abs() {
            slot.tick_counters_d4[3] = 0;
        }
    }

    // Translation / orbit step (+0xd4 / +0xe4).
    let mut phase_pos = phase_from_counter(slot.tick_counters_d4[0], slot.period_ticks_e4[0]);
    let pos = match slot.record.motion_mode {
        1 => {
            // FUN_0045c3b0: XZ circle (delta = -(phase * 360 / 5760)).
            let delta = -(((i64::from(phase_pos) * 360) / 5760) as i16);
            let angle = slot.record.initial_angle_bam.wrapping_sub(delta as u16) & 0xfff;
            let radius = qdiv(slot.record.orbit_diameter_q16, TWO_Q16);
            let offset = scale([sin_q16(angle).raw(), 0, cos_q16(angle).raw()], radius);
            sub(slot.anchor_pos_c8, offset)
        }
        2 => {
            // FUN_0045c4b0: opposite-direction XZ circle (delta = +(phase * 360 / 5760)).
            let delta = ((i64::from(phase_pos) * 360) / 5760) as i16;
            let angle = slot.record.initial_angle_bam.wrapping_sub(delta as u16) & 0xfff;
            let radius = qdiv(slot.record.orbit_diameter_q16, TWO_Q16);
            let offset = scale([sin_q16(angle).raw(), 0, cos_q16(angle).raw()], radius);
            sub(slot.anchor_pos_c8, offset)
        }
        3 => {
            // FUN_0045c5b0: one-shot linear from target toward anchor.
            if phase_pos == 0 && slot.mode3_started_00 != 0 {
                phase_pos = ONE_Q16;
            }
            let target = slot.record.position_or_target_q16;
            let pos = add(target, scale(sub(slot.anchor_pos_c8, target), phase_pos));
            if phase_pos == ONE_Q16 {
                slot.record.motion_mode = 0;
            }
            slot.mode3_started_00 = 1;
            pos
        }
        4 => {
            // FUN_0045c690: ping-pong linear between target and anchor.
            let t_half = if phase_pos < HALF_Q16 {
                phase_pos
            } else {
                ONE_Q16.wrapping_sub(phase_pos)
            };
            let t = q(t_half, TWO_Q16);
            let target = slot.record.position_or_target_q16;
            add(target, scale(sub(slot.anchor_pos_c8, target), t))
        }
        _ => [
            slot.base_tf_08[12],
            slot.base_tf_08[13],
            slot.base_tf_08[14],
        ],
    };
    slot.curr_tf_48[12] = pos[0];
    slot.curr_tf_48[13] = pos[1];
    slot.curr_tf_48[14] = pos[2];

    if slot.period_ticks_e4[0] != 0 {
        slot.tick_counters_d4[0] = slot.tick_counters_d4[0].wrapping_add(1);
        if slot.tick_counters_d4[0] == slot.period_ticks_e4[0] {
            slot.tick_counters_d4[0] = 0;
        }
    }
}

/// Interpolate a bound `.moj` slot's transform for rendering (`FUN_0045c9c0` at
/// `0x45c9c0..0x45ca8d`).
///
/// Calls `FUN_00435400` ([`blend_pose`]) between `prev_tf_88` and `curr_tf_48`
/// at `alpha_q16`, and when `record.terrain_snap_flag != 0` snaps Y (`+0x34`) to
/// `ground_y_q16` if provided.
#[must_use]
pub fn interpolate_moj_slot_45c9c0(
    slot: &MojRuntimeSlot,
    alpha_q16: i32,
    ground_y_q16: Option<i32>,
) -> [i32; 16] {
    let mut tf = blend_pose(&slot.prev_tf_88, &slot.curr_tf_48, alpha_q16);
    if slot.record.terrain_snap_flag != 0
        && let Some(ground_y) = ground_y_q16
    {
        tf[13] = ground_y;
    }
    tf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(mode: u32) -> MojRecord {
        MojRecord {
            terrain_snap_flag: 1,
            initial_angle_bam: 0,
            angle_padding: 0,
            orbit_diameter_q16: 0x10_0000, // 16.0 diameter -> 8.0 radius
            reserved_0c: 0,
            position_or_target_q16: [0x20_0000, 0x2_0000, 0x30_0000],
            translation_period_seconds: 1,
            rotation_period_seconds: [1, 0, 0],
            motion_mode: mode,
        }
    }

    fn node_with_pos(pos: [i32; 3]) -> [i32; 16] {
        let mut tf = IDENTITY_4X4_Q16;
        tf[12] = pos[0];
        tf[13] = pos[1];
        tf[14] = pos[2];
        tf
    }

    #[test]
    fn bind_matches_xz_and_computes_circular_and_linear_anchors() {
        let rec_circle = sample_record(1);
        let node_tf = node_with_pos([0x20_0000, 0x5_0000, 0x30_0000]);
        let slot = bind_moj_scene_node_45be90(rec_circle, 3, &node_tf, false).unwrap();
        assert_eq!(slot.scene_node_index, 3);
        assert_eq!(slot.period_ticks_e4, [25, 25, 0, 0]);
        // angle = 0 -> sin = 0, cos = 1.0; radius = 16.0 / 2 = 8.0 -> Z += 8.0.
        assert_eq!(slot.anchor_pos_c8, [0x20_0000, 0x5_0000, 0x38_0000]);

        let rec_linear = sample_record(4);
        let slot_lin = bind_moj_scene_node_45be90(rec_linear, 1, &node_tf, true).unwrap();
        assert_eq!(slot_lin.period_ticks_e4, [50, 50, 0, 0]);
        // Linear modes use full distance (16.0) without halving.
        assert_eq!(slot_lin.anchor_pos_c8, [0x20_0000, 0x5_0000, 0x40_0000]);
    }

    #[test]
    fn bind_all_clears_table_when_any_active_record_is_unbound() {
        let rec = sample_record(1);
        let matching = [node_with_pos([0x20_0000, 0, 0x30_0000])];
        assert_eq!(
            bind_all_moj_records_45c160(&[rec], &matching, false).len(),
            1
        );

        let non_matching = [node_with_pos([0, 0, 0])];
        assert!(bind_all_moj_records_45c160(&[rec], &non_matching, false).is_empty());
    }

    #[test]
    fn step_and_interpolate_orbit_and_ping_pong_modes() {
        let rec = sample_record(4);
        let node_tf = node_with_pos([0x20_0000, 0x2_0000, 0x30_0000]);
        let mut slot = bind_moj_scene_node_45be90(rec, 0, &node_tf, false).unwrap();

        // Tick 0: phase = 0 -> at target Z = 0x30_0000.
        step_moj_slot_45c1b0(&mut slot);
        assert_eq!(slot.curr_tf_48[14], 0x30_0000);

        // Advance to half period (tick 12 of 25) -> moves toward anchor Z = 0x40_0000.
        for _ in 0..12 {
            step_moj_slot_45c1b0(&mut slot);
        }
        assert!(slot.curr_tf_48[14] > 0x3c_0000);

        // Render interpolation with terrain snap overrides Y (+0x34 = index 13).
        let rendered = interpolate_moj_slot_45c9c0(&slot, HALF_Q16, Some(0x7_8000));
        assert_eq!(rendered[13], 0x7_8000);
    }
}
