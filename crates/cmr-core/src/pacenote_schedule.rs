//! Reconstruction of the co-driver call *scheduling* and playback eventing (the
//! confidently-recovered G07 pieces): the audio event ring, the incremental
//! per-frame sequencer, the modifier swap, the into-link decision, the submit
//! routing, and the on-screen pacenote-strip arithmetic.
//!
//! The co-driver has **two independent queues**:
//!
//! - the **audio ring** `DAT_005590c8`: 8 entries of one packed note-id `u32`
//!   each (stride 4 bytes), with a tail `DAT_00559108` and head `DAT_0055910c`;
//! - the **visual FIFO** `DAT_004f2120` (depth 2, two interleaved tracks) plus a
//!   display table `DAT_0057c178` (stride `0x14`, `0x12` records per track) that
//!   drives the on-screen pacenote strip.
//!
//! Submit (`FUN_00444380`) optionally swaps the direction modifier (bits 7-8,
//! `1<->2`, when the mirror-stage config bit is set), then either enqueues the
//! whole note id into the audio ring or routes it to the visual FIFO only. The
//! sequencer tick (`FUN_00444450`, run once per frame before Mechanics, gated by
//! `QSWaveMixIsChannelDone` on the single shared speech channel) plays **one**
//! bitfield group per tick - the lowest-set field first, using the shift/mask
//! table [`crate::audio_pacenote::PACENOTE_WORD_FIELDS`] - clears that field and
//! writes the remainder back; the ring head only advances once an entry reaches
//! `0` (the phrase is complete). When the next entry carries a word/corner field
//! (`& 0x78`) the sequencer first plays the "INTO" link
//! ([`crate::audio_pacenote::INTO_LINK_SLOT_OFFSET`]), giving the "corner-A INTO
//! corner-B" cadence.
//!
//! The visual FIFO push places a word at the current depth slot and a second
//! "accumulate" call adds the display-row bump `9*(modifier-1)` and commits the
//! entry; this resolves the previously-ambiguous `9*(n-1)` term as display-list
//! index arithmetic, not an audio ring stride.
//!
//! See evidence `rea.official-eu.sim-pacenote-schedule`.
//!
//! Observations: the ring stride/indices/wrap, the per-field incremental decode
//! and write-back, the modifier swap, the `& 0x78` into-link test, the submit
//! routing predicate, and the `9*(mod-1)` / `0x14`-stride / `0x12`-per-track
//! display arithmetic. Inference: scheduling is **event-driven** (enqueue on
//! trigger-volume crossing, de-duplicated per player by a fire-once bit; drain
//! gated only by channel-free status) with **no stage-clock or distance
//! sampling** - the look-ahead is authored into the track's trigger-volume
//! placement. The display-record layout and over-depth FIFO behavior are
//! recovered. Unknown: the exact semantics of the `DAT_00550c08`/`DAT_00550e90`
//! mode gates.

use crate::audio_pacenote::{PACENOTE_EVENT_RING_SIZE, PACENOTE_WORD_FIELDS};

/// Bit mask of the word/corner field (bits 3-6); a non-zero value in the *next*
/// ring entry triggers the "INTO" link before it (`FUN_00444450`).
pub const INTO_LINK_TRIGGER_MASK: u32 = 0x78;

/// Shift of the direction-modifier field (bits 7-8) swapped by the mirror-stage
/// option.
pub const MODIFIER_SHIFT: u32 = 7;

/// Mask of the direction-modifier field after shifting.
pub const MODIFIER_MASK: u32 = 0x3;

/// Display-record stride in the on-screen pacenote table `DAT_0057c178`.
pub const DISPLAY_RECORD_STRIDE: usize = 0x14;

/// Number of display records per track in `DAT_0057c178`.
pub const DISPLAY_RECORDS_PER_TRACK: usize = 0x12;

/// Swap the direction modifier `1<->2` in a packed note id (`0` and `3` are
/// unchanged), reproducing the mirror-stage remap in `FUN_00444380`:
/// `if swap { m = (id>>7)&3; m = {1->2, 2->1}; id = (id & ~(3<<7)) | (m<<7) }`.
pub fn swap_modifier(id: u32, swap: bool) -> u32 {
    if !swap {
        return id;
    }
    let modifier = (id >> MODIFIER_SHIFT) & MODIFIER_MASK;
    let swapped = match modifier {
        1 => 2,
        2 => 1,
        other => other,
    };
    (id & !(MODIFIER_MASK << MODIFIER_SHIFT)) | (swapped << MODIFIER_SHIFT)
}

/// Whether the audio ring should receive a submitted note: one replay car and
/// either one active player or track 0 (`DAT_00550c08 == 1 &&
/// (DAT_00550e90 == 1 || track == 0)`).
pub fn routes_to_audio(replay_car_count: u32, active_player_count: u32, track: u32) -> bool {
    replay_car_count == 1 && (active_player_count == 1 || track == 0)
}

/// Whether the next ring entry carries a word/corner field, so the sequencer
/// inserts the INTO link ahead of it (`next & 0x78 != 0`).
pub fn needs_into_link(next_entry: u32) -> bool {
    next_entry & INTO_LINK_TRIGGER_MASK != 0
}

/// The on-screen display-row bump a modifier contributes, resolving the
/// recovered `value += 9*(modifier-1)` accumulate (`FUN_00444320`).
pub fn modifier_row_bump(modifier: u32) -> i32 {
    modifier as i32 * 9 - 9
}

/// Index of a display record in `DAT_0057c178` for a note `value` on `track`,
/// reproducing the `FUN_004619e0` guarded index `(value + track*0x12)` (each
/// record is [`DISPLAY_RECORD_STRIDE`] bytes). `None` when `value` is outside the
/// recovered `0..0x12` range.
pub fn display_record_index(value: i32, track: usize) -> Option<usize> {
    if (0..DISPLAY_RECORDS_PER_TRACK as i32).contains(&value) {
        Some(value as usize + track * DISPLAY_RECORDS_PER_TRACK)
    } else {
        None
    }
}

/// One 20-byte display-table record: TTL, three padding bytes, two RGBA
/// colors, x position and signed y position.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CodriverDisplayRecord {
    pub ttl: u8,
    pub padding: [u8; 3],
    pub colors: [[u8; 4]; 2],
    pub x: u16,
    pub y: i16,
    pub tail: [u8; 4],
}

/// Encode the exact `0x14`-byte display-record layout.
#[must_use]
pub fn encode_display_record(record: &CodriverDisplayRecord) -> [u8; DISPLAY_RECORD_STRIDE] {
    let mut bytes = [0; DISPLAY_RECORD_STRIDE];
    bytes[0] = record.ttl;
    bytes[1..4].copy_from_slice(&record.padding);
    for (index, color) in record.colors.into_iter().enumerate() {
        bytes[4 + index * 4..8 + index * 4].copy_from_slice(&color);
    }
    bytes[12..14].copy_from_slice(&record.x.to_le_bytes());
    bytes[14..16].copy_from_slice(&record.y.to_le_bytes());
    bytes[16..20].copy_from_slice(&record.tail);
    bytes
}

/// Decode a display record, rejecting truncated input.
#[must_use]
pub fn decode_display_record(bytes: &[u8]) -> Option<CodriverDisplayRecord> {
    if bytes.len() != DISPLAY_RECORD_STRIDE {
        return None;
    }
    Some(CodriverDisplayRecord {
        ttl: bytes[0],
        padding: bytes[1..4].try_into().ok()?,
        colors: [bytes[4..8].try_into().ok()?, bytes[8..12].try_into().ok()?],
        x: u16::from_le_bytes(bytes[12..14].try_into().ok()?),
        y: i16::from_le_bytes(bytes[14..16].try_into().ok()?),
        tail: bytes[16..20].try_into().ok()?,
    })
}

/// Depth-2 visual FIFO used by the two interleaved co-driver tracks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VisualFifo {
    pub notes: [i32; 2],
    pub depth: u8,
}

impl Default for VisualFifo {
    fn default() -> Self {
        Self {
            notes: [0xff; 2],
            depth: 0,
        }
    }
}

impl VisualFifo {
    /// Evict the current note's display record and apply native depth handling.
    pub fn evict(&mut self, record: &mut CodriverDisplayRecord) {
        if self.depth == 0 {
            return;
        }
        record.ttl = 0;
        if self.depth == 2 {
            self.notes[0] = self.notes[1];
            self.depth = 1;
            self.notes[1] = 0xff;
        } else {
            self.notes[0] = 0xff;
            self.depth -= 1;
        }
    }

    /// Push a note, rotating out the oldest entry when already full.
    pub fn push(&mut self, note: i32, record: &mut CodriverDisplayRecord) {
        if self.depth < 2 {
            self.notes[usize::from(self.depth)] = note;
            self.depth += 1;
            return;
        }
        let next = self.notes[1];
        self.evict(record);
        self.depth = self.depth.wrapping_add(1);
        self.notes[0] = next;
        self.notes[1] = note;
    }

    /// Add a modifier's row bump, preserving the native over-depth counter.
    pub fn accumulate(&mut self, modifier: u32) {
        if self.depth < 2 {
            let index = usize::from(self.depth);
            self.notes[index] = self.notes[index].wrapping_add(modifier_row_bump(modifier));
            self.depth += 1;
        } else {
            self.notes[1] = self.notes[1].wrapping_add(modifier_row_bump(modifier));
        }
    }
}

/// One component the sequencer speaks on a tick: the bank slot offset played and
/// the remaining packed id after that field's bits are cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpokenComponent {
    /// Bank-relative slot offset played (`slot_delta + value`).
    pub bank_offset: i32,
    /// The packed id with the spoken field's bits cleared (written back to the
    /// ring entry; the head advances only when this reaches `0`).
    pub remainder: u32,
}

/// The next component the sequencer plays for a packed ring entry, reproducing
/// the lowest-set-field-first decode in `FUN_00444450`: scan
/// [`PACENOTE_WORD_FIELDS`] in order, play the first non-zero field as slot
/// `slot_delta + value`, and clear that field's bits. `None` when the entry is
/// `0` (the phrase is complete) or carries no recognised field.
pub fn next_component(entry: u32) -> Option<SpokenComponent> {
    for field in PACENOTE_WORD_FIELDS {
        let value = (entry >> field.shift) & field.mask;
        if value != 0 {
            return Some(SpokenComponent {
                bank_offset: field.slot_delta + value as i32,
                remainder: entry & !(field.mask << field.shift),
            });
        }
    }
    None
}

/// The co-driver audio event ring: 8 packed note-id entries drained one
/// component per frame (`DAT_005590c8` + head/tail indices).
///
/// Enqueue reproduces the shipped blind write + tail advance with no full check
/// (`FUN_00444380`): on overflow the oldest unread entry is overwritten, which
/// can collapse the ring to empty - a faithfully-preserved quirk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacenoteRing {
    entries: [u32; PACENOTE_EVENT_RING_SIZE],
    head: usize,
    tail: usize,
}

impl Default for PacenoteRing {
    fn default() -> Self {
        Self::new()
    }
}

impl PacenoteRing {
    /// An empty ring (`FUN_004440f0` zeroes both indices).
    pub const fn new() -> Self {
        PacenoteRing {
            entries: [0; PACENOTE_EVENT_RING_SIZE],
            head: 0,
            tail: 0,
        }
    }

    /// Whether the ring holds no entries (`head == tail`).
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Enqueue a packed note id at the tail and advance it (mod 8), as the
    /// shipped submit does - a blind write with no full check.
    pub fn enqueue(&mut self, id: u32) {
        self.entries[self.tail] = id;
        self.tail = (self.tail + 1) % PACENOTE_EVENT_RING_SIZE;
    }

    /// The current head entry (the packed id being drained), or `None` if empty.
    pub fn head(&self) -> Option<u32> {
        if self.is_empty() {
            None
        } else {
            Some(self.entries[self.head])
        }
    }

    /// The entry after the head (used for the into-link look-ahead), or `None`
    /// when the head is the only / last entry.
    pub fn following(&self) -> Option<u32> {
        let next = (self.head + 1) % PACENOTE_EVENT_RING_SIZE;
        if next == self.tail {
            None
        } else {
            Some(self.entries[next])
        }
    }

    /// Write the drained remainder back into the head entry.
    pub fn set_head(&mut self, remainder: u32) {
        if !self.is_empty() {
            self.entries[self.head] = remainder;
        }
    }

    /// Advance past a completed head entry (mod 8).
    pub fn advance(&mut self) {
        if !self.is_empty() {
            self.head = (self.head + 1) % PACENOTE_EVENT_RING_SIZE;
        }
    }
}

/// The single-player co-driver speech queue: the submit path of
/// `FUN_00444380` that enqueues into the audio ring, and the per-frame
/// sequencer `FUN_00444450`.
///
/// `FUN_00444450` (pseudocode level), outside replays: when the speech channel
/// is busy, or the ring is empty, nothing happens. A zero head entry advances
/// the head; if the new head is queued and carries a corner field (`& 0x78`),
/// the INTO link (`+0x27`) plays and the into flag `DAT_005590f0` is set.
/// Otherwise the lowest set field plays (`slot_delta + value`, in
/// [`PACENOTE_WORD_FIELDS`] order), is cleared and the remainder is written
/// back; the number field is kept in `DAT_00559110`, the direction field
/// clears the into flag. A non-zero entry with no recognised field plays
/// nothing and stays at the head. Each play is followed by restoring the
/// channel volume (`FUN_0047eba0(channel, 0x7d00)`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodriverSequencer {
    ring: PacenoteRing,
    /// `DAT_005590f0`: the last play was the INTO link.
    pub into_link_5590f0: bool,
    /// `DAT_00559110`: the last number/severity value spoken.
    pub last_number_559110: u32,
}

impl CodriverSequencer {
    /// `FUN_00444380`'s audio path (one viewport and one car, or player 0):
    /// swap the direction on a mirrored stage, then enqueue.
    pub fn submit(&mut self, word: u32, mirrored: bool) {
        self.ring.enqueue(swap_modifier(word, mirrored));
    }

    /// One `FUN_00444450` call. Returns the bank-relative slot to play on the
    /// speech channel, after which the caller restores its volume.
    pub fn tick(&mut self, channel_busy: bool) -> Option<i32> {
        if channel_busy {
            return None;
        }
        let entry = self.ring.head()?;
        if entry == 0 {
            self.ring.advance();
            if self
                .ring
                .head()
                .is_some_and(|next| next & INTO_LINK_TRIGGER_MASK != 0)
            {
                self.into_link_5590f0 = true;
                return Some(crate::audio_pacenote::INTO_LINK_SLOT_OFFSET as i32);
            }
            return None;
        }
        let (index, field) = PACENOTE_WORD_FIELDS
            .iter()
            .enumerate()
            .find(|(_, field)| (entry >> field.shift) & field.mask != 0)?;
        let value = (entry >> field.shift) & field.mask;
        match index {
            2 => self.last_number_559110 = value,
            3 => self.into_link_5590f0 = false,
            _ => {}
        }
        self.ring.set_head(entry & !(field.mask << field.shift));
        Some(field.slot_delta + value as i32)
    }
}

/// 18 `64x64` tile source rectangles `(x, y, w, h)` in `images/osdnew/arrws.bmp`
/// initialized by `FUN_00462230` (`0x462403..0x4625a5`, stride `0x14` at
/// `0x57c17c`).
///
/// Indices `0..=8` are RIGHT corners (`dir == 1`, drawn with `flags = 0x08`
/// unflipped); indices `9..=17` are LEFT corners (`dir == 2`, drawn with
/// `flags = 0x01` horizontally flipped by `FUN_004625d0` / `0x49be2f`).
pub const PACENOTE_ARROW_TILE_RECTS: [(i16, i16, i16, i16); DISPLAY_RECORDS_PER_TRACK] = [
    (192, 64, 64, 64), // 0: right 0
    (192, 64, 64, 64), // 1: right 1 (hairpin)
    (128, 64, 64, 64), // 2: right 2
    (64, 64, 64, 64),  // 3: right 3
    (0, 64, 64, 64),   // 4: right 4
    (192, 0, 64, 64),  // 5: right 5
    (128, 0, 64, 64),  // 6: right 6
    (64, 0, 64, 64),   // 7: right 7 (easy/fast)
    (0, 0, 64, 64),    // 8: right 8 (kink/crest)
    (192, 64, 64, 64), // 9: left 0
    (192, 64, 64, 64), // 10: left 1 (hairpin)
    (128, 64, 64, 64), // 11: left 2
    (64, 64, 64, 64),  // 12: left 3
    (0, 64, 64, 64),   // 13: left 4
    (192, 0, 64, 64),  // 14: left 5
    (128, 0, 64, 64),  // 15: left 6
    (64, 0, 64, 64),   // 16: left 7
    (0, 0, 64, 64),    // 17: left 8
];

/// Initial waypoint TTL written by `FUN_004625d0` (`0x4627d2`: `0x0c`).
pub const PACENOTE_ARROW_INITIAL_TTL: i8 = 12;
/// Base X coordinate in 640x480 HUD space (`0x4627dd`: `0x120` = 288).
pub const PACENOTE_ARROW_X: i16 = 0x120;
/// Starting Y coordinate before sliding down (`0x4627e6`: `0xffc0` = -64).
pub const PACENOTE_ARROW_START_Y: i16 = -64;
/// Vertical slide velocity per tick in 640x480 pixels (`0x462821`/`0x462828`).
pub const PACENOTE_ARROW_SLIDE_STEP: i16 = 10;
/// Translucent arrow alpha (`0x4625ec`: `0xaa`).
pub const PACENOTE_ARROW_ALPHA: u8 = 0xaa;

/// One visible pacenote corner arrow sprite in 640x480 HUD space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacenoteArrowSprite {
    /// Tile index `0..=17` into [`PACENOTE_ARROW_TILE_RECTS`].
    pub arrow_id: u8,
    /// Source rectangle `(x, y, w, h)` in `images/osdnew/arrws.bmp`.
    pub source: (i16, i16, i16, i16),
    /// Destination X in 640x480 HUD space (`0x120` = 288).
    pub dst_x: i16,
    /// Destination Y in 640x480 HUD space (`record.y + 32`, `0x46286c..0x462875`).
    pub dst_y: i16,
    /// `true` for left arrows (`arrow_id >= 9`, `flags = 0x01` horizontal flip);
    /// `false` for right arrows (`arrow_id <= 8`, `flags = 0x08` unflipped).
    pub flip_x: bool,
}

/// Live on-screen pacenote corner arrow HUD state (`DAT_004f2120`,
/// `DAT_00559100`, `DAT_0057c178[0x12]`), reconstructing `FUN_004442b0`,
/// `FUN_00444320`, `FUN_004619e0`, `FUN_004625d0`, and the waypoint TTL
/// countdown at `0x462e41..0x462e6c`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacenoteArrowHud {
    slots: [i32; 2],
    depth: usize,
    ttl: [i8; DISPLAY_RECORDS_PER_TRACK],
    y: [i16; DISPLAY_RECORDS_PER_TRACK],
}

impl Default for PacenoteArrowHud {
    fn default() -> Self {
        Self::new()
    }
}

impl PacenoteArrowHud {
    /// Empty arrow queue with all 18 display records inactive.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [0xff; 2],
            depth: 0,
            ttl: [0; DISPLAY_RECORDS_PER_TRACK],
            y: [PACENOTE_ARROW_START_Y; DISPLAY_RECORDS_PER_TRACK],
        }
    }

    /// Clear all active arrows (e.g. on stage start or restart).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Current FIFO depth (`0..=2`).
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Active FIFO slots (`0xff` = empty).
    #[must_use]
    pub const fn slots(&self) -> [i32; 2] {
        self.slots
    }

    /// Submit a packed co-driver call word (`FUN_00444380` / `FUN_00444450`
    /// visual arrow path).
    ///
    /// When `into_link` is `false` and an arrow is already displayed, the older
    /// slot is dismissed first (`0x4445c8..0x4445e3`); when `into_link` is
    /// `true`, up to 2 arrows stack vertically.
    pub fn submit_word(&mut self, word: u32, mirrored: bool, into_link: bool) {
        let word = swap_modifier(word, mirrored);
        let number = ((word >> 3) & 0xf) as i32;
        let dir = (word >> MODIFIER_SHIFT) & MODIFIER_MASK;
        if dir == 0 {
            return;
        }
        if self.depth != 0 && !into_link {
            self.dismiss(self.slots[0]);
        }
        self.push_number(number);
        self.accumulate_direction(dir);
    }

    /// `FUN_004442b0`: place `number` into the current FIFO slot without
    /// incrementing `depth` (unless already full at depth 2, in which case slot
    /// 0 is dismissed and slot 1 is shifted down).
    fn push_number(&mut self, number: i32) {
        if self.depth < 2 {
            self.slots[self.depth] = number;
            return;
        }
        let oldest = self.slots[0];
        let next = self.slots[1];
        self.dismiss(oldest);
        self.depth = self.depth.saturating_add(1).min(2);
        self.slots[0] = next;
        self.slots[1] = number;
    }

    /// `FUN_00444320`: apply `modifier_row_bump(dir)` (`0` for right, `+9` for
    /// left) and commit the slot by incrementing `depth` when `< 2`.
    fn accumulate_direction(&mut self, dir: u32) {
        let bump = modifier_row_bump(dir);
        if self.depth < 2 {
            self.slots[self.depth] = self.slots[self.depth].wrapping_add(bump);
            self.depth += 1;
        } else {
            self.slots[1] = self.slots[1].wrapping_add(bump);
        }
    }

    /// `FUN_004619e0`: dismiss `arrow_id`, zeroing its TTL and shifting slot 1
    /// down to slot 0 when `depth == 2`.
    pub fn dismiss(&mut self, arrow_id: i32) {
        if self.depth == 0 || !(0..DISPLAY_RECORDS_PER_TRACK as i32).contains(&arrow_id) {
            return;
        }
        self.ttl[arrow_id as usize] = 0;
        if self.depth == 2 {
            self.slots[0] = self.slots[1];
            self.slots[1] = 0xff;
            self.depth = 1;
        } else {
            self.slots[0] = 0xff;
            self.depth -= 1;
        }
    }

    /// `0x462e41..0x462e6c`: decrement TTL on every active arrow each time the
    /// car crosses a track waypoint, dismissing any arrow whose TTL reaches 0.
    pub fn on_waypoint_advanced(&mut self, waypoints_crossed: u32) {
        for _ in 0..waypoints_crossed {
            for id in 0..DISPLAY_RECORDS_PER_TRACK {
                if self.ttl[id] > 0 {
                    self.ttl[id] -= 1;
                    if self.ttl[id] <= 0 {
                        self.dismiss(id as i32);
                    }
                }
            }
        }
    }

    /// Advance the vertical slide animation by one simulation tick
    /// (`FUN_004625d0`, `0x46270e..0x462924`) and return the active sprites.
    pub fn step_animation(&mut self) -> Vec<PacenoteArrowSprite> {
        let count = self.visible_slot_count();
        if count == 0 {
            return Vec::new();
        }
        let mut target_y = (count as i16) * 65 - 46;
        let mut out = Vec::with_capacity(count);
        for slot_idx in 0..count {
            let id = self.slots[slot_idx];
            if (0..DISPLAY_RECORDS_PER_TRACK as i32).contains(&id) {
                let idx = id as usize;
                if self.ttl[idx] <= 0 {
                    self.ttl[idx] = PACENOTE_ARROW_INITIAL_TTL;
                    self.y[idx] = PACENOTE_ARROW_START_Y;
                }
                let cur_y = self.y[idx];
                if cur_y > target_y - PACENOTE_ARROW_SLIDE_STEP
                    && cur_y < target_y + PACENOTE_ARROW_SLIDE_STEP
                {
                    self.y[idx] = target_y;
                } else if cur_y < target_y {
                    self.y[idx] = cur_y.saturating_add(PACENOTE_ARROW_SLIDE_STEP);
                } else if cur_y > target_y {
                    self.y[idx] = cur_y.saturating_sub(PACENOTE_ARROW_SLIDE_STEP);
                }
                out.push(Self::make_sprite(idx, self.y[idx]));
            }
            target_y -= 65;
        }
        out
    }

    /// Read-only snapshot of the currently visible sprites without stepping
    /// animation (used for sub-tick interpolated frames).
    #[must_use]
    pub fn current_sprites(&self) -> Vec<PacenoteArrowSprite> {
        let count = self.visible_slot_count();
        let mut out = Vec::with_capacity(count);
        for slot_idx in 0..count {
            let id = self.slots[slot_idx];
            if (0..DISPLAY_RECORDS_PER_TRACK as i32).contains(&id) {
                let idx = id as usize;
                if self.ttl[idx] > 0 {
                    out.push(Self::make_sprite(idx, self.y[idx]));
                }
            }
        }
        out
    }

    fn visible_slot_count(&self) -> usize {
        if self.depth == 0 {
            0
        } else if self.slots[0] == self.slots[1] {
            1
        } else {
            self.depth.min(2)
        }
    }

    fn make_sprite(idx: usize, y: i16) -> PacenoteArrowSprite {
        let source = PACENOTE_ARROW_TILE_RECTS[idx];
        // `0x462869..0x462878`: dst_y = record.y + (src_h / 2) = record.y + 32.
        let dst_y = y.saturating_add(source.3 / 2);
        // `0x462832..0x4628b9`: id <= 8 uses flags 0x08 (right, unflipped),
        // id >= 9 uses flags 0x01 (left, horizontally flipped).
        PacenoteArrowSprite {
            arrow_id: idx as u8,
            source,
            dst_x: PACENOTE_ARROW_X,
            dst_y,
            flip_x: idx >= 9,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_pacenote::INTO_LINK_SLOT_OFFSET;

    #[test]
    fn modifier_swap_only_exchanges_one_and_two() {
        // modifier 1 (RIGHT) <-> 2 (LEFT) in bits 7-8.
        assert_eq!(swap_modifier(1 << 7, true), 2 << 7);
        assert_eq!(swap_modifier(2 << 7, true), 1 << 7);
        // 0 and 3 are unchanged; other bits are preserved.
        assert_eq!(swap_modifier(0x1f, true), 0x1f);
        assert_eq!(swap_modifier(3 << 7, true), 3 << 7);
        // No swap when the option is off.
        assert_eq!(swap_modifier(1 << 7, false), 1 << 7);
    }

    #[test]
    fn audio_routing_honours_the_mode_gate() {
        assert!(routes_to_audio(1, 1, 1));
        assert!(routes_to_audio(1, 2, 0));
        assert!(!routes_to_audio(1, 2, 1));
        assert!(!routes_to_audio(2, 1, 0));
        assert!(!routes_to_audio(0, 1, 0));
    }

    #[test]
    fn into_link_fires_only_for_a_following_word_field() {
        // A following entry with a corner/word field (bits 3-6) links.
        assert!(needs_into_link(0x3 << 3));
        // One with only non-word fields does not.
        assert!(!needs_into_link(0x1 << 7));
        assert!(!needs_into_link(0));
        // The link plays the documented INTO bank slot.
        assert_eq!(INTO_LINK_SLOT_OFFSET, 0x27);
    }

    #[test]
    fn display_arithmetic_matches_recovery() {
        // 9*(modifier-1): modifier 1 -> 0, 2 -> 9, 3 -> 18.
        assert_eq!(modifier_row_bump(1), 0);
        assert_eq!(modifier_row_bump(2), 9);
        assert_eq!(modifier_row_bump(3), 18);
        // Display index: value + track*0x12, guarded to 0..0x12.
        assert_eq!(display_record_index(5, 0), Some(5));
        assert_eq!(display_record_index(5, 1), Some(5 + 0x12));
        assert_eq!(display_record_index(0x12, 0), None);
        assert_eq!(display_record_index(-1, 0), None);
    }

    #[test]
    fn display_record_codec_preserves_every_field_and_rejects_truncation() {
        let record = CodriverDisplayRecord {
            ttl: 12,
            padding: [1, 2, 3],
            colors: [[0xc0, 0x40, 0x40, 0x40], [0x80, 0x40, 0x40, 0x40]],
            x: 0x120,
            y: -2,
            tail: [4, 5, 6, 7],
        };
        let bytes = encode_display_record(&record);
        assert_eq!(bytes[0], 12);
        assert_eq!(&bytes[1..4], &[1, 2, 3]);
        assert_eq!(&bytes[4..8], &[0xc0, 0x40, 0x40, 0x40]);
        assert_eq!(&bytes[8..12], &[0x80, 0x40, 0x40, 0x40]);
        assert_eq!(&bytes[12..14], &0x120_u16.to_le_bytes());
        assert_eq!(&bytes[14..16], &(-2_i16).to_le_bytes());
        assert_eq!(&bytes[16..20], &[4, 5, 6, 7]);
        assert_eq!(decode_display_record(&bytes), Some(record));
        assert_eq!(decode_display_record(&bytes[..19]), None);
    }

    #[test]
    fn visual_fifo_rotates_accumulates_and_preserves_over_depth_counter() {
        let mut record = CodriverDisplayRecord {
            ttl: 12,
            ..CodriverDisplayRecord::default()
        };
        let mut fifo = VisualFifo::default();
        fifo.accumulate(2);
        assert_eq!(fifo.notes, [0x108, 0xff]);
        assert_eq!(fifo.depth, 1);
        fifo.push(4, &mut record);
        assert_eq!(fifo.notes, [0x108, 4]);
        assert_eq!(fifo.depth, 2);
        fifo.accumulate(2);
        assert_eq!(fifo.notes, [0x108, 13]);
        assert_eq!(fifo.depth, 2);
        fifo.push(7, &mut record);
        assert_eq!(fifo.notes, [13, 7]);
        assert_eq!(fifo.depth, 2);
        assert_eq!(record.ttl, 0);
        fifo.depth = 3;
        fifo.notes = [1, 2];
        fifo.push(9, &mut record);
        assert_eq!(fifo.notes, [2, 9]);
        assert_eq!(fifo.depth, 3);
        fifo.accumulate(3);
        assert_eq!(fifo.notes, [2, 27]);
        assert_eq!(fifo.depth, 3);
    }

    #[test]
    fn sequencer_drains_one_field_per_tick_lowest_first() {
        // A compound note: field0 value 1 (CAUTION), word field (bits 3-6) = 3,
        // modifier (bits 7-8) = 2 (LEFT).
        let note = 1 | (3 << 3) | (2 << 7);
        // First tick: lowest field (bits 0-1, value 1) -> slot_delta -1 + 1 = 0.
        let first = next_component(note).expect("field present");
        assert_eq!(first.bank_offset, 0);
        assert_eq!(first.remainder, note & !0x3);
        // Next: the word field (bits 3-6, value 3) -> slot_delta 2 + 3 = 5.
        let second = next_component(first.remainder).expect("word field present");
        assert_eq!(second.bank_offset, 5);
        assert_eq!(second.remainder, first.remainder & !(0xf << 3));
        // Next: the modifier (bits 7-8, value 2) -> slot_delta 0xa + 2 = 0xc (LEFT).
        let third = next_component(second.remainder).expect("modifier present");
        assert_eq!(third.bank_offset, 0xc);
        // The remainder is now zero: the phrase is complete.
        assert_eq!(third.remainder, 0);
        assert_eq!(next_component(third.remainder), None);
    }

    #[test]
    fn ring_enqueues_drains_and_looks_ahead() {
        let mut ring = PacenoteRing::new();
        assert!(ring.is_empty());
        assert_eq!(ring.head(), None);

        let a = 1 | (3 << 3);
        let b = 2 << 7;
        ring.enqueue(a);
        ring.enqueue(b);
        assert_eq!(ring.head(), Some(a));
        // The following entry (b) is visible for the into-link look-ahead.
        assert_eq!(ring.following(), Some(b));

        // Drain entry a one component at a time, writing back the remainder.
        let mut cur = ring.head().unwrap();
        while let Some(component) = next_component(cur) {
            cur = component.remainder;
            ring.set_head(cur);
        }
        assert_eq!(ring.head(), Some(0));
        // Entry complete: advance to b, which is now the only entry.
        ring.advance();
        assert_eq!(ring.head(), Some(b));
        assert_eq!(ring.following(), None);
        ring.advance();
        assert!(ring.is_empty());
    }

    #[test]
    fn ring_overflow_overwrites_blindly() {
        // The shipped ring has no full check: enqueuing a full ring's worth plus
        // one advances the tail into the head (the faithfully-preserved quirk).
        let mut ring = PacenoteRing::new();
        for i in 0..PACENOTE_EVENT_RING_SIZE as u32 {
            ring.enqueue(i + 1);
        }
        // Tail wrapped back onto the head: the ring reads empty despite writes.
        assert!(ring.is_empty());
    }

    #[test]
    fn the_sequencer_speaks_one_field_per_free_tick_and_links_corners() {
        let mut speech = CodriverSequencer::default();
        assert_eq!(speech.tick(false), None);
        // "3 RIGHT" (number 3 at bits 3-6, direction 1 at bits 7-8), then
        // "CAUTION" alone, then another corner "2 LEFT".
        speech.submit((3 << 3) | (1 << 7), false);
        speech.submit(1, false);
        speech.submit((2 << 3) | (2 << 7), false);
        assert_eq!(speech.tick(true), None, "a busy channel waits");
        assert_eq!(speech.tick(false), Some(2 + 3));
        assert_eq!(speech.last_number_559110, 3);
        assert_eq!(speech.tick(false), Some(0xa + 1));
        // The emptied entry advances; CAUTION has no corner field: no link.
        assert_eq!(speech.tick(false), None);
        assert_eq!(speech.tick(false), Some(0));
        // The next entry is a corner: the INTO link plays first.
        assert_eq!(speech.tick(false), Some(0x27));
        assert!(speech.into_link_5590f0);
        assert_eq!(speech.tick(false), Some(2 + 2));
        assert_eq!(speech.tick(false), Some(0xa + 2));
        assert!(!speech.into_link_5590f0);
        assert_eq!(speech.tick(false), None);
        assert_eq!(speech.tick(false), None);
    }

    #[test]
    fn a_mirrored_stage_swaps_the_spoken_direction() {
        let mut speech = CodriverSequencer::default();
        speech.submit(1 << 7, true);
        assert_eq!(speech.tick(false), Some(0xa + 2));
    }

    #[test]
    fn an_entry_without_a_known_field_stays_at_the_head() {
        let mut speech = CodriverSequencer::default();
        speech.submit(1 << 20, false);
        assert_eq!(speech.tick(false), None);
        assert_eq!(speech.tick(false), None);
    }

    #[test]
    fn arrow_hud_maps_right_and_left_corners_and_slides_to_target() {
        let mut hud = PacenoteArrowHud::new();
        // 3 Right: number=3 (bits 3..6), dir=1 (RIGHT, bits 7..8).
        let right3 = (3 << 3) | (1 << 7);
        hud.submit_word(right3, false, false);
        assert_eq!(hud.depth(), 1);
        assert_eq!(hud.slots(), [3, 0xff]);

        // First animation step initializes y=-64 and slides +10 -> -54 -> dst_y = -22.
        let s0 = hud.step_animation();
        assert_eq!(s0.len(), 1);
        assert_eq!(s0[0].arrow_id, 3);
        assert_eq!(s0[0].source, (64, 64, 64, 64));
        assert_eq!(s0[0].dst_x, 288);
        assert_eq!(s0[0].dst_y, -22);
        assert!(!s0[0].flip_x, "right arrows use unflipped flags 0x08");

        // After 10 ticks, settles at target_y = 1*65 - 46 = 19 -> dst_y = 19 + 32 = 51.
        for _ in 0..10 {
            hud.step_animation();
        }
        let settled = hud.current_sprites();
        assert_eq!(settled[0].dst_y, 51);

        // Linked "INTO 5 Left" stacks a second arrow above the first.
        let left5 = (5 << 3) | (2 << 7);
        hud.submit_word(left5, false, true);
        assert_eq!(hud.depth(), 2);
        assert_eq!(hud.slots(), [3, 14]);
        for _ in 0..20 {
            hud.step_animation();
        }
        let stacked = hud.current_sprites();
        assert_eq!(stacked.len(), 2);
        assert_eq!(stacked[0].arrow_id, 3);
        assert_eq!(stacked[0].dst_y, 84 + 32);
        assert!(!stacked[0].flip_x);
        assert_eq!(stacked[1].arrow_id, 14);
        assert_eq!(stacked[1].dst_y, 19 + 32);
        assert!(stacked[1].flip_x, "left arrows use flipped flags 0x01");
    }

    #[test]
    fn arrow_hud_dismisses_after_twelve_waypoints() {
        let mut hud = PacenoteArrowHud::new();
        hud.submit_word((4 << 3) | (2 << 7), false, false);
        assert_eq!(hud.step_animation().len(), 1);
        hud.on_waypoint_advanced(11);
        assert_eq!(hud.depth(), 1);
        hud.on_waypoint_advanced(1);
        assert_eq!(hud.depth(), 0);
        assert!(hud.current_sprites().is_empty());
    }
}
