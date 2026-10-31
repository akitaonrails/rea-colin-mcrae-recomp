//! Per-car stage progress along the `.cat` node spline - the recovered
//! "CatsEye" split/checkpoint tracker `FUN_00462ac0`, called once per physics
//! tick at the tail of the Mechanics update `FUN_0043afd0`.
//!
//! This is the authoritative, **geometry-free** checkpoint mechanism of normal
//! play (the trigger-volume gates in [`cmr_formats::stage`] are a separate
//! pacenote/zone/surface layer). It walks a per-car cursor forward along the
//! parsed `.cat` nodes ([`cmr_formats::stage::CatNode`]); crossing a split-
//! flagged node samples the stage clock as that split's time, and reaching the
//! final gate freezes the finish time.
//!
//! # Recovered contract (evidence `rea.official-eu.stage-checkpoints`)
//!
//! All world positions, direction vectors and the stage length share **one
//! Q16.16 fixed-point world space** (1.0 = `0x10000`); there is no scale to
//! invent. The per-car block holds the car centre at `+0xC8/+0xCC/+0xD0`
//! (x,y,z); a `.cat` node is `0x1C` bytes = position `+0x00`, direction `+0x0C`,
//! per-car split-flag byte `+0x18`, sequence index `+0x1A`.
//!
//! **OBSERVATION.** The crossing test is a plane test against the current node,
//! taken in the **horizontal x-z plane only (Y forced to 0)**: `dot = (node.pos
//! - car.pos) . node.dir`, computed in Q16.16 (`FUN_00462ac0` dots the car->node
//! delta with the node direction, shifting the 64-bit products right by `0x10`).
//! `dot < 0` means the car is past the node, so the cursor advances; a backward
//! test against the previous node steps the cursor back (regression handling).
//! Advancing past a node whose split bit for this car is set
//! (`node+0x18 & (1 << car)`) clears that bit, increments the per-car split
//! count (`DAT_005ffb88`), and records the clock readout `FUN_0047fd20` (centiseconds)
//! as the split time. The finish freezes (`DAT_005739e0 = 1`, time latched into
//! `DAT_00573a80`) when the split count reaches `8` (the loader asserts "should
//! be 7" intermediate splits, so 7 splits + finish = 8 gates), or when the
//! cursor reaches the node count `DAT_0057c448`.
//!
//! **OBSERVATION.** `FUN_00461e90` normalizes the delta before the dot and
//! pre-scales components outside ±100.0 by signed division by 512;
//! `normalize_delta_pre_transform` implements that path. The bit-clear dedup
//! means a node already counted for a car never re-fires, even after a
//! regression re-crosses it.
//!
//! **BOUNDARY.** The spawn-position writer is not identified by current static
//! evidence. One node advances per [`StageProgress::update`] call to match the
//! shipped once-per-tick cadence.

use crate::fixed_point::{Fixed16, Vec3};
use cmr_formats::stage::CatNode;

/// Number of gates that complete a stage: 7 intermediate splits + the finish
/// (the recovered finish test `DAT_005ffb88 == 8`; the loader's "should be 7"
/// diagnostic counts the intermediate splits only).
pub const FINISH_GATE_COUNT: u32 = 8;

/// What happened to the cursor on one [`StageProgress::update`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageEvent {
    /// The car neither crossed the current node nor regressed this tick.
    Idle,
    /// The cursor stepped back to an earlier node (regression handling).
    Regressed {
        /// The node index the cursor stepped back to.
        to: usize,
    },
    /// Crossed a non-split node; the cursor advanced past it.
    Advanced {
        /// The node index that was crossed.
        node: usize,
    },
    /// Crossed a split/checkpoint node; its split time was recorded.
    Split {
        /// The node index that was crossed.
        node: usize,
        /// The 1-based split number (the per-car split count after this split).
        split_number: u32,
        /// The stage clock readout (centiseconds) latched for this split.
        centiseconds: i32,
    },
    /// Crossed the final gate; the stage finish time is frozen.
    Finished {
        /// The node index whose crossing completed the stage.
        node: usize,
        /// The frozen finish time in centiseconds.
        centiseconds: i32,
    },
}

/// Duration (in 25 Hz ticks) that a crossed split time and delta stay visible on
/// the race HUD (`FUN_00462ac0` sets `0x5ffc2c = 0x78 = 120`, and `FUN_0047fe50`
/// fades alpha over the final 24 ticks).
pub const SPLIT_DISPLAY_TICKS: u8 = 120;

/// A per-car cursor walking the `.cat` node spline, reproducing `FUN_00462ac0`.
///
/// Construct one per car, then feed it the car's world position and the current
/// stage-clock centisecond readout once per physics tick via [`Self::update`].
#[derive(Clone, Debug)]
pub struct StageProgress<'a> {
    nodes: &'a [CatNode],
    car_bit: u8,
    cursor: usize,
    /// Per-node split bits still pending for this car (cleared as crossed),
    /// mirroring the shipped in-place clear of `node+0x18` without mutating the
    /// parsed nodes.
    pending_bits: Vec<u8>,
    splits_passed: u32,
    split_times: Vec<i32>,
    /// Cumulative 3D distance fractions (in Q16.16 `0..=0x1_0000`) for each
    /// intermediate split gate along the `.cat` spline (`FUN_00462230`,
    /// `0x57c2fc` / `0x57c3a8` / `0x57aff0`), used by `FUN_00463080` to draw
    /// progress-rail split tick marks and colored sector bars.
    split_progress_q16: Vec<i32>,
    /// Countdown timer (`0x5ffc2c`, `120..=0`) showing the most recent split
    /// time and top-center split delta (`FUN_0047fe50`).
    split_display_ticks: u8,
    /// Most recently latched split time in centiseconds (`0x5ffb30`).
    last_split_centiseconds: Option<i32>,
    finished: bool,
    finish_centiseconds: Option<i32>,
    /// `DAT_0057b008[car]`: regressions since the last forward crossing,
    /// held at 5.
    reverse_crossings: u8,
}

impl<'a> StageProgress<'a> {
    /// Start a fresh run for `car` (0-based; only 0/1 occur in the 2-player
    /// shipped build, but the split-flag byte addresses up to 8 cars) over the
    /// stage's parsed `.cat` nodes. The cursor begins at node 0, the start
    /// reference.
    #[must_use]
    pub fn new(nodes: &'a [CatNode], car: u8) -> Self {
        let car_bit = 1u8 << (car & 7);
        let mut pending_bits: Vec<u8> = nodes.iter().map(|n| n.split_flag).collect();
        // In `FUN_004620f0` (`0x462109..0x462119`), `0x57af9c[car]` is
        // initialized to `-1` (`0xffffffff`). Normal `.cat` stage files flag 9
        // nodes (`CAT[0]` start line + 7 intermediate splits + `CAT[last]`
        // finish line): crossing `CAT[0]` increments `0x57af9c` from `-1` to `0`
        // (start line) and does not count as Split 1 (`0x5ffb88` is only
        // updated for `1..=7`). Clearing the start-line split bit when all 9
        // gates are present keeps `splits_passed` aligned 1:1 with `1..=8`
        // (7 intermediate splits + finish).
        let split_count = pending_bits
            .iter()
            .filter(|&&bits| bits & car_bit != 0)
            .count();
        if split_count > FINISH_GATE_COUNT as usize
            && let Some(first) = pending_bits.first_mut()
        {
            *first &= !car_bit;
        }
        let split_progress_q16 = compute_split_progress_q16(nodes, &pending_bits, car_bit);
        StageProgress {
            nodes,
            car_bit,
            cursor: 0,
            pending_bits,
            splits_passed: 0,
            split_times: Vec::new(),
            split_progress_q16,
            split_display_ticks: 0,
            last_split_centiseconds: None,
            finished: false,
            finish_centiseconds: None,
            reverse_crossings: 0,
        }
    }

    /// `DAT_0057b008[car]`: regressions in a row, at most 5
    /// (`FUN_004671f0` tests for 5).
    #[must_use]
    pub fn reverse_crossings(&self) -> u8 {
        self.reverse_crossings
    }

    /// The current node cursor (`DAT_0057ae70[car]`).
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Start the run again from node 0 with every split pending, as a fresh
    /// stage load does.
    pub fn restart(&mut self) {
        let car = self.car_bit.trailing_zeros() as u8;
        *self = Self::new(self.nodes, car);
    }

    /// The parsed nodes this cursor walks.
    #[must_use]
    pub fn nodes(&self) -> &'a [CatNode] {
        self.nodes
    }

    /// Write the cursor directly, as the reset dispatcher `FUN_00467210`
    /// does. Splits already passed stay passed.
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    /// The number of splits this car has passed (`DAT_005ffb88[car]`).
    #[must_use]
    pub fn splits_passed(&self) -> u32 {
        self.splits_passed
    }

    /// The recorded split times, in crossing order (centiseconds).
    #[must_use]
    pub fn split_times(&self) -> &[i32] {
        &self.split_times
    }

    /// Normalized cumulative 3D distance fractions (`0..=0x1_0000`) for the
    /// intermediate split gates along the stage (`FUN_00462230` / `FUN_00463080`).
    #[must_use]
    pub fn split_progress_q16(&self) -> &[i32] {
        &self.split_progress_q16
    }

    /// Remaining HUD display ticks (`120..=0`, `0x5ffc2c`) for the most recent
    /// split readout (`FUN_0047fe50`).
    #[must_use]
    pub fn split_display_ticks(&self) -> u8 {
        self.split_display_ticks
    }

    /// Most recently latched split time in centiseconds (`0x5ffb30`).
    #[must_use]
    pub fn last_split_centiseconds(&self) -> Option<i32> {
        self.last_split_centiseconds
    }

    /// Current distance progress (`0..=0x1_0000`) along the `.cat` node spline
    /// (`FUN_00464130` / `FUN_00462ac0`).
    #[must_use]
    pub fn distance_progress_q16(&self) -> i32 {
        if self.finished || self.nodes.len() <= 1 {
            return 0x1_0000;
        }
        let max_idx = (self.nodes.len() - 1) as i64;
        let cur = (self.cursor.min(self.nodes.len() - 1)) as i64;
        ((cur << 16) / max_idx).clamp(0, 0x1_0000) as i32
    }

    /// Whether the stage finish has been crossed.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The frozen finish time in centiseconds, once finished.
    #[must_use]
    pub fn finish_centiseconds(&self) -> Option<i32> {
        self.finish_centiseconds
    }

    /// Advance the cursor for one physics tick given the car's world position
    /// and the current stage-clock readout in centiseconds.
    ///
    /// Reproduces one pass of `FUN_00462ac0`: a forward crossing test against
    /// the current node first (`0x462b79..0x462e6e`), else a regression step
    /// back when the car has fallen behind the previous node
    /// (`0x462e73..0x462fbf`), then the finish check when `cursor >= nodes.len()`
    /// (`0x462fc2..0x463013`). Once finished, further calls are
    /// [`StageEvent::Idle`].
    pub fn update(&mut self, car_pos: Vec3, clock_centiseconds: i32) -> StageEvent {
        self.split_display_ticks = self.split_display_ticks.saturating_sub(1);
        if self.finished || self.cursor >= self.nodes.len() {
            return StageEvent::Idle;
        }

        // Forward crossing (`0x462b79..0x462e6e`): tested first in `FUN_00462ac0`.
        if horizontal_dot(&self.nodes[self.cursor], car_pos).raw() < 0 {
            let node = self.cursor;
            let is_split = self.pending_bits[node] & self.car_bit != 0;
            if is_split {
                self.pending_bits[node] &= !self.car_bit;
                self.splits_passed += 1;
                self.split_times.push(clock_centiseconds);
                self.last_split_centiseconds = Some(clock_centiseconds);
                self.split_display_ticks = SPLIT_DISPLAY_TICKS;
            }
            self.cursor += 1;
            self.reverse_crossings = 0;

            // `0x462fc2..0x463013`: stage finish triggers when `cursor` reaches
            // `DAT_0057c448` (`nodes.len()`), or in synthetic tests where every
            // node is a split and 8 splits have been crossed with no start-gate
            // bit cleared.
            if self.cursor >= self.nodes.len() {
                self.finished = true;
                self.finish_centiseconds = Some(clock_centiseconds);
                return StageEvent::Finished {
                    node,
                    centiseconds: clock_centiseconds,
                };
            }
            if is_split {
                return StageEvent::Split {
                    node,
                    split_number: self.splits_passed,
                    centiseconds: clock_centiseconds,
                };
            }
            return StageEvent::Advanced { node };
        }

        // Regression (`0x462e73..0x462fbf`): tested in the `else` branch of
        // `FUN_00462ac0` when the car has moved back in front of `cursor - 1`.
        if self.cursor > 0 && horizontal_dot(&self.nodes[self.cursor - 1], car_pos).raw() > 0 {
            if self.reverse_crossings != 5 {
                self.reverse_crossings += 1;
            }
            self.cursor -= 1;
            return StageEvent::Regressed { to: self.cursor };
        }

        StageEvent::Idle
    }
}

/// Compute the normalized cumulative 3D spline distance (`0..=0x1_0000`) for
/// each intermediate split node (`FUN_00462230`, `0x462230..0x4623c0`).
fn compute_split_progress_q16(nodes: &[CatNode], pending_bits: &[u8], car_bit: u8) -> Vec<i32> {
    if nodes.len() < 2 {
        return Vec::new();
    }
    let mut cumulative = Vec::with_capacity(nodes.len());
    cumulative.push(0.0_f64);
    let mut total = 0.0_f64;
    for pair in nodes.windows(2) {
        let dx = f64::from(pair[1].position[0].wrapping_sub(pair[0].position[0])) / 65536.0;
        let dy = f64::from(pair[1].position[1].wrapping_sub(pair[0].position[1])) / 65536.0;
        let dz = f64::from(pair[1].position[2].wrapping_sub(pair[0].position[2])) / 65536.0;
        total += (dx * dx + dy * dy + dz * dz).sqrt();
        cumulative.push(total);
    }
    if total <= f64::EPSILON {
        return Vec::new();
    }
    let last_node = nodes.len() - 1;
    let mut splits = Vec::new();
    for (idx, &bits) in pending_bits.iter().enumerate() {
        if idx < last_node && bits & car_bit != 0 {
            let q16 = ((cumulative[idx] / total) * 65536.0).round() as i32;
            splits.push(q16.clamp(0, 0x1_0000));
            if splits.len() == (FINISH_GATE_COUNT as usize).saturating_sub(1) {
                break;
            }
        }
    }
    splits
}

/// Normalize the checkpoint delta exactly as `FUN_00461e90`. Components outside
/// the inclusive ±100.0 interval first use signed division by 512, preventing
/// the square-sum from overflowing before legacy-table normalization.
#[must_use]
pub fn normalize_delta_pre_transform(delta: [i32; 3]) -> [i32; 3] {
    let scaled = if delta
        .iter()
        .any(|value| *value > 0x640000 || *value < -0x640000)
    {
        delta.map(|value| value.wrapping_add((value >> 31) & 0x1ff) >> 9)
    } else {
        delta
    };
    crate::compiled_car_initializer::normalize_compiled_lane_vector(scaled)
}

/// The recovered horizontal crossing dot: the `FUN_00461e90`-normalized
/// `(node.pos - car.pos)` dotted with node.dir in x-z only, in Q16.16.
fn horizontal_dot(node: &CatNode, car_pos: Vec3) -> Fixed16 {
    let delta = normalize_delta_pre_transform([
        node.position[0].wrapping_sub(car_pos.x.raw()),
        node.position[1].wrapping_sub(car_pos.y.raw()),
        node.position[2].wrapping_sub(car_pos.z.raw()),
    ]);
    Fixed16::from_raw(delta[0])
        .mul(Fixed16::from_raw(node.direction[0]))
        .add(Fixed16::from_raw(delta[2]).mul(Fixed16::from_raw(node.direction[2])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_pre_transform_normalizes_and_prescales_only_large_components() {
        assert_eq!(normalize_delta_pre_transform([0, 0, 0]), [0, 0, 0]);
        assert_eq!(
            normalize_delta_pre_transform([0x10000, 0, 0]),
            [65522, 0, 0]
        );
        assert_eq!(
            normalize_delta_pre_transform([0x640001, 0, 0]),
            [65530, 0, 0]
        );
        assert_eq!(
            normalize_delta_pre_transform([i32::MIN, 0, 0]),
            [-65472, 0, 0]
        );
    }

    /// A `.cat` node at whole-unit position/direction, Q16.16-encoded.
    fn node(pos: [i32; 3], dir: [i32; 3], split: u8) -> CatNode {
        CatNode {
            position: [pos[0] << 16, pos[1] << 16, pos[2] << 16],
            direction: [dir[0] << 16, dir[1] << 16, dir[2] << 16],
            split_flag: split,
            reserved: 0,
            seq_index: 0,
        }
    }

    /// Car world position at whole-unit (x, z), on the ground (y = 0).
    fn at(x: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fixed16::from_int(x),
            y: Fixed16::ZERO,
            z: Fixed16::from_int(z),
        }
    }

    /// A straight +x line of three nodes at x = 1, 2, 3, all facing +x.
    fn straight_line(split: [u8; 3]) -> Vec<CatNode> {
        vec![
            node([1, 0, 0], [1, 0, 0], split[0]),
            node([2, 0, 0], [1, 0, 0], split[1]),
            node([3, 0, 0], [1, 0, 0], split[2]),
        ]
    }

    #[test]
    fn cursor_advances_node_by_node_as_the_car_passes_each() {
        let nodes = straight_line([0, 0, 0]);
        let mut p = StageProgress::new(&nodes, 0);

        // Before node 0: nothing happens.
        assert_eq!(p.update(at(0, 0), 0), StageEvent::Idle);
        assert_eq!(p.cursor(), 0);

        // Past node 0 (x = 1): advance to cursor 1.
        assert_eq!(p.update(at(2, 0), 10), StageEvent::Advanced { node: 0 });
        assert_eq!(p.cursor(), 1);

        // Between node 1 and node 2 would be idle; jump past node 1.
        assert_eq!(p.update(at(2, 0), 11), StageEvent::Idle); // exactly at node1 -> not strictly past
        assert_eq!(p.update(at(3, 0), 12), StageEvent::Advanced { node: 1 });
        assert_eq!(p.cursor(), 2);

        // Past the last node finishes by node count (no splits on this line).
        assert_eq!(
            p.update(at(4, 0), 20),
            StageEvent::Finished {
                node: 2,
                centiseconds: 20
            }
        );
        assert!(p.finished());
        assert_eq!(p.finish_centiseconds(), Some(20));
        assert_eq!(p.splits_passed(), 0);
        // Further updates are inert.
        assert_eq!(p.update(at(5, 0), 21), StageEvent::Idle);
    }

    #[test]
    fn split_flagged_crossings_record_the_clock_in_order() {
        // Node 0 and node 1 are splits for car 0 (bit 0); node 2 is the finish.
        let nodes = straight_line([0b1, 0b1, 0]);
        let mut p = StageProgress::new(&nodes, 0);

        assert_eq!(
            p.update(at(2, 0), 100),
            StageEvent::Split {
                node: 0,
                split_number: 1,
                centiseconds: 100
            }
        );
        assert_eq!(
            p.update(at(3, 0), 250),
            StageEvent::Split {
                node: 1,
                split_number: 2,
                centiseconds: 250
            }
        );
        assert_eq!(
            p.update(at(4, 0), 400),
            StageEvent::Finished {
                node: 2,
                centiseconds: 400
            }
        );
        assert_eq!(p.split_times(), &[100, 250]);
        assert_eq!(p.finish_centiseconds(), Some(400));
    }

    #[test]
    fn a_split_is_per_car_so_another_car_is_unaffected() {
        // Node 0 is a split only for car 1 (bit 1), not car 0.
        let nodes = straight_line([0b10, 0, 0]);

        let mut car0 = StageProgress::new(&nodes, 0);
        assert_eq!(car0.update(at(2, 0), 5), StageEvent::Advanced { node: 0 });
        assert_eq!(car0.splits_passed(), 0);

        let mut car1 = StageProgress::new(&nodes, 1);
        assert_eq!(
            car1.update(at(2, 0), 5),
            StageEvent::Split {
                node: 0,
                split_number: 1,
                centiseconds: 5
            }
        );
    }

    #[test]
    fn regression_steps_the_cursor_back_and_a_passed_split_does_not_refire() {
        let nodes = straight_line([0b1, 0, 0]);
        let mut p = StageProgress::new(&nodes, 0);

        // Cross the split at node 0.
        assert_eq!(
            p.update(at(2, 0), 30),
            StageEvent::Split {
                node: 0,
                split_number: 1,
                centiseconds: 30
            }
        );
        assert_eq!(p.cursor(), 1);

        // Fall back in front of node 0: the cursor regresses.
        assert_eq!(p.update(at(0, 0), 31), StageEvent::Regressed { to: 0 });
        assert_eq!(p.cursor(), 0);

        // Re-cross node 0: it advances but the already-counted split does NOT
        // re-fire (the per-car bit was cleared).
        assert_eq!(p.update(at(2, 0), 40), StageEvent::Advanced { node: 0 });
        assert_eq!(p.splits_passed(), 1);
        assert_eq!(p.split_times(), &[30]);
    }

    #[test]
    fn regressions_count_up_to_five_and_a_forward_crossing_clears_them() {
        let nodes = straight_line([0, 0, 0]);
        let mut p = StageProgress::new(&nodes, 0);
        p.update(at(2, 0), 0);
        p.update(at(3, 0), 0);
        assert_eq!(p.cursor(), 2);
        for _ in 0..7 {
            p.set_cursor(2);
            p.update(at(1, 0), 0);
        }
        assert_eq!(p.reverse_crossings(), 5, "held at 5");
        p.update(at(3, 0), 0);
        assert_eq!(p.reverse_crossings(), 0);
    }

    #[test]
    fn the_crossing_test_ignores_the_vertical_axis() {
        // A node high above the car (large +y) still triggers on the x-z dot.
        let nodes = vec![
            node([1, 1000, 0], [1, 1000, 0], 0),
            node([2, 1000, 0], [1, 0, 0], 0),
        ];
        let mut p = StageProgress::new(&nodes, 0);
        // Car on the ground (y = 0) still crosses node 0 by its x-z position.
        assert_eq!(p.update(at(2, 0), 0), StageEvent::Advanced { node: 0 });
    }

    #[test]
    fn a_diagonal_gate_triggers_on_its_own_forward_direction() {
        // Node facing the +x+z diagonal at (2,0,2).
        let nodes = vec![
            node([2, 0, 2], [1, 0, 1], 0b1),
            node([4, 0, 4], [1, 0, 1], 0),
        ];
        let mut p = StageProgress::new(&nodes, 0);
        // Before the gate along the diagonal: dot > 0.
        assert_eq!(p.update(at(1, 1), 0), StageEvent::Idle);
        // Past the gate: dot < 0.
        assert_eq!(
            p.update(at(3, 3), 77),
            StageEvent::Split {
                node: 0,
                split_number: 1,
                centiseconds: 77
            }
        );
    }

    #[test]
    fn nine_gate_cat_spline_ignores_start_gate_split_and_finishes_only_at_final_node() {
        // Real `.cat` files have 9 split-flagged nodes: node 0 (start gate),
        // 7 intermediate splits, and the final node (finish gate). Crossing the
        // 7th intermediate split must NOT finish the stage prematurely; only
        // crossing the final node finishes the stage.
        let nodes: Vec<CatNode> = (1..=9).map(|x| node([x, 0, 0], [1, 0, 0], 0b1)).collect();
        let mut p = StageProgress::new(&nodes, 0);
        // Node 0's start-gate split bit was cleared at init, so crossing node 0
        // advances without incrementing `splits_passed`.
        assert_eq!(p.update(at(2, 0), 100), StageEvent::Advanced { node: 0 });
        assert_eq!(p.splits_passed(), 0);

        // Crossing nodes 1..=7 fires splits 1..=7 without finishing early.
        for split_num in 1..=7 {
            let x = (split_num as i32) + 2;
            assert_eq!(
                p.update(at(x, 0), x * 100),
                StageEvent::Split {
                    node: split_num as usize,
                    split_number: split_num,
                    centiseconds: x * 100,
                }
            );
            assert!(!p.finished());
        }
        assert_eq!(p.splits_passed(), 7);
        assert_eq!(p.split_display_ticks(), SPLIT_DISPLAY_TICKS);

        // Crossing the final node (index 8 at x = 9) finishes the stage.
        assert_eq!(
            p.update(at(10, 0), 1000),
            StageEvent::Finished {
                node: 8,
                centiseconds: 1000
            }
        );
        assert!(p.finished());
        assert_eq!(p.splits_passed(), FINISH_GATE_COUNT);
        assert_eq!(p.cursor(), 9);
    }
}
