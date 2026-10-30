//! The rally school's reference runs (`<task>.rsf`, `FUN_00473140`): the
//! instructor's time in centiseconds, then 200 line records of 40 bytes, one
//! per school line crossed, in crossing order (the loop tasks keep their
//! four laps one after the other). Loop tasks' files carry 24012 more bytes
//! that nothing reads; they are kept uninterpreted.
//!
//! Pure and bounds-checked; the payload is the stored asset after its
//! unwrap.

use std::fmt;

/// The records a reference run holds.
pub const RSF_RECORDS: usize = 200;
/// One record's size and the block's (time plus records).
pub const RSF_RECORD_SIZE: usize = 0x28;
pub const RSF_BLOCK_SIZE: usize = 4 + RSF_RECORDS * RSF_RECORD_SIZE;

/// One line record (`5f1e2c + 0x28 i`; the runtime capture has the same
/// layout).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineRecord {
    /// The car's position at the crossing.
    pub position: [i32; 3],
    /// The heading `0..0x800`.
    pub heading: i16,
    /// Control `+0x3e0`.
    pub word_3e0: i16,
    /// `|velocity . lateral|`.
    pub slide: u32,
    pub speed: i32,
    /// Control `+0x2ec` (brake) and `+0x2f4` (handbrake).
    pub brake: i32,
    pub handbrake: i32,
    /// The mean distance to the cones (loop tasks).
    pub cones: i32,
    pub gear: u8,
    /// The school line id.
    pub line: u8,
}

impl LineRecord {
    /// Decode one 40-byte record.
    pub fn parse(bytes: &[u8; RSF_RECORD_SIZE]) -> Self {
        let i32_at =
            |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        let i16_at = |o: usize| i16::from_le_bytes([bytes[o], bytes[o + 1]]);
        Self {
            position: [i32_at(0), i32_at(4), i32_at(8)],
            heading: i16_at(0xc),
            word_3e0: i16_at(0xe),
            slide: i32_at(0x10) as u32,
            speed: i32_at(0x14),
            brake: i32_at(0x18),
            handbrake: i32_at(0x1c),
            cones: i32_at(0x20),
            gear: bytes[0x24],
            line: bytes[0x25],
        }
    }
}

/// One parsed reference run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchoolReference {
    pub time_cs: i32,
    pub records: Vec<LineRecord>,
    /// Bytes past the block (the loop tasks' 24012), uninterpreted.
    pub trailing: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RsfError {
    Truncated { len: usize, needed: usize },
}

impl fmt::Display for RsfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { len, needed } => {
                write!(f, "reference run is {len} bytes, needs {needed}")
            }
        }
    }
}

impl std::error::Error for RsfError {}

/// Parse a reference run.
pub fn parse_rsf(bytes: &[u8]) -> Result<SchoolReference, RsfError> {
    if bytes.len() < RSF_BLOCK_SIZE {
        return Err(RsfError::Truncated {
            len: bytes.len(),
            needed: RSF_BLOCK_SIZE,
        });
    }
    let time_cs = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let (records, _) = bytes[4..RSF_BLOCK_SIZE].as_chunks::<RSF_RECORD_SIZE>();
    Ok(SchoolReference {
        time_cs,
        records: records.iter().map(LineRecord::parse).collect(),
        trailing: bytes[RSF_BLOCK_SIZE..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_parses_its_time_and_records() {
        let mut bytes = vec![0u8; RSF_BLOCK_SIZE + 3];
        bytes[0..4].copy_from_slice(&779i32.to_le_bytes());
        let r = 4 + RSF_RECORD_SIZE;
        bytes[r..r + 4].copy_from_slice(&(-5i32).to_le_bytes());
        bytes[r + 0xc..r + 0xe].copy_from_slice(&0x7ffi16.to_le_bytes());
        bytes[r + 0x14..r + 0x18].copy_from_slice(&0x1_8000i32.to_le_bytes());
        bytes[r + 0x24] = 3;
        bytes[r + 0x25] = 17;
        let run = parse_rsf(&bytes).unwrap();
        assert_eq!(run.time_cs, 779);
        assert_eq!(run.records.len(), RSF_RECORDS);
        let one = run.records[1];
        assert_eq!(
            (one.position[0], one.heading, one.speed, one.gear, one.line),
            (-5, 0x7ff, 0x1_8000, 3, 17)
        );
        assert_eq!(run.trailing.len(), 3);
    }

    #[test]
    fn a_short_run_is_rejected() {
        assert_eq!(
            parse_rsf(&[0; 100]),
            Err(RsfError::Truncated {
                len: 100,
                needed: RSF_BLOCK_SIZE
            })
        );
    }
}
