//! Reconstruction of the replay trace format and the display-snapshot buffer
//! (the confidently-recovered G08 pieces).
//!
//! The game has two distinct replay subsystems:
//!
//! - the **authoritative input trace** (the "Replay" class, `FUN_0046d7b0` /
//!   `FUN_0046de30`): per-car, **input-based re-simulation**, saved to / loaded
//!   from `.SSR` files and also streamed for the network replay car. Each object
//!   is a `0x40`-byte header, a per-frame `0x151ec`-byte initial-state seed
//!   (Region A), then `nTicks` 4-byte packed control records (Region B). A record
//!   holds the quantized, XOR-delta-coded actuators (steer/throttle/brake by
//!   `>>10`, plus gear/flags); on playback the normal physics step re-runs over
//!   the decoded inputs (`FUN_0046cd70` / `FUN_0046dc20`). `nTicks` is `0x1d4c`
//!   (7500 = 300 s at 25 Hz) or `0x3a98` at the 50 Hz option. Files load through
//!   the shared loader `FUN_00495ed0`, whose optional archive container carries
//!   the `"CMPR"` magic + 4-byte size. A final little-endian `u16` follows a
//!   single frame's records. It is the zero-based current/end write cursor:
//!   aggregate scans include `0..=end`, while playback stops at `0..end`.
//! - the **render snapshot buffer** (`FUN_0046c7d0` / `FUN_0046ca70`, gated on the
//!   record flag): 24-byte keyframes (a position vec3 of `i32` plus two
//!   `3xi16` compressed orientation axes) captured every 4th subframe (6.25 Hz),
//!   up to `0x8ca` per car across 6 car lanes, interpolated into the render
//!   transforms for a smooth action replay. This never feeds physics.
//!
//! See evidence `rea.official-eu.sim-replay`.
//!
//! Observations: the object sizing/region strides, the `nTicks` values, the
//! `>>10` quantization + XOR-delta codec, the `.SSR` extension and `"CMPR"`
//! container magic, and the snapshot record shape/capacity/compression.
//! Inferences: Region A is the deterministic re-sim seed; track/car identity is
//! carried in the `.SSR` filename. The four record-byte assignments and Region-A
//! section layout are recovered. Unknowns (not reproduced): the explicit `.SSR`
//! write path and the network wire framing.

use crate::camera_crossfade::blend_pose;
use crate::drivetrain::cross_q16;
use crate::fixed_point::Fixed16;

/// Per-car input-trace ticks at 25 Hz (`0x1d4c` = 7500 = 300 s).
pub const REPLAY_TICKS_25HZ: u32 = 0x1d4c;

/// Per-car input-trace ticks at the 50 Hz option (`0x3a98` = 15000).
pub const REPLAY_TICKS_50HZ: u32 = 0x3a98;

/// Maximum replay duration in seconds (both rates: `ticks / rate`).
pub const REPLAY_MAX_DURATION_SECONDS: u32 = 300;

/// Bytes of the per-frame initial-state seed block (Region A), `0x151ec`.
pub const REPLAY_INITIAL_STATE_BYTES: usize = 0x151ec;

/// Dwords of the Region-A block (`0x151ec / 4 = 0x547b`).
pub const REPLAY_INITIAL_STATE_DWORDS: usize = 0x547b;

/// Region-A car-type block offset (`0x0000`) and size (`0xcd0`).
pub const REPLAY_CAR_TYPE_OFFSET: usize = 0x0000;
pub const REPLAY_CAR_TYPE_BYTES: usize = 0xcd0;
/// Region-A control-block offset (`0xcd0`) and size (`0x464`).
pub const REPLAY_CONTROL_OFFSET: usize = 0xcd0;
pub const REPLAY_CONTROL_BYTES: usize = 0x464;
/// Region-A reserved header gap (`0x1134..0x11f4`, `0xc0` bytes) before the physics block.
pub const REPLAY_RESERVED_GAP_OFFSET: usize = 0x1134;
pub const REPLAY_RESERVED_GAP_BYTES: usize = 0x00c0;
/// Region-A physics-block offset (`0x11f4`) and size (`0x768`).
pub const REPLAY_PHYSICS_OFFSET: usize = 0x11f4;
pub const REPLAY_PHYSICS_BYTES: usize = 0x768;
/// Region-A object-record offset (`0x195c`) and stride (`0xa0`).
pub const REPLAY_OBJECTS_OFFSET: usize = 0x195c;
pub const REPLAY_OBJECT_RECORD_BYTES: usize = 0xa0;
/// Region-A trailing state/timer offsets (`0x151dc`/`0x151e8`) and sizes (`12`/`4`).
pub const REPLAY_TRAILING_STATE_OFFSET: usize = 0x151dc;
pub const REPLAY_TRAILING_STATE_BYTES: usize = 12;
pub const REPLAY_TIMER_OFFSET: usize = 0x151e8;
pub const REPLAY_TIMER_BYTES: usize = 4;
/// Maximum object records before the trailing 12-byte state block (`0x1f4`).
pub const REPLAY_MAX_OBJECTS: usize = 0x1f4;

/// Checked offset of one Region-A object record.
#[must_use]
pub fn replay_object_record_offset(index: usize) -> Option<usize> {
    (index < REPLAY_MAX_OBJECTS)
        .then_some(REPLAY_OBJECTS_OFFSET + index * REPLAY_OBJECT_RECORD_BYTES)
}

/// Bytes of one per-tick control record (Region B element).
pub const REPLAY_TICK_RECORD_BYTES: usize = 4;

/// Bytes of the replay-object header prefix.
pub const REPLAY_OBJECT_HEADER_BYTES: usize = 0x40;

/// Bytes of the per-frame compressed-record end/write cursor after Region B.
pub const REPLAY_RECORD_END_CURSOR_BYTES: usize = 2;

/// Per-frame fixed bytes in the object-size formula: Region A plus the record
/// end cursor (`0x151ec + 2 = 0x151ee`).
pub const REPLAY_FRAME_FIXED_BYTES: usize =
    REPLAY_INITIAL_STATE_BYTES + REPLAY_RECORD_END_CURSOR_BYTES;

/// The `.SSR` replay/shadow file extension (uppercase, as the format literal
/// `"%s%s%c%d%c.SSR"` produces).
pub const REPLAY_FILE_EXTENSION: &str = ".SSR";

/// Archive-container magic for the shared loader (`"CMPR"`).
pub const CMPR_MAGIC: [u8; 4] = *b"CMPR";

/// Bytes of the `"CMPR"` container header (magic + 4-byte little-endian size).
pub const CMPR_HEADER_BYTES: usize = 8;

/// Bytes of one render-snapshot keyframe (pos `3xi32` + 2 x `3xi16`).
pub const SNAPSHOT_RECORD_BYTES: usize = 0x18;

/// Render-snapshot keyframes stored per car (`0x8ca` = 2250 ~ 360 s at 6.25 Hz).
pub const SNAPSHOT_CAPACITY_PER_CAR: usize = 0x8ca;

/// Render-snapshot car lanes.
pub const SNAPSHOT_CAR_LANES: usize = 6;

/// Total render-snapshot keyframe entries (`0x8ca * 6 = 0x34bc`).
pub const SNAPSHOT_BUFFER_ENTRIES: usize = SNAPSHOT_CAPACITY_PER_CAR * SNAPSHOT_CAR_LANES;

/// Right shift used to quantize an actuator into a record field (`>>10`).
pub const ACTUATOR_QUANT_SHIFT: u32 = 10;

/// Mask kept after quantization (6-bit field, `0x3f`).
pub const ACTUATOR_QUANT_MASK: u8 = 0x3f;

/// Orientation-axis compression scale applied before clamping (`0x8000`).
pub const ORIENTATION_COMPRESS_SCALE: i64 = 0x8000;

/// Orientation-axis compression clamp magnitude (`0x7ff9`, inside `i16` range).
pub const ORIENTATION_COMPRESS_CLAMP: i32 = 0x7ff9;

/// Per-subframe render-snapshot interpolation step in Q16.16 (`0x4000` = 0.25);
/// subframes `0..=3` give factors `0.0, 0.25, 0.5, 0.75`.
pub const SNAPSHOT_INTERP_STEP: i32 = 0x4000;

/// The per-car input-trace tick count for the active rate (`FUN_00422be0`:
/// `((double_rate)+1) * 0x1d4c`).
pub fn replay_ticks(double_rate: bool) -> u32 {
    if double_rate {
        REPLAY_TICKS_50HZ
    } else {
        REPLAY_TICKS_25HZ
    }
}

/// Serialized replay-object size in bytes, reproducing `FUN_0046d7b0`:
/// `(nTicks*4 + 0x151ec + 2) * nFrames + 0x40`. The file loader validates the
/// loaded size against this exact value.
pub fn replay_object_size(n_ticks: u32, n_frames: u32) -> usize {
    (n_ticks as usize * REPLAY_TICK_RECORD_BYTES + REPLAY_FRAME_FIXED_BYTES) * n_frames as usize
        + REPLAY_OBJECT_HEADER_BYTES
}

/// Byte offset of the per-tick record stream (Region B) within the object:
/// after the header and the `nFrames` Region-A seed blocks.
pub fn replay_region_b_offset(n_frames: u32) -> usize {
    REPLAY_OBJECT_HEADER_BYTES + n_frames as usize * REPLAY_INITIAL_STATE_BYTES
}

/// Byte offset of the record end/write cursor in a one-frame replay object.
///
/// The native pointer fixup establishes the multi-frame array placement; this
/// serialized accessor deliberately exposes only the currently needed
/// one-frame layout.
#[must_use]
pub fn replay_single_frame_record_end_cursor_offset(n_ticks: u32) -> Option<usize> {
    REPLAY_OBJECT_HEADER_BYTES
        .checked_add(REPLAY_INITIAL_STATE_BYTES)?
        .checked_add((n_ticks as usize).checked_mul(REPLAY_TICK_RECORD_BYTES)?)
}

/// Read the zero-based record end/write cursor from an exactly-sized one-frame replay.
/// Truncated and overlong inputs are rejected rather than partially accepted.
#[must_use]
pub fn replay_single_frame_record_end_cursor(bytes: &[u8], n_ticks: u32) -> Option<u16> {
    let offset = replay_single_frame_record_end_cursor_offset(n_ticks)?;
    let end = offset.checked_add(REPLAY_RECORD_END_CURSOR_BYTES)?;
    if bytes.len() != end {
        return None;
    }
    let raw = bytes.get(offset..end)?;
    Some(u16::from_le_bytes(raw.try_into().ok()?))
}

/// Record indices included by the duration/sum consumer (`0..=end_cursor`).
#[must_use]
pub fn replay_record_aggregate_range(
    end_cursor: u16,
    record_capacity: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    let end = usize::from(end_cursor);
    (end < record_capacity).then_some(0..=end)
}

/// Record indices decoded by playback before it reaches the boundary (`0..end_cursor`).
#[must_use]
pub fn replay_record_playback_range(
    end_cursor: u16,
    record_capacity: usize,
) -> Option<std::ops::Range<usize>> {
    let end = usize::from(end_cursor);
    (end < record_capacity).then_some(0..end)
}

/// Whether a loaded file's size matches the replay object it claims to be
/// (`FUN_0046ddc0`'s size check).
pub fn replay_file_size_valid(loaded_bytes: usize, n_ticks: u32, n_frames: u32) -> bool {
    loaded_bytes == replay_object_size(n_ticks, n_frames)
}

/// Quantize a Q16.16 actuator value into its 6-bit record field (`(v >> 10) &
/// 0x3f`).
pub fn quantize_actuator(value: i32) -> u8 {
    ((value >> ACTUATOR_QUANT_SHIFT) & ACTUATOR_QUANT_MASK as i32) as u8
}

/// XOR-delta encode a quantized field against the previous record's field
/// (`(prev ^ cur) & 0x3f`).
pub fn delta_encode(previous: u8, current: u8) -> u8 {
    (previous ^ current) & ACTUATOR_QUANT_MASK
}

/// XOR-delta decode a record field given the previous one (the inverse of
/// [`delta_encode`], since XOR is self-inverse).
pub fn delta_decode(previous: u8, encoded: u8) -> u8 {
    (previous ^ encoded) & ACTUATOR_QUANT_MASK
}

/// Native control fields consumed by `FUN_0046cd70`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayActuators {
    pub steer: i32,
    pub steer_alternate: i32,
    pub throttle: i32,
    pub brake: i32,
    pub handbrake: bool,
    pub flag_310: bool,
    pub gear: i32,
    pub analog_steer: bool,
    pub analog_throttle: bool,
    pub analog_brake: bool,
}

/// One four-byte input-trace record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayControlRecord(pub [u8; REPLAY_TICK_RECORD_BYTES]);

/// Encode one record; the double-XOR steer/gear sequences become their current
/// values, while the repeat counter alone retains history.
#[must_use]
pub fn encode_replay_control(
    input: ReplayActuators,
    previous: ReplayControlRecord,
    recording_active: bool,
) -> ReplayControlRecord {
    let analog_steer = input.analog_steer && recording_active;
    let analog_throttle = input.analog_throttle && recording_active;
    let analog_brake = input.analog_brake && recording_active;
    let mut steer_byte = (previous.0[3] & 0x7f) | u8::from(analog_steer) << 7;
    let steer_value = if input.steer < 1 {
        steer_byte = (previous.0[3] & 0x3f) | u8::from(analog_steer) << 7;
        if analog_steer {
            quantize_actuator(input.steer_alternate)
        } else {
            u8::from(input.steer_alternate != 0)
        }
    } else {
        steer_byte |= 0x40;
        if analog_steer {
            quantize_actuator(input.steer)
        } else {
            u8::from(input.steer != 0)
        }
    };
    steer_byte = (steer_byte & 0xc0) | (steer_value & 0x3f);
    let throttle = if analog_throttle {
        quantize_actuator(input.throttle)
    } else {
        u8::from(input.throttle != 0)
    };
    let brake = if analog_brake {
        quantize_actuator(input.brake)
    } else {
        u8::from(input.brake != 0)
    };
    let gear = (input.gear.wrapping_add(1) as u8) & 3;
    let repeat = previous.0[2].wrapping_add(1) & 0x3f;
    ReplayControlRecord([
        u8::from(analog_throttle) | u8::from(analog_brake) << 1 | (throttle & 0x3f) << 2,
        (brake & 0x3f) << 2 | gear,
        u8::from(input.handbrake) << 7 | u8::from(input.flag_310) << 6 | repeat,
        steer_byte,
    ])
}

/// Decoded actuator/mode values and the native repeat-counter outcome.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayDecodedControl {
    pub actuators: ReplayActuators,
    pub control_410: bool,
    pub control_414: bool,
    pub control_418: bool,
    pub next_repeat: u8,
    pub repeat_complete: bool,
}

/// Decode a record and advance the independent repeat counter.
#[must_use]
pub fn decode_replay_control(
    record: ReplayControlRecord,
    previous_repeat: u8,
) -> ReplayDecodedControl {
    let steer_byte = record.0[3];
    let steer = if steer_byte & 0x80 == 0 {
        if steer_byte & 0x3f == 0 { 0 } else { -66 }
    } else {
        i32::from(steer_byte & 0x3f) << 10
    };
    let (steer, steer_alternate) = if steer_byte & 0x40 == 0 {
        (0, steer)
    } else {
        (steer, 0)
    };
    let throttle = if record.0[0] & 1 == 0 {
        if record.0[0] & 0xfc == 0 { 0 } else { -66 }
    } else {
        i32::from(record.0[0] >> 2) << 10
    };
    let brake = if record.0[0] & 2 == 0 {
        if record.0[1] & 0xfc == 0 { 0 } else { -66 }
    } else {
        i32::from(record.0[1] >> 2) << 10
    };
    let next_repeat = previous_repeat.wrapping_add(1);
    let repeat_complete = next_repeat >= record.0[2] & 0x3f;
    ReplayDecodedControl {
        actuators: ReplayActuators {
            steer,
            steer_alternate,
            throttle,
            brake,
            handbrake: record.0[2] & 0x80 != 0,
            flag_310: record.0[2] & 0x40 != 0,
            gear: i32::from(record.0[1] & 3) - 1,
            analog_steer: steer_byte & 0x80 != 0,
            analog_throttle: record.0[0] & 1 != 0,
            analog_brake: record.0[0] & 2 != 0,
        },
        control_410: steer_byte & 0x80 != 0,
        control_414: record.0[0] & 1 != 0,
        control_418: record.0[0] & 2 != 0,
        next_repeat: if repeat_complete { 0 } else { next_repeat },
        repeat_complete,
    }
}

/// Whether the network replay consumer accepts a record instead of suppressing
/// it as a duplicate. Repeat `0` suppresses unconditionally; `0x3f` always accepts.
#[must_use]
pub fn network_replay_record_accepted(
    previous: ReplayControlRecord,
    current: ReplayControlRecord,
) -> bool {
    let repeat = current.0[2] & 0x3f;
    if repeat == 0 {
        return false;
    }
    if repeat == 0x3f {
        return true;
    }
    let throttle = previous.0[0] ^ current.0[0];
    let brake_gear = previous.0[1] ^ current.0[1];
    let steer = previous.0[3] ^ current.0[3];
    let flags = previous.0[2] ^ current.0[2];
    !(throttle & 0xfc == 0
        && brake_gear & 0xfc == 0
        && steer & 0x7f == 0
        && flags & 0xc0 == 0
        && brake_gear & 3 == 0
        && throttle & 3 == 0
        && steer & 0x80 == 0)
}

/// Parse a `"CMPR"` container header, returning the payload size, or `None` if
/// the magic is absent or the slice is too short.
pub fn parse_cmpr_header(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < CMPR_HEADER_BYTES || bytes[0..4] != CMPR_MAGIC {
        return None;
    }
    Some(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]))
}

/// Flat index of a render-snapshot keyframe for `car` at `tick`
/// (`[car*0x8ca + tick]`), or `None` if either is out of range.
pub fn snapshot_record_index(car: usize, tick: usize) -> Option<usize> {
    if car < SNAPSHOT_CAR_LANES && tick < SNAPSHOT_CAPACITY_PER_CAR {
        Some(car * SNAPSHOT_CAPACITY_PER_CAR + tick)
    } else {
        None
    }
}

/// Compress one orientation-axis component into the `i16` stored in a snapshot
/// keyframe, reproducing `FUN_0046c710`: scale by `0x8000` (Q16.16) and clamp to
/// `±0x7ff9`.
pub fn compress_orientation(component: Fixed16) -> i16 {
    let scaled = (component.raw() as i64 * ORIENTATION_COMPRESS_SCALE) >> 16;
    scaled.clamp(
        -(ORIENTATION_COMPRESS_CLAMP as i64),
        ORIENTATION_COMPRESS_CLAMP as i64,
    ) as i16
}

/// The render-snapshot interpolation factor for a subframe `0..=3`
/// (`subframe * 0.25`), reproducing `FUN_0046ca70`'s sub-frame blend.
pub fn snapshot_interp_factor(subframe: u32) -> Fixed16 {
    Fixed16::from_raw((subframe & 3) as i32 * SNAPSHOT_INTERP_STEP)
}

/// Orientation-axis decompression scale (`FUN_0046c910`: `(c as i64 * 0x20000) >> 16` = `(c as i32) << 1`).
pub const ORIENTATION_DECOMPRESS_SCALE: i64 = 0x2_0000;

/// Per-axis position jump threshold (`0x14_0000` = `20.0` in Q16.16) above which
/// `FUN_0046ca70` suppresses interpolation and teleports to the current keyframe.
pub const SNAPSHOT_TELEPORT_THRESHOLD_Q16: i32 = 0x14_0000;

/// Decompress one `i16` orientation component back into Q16.16 (`FUN_0046c910`).
#[must_use]
pub const fn decompress_orientation(compressed: i16) -> i32 {
    ((compressed as i64 * ORIENTATION_DECOMPRESS_SCALE) >> 16) as i32
}

/// One 24-byte (`0x18`) render-snapshot keyframe (`DAT_00592f84` element).
///
/// Layout (`FUN_0046c7d0` / `FUN_0046c910`):
/// - `+0x00..+0x0c`: `[i32; 3]` Q16.16 position `(x, y, z)` from `[car_render_tf + 0x30..+0x3c]`.
/// - `+0x0c..+0x12`: `[i16; 3]` compressed Row 0 (Right / X-axis from `[car_render_tf + 0x00..+0x0c]`).
/// - `+0x12..+0x18`: `[i16; 3]` compressed Row 1 (Up / Y-axis from `[car_render_tf + 0x10..+0x1c]`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SnapshotKeyframe {
    pub pos: [i32; 3],
    pub row0_compressed: [i16; 3],
    pub row1_compressed: [i16; 3],
}

impl SnapshotKeyframe {
    /// Capture a keyframe from a 4x4 Q16.16 render transform (`FUN_0046c7d0`).
    #[must_use]
    pub fn from_transform_46c7d0(tf: &[i32; 16]) -> Self {
        Self {
            pos: [tf[12], tf[13], tf[14]],
            row0_compressed: [
                compress_orientation(Fixed16::from_raw(tf[0])),
                compress_orientation(Fixed16::from_raw(tf[1])),
                compress_orientation(Fixed16::from_raw(tf[2])),
            ],
            row1_compressed: [
                compress_orientation(Fixed16::from_raw(tf[4])),
                compress_orientation(Fixed16::from_raw(tf[5])),
                compress_orientation(Fixed16::from_raw(tf[6])),
            ],
        }
    }

    /// Unpack a keyframe into a 4x4 Q16.16 render transform (`FUN_0046c910`),
    /// reconstructing Row 2 via `FUN_0048e810` (`cross_q16(row0, row1)`).
    #[must_use]
    pub fn to_transform_46c910(&self) -> [i32; 16] {
        let r0 = [
            decompress_orientation(self.row0_compressed[0]),
            decompress_orientation(self.row0_compressed[1]),
            decompress_orientation(self.row0_compressed[2]),
        ];
        let r1 = [
            decompress_orientation(self.row1_compressed[0]),
            decompress_orientation(self.row1_compressed[1]),
            decompress_orientation(self.row1_compressed[2]),
        ];
        let r2 = cross_q16(r0, r1);
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
            self.pos[0],
            self.pos[1],
            self.pos[2],
            0x1_0000,
        ]
    }

    /// Serialize to the exact 24-byte (`0x18`) little-endian representation.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; SNAPSHOT_RECORD_BYTES] {
        let mut out = [0u8; SNAPSHOT_RECORD_BYTES];
        out[0..4].copy_from_slice(&self.pos[0].to_le_bytes());
        out[4..8].copy_from_slice(&self.pos[1].to_le_bytes());
        out[8..12].copy_from_slice(&self.pos[2].to_le_bytes());
        out[12..14].copy_from_slice(&self.row0_compressed[0].to_le_bytes());
        out[14..16].copy_from_slice(&self.row0_compressed[1].to_le_bytes());
        out[16..18].copy_from_slice(&self.row0_compressed[2].to_le_bytes());
        out[18..20].copy_from_slice(&self.row1_compressed[0].to_le_bytes());
        out[20..22].copy_from_slice(&self.row1_compressed[1].to_le_bytes());
        out[22..24].copy_from_slice(&self.row1_compressed[2].to_le_bytes());
        out
    }

    /// Parse from a 24-byte (`0x18`) little-endian slice.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != SNAPSHOT_RECORD_BYTES {
            return None;
        }
        Some(Self {
            pos: [
                i32::from_le_bytes(bytes[0..4].try_into().ok()?),
                i32::from_le_bytes(bytes[4..8].try_into().ok()?),
                i32::from_le_bytes(bytes[8..12].try_into().ok()?),
            ],
            row0_compressed: [
                i16::from_le_bytes(bytes[12..14].try_into().ok()?),
                i16::from_le_bytes(bytes[14..16].try_into().ok()?),
                i16::from_le_bytes(bytes[16..18].try_into().ok()?),
            ],
            row1_compressed: [
                i16::from_le_bytes(bytes[18..20].try_into().ok()?),
                i16::from_le_bytes(bytes[20..22].try_into().ok()?),
                i16::from_le_bytes(bytes[22..24].try_into().ok()?),
            ],
        })
    }
}

/// Whether two snapshot positions exceed the `20.0` (`0x14_0000`) per-axis
/// teleport threshold in `FUN_0046ca70`.
#[must_use]
pub fn snapshot_exceeds_teleport_threshold_46ca70(prev_pos: [i32; 3], curr_pos: [i32; 3]) -> bool {
    (0..3).any(|i| {
        let diff = prev_pos[i].wrapping_sub(curr_pos[i]);
        let abs_diff = if diff < 0 { diff.wrapping_neg() } else { diff };
        abs_diff > SNAPSHOT_TELEPORT_THRESHOLD_Q16
    })
}

/// Per-car render-snapshot keyframe lane (`0x8ca` = 2250 capacity, `FUN_0046c7d0` / `FUN_0046ca70`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SnapshotCarLane {
    frames: Vec<SnapshotKeyframe>,
}

impl SnapshotCarLane {
    #[must_use]
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    #[must_use]
    pub fn keyframes(&self) -> &[SnapshotKeyframe] {
        &self.frames
    }

    /// Capture a keyframe on every 4th physics tick (`(subframe_counter & 3) == 0`)
    /// up to `SNAPSHOT_CAPACITY_PER_CAR` (`0x8ca` = 2250), reproducing `FUN_0046c7d0`.
    pub fn capture_tick_46c7d0(&mut self, subframe_counter: u32, tf: &[i32; 16]) -> bool {
        if (subframe_counter & 3) != 0 || self.frames.len() >= SNAPSHOT_CAPACITY_PER_CAR {
            return false;
        }
        self.frames.push(SnapshotKeyframe::from_transform_46c7d0(tf));
        true
    }

    /// Evaluate the playback render transform at `(keyframe_index, subframe)` (`FUN_0046ca70`).
    ///
    /// - When `(subframe & 3) == 0`, unpacks `frames[keyframe_index]` directly (`FUN_0046c910`).
    /// - When `(subframe & 3)` is `1..=3`, unpacks `frames[keyframe_index - 1]` and
    ///   `frames[keyframe_index]`; if any axis delta exceeds `0x14_0000` (`20.0`), snaps
    ///   directly to `frames[keyframe_index]`, otherwise blends via `FUN_00435400` (`blend_pose`)
    ///   with `alpha = (subframe & 3) * 0x4000`.
    #[must_use]
    pub fn evaluate_playback_46ca70(&self, keyframe_index: usize, subframe: u32) -> Option<[i32; 16]> {
        let sub = subframe & 3;
        let curr = self.frames.get(keyframe_index)?;
        let curr_tf = curr.to_transform_46c910();
        if sub == 0 {
            return Some(curr_tf);
        }
        let prev_idx = keyframe_index.checked_sub(1)?;
        let prev = self.frames.get(prev_idx)?;
        if snapshot_exceeds_teleport_threshold_46ca70(prev.pos, curr.pos) {
            return Some(curr_tf);
        }
        let prev_tf = prev.to_transform_46c910();
        let alpha = snapshot_interp_factor(sub).raw();
        Some(blend_pose(&prev_tf, &curr_tf, alpha))
    }
}

/// Borrowed view over a `0x151ec`-byte Region-A initial-state seed block
/// (`FUN_0046d370` save / `FUN_0046d460` restore).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayRegionAView<'a> {
    bytes: &'a [u8; REPLAY_INITIAL_STATE_BYTES],
}

impl<'a> ReplayRegionAView<'a> {
    /// Wrap an exact `0x151ec`-byte array.
    #[must_use]
    pub const fn new(bytes: &'a [u8; REPLAY_INITIAL_STATE_BYTES]) -> Self {
        Self { bytes }
    }

    /// Borrow the full `0x151ec`-byte Region-A slice.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8; REPLAY_INITIAL_STATE_BYTES] {
        self.bytes
    }

    /// Borrow from a byte slice if its length is `REPLAY_INITIAL_STATE_BYTES` (`0x151ec`).
    #[must_use]
    pub fn from_slice(slice: &'a [u8]) -> Option<Self> {
        let array: &'a [u8; REPLAY_INITIAL_STATE_BYTES] = slice.try_into().ok()?;
        Some(Self { bytes: array })
    }

    /// Borrow the Region-A block of frame `frame_index` from a full serialized `.SSR`/`.RSP` buffer.
    #[must_use]
    pub fn from_replay_object(
        object_bytes: &'a [u8],
        n_ticks: u32,
        n_frames: u32,
        frame_index: u32,
    ) -> Option<Self> {
        if frame_index >= n_frames || !replay_file_size_valid(object_bytes.len(), n_ticks, n_frames) {
            return None;
        }
        let start = REPLAY_OBJECT_HEADER_BYTES + (frame_index as usize) * REPLAY_INITIAL_STATE_BYTES;
        let end = start + REPLAY_INITIAL_STATE_BYTES;
        Self::from_slice(object_bytes.get(start..end)?)
    }

    /// Car-type / suspension / wheel setup block (`+0x0000..+0x0cd0`, `0x50d950 + car * 0xcd0`).
    #[must_use]
    pub fn car_type_bytes(&self) -> &'a [u8; REPLAY_CAR_TYPE_BYTES] {
        self.bytes[REPLAY_CAR_TYPE_OFFSET..REPLAY_CAR_TYPE_OFFSET + REPLAY_CAR_TYPE_BYTES]
            .try_into()
            .expect("fixed car-type slice length")
    }

    /// Control / drivetrain struct (`+0x0cd0..+0x1134`, `0x5fffc8 + car * 0x464`).
    #[must_use]
    pub fn control_bytes(&self) -> &'a [u8; REPLAY_CONTROL_BYTES] {
        self.bytes[REPLAY_CONTROL_OFFSET..REPLAY_CONTROL_OFFSET + REPLAY_CONTROL_BYTES]
            .try_into()
            .expect("fixed control slice length")
    }

    /// Reserved header gap (`+0x1134..+0x11f4`, `0xc0` bytes) skipped before `FUN_0046d010`.
    #[must_use]
    pub fn reserved_gap_bytes(&self) -> &'a [u8; REPLAY_RESERVED_GAP_BYTES] {
        self.bytes[REPLAY_RESERVED_GAP_OFFSET..REPLAY_RESERVED_GAP_OFFSET + REPLAY_RESERVED_GAP_BYTES]
            .try_into()
            .expect("fixed reserved-gap slice length")
    }

    /// Physics body struct (`+0x11f4..+0x195c`, `0x700300 + car * 0x9c0`).
    #[must_use]
    pub fn physics_bytes(&self) -> &'a [u8; REPLAY_PHYSICS_BYTES] {
        self.bytes[REPLAY_PHYSICS_OFFSET..REPLAY_PHYSICS_OFFSET + REPLAY_PHYSICS_BYTES]
            .try_into()
            .expect("fixed physics slice length")
    }

    /// Moving-object slot `index` (`0..500`, `+0x195c + index * 0xa0`).
    #[must_use]
    pub fn object_record_bytes(&self, index: usize) -> Option<&'a [u8; REPLAY_OBJECT_RECORD_BYTES]> {
        let offset = replay_object_record_offset(index)?;
        self.bytes
            .get(offset..offset + REPLAY_OBJECT_RECORD_BYTES)?
            .try_into()
            .ok()
    }

    /// Trailing 3 dwords (`+0x151dc..+0x151e8`: `[0x57ae70[car], 0x5515fc[car], 0x57b030[car]]`).
    #[must_use]
    pub fn trailing_state_dwords(&self) -> [i32; 3] {
        let base = REPLAY_TRAILING_STATE_OFFSET;
        [
            i32::from_le_bytes(self.bytes[base..base + 4].try_into().unwrap()),
            i32::from_le_bytes(self.bytes[base + 4..base + 8].try_into().unwrap()),
            i32::from_le_bytes(self.bytes[base + 8..base + 12].try_into().unwrap()),
        ]
    }

    /// Trailing stage timer (`+0x151e8..+0x151ec`, `0x57b0ac[car]` in centiseconds).
    #[must_use]
    pub fn timer_centiseconds(&self) -> i32 {
        let base = REPLAY_TIMER_OFFSET;
        i32::from_le_bytes(self.bytes[base..base + 4].try_into().unwrap())
    }

    /// Read the car's Q16.16 world position (`physics + 0xc8..+0xd4`) from the physics block.
    #[must_use]
    pub fn physics_position_q16(&self) -> [i32; 3] {
        let p = self.physics_bytes();
        [
            i32::from_le_bytes(p[0xc8..0xcc].try_into().unwrap()),
            i32::from_le_bytes(p[0xcc..0xd0].try_into().unwrap()),
            i32::from_le_bytes(p[0xd0..0xd4].try_into().unwrap()),
        ]
    }

    /// Read the car's Q16.16 world velocity (`physics + 0xd4..+0xe0`) from the physics block.
    #[must_use]
    pub fn physics_velocity_q16(&self) -> [i32; 3] {
        let p = self.physics_bytes();
        [
            i32::from_le_bytes(p[0xd4..0xd8].try_into().unwrap()),
            i32::from_le_bytes(p[0xd8..0xdc].try_into().unwrap()),
            i32::from_le_bytes(p[0xdc..0xe0].try_into().unwrap()),
        ]
    }

    /// Read the car's 3x3 Q16.16 orientation basis rows (`physics + 0x258..+0x27c`).
    #[must_use]
    pub fn physics_basis_rows_q16(&self) -> [[i32; 3]; 3] {
        let p = self.physics_bytes();
        let mut rows = [[0i32; 3]; 3];
        for (r, row) in rows.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                let off = 0x258 + (r * 3 + c) * 4;
                *cell = i32::from_le_bytes(p[off..off + 4].try_into().unwrap());
            }
        }
        rows
    }

    /// Restore the control block into `dest`, reproducing `FUN_0046d150`:
    /// preserves the first two dwords (`dest[0..8]`) and copies `+0x08..+0x464`.
    pub fn restore_control_46d150(&self, dest: &mut [u8; REPLAY_CONTROL_BYTES]) {
        let src = self.control_bytes();
        dest[8..].copy_from_slice(&src[8..]);
    }

    /// Restore moving-object slot `index` into `dest`, reproducing `FUN_0046d2b0`:
    /// preserves the first dword (`dest[0..4]`, scene-node pointer) and copies `+0x04..+0xa0`.
    pub fn restore_object_slot_46d2b0(
        &self,
        index: usize,
        dest: &mut [u8; REPLAY_OBJECT_RECORD_BYTES],
    ) -> bool {
        let Some(src) = self.object_record_bytes(index) else {
            return false;
        };
        dest[4..].copy_from_slice(&src[4..]);
        true
    }
}

/// Run-length-encoded input-trace playback stepper (`FUN_0046dc20`).
///
/// Advances through Region-B records in `0..end_cursor`: each tick decodes the
/// current 4-byte `ReplayControlRecord` with the running repeat counter, resets
/// the repeat counter and increments `cursor` when `repeat_complete` is reached,
/// and stops once `cursor >= end_cursor`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayPlaybackStepper {
    pub cursor: usize,
    pub end_cursor: u16,
    pub repeat_counter: u8,
}

impl ReplayPlaybackStepper {
    #[must_use]
    pub const fn new(end_cursor: u16) -> Self {
        Self {
            cursor: 0,
            end_cursor,
            repeat_counter: 0,
        }
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.cursor >= self.end_cursor as usize
    }

    /// Step one physics tick of replay input playback (`FUN_0046dc20`).
    pub fn step_tick_46dc20(
        &mut self,
        records: &[ReplayControlRecord],
    ) -> Option<ReplayDecodedControl> {
        if self.cursor >= usize::from(self.end_cursor) {
            return None;
        }
        let record = *records.get(self.cursor)?;
        let decoded = decode_replay_control(record, self.repeat_counter);
        self.repeat_counter = decoded.next_repeat;
        if decoded.repeat_complete {
            self.cursor += 1;
        }
        Some(decoded)
    }
}

/// The `0x40`-byte header of a replay object (`.SSR` / `.RSP` file prefix or
/// runtime allocation at `FUN_0046d7b0`..`FUN_0046de30`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayObjectHeader {
    /// `+0x00`: distance-gated pan mode (`FUN_0046d870` `arg2`, `FUN_0046d8f0`
    /// `0x23_0000 <= dist <= 0x32_0000` gate).
    pub distance_gated_mode_00: u32,
    /// `+0x04`: playback active flag (`0` = idle, `1` = playing).
    pub is_playing_04: u32,
    /// `+0x08`: playback frame active flag.
    pub playback_frame_active_08: u32,
    /// `+0x0c`: recording active flag.
    pub is_recording_0c: u32,
    /// `+0x10`: recording frame active flag.
    pub recording_frame_active_10: u32,
    /// `+0x14`: playback frame transition pulse.
    pub playback_frame_transition_14: u32,
    /// `+0x18`: recording frame transition pulse.
    pub recording_frame_transition_18: u32,
    /// `+0x1c`: target car slot index (`0..8`).
    pub car_idx_1c: u8,
    /// `+0x1d`: running RLE repeat counter (`0..64`).
    pub rle_repeat_counter_1d: u8,
    /// `+0x1e..+0x20`: alignment bytes.
    pub pad_1e: [u8; 2],
    /// `+0x20`: serialized Region-A pointer (relocated on load by `FUN_0046de30`
    /// to offset `0x40`).
    pub region_a_ptr_20: u32,
    /// `+0x24`: current playback Region-A pointer.
    pub playback_region_a_cur_24: u32,
    /// `+0x28`: current recording Region-A pointer.
    pub recording_region_a_cur_28: u32,
    /// `+0x2c`: serialized Region-B pointer (relocated on load by `FUN_0046de30`
    /// to offset `0x40 + n_frames * 0x151ec`).
    pub region_b_ptr_2c: u32,
    /// `+0x30`: frame capacity (`1` in all `.SSR` / `.RSP` files).
    pub n_frames_30: i16,
    /// `+0x32`: tick capacity per frame (`0x1d4c` = 7500 at 25 Hz, `0x3a98` = 15000 at 50 Hz).
    pub n_ticks_32: i16,
    /// `+0x34`: number of recorded frames (`0..=n_frames_30`; `FUN_0046db30`
    /// requires `start_frame < recorded_frames_34`).
    pub recorded_frames_34: i16,
    /// `+0x36`: alignment word.
    pub pad_36: u16,
    /// `+0x38`: serialized Region-C pointer (relocated on load by `FUN_0046de30`
    /// to offset `0x40 + n_frames * (0x151ec + n_ticks * 4)`).
    pub region_c_ptr_38: u32,
    /// `+0x3c`: current playback frame index.
    pub playback_frame_idx_3c: i16,
    /// `+0x3e`: current playback Region-B record cursor.
    pub playback_record_cursor_3e: i16,
}

impl ReplayObjectHeader {
    /// Parse the `0x40`-byte header from little-endian bytes (`FUN_0046ddc0`).
    #[must_use]
    pub fn from_bytes(bytes: &[u8; REPLAY_OBJECT_HEADER_BYTES]) -> Self {
        let u32_at = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
        let u16_at = |off: usize| u16::from_le_bytes(bytes[off..off + 2].try_into().unwrap());
        let i16_at = |off: usize| i16::from_le_bytes(bytes[off..off + 2].try_into().unwrap());
        Self {
            distance_gated_mode_00: u32_at(0x00),
            is_playing_04: u32_at(0x04),
            playback_frame_active_08: u32_at(0x08),
            is_recording_0c: u32_at(0x0c),
            recording_frame_active_10: u32_at(0x10),
            playback_frame_transition_14: u32_at(0x14),
            recording_frame_transition_18: u32_at(0x18),
            car_idx_1c: bytes[0x1c],
            rle_repeat_counter_1d: bytes[0x1d],
            pad_1e: [bytes[0x1e], bytes[0x1f]],
            region_a_ptr_20: u32_at(0x20),
            playback_region_a_cur_24: u32_at(0x24),
            recording_region_a_cur_28: u32_at(0x28),
            region_b_ptr_2c: u32_at(0x2c),
            n_frames_30: i16_at(0x30),
            n_ticks_32: i16_at(0x32),
            recorded_frames_34: i16_at(0x34),
            pad_36: u16_at(0x36),
            region_c_ptr_38: u32_at(0x38),
            playback_frame_idx_3c: i16_at(0x3c),
            playback_record_cursor_3e: i16_at(0x3e),
        }
    }

    /// Serialize the `0x40`-byte header to little-endian bytes (`FUN_0046de00`).
    #[must_use]
    pub fn to_bytes(&self) -> [u8; REPLAY_OBJECT_HEADER_BYTES] {
        let mut out = [0u8; REPLAY_OBJECT_HEADER_BYTES];
        out[0x00..0x04].copy_from_slice(&self.distance_gated_mode_00.to_le_bytes());
        out[0x04..0x08].copy_from_slice(&self.is_playing_04.to_le_bytes());
        out[0x08..0x0c].copy_from_slice(&self.playback_frame_active_08.to_le_bytes());
        out[0x0c..0x10].copy_from_slice(&self.is_recording_0c.to_le_bytes());
        out[0x10..0x14].copy_from_slice(&self.recording_frame_active_10.to_le_bytes());
        out[0x14..0x18].copy_from_slice(&self.playback_frame_transition_14.to_le_bytes());
        out[0x18..0x1c].copy_from_slice(&self.recording_frame_transition_18.to_le_bytes());
        out[0x1c] = self.car_idx_1c;
        out[0x1d] = self.rle_repeat_counter_1d;
        out[0x1e..0x20].copy_from_slice(&self.pad_1e);
        out[0x20..0x24].copy_from_slice(&self.region_a_ptr_20.to_le_bytes());
        out[0x24..0x28].copy_from_slice(&self.playback_region_a_cur_24.to_le_bytes());
        out[0x28..0x2c].copy_from_slice(&self.recording_region_a_cur_28.to_le_bytes());
        out[0x2c..0x30].copy_from_slice(&self.region_b_ptr_2c.to_le_bytes());
        out[0x30..0x32].copy_from_slice(&self.n_frames_30.to_le_bytes());
        out[0x32..0x34].copy_from_slice(&self.n_ticks_32.to_le_bytes());
        out[0x34..0x36].copy_from_slice(&self.recorded_frames_34.to_le_bytes());
        out[0x36..0x38].copy_from_slice(&self.pad_36.to_le_bytes());
        out[0x38..0x3c].copy_from_slice(&self.region_c_ptr_38.to_le_bytes());
        out[0x3c..0x3e].copy_from_slice(&self.playback_frame_idx_3c.to_le_bytes());
        out[0x3e..0x40].copy_from_slice(&self.playback_record_cursor_3e.to_le_bytes());
        out
    }

    /// Relocated byte offsets `(region_a_offset, region_b_offset, region_c_offset)`
    /// within the replay object buffer, reproducing `FUN_0046de30`.
    #[must_use]
    pub fn relocated_offsets_46de30(&self) -> Option<(usize, usize, usize)> {
        let n_frames = usize::try_from(self.n_frames_30).ok()?;
        let n_ticks = usize::try_from(self.n_ticks_32).ok()?;
        if n_frames == 0 || n_ticks == 0 {
            return None;
        }
        let region_a = REPLAY_OBJECT_HEADER_BYTES;
        let region_b = region_a.checked_add(n_frames.checked_mul(REPLAY_INITIAL_STATE_BYTES)?)?;
        let region_b_bytes = n_frames.checked_mul(n_ticks.checked_mul(REPLAY_TICK_RECORD_BYTES)?)?;
        let region_c = region_b.checked_add(region_b_bytes)?;
        Some((region_a, region_b, region_c))
    }
}

/// Bounds-checked view over a loaded `.SSR` or `.RSP` replay file (`FUN_0046ddc0`
/// + `FUN_0046de30`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayFileView<'a> {
    bytes: &'a [u8],
    header: ReplayObjectHeader,
    region_a_offset: usize,
    region_b_offset: usize,
    region_c_offset: usize,
}

impl<'a> ReplayFileView<'a> {
    /// Validate and wrap a loaded replay buffer (`FUN_0046ddc0` size check +
    /// `FUN_0046de30` pointer relocation).
    #[must_use]
    pub fn from_bytes(bytes: &'a [u8]) -> Option<Self> {
        let header_bytes: &[u8; REPLAY_OBJECT_HEADER_BYTES] =
            bytes.get(..REPLAY_OBJECT_HEADER_BYTES)?.try_into().ok()?;
        let header = ReplayObjectHeader::from_bytes(header_bytes);
        let n_frames = u32::try_from(header.n_frames_30).ok()?;
        let n_ticks = u32::try_from(header.n_ticks_32).ok()?;
        if n_frames == 0
            || n_ticks == 0
            || header.recorded_frames_34 < 0
            || header.recorded_frames_34 > header.n_frames_30
        {
            return None;
        }
        if !replay_file_size_valid(bytes.len(), n_ticks, n_frames) {
            return None;
        }
        let (region_a_offset, region_b_offset, region_c_offset) =
            header.relocated_offsets_46de30()?;
        Some(Self {
            bytes,
            header,
            region_a_offset,
            region_b_offset,
            region_c_offset,
        })
    }

    #[must_use]
    pub const fn header(&self) -> &ReplayObjectHeader {
        &self.header
    }

    #[must_use]
    pub const fn n_frames(&self) -> usize {
        self.header.n_frames_30 as usize
    }

    #[must_use]
    pub const fn n_ticks(&self) -> usize {
        self.header.n_ticks_32 as usize
    }

    #[must_use]
    pub const fn recorded_frames(&self) -> usize {
        self.header.recorded_frames_34 as usize
    }

    /// Return the Region-A initial-state seed block for `frame_idx` (`0..recorded_frames`).
    #[must_use]
    pub fn region_a(&self, frame_idx: usize) -> Option<ReplayRegionAView<'a>> {
        if frame_idx >= self.recorded_frames() {
            return None;
        }
        let start = self
            .region_a_offset
            .checked_add(frame_idx.checked_mul(REPLAY_INITIAL_STATE_BYTES)?)?;
        let end = start.checked_add(REPLAY_INITIAL_STATE_BYTES)?;
        let slice: &'a [u8; REPLAY_INITIAL_STATE_BYTES] =
            self.bytes.get(start..end)?.try_into().ok()?;
        Some(ReplayRegionAView::new(slice))
    }

    /// Return the Region-C end-cursor `u16` (`[region_c + frame_idx * 2]`) for `frame_idx`.
    #[must_use]
    pub fn region_c_end_cursor(&self, frame_idx: usize) -> Option<u16> {
        if frame_idx >= self.recorded_frames() {
            return None;
        }
        let start = self.region_c_offset.checked_add(frame_idx.checked_mul(2)?)?;
        let raw: [u8; 2] = self.bytes.get(start..start + 2)?.try_into().ok()?;
        let end_cursor = u16::from_le_bytes(raw);
        if usize::from(end_cursor) > self.n_ticks() {
            return None;
        }
        Some(end_cursor)
    }

    /// Read one 4-byte Region-B record `(frame_idx, record_idx)`.
    #[must_use]
    pub fn region_b_record(
        &self,
        frame_idx: usize,
        record_idx: usize,
    ) -> Option<ReplayControlRecord> {
        if frame_idx >= self.recorded_frames() || record_idx >= self.n_ticks() {
            return None;
        }
        let frame_base = self
            .region_b_offset
            .checked_add(frame_idx.checked_mul(self.n_ticks() * REPLAY_TICK_RECORD_BYTES)?)?;
        let start = frame_base.checked_add(record_idx.checked_mul(REPLAY_TICK_RECORD_BYTES)?)?;
        let raw: [u8; 4] = self.bytes.get(start..start + 4)?.try_into().ok()?;
        Some(ReplayControlRecord(raw))
    }

    /// Collect the active Region-B records `0..end_cursor` for `frame_idx`, together
    /// with a [`ReplayPlaybackStepper`] ready to step `FUN_0046dc20`.
    #[must_use]
    pub fn frame_playback(
        &self,
        frame_idx: usize,
    ) -> Option<(Vec<ReplayControlRecord>, ReplayPlaybackStepper)> {
        let end_cursor = self.region_c_end_cursor(frame_idx)?;
        let count = usize::from(end_cursor);
        let mut records = Vec::with_capacity(count);
        for idx in 0..count {
            records.push(self.region_b_record(frame_idx, idx)?);
        }
        Some((records, ReplayPlaybackStepper::new(end_cursor)))
    }
}

/// Subdirectory under `rallschl\pans\` selected by `(level, task)` in `FUN_0046de70`.
///
/// Tasks `0` and `1` in every level take place on the Skid Pan (`"skid"`).
/// Tasks `2`, `3`, and `4` select `"easy"` (`level == 0`), `"medium"` (`level == 1`),
/// or `"hard"` (`level == 2`).
#[must_use]
pub const fn school_rsp_subdir_46de70(level: u8, task: u8) -> Option<&'static str> {
    match task {
        0 | 1 => Some("skid"),
        2..=4 => match level {
            0 => Some("easy"),
            1 => Some("medium"),
            2 => Some("hard"),
            _ => None,
        },
        _ => None,
    }
}

/// Relative path under the game root for the Rally School intro `.rsp` pan
/// (`FUN_0046de70` -> `DAT_005ef444`), loaded only when `task < 2`.
#[must_use]
pub fn school_intro_rsp_path_46de70(level: u8, task: u8) -> Option<String> {
    if task >= 2 {
        return None;
    }
    let subdir = school_rsp_subdir_46de70(level, task)?;
    Some(format!("rallschl\\pans\\{subdir}\\intro.rsp"))
}

/// Relative path under the game root for the Rally School exit `.rsp` demonstration
/// (`FUN_0046de70` -> `DAT_005ef440`), loaded for every `(level, task)`.
#[must_use]
pub fn school_exit_rsp_path_46de70(level: u8, task: u8) -> Option<String> {
    let subdir = school_rsp_subdir_46de70(level, task)?;
    Some(format!("rallschl\\pans\\{subdir}\\exit.rsp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_counts_and_duration_match_recovery() {
        assert_eq!(replay_ticks(false), 0x1d4c);
        assert_eq!(replay_ticks(true), 0x3a98);
        // 7500 ticks / 25 Hz = 300 s; 15000 / 50 Hz = 300 s.
        assert_eq!(REPLAY_TICKS_25HZ / 25, REPLAY_MAX_DURATION_SECONDS);
        assert_eq!(REPLAY_TICKS_50HZ / 50, REPLAY_MAX_DURATION_SECONDS);
        // Region A is 0x547b dwords = 0x151ec bytes.
        assert_eq!(REPLAY_INITIAL_STATE_DWORDS * 4, REPLAY_INITIAL_STATE_BYTES);
    }

    #[test]
    fn object_size_and_region_layout_match_the_loader_check() {
        let size = replay_object_size(REPLAY_TICKS_25HZ, 1);
        // Header + Region A + 7500 control records + end-cursor u16.
        assert_eq!(
            size,
            REPLAY_OBJECT_HEADER_BYTES
                + REPLAY_INITIAL_STATE_BYTES
                + 7500 * REPLAY_TICK_RECORD_BYTES
                + REPLAY_RECORD_END_CURSOR_BYTES
        );
        assert_eq!(REPLAY_FRAME_FIXED_BYTES, 0x151ee);
        // The loader validates the loaded size against the same formula.
        assert!(replay_file_size_valid(size, REPLAY_TICKS_25HZ, 1));
        assert!(!replay_file_size_valid(size + 1, REPLAY_TICKS_25HZ, 1));
        // Region B starts after the header and the single Region-A seed block.
        assert_eq!(
            replay_region_b_offset(1),
            REPLAY_OBJECT_HEADER_BYTES + REPLAY_INITIAL_STATE_BYTES
        );
    }

    #[test]
    fn single_frame_record_end_cursor_is_distinct_and_bounds_checked() {
        let n_ticks = 3;
        let size = replay_object_size(n_ticks, 1);
        let offset = replay_single_frame_record_end_cursor_offset(n_ticks).unwrap();
        assert_eq!(offset + REPLAY_RECORD_END_CURSOR_BYTES, size);

        let mut replay = vec![0; size];
        replay[offset..offset + 2].copy_from_slice(&0x1234_u16.to_le_bytes());
        assert_eq!(
            replay_single_frame_record_end_cursor(&replay, n_ticks),
            Some(0x1234)
        );
        assert_eq!(
            replay_single_frame_record_end_cursor(&replay[..size - 1], n_ticks),
            None
        );
        replay.push(0);
        assert_eq!(
            replay_single_frame_record_end_cursor(&replay, n_ticks),
            None
        );
    }

    #[test]
    fn record_end_cursor_has_consumer_specific_inclusive_and_exclusive_ranges() {
        assert_eq!(
            replay_record_aggregate_range(2, 4)
                .unwrap()
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(
            replay_record_playback_range(2, 4)
                .unwrap()
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            replay_record_aggregate_range(0, 4)
                .unwrap()
                .collect::<Vec<_>>(),
            [0]
        );
        assert!(replay_record_playback_range(0, 4).unwrap().is_empty());
        assert!(replay_record_aggregate_range(4, 4).is_none());
        assert!(replay_record_playback_range(4, 4).is_none());
    }

    #[test]
    fn region_a_sections_and_object_bounds_match_the_copy_routines() {
        assert_eq!(REPLAY_CAR_TYPE_BYTES, REPLAY_CONTROL_OFFSET);
        assert_eq!(REPLAY_CONTROL_OFFSET + REPLAY_CONTROL_BYTES, 0x1134);
        assert_eq!(
            REPLAY_PHYSICS_OFFSET + REPLAY_PHYSICS_BYTES,
            REPLAY_OBJECTS_OFFSET
        );
        assert_eq!(replay_object_record_offset(0), Some(REPLAY_OBJECTS_OFFSET));
        assert_eq!(
            replay_object_record_offset(REPLAY_MAX_OBJECTS - 1),
            Some(REPLAY_TRAILING_STATE_OFFSET - REPLAY_OBJECT_RECORD_BYTES)
        );
        assert_eq!(replay_object_record_offset(REPLAY_MAX_OBJECTS), None);
        assert_eq!(REPLAY_TIMER_OFFSET + 4, REPLAY_INITIAL_STATE_BYTES);
    }

    #[test]
    fn actuator_quantize_and_delta_round_trip() {
        // >>10 then 6-bit mask: 0x1_0000 >> 10 = 0x40, & 0x3f = 0.
        assert_eq!(quantize_actuator(0x1_0000), 0);
        // 0x8c00 >> 10 = 0x23, & 0x3f = 0x23.
        assert_eq!(quantize_actuator(0x8c00), 0x23);
        assert_eq!(quantize_actuator(0), 0);
        // XOR-delta is self-inverse: decode(prev, encode(prev, cur)) == cur.
        let prev = 0x2a;
        let cur = 0x15;
        let encoded = delta_encode(prev, cur);
        assert_eq!(delta_decode(prev, encoded), cur);
        // A field equal to the previous encodes as zero (the delta's purpose).
        assert_eq!(delta_encode(0x3f, 0x3f), 0);
    }

    #[test]
    fn analog_control_record_is_byte_exact_and_round_trips() {
        let input = ReplayActuators {
            steer: 0x8c00,
            steer_alternate: 0,
            throttle: 0x4c00,
            brake: 0x2c00,
            handbrake: true,
            flag_310: true,
            gear: 2,
            analog_steer: true,
            analog_throttle: true,
            analog_brake: true,
        };
        let record =
            encode_replay_control(input, ReplayControlRecord([0x55, 0x02, 0x10, 0x20]), true);
        assert_eq!(record.0, [0x4f, 0x2f, 0xd1, 0xe3]);
        let decoded = decode_replay_control(record, 0x10);
        assert_eq!(decoded.actuators, input);
        assert!(decoded.control_410);
        assert!(decoded.control_414);
        assert!(decoded.control_418);
        assert!(decoded.repeat_complete);
        assert_eq!(decoded.next_repeat, 0);
    }

    #[test]
    fn digital_controls_decode_to_minus_66_and_repeat_uses_history() {
        let input = ReplayActuators {
            steer: 0,
            steer_alternate: 1,
            throttle: 1,
            brake: 1,
            handbrake: true,
            flag_310: true,
            gear: -1,
            analog_steer: true,
            analog_throttle: true,
            analog_brake: true,
        };
        let record = encode_replay_control(input, ReplayControlRecord::default(), false);
        assert_eq!(record.0, [0x04, 0x04, 0xc1, 0x01]);
        let decoded = decode_replay_control(record, 0);
        assert_eq!(decoded.actuators.steer, 0);
        assert_eq!(decoded.actuators.steer_alternate, -66);
        assert_eq!(decoded.actuators.throttle, -66);
        assert_eq!(decoded.actuators.brake, -66);
        assert_eq!(decoded.actuators.gear, -1);
        assert!(!decoded.control_410);
        assert!(!decoded.control_414);
        assert!(!decoded.control_418);
        assert!(decoded.repeat_complete);
        let two_ticks = ReplayControlRecord([0, 0, 2, 0]);
        let first = decode_replay_control(two_ticks, 0);
        assert!(!first.repeat_complete);
        assert_eq!(first.next_repeat, 1);
        let second = decode_replay_control(two_ticks, first.next_repeat);
        assert!(second.repeat_complete);
        assert_eq!(second.next_repeat, 0);
    }

    #[test]
    fn network_replay_suppresses_only_duplicate_records() {
        let previous = ReplayControlRecord([0x4f, 0x2f, 0xd1, 0xe3]);
        assert!(!network_replay_record_accepted(previous, previous));
        assert!(!network_replay_record_accepted(
            previous,
            ReplayControlRecord([0x4f, 0x2f, 0xd2, 0xe3])
        ));
        assert!(network_replay_record_accepted(
            previous,
            ReplayControlRecord([0x4f, 0x2f, 0xff, 0xe3])
        ));
        assert!(network_replay_record_accepted(
            previous,
            ReplayControlRecord([0x50, 0x2f, 0xd1, 0xe3])
        ));
        assert!(network_replay_record_accepted(
            previous,
            ReplayControlRecord([0x4f, 0x2f, 0xd1, 0xe2])
        ));
    }

    #[test]
    fn cmpr_header_parse_rejects_bad_magic_and_short_input() {
        let mut header = Vec::from(CMPR_MAGIC);
        header.extend_from_slice(&0x1234u32.to_le_bytes());
        assert_eq!(parse_cmpr_header(&header), Some(0x1234));
        // Wrong magic.
        let bad = *b"ZZZZ\x00\x00\x00\x00";
        assert_eq!(parse_cmpr_header(&bad), None);
        // Too short.
        assert_eq!(parse_cmpr_header(b"CMP"), None);
    }

    #[test]
    fn snapshot_indexing_and_capacity() {
        assert_eq!(SNAPSHOT_BUFFER_ENTRIES, 0x34bc);
        assert_eq!(snapshot_record_index(0, 0), Some(0));
        assert_eq!(snapshot_record_index(1, 5), Some(0x8ca + 5));
        assert_eq!(snapshot_record_index(5, 0x8c9), Some(5 * 0x8ca + 0x8c9));
        // Out of range.
        assert_eq!(snapshot_record_index(6, 0), None);
        assert_eq!(snapshot_record_index(0, 0x8ca), None);
    }

    #[test]
    fn orientation_compression_scales_and_clamps() {
        // 0.5 (0x8000) -> (0x8000*0x8000)>>16 = 0x4000.
        assert_eq!(compress_orientation(Fixed16::from_raw(0x8000)), 0x4000);
        // 1.0 would scale to 0x8000 but clamps to 0x7ff9 (fits i16).
        assert_eq!(compress_orientation(Fixed16::ONE), 0x7ff9);
        // -1.0 clamps to -0x7ff9.
        assert_eq!(compress_orientation(-Fixed16::ONE), -0x7ff9);
        // Zero stays zero.
        assert_eq!(compress_orientation(Fixed16::ZERO), 0);
    }

    #[test]
    fn snapshot_interp_factor_steps_in_quarters() {
        assert_eq!(snapshot_interp_factor(0), Fixed16::ZERO);
        assert_eq!(snapshot_interp_factor(1), Fixed16::from_raw(0x4000)); // 0.25
        assert_eq!(snapshot_interp_factor(2), Fixed16::from_raw(0x8000)); // 0.50
        assert_eq!(snapshot_interp_factor(3), Fixed16::from_raw(0xc000)); // 0.75
        // Subframe wraps modulo 4.
        assert_eq!(snapshot_interp_factor(4), Fixed16::ZERO);
    }

    #[test]
    fn snapshot_keyframe_packs_24_bytes_and_reconstructs_cross_row() {
        let tf = [
            0x1_0000, 0, 0, 0, // Row 0 = +X
            0, 0x1_0000, 0, 0, // Row 1 = +Y
            0, 0, 0x1_0000, 0, // Row 2 = +Z
            0x10_0000, -0x2_0000, 0x45_0000, 0x1_0000,
        ];
        let kf = SnapshotKeyframe::from_transform_46c7d0(&tf);
        assert_eq!(kf.pos, [0x10_0000, -0x2_0000, 0x45_0000]);
        assert_eq!(kf.row0_compressed, [0x7ff9, 0, 0]);
        assert_eq!(kf.row1_compressed, [0, 0x7ff9, 0]);

        let bytes = kf.to_bytes();
        assert_eq!(bytes.len(), SNAPSHOT_RECORD_BYTES);
        assert_eq!(SnapshotKeyframe::from_bytes(&bytes), Some(kf));
        assert!(SnapshotKeyframe::from_bytes(&bytes[..23]).is_none());

        let unpacked = kf.to_transform_46c910();
        // 0x7ff9 << 1 = 0xfff2
        assert_eq!(unpacked[0], 0xfff2);
        assert_eq!(unpacked[5], 0xfff2);
        // Row 2 = cross(row0, row1) = (0, 0, (0xfff2 * 0xfff2) >> 16) = (0, 0, 0xffe4)
        assert_eq!(unpacked[8], 0);
        assert_eq!(unpacked[9], 0);
        assert_eq!(unpacked[10], 0xffe4);
        assert_eq!(&unpacked[12..16], &[0x10_0000, -0x2_0000, 0x45_0000, 0x1_0000]);
    }

    #[test]
    fn snapshot_car_lane_captures_every_fourth_tick_and_interpolates_or_teleports() {
        let mut lane = SnapshotCarLane::new();
        let tf0 = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000,
        ];
        let tf1 = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0x4_0000, 0, 0, 0x1_0000,
        ];
        let tf_teleport = [
            0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0, 0, 0, 0x1_0000, 0, 0x20_0000, 0, 0, 0x1_0000,
        ];

        assert!(lane.capture_tick_46c7d0(0, &tf0));
        assert!(!lane.capture_tick_46c7d0(1, &tf0));
        assert!(!lane.capture_tick_46c7d0(2, &tf0));
        assert!(!lane.capture_tick_46c7d0(3, &tf0));
        assert!(lane.capture_tick_46c7d0(4, &tf1));
        assert!(lane.capture_tick_46c7d0(8, &tf_teleport));
        assert_eq!(lane.keyframes().len(), 3);

        // Subframe 2 between keyframe 0 (x=0) and keyframe 1 (x=4.0) interpolates to x=2.0
        let mid = lane.evaluate_playback_46ca70(1, 2).unwrap();
        assert_eq!(mid[12], 0x2_0000);

        // Subframe 2 between keyframe 1 (x=4.0) and keyframe 2 (x=32.0, delta=28.0 > 20.0) teleports to x=32.0
        let tele = lane.evaluate_playback_46ca70(2, 2).unwrap();
        assert_eq!(tele[12], 0x20_0000);
    }

    #[test]
    fn region_a_view_slices_and_partial_preserve_restores_match_binary() {
        let mut seed = Box::new([0u8; REPLAY_INITIAL_STATE_BYTES]);
        // Set control bytes at +0xcd0..+0x1134
        seed[REPLAY_CONTROL_OFFSET..REPLAY_CONTROL_OFFSET + 8]
            .copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        seed[REPLAY_CONTROL_OFFSET + 8..REPLAY_CONTROL_OFFSET + 12]
            .copy_from_slice(&0xdead_beefu32.to_le_bytes());

        // Set physics position at +0x11f4 + 0xc8
        let pos_off = REPLAY_PHYSICS_OFFSET + 0xc8;
        seed[pos_off..pos_off + 4].copy_from_slice(&0x11_0000i32.to_le_bytes());
        seed[pos_off + 4..pos_off + 8].copy_from_slice(&0x22_0000i32.to_le_bytes());
        seed[pos_off + 8..pos_off + 12].copy_from_slice(&0x33_0000i32.to_le_bytes());

        // Set object 0 at +0x195c
        seed[REPLAY_OBJECTS_OFFSET..REPLAY_OBJECTS_OFFSET + 4]
            .copy_from_slice(&0xaaaa_bbbbu32.to_le_bytes());
        seed[REPLAY_OBJECTS_OFFSET + 4..REPLAY_OBJECTS_OFFSET + 8]
            .copy_from_slice(&0x1234_5678u32.to_le_bytes());

        // Set trailing state & timer
        seed[REPLAY_TRAILING_STATE_OFFSET..REPLAY_TRAILING_STATE_OFFSET + 4]
            .copy_from_slice(&7i32.to_le_bytes());
        seed[REPLAY_TIMER_OFFSET..REPLAY_TIMER_OFFSET + 4].copy_from_slice(&100i32.to_le_bytes());

        let view = ReplayRegionAView::new(&seed);
        assert_eq!(
            view.physics_position_q16(),
            [0x11_0000, 0x22_0000, 0x33_0000]
        );
        assert_eq!(view.trailing_state_dwords()[0], 7);
        assert_eq!(view.timer_centiseconds(), 100);

        // Control restore preserves first 8 bytes (2 dwords)
        let mut ctl_dest = [0x99u8; REPLAY_CONTROL_BYTES];
        view.restore_control_46d150(&mut ctl_dest);
        assert_eq!(&ctl_dest[0..8], &[0x99; 8]);
        assert_eq!(
            u32::from_le_bytes(ctl_dest[8..12].try_into().unwrap()),
            0xdead_beef
        );

        // Object slot restore preserves first 4 bytes (scene node pointer)
        let mut obj_dest = [0x77u8; REPLAY_OBJECT_RECORD_BYTES];
        assert!(view.restore_object_slot_46d2b0(0, &mut obj_dest));
        assert_eq!(&obj_dest[0..4], &[0x77; 4]);
        assert_eq!(
            u32::from_le_bytes(obj_dest[4..8].try_into().unwrap()),
            0x1234_5678
        );
        assert!(!view.restore_object_slot_46d2b0(REPLAY_MAX_OBJECTS, &mut obj_dest));
    }

    #[test]
    fn replay_playback_stepper_expands_rle_repeats_until_end_cursor() {
        let records = [
            ReplayControlRecord([0x04, 0x00, 0x02, 0x00]), // digital throttle, repeat = 2 ticks
            ReplayControlRecord([0x00, 0x04, 0x01, 0x00]), // digital brake, repeat = 1 tick
            ReplayControlRecord([0x04, 0x00, 0x05, 0x00]), // beyond end_cursor=2 (must not be read)
        ];
        let mut stepper = ReplayPlaybackStepper::new(2);
        let t0 = stepper.step_tick_46dc20(&records).unwrap();
        assert_eq!(t0.actuators.throttle, -66);
        assert!(!t0.repeat_complete);
        assert_eq!(stepper.cursor, 0);

        let t1 = stepper.step_tick_46dc20(&records).unwrap();
        assert_eq!(t1.actuators.throttle, -66);
        assert!(t1.repeat_complete);
        assert_eq!(stepper.cursor, 1);

        let t2 = stepper.step_tick_46dc20(&records).unwrap();
        assert_eq!(t2.actuators.brake, -66);
        assert!(t2.repeat_complete);
        assert_eq!(stepper.cursor, 2);
        assert!(stepper.is_finished());
        assert!(stepper.step_tick_46dc20(&records).is_none());
    }

    #[test]
    fn replay_object_header_and_file_view_round_trip_and_validate_bounds() {
        let n_ticks = 4u16;
        let size = replay_object_size(u32::from(n_ticks), 1);
        let mut buf = vec![0u8; size];
        let hdr = ReplayObjectHeader {
            distance_gated_mode_00: 1,
            car_idx_1c: 0,
            n_frames_30: 1,
            n_ticks_32: n_ticks as i16,
            recorded_frames_34: 1,
            ..ReplayObjectHeader::default()
        };
        buf[..REPLAY_OBJECT_HEADER_BYTES].copy_from_slice(&hdr.to_bytes());
        assert_eq!(
            ReplayObjectHeader::from_bytes(buf[..REPLAY_OBJECT_HEADER_BYTES].try_into().unwrap()),
            hdr
        );

        let (ra, rb, rc) = hdr.relocated_offsets_46de30().unwrap();
        assert_eq!(ra, 0x40);
        assert_eq!(rb, 0x40 + REPLAY_INITIAL_STATE_BYTES);
        assert_eq!(rc, rb + 4 * REPLAY_TICK_RECORD_BYTES);

        // Set position in Region A
        let pos_off = ra + REPLAY_PHYSICS_OFFSET + 0xc8;
        buf[pos_off..pos_off + 4].copy_from_slice(&0x5_0000i32.to_le_bytes());
        // Set 2 records in Region B and end_cursor = 2 in Region C
        buf[rb..rb + 4].copy_from_slice(&[0x04, 0x00, 0x02, 0x00]);
        buf[rb + 4..rb + 8].copy_from_slice(&[0x00, 0x04, 0x01, 0x00]);
        buf[rc..rc + 2].copy_from_slice(&2u16.to_le_bytes());

        let view = ReplayFileView::from_bytes(&buf).unwrap();
        assert_eq!(view.n_frames(), 1);
        assert_eq!(view.n_ticks(), 4);
        assert_eq!(view.recorded_frames(), 1);
        assert_eq!(
            view.region_a(0).unwrap().physics_position_q16()[0],
            0x5_0000
        );
        assert_eq!(view.region_c_end_cursor(0), Some(2));
        let (records, mut stepper) = view.frame_playback(0).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            stepper.step_tick_46dc20(&records).unwrap().actuators.throttle,
            -66
        );

        // Reject truncated buffer
        assert!(ReplayFileView::from_bytes(&buf[..size - 1]).is_none());
    }

    #[test]
    fn school_rsp_paths_match_fun_0046de70_jump_table() {
        // Tasks 0 and 1 in all levels map to skid/intro.rsp and skid/exit.rsp
        for level in 0..=2 {
            for task in 0..=1 {
                assert_eq!(
                    school_intro_rsp_path_46de70(level, task).as_deref(),
                    Some("rallschl\\pans\\skid\\intro.rsp")
                );
                assert_eq!(
                    school_exit_rsp_path_46de70(level, task).as_deref(),
                    Some("rallschl\\pans\\skid\\exit.rsp")
                );
            }
        }
        // Tasks 2..=4 have no intro.rsp and map exit.rsp by level (easy, medium, hard)
        for task in 2..=4 {
            assert_eq!(school_intro_rsp_path_46de70(0, task), None);
            assert_eq!(
                school_exit_rsp_path_46de70(0, task).as_deref(),
                Some("rallschl\\pans\\easy\\exit.rsp")
            );
            assert_eq!(
                school_exit_rsp_path_46de70(1, task).as_deref(),
                Some("rallschl\\pans\\medium\\exit.rsp")
            );
            assert_eq!(
                school_exit_rsp_path_46de70(2, task).as_deref(),
                Some("rallschl\\pans\\hard\\exit.rsp")
            );
        }
        assert_eq!(school_exit_rsp_path_46de70(3, 2), None);
        assert_eq!(school_exit_rsp_path_46de70(0, 5), None);
    }
}
