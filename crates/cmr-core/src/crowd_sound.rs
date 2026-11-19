//! The crowd ambience feeder of `FUN_0041cfe0` (3D emitter attenuation `FUN_004866b0`).
//!
//! At stage load `FUN_00481990` walks the stage's scene nodes and keeps, in
//! node order and at most 49, the positions (local matrix translation,
//! node `+0x94 + 0x30`) of the nodes whose id (`+0x2c & 0xfff`) is `0xe`. A
//! node whose class high half has bit `0x2000` clear and either of its low two
//! bits set is skipped. Each tick the crowd volume is the sum of the squared
//! integer distances (`(point - car) >> 16` per axis) of the points within
//! 25 units of player 0's car, times the crowd factor `DAT_004eb2b0`, over
//! 5000; with no point in range the crowd stops. The value is capped to 32000
//! above `0x7d00` and passed through the centre-proximity curve before the
//! ambience controller ([`crate::audio_ambience`]) plays it. The sum grows
//! with distance and with the number of points, kept as recovered.
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.crowd-sound`.

use crate::audio_ambience::AmbienceCue;
use crate::engine_listener::centre_curve;

/// The crowd node id.
pub const CROWD_NODE_ID: u32 = 0xe;
/// At most this many crowd points are gathered (`ebx < 0x31`).
pub const MAX_CROWD_POINTS: usize = 0x31;
/// The squared-distance radius (`< 0x271`, 25 units).
pub const CROWD_RADIUS_SQUARED: i32 = 0x271;

/// `FUN_00481990`'s crowd gather over the stage nodes' class dwords and
/// 16-word local matrices, in node order.
pub fn crowd_points(nodes: impl IntoIterator<Item = (u32, [i32; 16])>) -> Vec<[i32; 3]> {
    let mut points = Vec::new();
    for (class, matrix) in nodes {
        let high = class >> 16;
        if high & 0x2000 == 0 && high & 3 != 0 {
            continue;
        }
        if class & 0xfff == CROWD_NODE_ID && points.len() < MAX_CROWD_POINTS {
            points.push([matrix[12], matrix[13], matrix[14]]);
        }
    }
    points
}

/// The crowd volume before the proximity curve, or `None` with no point in
/// range.
pub fn crowd_volume(points: &[[i32; 3]], car: [i32; 3], crowd_k: i32) -> Option<i32> {
    let mut sum = 0_i32;
    let mut count = 0;
    for point in points {
        let d2 = (0..3).fold(0_i32, |d2, axis| {
            let d = point[axis].wrapping_sub(car[axis]) >> 16;
            d2.wrapping_add(d.wrapping_mul(d))
        });
        if d2 < CROWD_RADIUS_SQUARED {
            sum = sum.wrapping_add(d2);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let v = crowd_k.wrapping_mul(sum) / 5000;
    Some(if v < 0x7d01 { v } else { 32000 })
}

/// The crowd branch of `FUN_0041cfe0` (no storm, `FUN_0048d070() == 0`):
/// the volume through the centre-proximity curve `FUN_0047f9c0(0, v)`, ready
/// for [`crate::audio_ambience::AmbienceController::crowd_or_storm_tick`].
pub fn crowd_cue(
    points: &[[i32; 3]],
    car: [i32; 3],
    crowd_k: i32,
    centre_proximity: i32,
) -> AmbienceCue {
    AmbienceCue::Crowd(
        crowd_volume(points, car, crowd_k).map(|v| centre_curve(centre_proximity, v)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cue_takes_the_centre_curve() {
        let points = [[3 * ONE, 0, 0]];
        assert_eq!(
            crowd_cue(&points, [0; 3], 50_000, 500),
            AmbienceCue::Crowd(Some(90 * 500 / 1000))
        );
        assert_eq!(
            crowd_cue(&points, [0; 3], 50_000, 0x14d),
            AmbienceCue::Crowd(Some(30))
        );
        assert_eq!(
            crowd_cue(&[], [0; 3], 50_000, 500),
            AmbienceCue::Crowd(None)
        );
    }

    const ONE: i32 = 0x1_0000;

    fn at(x: i32) -> [i32; 16] {
        let mut m = [0; 16];
        m[12] = x * ONE;
        m
    }

    #[test]
    fn only_unskipped_crowd_nodes_are_gathered() {
        let a = at(1);
        let b = at(2);
        let c = at(3);
        let d = at(4);
        let points = crowd_points([(0xe, a), (0xf, b), (0x0001_000e, c), (0x2001_000e, d)]);
        assert_eq!(points, vec![[ONE, 0, 0], [4 * ONE, 0, 0]]);
        let all = crowd_points(std::iter::repeat_n((0xe, at(0)), 60));
        assert_eq!(all.len(), MAX_CROWD_POINTS);
    }

    #[test]
    fn the_volume_sums_squared_distances_in_range() {
        let points = [[3 * ONE, 0, 0], [0, 0, 4 * ONE], [30 * ONE, 0, 0]];
        assert_eq!(crowd_volume(&points, [0; 3], 5000), Some(9 + 16));
        assert_eq!(crowd_volume(&points[2..], [0; 3], 5000), None);
        assert_eq!(crowd_volume(&points, [0; 3], 50_000_000), Some(32000));
    }
}
