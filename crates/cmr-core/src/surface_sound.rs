//! Reconstruction of the surface roll and skid sound (`FUN_00489890`) and its
//! bank (`FUN_004892b0`).
//!
//! The bank loads, for every sound group a stage's `.srf` list mentions, the
//! exterior/interior roll waves (`%d_o%d`, `%d_i%d`) and skid waves
//! (`%d_sko%d`, `%d_ski%d`) from `sounds\newsurfs\`, and records each wave's
//! slot in four `u8[120][6]` maps; the interior maps fall back to the
//! exterior waves. Groups 14 and 42 alias the last skid waves of 10 and 30.
//!
//! The tick (player 0, single view) runs after the engine sound. It counts
//! the wheels per surface: a surface with a roll wave on two or more wheels
//! gets a roll channel and a skid channel (primary on 0/1, secondary on 2/3);
//! otherwise "low" surfaces (below 9, `0x32`, `0x5a`) get per-wheel skid loops
//! and a speed-driven bed on channel 3. Roll volume follows speed and slip,
//! skid volume follows the larger of the lateral excess and half the
//! longitudinal excess over `0x3333`; attack is instant, release `/8` (or
//! `/16` with an 8000 floor for skids). It never changes pitch.
//!
//! Every quirk of `.work/surface-impact-audio-notes.md` section 10 is kept:
//! the surface words are rewritten into the physics control record
//! (`0x5a -> 0`, `4 -> 2`, `9 -> 0xe`), the secondary double count, `rpick`
//! reaching `n`, the alias rows, raw-id versus group indexing on the per-wheel
//! path, and the any-channel retry (which spins forever in the original when
//! every candidate is already playing; this port reports that instead).
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.surface-sound`.

use crate::engine_sound::EngineVoices;
use crate::game_rng::GameLcg;

/// Number of sound groups (and surface ids) the tables cover.
pub const GROUPS: usize = 120;
/// Width of each slot-map row.
pub const MAP_WIDTH: usize = 6;
/// The "nothing" sentinel the fades write to the last-group words.
pub const NO_GROUP: i32 = 999;
/// The slip threshold (`0x3333`).
const SLIP: i32 = 0x3333;

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

/// `__ftol`: truncate toward zero; out of range gives the indefinite integer.
fn ftol(value: f64) -> i32 {
    if value.is_finite() && value > -2_147_483_649.0 && value < 2_147_483_648.0 {
        value as i32
    } else {
        i32::MIN
    }
}

/// The static surface-audio tables (`0x4f9408`, `0x4f95e8`, `0x4f9750`,
/// `0x4f9930`, `0x4f996c`, `0x4f9970..`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceAudioTables {
    /// Per group `{ski, i, sko, o}` variant counts.
    pub counts: [[u8; 4]; GROUPS],
    /// Per group (or raw id) volume percent for camera classes 0, 1, 2.
    pub class_percent: [[i8; 3]; GROUPS],
    /// Surface id -> sound group.
    pub surface_group: [i32; GROUPS],
    /// The wet surface ids.
    pub wet_surfaces: [i32; 15],
    /// `K`.
    pub surface_k: i32,
    /// The static defaults of the last-group/last-slot words `0x4f9970..`.
    pub initial_last_state: [i32; 4],
}

const SKI: usize = 0;
const I: usize = 1;
const SKO: usize = 2;
const O: usize = 3;

/// A bank build or tick the reconstruction cannot serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSoundError {
    /// A surface id indexes past the group table and the words that follow it.
    SurfaceIdOutOfRange(i32),
    /// A group read from the table is outside the 120 groups.
    GroupOutOfRange(i32),
    /// A slot-map read lands outside the four maps and their neighbours.
    MapIndexOutOfRange { row: i32, column: i32 },
    /// Every skid variant of a surface is already playing: the original
    /// `FUN_00495d40` retry loop never terminates here.
    RetryWouldSpin { surface: i32 },
}

impl std::fmt::Display for SurfaceSoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "surface sound: {self:?}")
    }
}

impl std::error::Error for SurfaceSoundError {}

impl SurfaceAudioTables {
    /// `grp[b]` for a raw `.srf` byte: the read runs past the 120 entries into
    /// the wet list, `K` and the last-state words, as the loader's unchecked
    /// index does.
    fn group_of_byte(&self, byte: u8) -> Result<i32, SurfaceSoundError> {
        let index = usize::from(byte);
        match index {
            0..GROUPS => Ok(self.surface_group[index]),
            120..135 => Ok(self.wet_surfaces[index - 120]),
            135 => Ok(self.surface_k),
            136..140 => Ok(self.initial_last_state[index - 136]),
            _ => Err(SurfaceSoundError::SurfaceIdOutOfRange(i32::from(byte))),
        }
    }

    fn group(&self, surface: i32) -> Result<i32, SurfaceSoundError> {
        usize::try_from(surface)
            .ok()
            .and_then(|index| self.surface_group.get(index).copied())
            .ok_or(SurfaceSoundError::SurfaceIdOutOfRange(surface))
    }

    fn count(&self, row: i32, variant: usize) -> Result<i32, SurfaceSoundError> {
        usize::try_from(row)
            .ok()
            .and_then(|index| self.counts.get(index))
            .map(|counts| i32::from(counts[variant] as i8))
            .ok_or(SurfaceSoundError::GroupOutOfRange(row))
    }

    fn percent(&self, row: i32, class: u8) -> Result<i32, SurfaceSoundError> {
        usize::try_from(row)
            .ok()
            .and_then(|index| self.class_percent.get(index))
            .map(|percents| i32::from(percents[usize::from(class.min(2))]))
            .ok_or(SurfaceSoundError::GroupOutOfRange(row))
    }
}

/// The four slot maps and the words around them that out-of-row reads land on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceMaps {
    /// `0x8295bc`, `0x82988c`, `0x829b5c` (contiguous) and `0x82a020`.
    pub sko: [[u8; MAP_WIDTH]; GROUPS],
    pub ski: [[u8; MAP_WIDTH]; GROUPS],
    pub i: [[u8; MAP_WIDTH]; GROUPS],
    pub o: [[u8; MAP_WIDTH]; GROUPS],
    /// `DAT_0082a2f8` / `DAT_0082a2fc`: the slots of `2_i0` and `2_o0`.
    pub slot_2_i0: u8,
    pub slot_2_o0: u8,
}

impl Default for SurfaceMaps {
    fn default() -> Self {
        Self {
            sko: [[0; MAP_WIDTH]; GROUPS],
            ski: [[0; MAP_WIDTH]; GROUPS],
            i: [[0; MAP_WIDTH]; GROUPS],
            o: [[0; MAP_WIDTH]; GROUPS],
            slot_2_i0: 0,
            slot_2_o0: 0,
        }
    }
}

/// Which map a lookup reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Map {
    Sko,
    Ski,
    I,
    O,
}

/// The surface sound's persistent words (BSS and the static `0x4f99xx`
/// defaults; none of them are reset at stage load except as noted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceSoundState {
    pub maps: SurfaceMaps,
    /// `DAT_0082a2f4`: the camera class (0 interior, 1 chase near, 2 other).
    pub class: u8,
    /// `v308`, `v30c`, `v310`, `v314`, `v318`.
    pub roll_primary: i32,
    pub skid_primary: i32,
    pub roll_secondary: i32,
    pub skid_secondary: i32,
    pub bed: i32,
    /// `DAT_0082a01c` / `DAT_0082a014`.
    pub primary: i32,
    pub secondary: i32,
    /// `DAT_0082a31c` / `320` / `324`.
    pub primary_active: bool,
    pub low_active: bool,
    pub secondary_active: bool,
    /// `0x4f9970/74/78/7c`: last group on 0 and 2, last slot on 0 and 2.
    pub last_group_0: i32,
    pub last_group_2: i32,
    pub last_slot_0: i32,
    pub last_slot_2: i32,
    /// `0x829e2c`: the last raw surface per per-wheel channel (`0xff` init).
    pub last_surface: [u8; 8],
    /// `DAT_0082a2f0`: the bank is loaded.
    pub loaded: bool,
}

impl SurfaceSoundState {
    /// BSS plus the static defaults of the last-state words.
    pub fn new(tables: &SurfaceAudioTables) -> Self {
        let [g0, g2, s0, s2] = tables.initial_last_state;
        Self {
            maps: SurfaceMaps::default(),
            class: 0,
            roll_primary: 0,
            skid_primary: 0,
            roll_secondary: 0,
            skid_secondary: 0,
            bed: 0,
            primary: 0,
            secondary: 0,
            primary_active: false,
            low_active: false,
            secondary_active: false,
            last_group_0: g0,
            last_group_2: g2,
            last_slot_0: s0,
            last_slot_2: s2,
            last_surface: [0; 8],
            loaded: false,
        }
    }

    /// A slot-map entry, including the out-of-row read at column 6 (`rpick`
    /// reaching `n`), which lands on the next row's entry 0 or, past the last
    /// row, on the next map / `last[0]` / the loaded flag's low byte.
    fn map(&self, map: Map, row: i32, column: i32) -> Result<u8, SurfaceSoundError> {
        let error = SurfaceSoundError::MapIndexOutOfRange { row, column };
        let (Ok(row_index), Ok(column_index)) = (usize::try_from(row), usize::try_from(column))
        else {
            return Err(error);
        };
        let flat = row_index * MAP_WIDTH + column_index;
        let table = match map {
            Map::Sko => &self.maps.sko,
            Map::Ski => &self.maps.ski,
            Map::I => &self.maps.i,
            Map::O => &self.maps.o,
        };
        if flat < GROUPS * MAP_WIDTH {
            return Ok(table[flat / MAP_WIDTH][flat % MAP_WIDTH]);
        }
        if flat == GROUPS * MAP_WIDTH {
            return Ok(match map {
                Map::Sko => self.maps.ski[0][0],
                Map::Ski => self.maps.i[0][0],
                Map::I => self.last_surface[0],
                Map::O => u8::from(self.loaded),
            });
        }
        Err(error)
    }
}

/// The waves the bank loads, in slot order, and the wet-wave request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceBank {
    /// Relative file names under `sounds\newsurfs\`.
    pub waves: Vec<String>,
    /// `DAT_0082a018`: a wet surface is on the stage.
    pub wet: bool,
}

/// `FUN_004892b0`: build the bank for a stage's `.srf` id list, writing the
/// slot maps into `state` (which keeps the entries of absent groups) and
/// running the init `FUN_00489270`.
pub fn load_surface_bank(
    tables: &SurfaceAudioTables,
    srf: &[u8],
    state: &mut SurfaceSoundState,
) -> Result<SurfaceBank, SurfaceSoundError> {
    let mut present = [false; GROUPS];
    let mut wet = false;
    for &byte in srf {
        let group = tables.group_of_byte(byte)?;
        let slot = usize::try_from(group)
            .ok()
            .filter(|&group| group < GROUPS)
            .ok_or(SurfaceSoundError::GroupOutOfRange(group))?;
        present[slot] = true;
        if tables.wet_surfaces.contains(&i32::from(byte)) {
            wet = true;
        }
    }
    let mut waves = Vec::new();
    // The alias locals: last sko (o alias) and last ski-or-sko (i alias) of
    // groups 10 and 30, zero until written.
    let mut alias_10 = (0u8, 0u8);
    let mut alias_30 = (0u8, 0u8);
    let maps = &mut state.maps;
    for group in 0..GROUPS {
        if group == 14 && present[10] {
            maps.o[14][0] = alias_10.0;
            maps.i[14][0] = alias_10.1;
            continue;
        }
        if group == 42 && present[30] {
            maps.o[42][0] = alias_30.0;
            maps.i[42][0] = alias_30.1;
            continue;
        }
        if !present[group] {
            continue;
        }
        let count = |variant: usize| (tables.counts[group][variant] as i8).max(0) as usize;
        for j in 0..count(O) {
            let slot = waves.len() as u8;
            waves.push(format!("{group}_o{j}.wav"));
            maps.o[group][j] = slot;
            maps.i[group][j] = slot;
            if group == 2 && j == 0 {
                maps.slot_2_o0 = slot;
            }
        }
        for j in 0..count(I) {
            let slot = waves.len() as u8;
            waves.push(format!("{group}_i{j}.wav"));
            maps.i[group][j] = slot;
            if group == 2 && j == 0 {
                maps.slot_2_i0 = slot;
            }
        }
        for j in 0..count(SKO) {
            let slot = waves.len() as u8;
            waves.push(format!("{group}_sko{j}.wav"));
            maps.sko[group][j] = slot;
            maps.ski[group][j] = slot;
            match group {
                10 => alias_10 = (slot, slot),
                30 => alias_30 = (slot, slot),
                _ => {}
            }
        }
        for j in 0..count(SKI) {
            let slot = waves.len() as u8;
            waves.push(format!("{group}_ski{j}.wav"));
            maps.ski[group][j] = slot;
            match group {
                10 => alias_10.1 = slot,
                30 => alias_30.1 = slot,
                _ => {}
            }
        }
    }
    state.last_surface = [0xff; 8];
    state.loaded = true;
    Ok(SurfaceBank { waves, wet })
}

/// `FUN_0048b680(player, camera_mode)`: the camera class; changing it stops
/// the four channels. Player 1 is ignored.
pub fn set_surface_class<V: EngineVoices>(
    state: &mut SurfaceSoundState,
    voices: &mut V,
    channel_base: u32,
    player: u8,
    camera_mode: i32,
) {
    if player == 1 {
        return;
    }
    state.class = match camera_mode {
        1..=4 | 10 => 0,
        5 => 1,
        _ => 2,
    };
    for n in 0..4 {
        if voices.playing(channel_base + n) {
            voices.stop(channel_base + n);
        }
    }
}

/// `FUN_0048b650`: with one player, stop the four surface channels and the
/// next one (the wet/rain channel the following loader allocated).
pub fn stop_surface_channels<V: EngineVoices>(voices: &mut V, channel_base: u32, one_player: bool) {
    if one_player {
        for n in 0..5 {
            voices.stop(channel_base + n);
        }
    }
}

/// The words one tick reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceTickInput {
    /// `ctl +0x42a + 2i`: the wheels' surface words.
    pub surfaces: [i16; 4],
    /// `car +0x73a + i`: 1 means the wheel has no contact.
    pub contact: [u8; 4],
    /// `ctl +0x3b0 + 4i`: longitudinal slip.
    pub longitudinal: [i32; 4],
    /// `ctl +0x3c0 + 4i`: lateral dot.
    pub lateral: [i32; 4],
    /// `car +0xec`.
    pub speed_ec: i32,
    /// `cfg[0x26] & 2`: the effects are off.
    pub effects_off: bool,
    /// `DAT_005ff9e4[0]`: the listener's centre proximity.
    pub centre_proximity: i32,
    /// `DAT_004f653c`: the effects master.
    pub master_percent: i32,
}

/// The tick's outcome: the surface words it wrote back into the control
/// record, and an absolute channel it stopped (`stop(0x11)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceTickOutcome {
    pub surfaces: [i16; 4],
    pub stopped_absolute_0x11: bool,
}

struct Tick<'a, V: EngineVoices> {
    voices: &'a mut V,
    tables: &'a SurfaceAudioTables,
    slot_base: u32,
    channel_base: u32,
    input: &'a SurfaceTickInput,
    class: u8,
}

impl<V: EngineVoices> Tick<'_, V> {
    fn ch(&self, n: u32) -> u32 {
        self.channel_base + n
    }
    fn playing(&self, n: u32) -> bool {
        self.voices.playing(self.ch(n))
    }
    fn stop(&mut self, n: u32) {
        let ch = self.ch(n);
        self.voices.stop(ch);
    }
    fn stop_if_playing(&mut self, n: u32) {
        if self.playing(n) {
            self.stop(n);
        }
    }
    /// `FUN_0047f9c0(0, v)`.
    fn curve(&self, v: i32) -> i32 {
        crate::engine_listener::centre_curve(self.input.centre_proximity, v)
    }
    /// `FUN_0047eb70`.
    fn volume(&mut self, n: u32, v: i32) {
        let scaled = self.input.master_percent.wrapping_mul(v) / 100;
        let ch = self.ch(n);
        self.voices.set_volume(ch, scaled);
    }
    fn curve_volume(&mut self, n: u32, v: i32) {
        let v = self.curve(v);
        self.volume(n, v);
    }
    fn play(&mut self, slot: u8, n: u32, looping: bool) {
        let ch = self.ch(n);
        let wave = self.slot_base + u32::from(slot);
        self.voices
            .play(wave, ch, if looping { u32::MAX } else { 0 });
    }
    fn roll_map(&self) -> Map {
        if self.class == 0 { Map::I } else { Map::O }
    }
    fn skid_map(&self) -> Map {
        if self.class == 0 { Map::Ski } else { Map::Sko }
    }
    fn roll_count(&self, row: i32) -> Result<i32, SurfaceSoundError> {
        self.tables.count(row, if self.class == 0 { I } else { O })
    }
    fn skid_count(&self, row: i32) -> Result<i32, SurfaceSoundError> {
        self.tables
            .count(row, if self.class == 0 { SKI } else { SKO })
    }
    /// `| ftol((double)(pct[x][cls] * v) * 0.8 * 0.01) |`.
    fn pct_volume(&self, row: i32, v: i32) -> Result<i32, SurfaceSoundError> {
        let percent = self.tables.percent(row, self.class)?;
        let mut value = f64::from(percent.wrapping_mul(v)) * 0.8 * 0.01;
        if value < 0.0 {
            value = -value;
        }
        Ok(ftol(value))
    }
    fn k_scaled(&self, factor: i32) -> i32 {
        ftol(f64::from(self.tables.surface_k.wrapping_mul(factor)) * 65536.0)
    }
}

/// `rpick(n) = (rand() * n) / 0x7fff`, which can return `n`.
fn rpick(rng: &mut GameLcg, n: i32) -> i32 {
    (rng.next_raw() as i32).wrapping_mul(n) / 0x7fff
}

fn hud_speed(speed_ec: i32) -> i32 {
    qmul(speed_ec, 0x43_1168)
}

fn speed_term(speed_ec: i32, k: i32, n: i32) -> i32 {
    let hud = hud_speed(speed_ec);
    if hud < 0x10000 {
        0
    } else {
        ((hud.min(0x64_0000) >> 16).wrapping_mul(k).wrapping_mul(n)) / 110
    }
}

fn max_abs(values: &[i32; 4], clamp: i32) -> i32 {
    values
        .iter()
        .map(|value| value.wrapping_abs().min(clamp))
        .fold(i32::MIN, i32::max)
}

impl SurfaceSoundState {
    /// The tick `FUN_00489890`.
    pub fn tick<V: EngineVoices>(
        &mut self,
        voices: &mut V,
        tables: &SurfaceAudioTables,
        rng: &mut GameLcg,
        slot_base: u32,
        channel_base: u32,
        input: &SurfaceTickInput,
    ) -> Result<SurfaceTickOutcome, SurfaceSoundError> {
        let mut surfaces = input.surfaces;
        let mut outcome = SurfaceTickOutcome {
            surfaces,
            stopped_absolute_0x11: false,
        };
        if input.effects_off {
            return Ok(outcome);
        }
        let mut t = Tick {
            voices,
            tables,
            slot_base,
            channel_base,
            input,
            class: self.class,
        };
        let lateral = max_abs(&input.lateral, 0x30000);
        let longitudinal = max_abs(&input.longitudinal, 0x30000);
        let a = lateral - SLIP;
        let b = (longitudinal - SLIP) / 2;

        if input.contact.iter().all(|&contact| contact == 1) {
            self.fade_secondary(&mut t);
            self.fade_primary(&mut t);
            return Ok(outcome);
        }

        let (mut n1, mut n2, mut low) = (0, 0, false);
        for surface in &mut surfaces {
            *surface = match *surface {
                0x5a => 0,
                4 => 2,
                9 => 0xe,
                other => other,
            };
            let s = i32::from(*surface);
            let is_low = if s <= 8 || s == 0x5a {
                true
            } else {
                let group = tables.group(s)?;
                t.roll_count(group)? == 0
            };
            if is_low {
                if s < 9 || s == 0x32 || s == 0x5a {
                    low = true;
                    self.low_active = true;
                }
                continue;
            }
            if n1 == 0 {
                self.primary_active = true;
                n1 = 1;
                self.primary = s;
            } else {
                if n2 != 0 && self.secondary == s {
                    n2 += 1;
                }
                if self.primary == s {
                    n1 += 1;
                } else {
                    self.secondary = s;
                    n2 += 1;
                    self.secondary_active = true;
                }
            }
        }
        outcome.surfaces = surfaces;
        if n1 != 0 {
            self.primary = tables.group(self.primary)?;
        }
        if n2 != 0 {
            self.secondary = tables.group(self.secondary)?;
        }

        // The stop transitions, then D.
        let run_secondary = 'stops: {
            if n2 == 0 && n1 == 0 && !low {
                break 'stops false;
            }
            if (n2 != 0 || n1 != 0) && !low && self.low_active {
                for n in 0..4 {
                    t.stop_if_playing(n);
                }
                self.low_active = false;
                break 'stops n2 > 1;
            }
            if n1 == 0 && self.primary_active {
                for n in 0..4 {
                    t.stop_if_playing(n);
                }
                self.primary_active = false;
                break 'stops n2 > 1;
            }
            if n2 == 0 && self.secondary_active {
                t.stop_if_playing(3);
                self.secondary_active = false;
                break 'stops false;
            }
            if self.last_group_0 != self.primary {
                t.stop_if_playing(0);
                t.stop_if_playing(1);
            }
            if n2 != 0 && self.last_group_2 != self.secondary {
                t.stop_if_playing(2);
                t.stop_if_playing(3);
            }
            n2 > 1
        };
        if run_secondary {
            self.secondary_block(&mut t, rng, n2, lateral, longitudinal, a, b)?;
        } else {
            self.fade_secondary(&mut t);
        }

        if n1 <= 1 {
            if low {
                self.per_wheel(&mut t, rng, &surfaces, &mut outcome)?;
            } else {
                self.fade_primary(&mut t);
            }
        } else {
            self.primary_block(&mut t, rng, n1, lateral, longitudinal, a)?;
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn secondary_block<V: EngineVoices>(
        &mut self,
        t: &mut Tick<'_, V>,
        rng: &mut GameLcg,
        n2: i32,
        lateral: i32,
        longitudinal: i32,
        a: i32,
        b: i32,
    ) -> Result<(), SurfaceSoundError> {
        let group = self.secondary;
        let mut v = speed_term(t.input.speed_ec, t.tables.surface_k, n2);
        let skids = t.skid_count(group)?;
        if skids == 0 {
            if a > b && lateral > SLIP {
                v = v.wrapping_add(qmul(a, t.k_scaled(2)) >> 16);
            } else if longitudinal > SLIP {
                v = v.wrapping_add(qmul(b, t.k_scaled(2)) >> 16);
            }
        } else if lateral <= SLIP && longitudinal <= SLIP {
            if t.playing(3) {
                self.skid_secondary -= self.skid_secondary / 16;
                if self.skid_secondary > 8000 {
                    t.curve_volume(3, self.skid_secondary);
                } else {
                    self.skid_secondary = 0;
                    t.volume(3, 0);
                }
            }
        } else {
            let x = if a > b { a } else { b };
            let skid = qmul(x, t.k_scaled(n2)) >> 16;
            let skid = t.pct_volume(group, skid)?.min(0x7fff);
            self.skid_secondary = skid.max(self.skid_secondary - self.skid_secondary / 8);
            if !t.playing(3) {
                let column = rpick(rng, skids);
                let slot = self.map(t.skid_map(), group, column)?;
                t.play(slot, 3, true);
            }
            t.curve_volume(3, self.skid_secondary);
        }
        self.roll_secondary = t.pct_volume(group, v)?;
        let rolls = t.roll_count(group)?;
        if rolls > 1 {
            let mut slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
            let mut retries = 0;
            while i32::from(slot) == self.last_slot_2 && retries < 3 {
                slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
                retries += 1;
            }
            self.last_slot_2 = i32::from(slot);
            if !t.playing(2) {
                t.play(slot, 2, false);
                self.last_group_2 = group;
            }
        } else if !t.playing(2) {
            if rolls != 0 {
                let slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
                t.play(slot, 2, true);
            }
            self.last_group_2 = group;
        }
        t.curve_volume(2, self.roll_secondary);
        Ok(())
    }

    fn primary_block<V: EngineVoices>(
        &mut self,
        t: &mut Tick<'_, V>,
        rng: &mut GameLcg,
        n1: i32,
        lateral: i32,
        longitudinal: i32,
        a: i32,
    ) -> Result<(), SurfaceSoundError> {
        let group = self.primary;
        let b = (longitudinal - SLIP) / 2;
        let mut v = speed_term(t.input.speed_ec, t.tables.surface_k, n1);
        let skids = t.skid_count(group)?;
        let k = t.tables.surface_k;
        if skids == 0 {
            if a > b && lateral > SLIP {
                v = v.wrapping_add(k.wrapping_mul(a).wrapping_shl(1) >> 16);
            } else if longitudinal > SLIP {
                v = v.wrapping_add(k.wrapping_mul(b).wrapping_shl(1) >> 16);
            }
        } else if lateral <= SLIP && longitudinal <= SLIP {
            if t.playing(1) {
                self.skid_primary -= self.skid_primary / 16;
                if self.skid_primary > 8000 {
                    t.curve_volume(1, self.skid_primary);
                } else {
                    t.stop(1);
                    self.skid_primary = 0;
                }
            }
        } else {
            let x = if a > b {
                a
            } else {
                (longitudinal.min(0x2_3333) - SLIP) / 2
            };
            let skid = qmul(x, t.k_scaled(n1)) >> 16;
            let skid = t.pct_volume(group, skid)?.min(0x7fff);
            self.skid_primary = skid.max(self.skid_primary - self.skid_primary / 8);
            if !t.playing(1) {
                let column = rpick(rng, skids);
                let slot = self.map(t.skid_map(), group, column)?;
                t.play(slot, 1, true);
            }
            t.curve_volume(1, self.skid_primary);
        }
        self.roll_primary = t.pct_volume(group, v)?;
        let rolls = t.roll_count(group)?;
        if rolls > 1 {
            let mut slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
            let mut retries = 0;
            while i32::from(slot) == self.last_slot_0 && retries < 3 {
                slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
                retries += 1;
            }
            self.last_slot_0 = i32::from(slot);
            if !t.playing(0) {
                t.play(slot, 0, false);
                self.last_group_0 = group;
            }
        } else if !t.playing(0) {
            if rolls != 0 {
                let slot = self.map(t.roll_map(), group, rpick(rng, rolls))?;
                t.play(slot, 0, true);
            }
            self.last_group_0 = group;
        }
        t.curve_volume(0, self.roll_primary);
        Ok(())
    }

    fn fade_secondary<V: EngineVoices>(&mut self, t: &mut Tick<'_, V>) {
        if t.playing(2) {
            self.roll_secondary -= self.roll_secondary / 8;
            if self.roll_secondary <= 0 {
                t.volume(2, 0);
                self.roll_secondary = 0;
                self.secondary = NO_GROUP;
                self.last_group_2 = NO_GROUP;
            } else {
                t.curve_volume(2, self.roll_secondary);
            }
        }
        if t.playing(3) {
            self.skid_secondary -= self.skid_secondary / 8;
            if self.skid_secondary <= 0 {
                t.volume(3, 0);
                self.skid_secondary = 0;
            } else {
                t.curve_volume(3, self.skid_secondary);
            }
        }
    }

    fn fade_primary<V: EngineVoices>(&mut self, t: &mut Tick<'_, V>) {
        if t.playing(0) {
            self.roll_primary -= self.roll_primary / 8;
            if self.roll_primary <= 0 {
                t.volume(0, 0);
                self.roll_primary = 0;
                self.primary = NO_GROUP;
                self.last_group_0 = NO_GROUP;
            } else {
                t.curve_volume(0, self.roll_primary);
            }
        }
        if t.playing(1) {
            self.skid_primary -= self.skid_primary / 8;
            if self.skid_primary <= 0 {
                t.volume(1, 0);
                self.skid_primary = 0;
            } else {
                t.curve_volume(1, self.skid_primary);
            }
        }
    }

    fn per_wheel<V: EngineVoices>(
        &mut self,
        t: &mut Tick<'_, V>,
        rng: &mut GameLcg,
        surfaces: &[i16; 4],
        outcome: &mut SurfaceTickOutcome,
    ) -> Result<(), SurfaceSoundError> {
        let input = t.input;
        let on = |i: usize, id: i16| surfaces[i] == id && input.contact[i] != 1;
        let count_2 = (0..4).filter(|&i| on(i, 2)).count();
        let count_32 = (0..4).filter(|&i| on(i, 0x32)).count();
        if count_2 != 0 {
            t.voices.stop(0x11);
            outcome.stopped_absolute_0x11 = true;
        }
        let wheels = if count_2 != 0 || count_32 != 0 { 3 } else { 4 };
        for (i, &s) in surfaces.iter().enumerate().take(wheels) {
            if s >= 10 {
                continue;
            }
            let n = i as u32;
            let lateral = input.lateral[i].wrapping_abs().min(0x30000);
            let longitudinal = input.longitudinal[i].wrapping_abs().min(0x30000);
            if input.contact[i] == 1 || (lateral <= SLIP && longitudinal <= SLIP) {
                t.stop_if_playing(n);
                continue;
            }
            let a = lateral - SLIP;
            let b = (longitudinal - SLIP) / 2;
            let x = if a > b { a } else { b };
            let v = (qmul(x, t.k_scaled(3)) >> 16).min(0x7fff);
            let raw = i32::from(s);
            let count = t.skid_count(raw)?;
            let row = t.tables.group(raw)?;
            if t.playing(n) {
                if i32::from(self.last_surface[i]) != i32::from(s as u16) {
                    t.stop(n);
                    if count != 0 {
                        let slot = self.map(t.skid_map(), row, rpick(rng, count))?;
                        t.play(slot, n, true);
                    }
                    self.last_surface[i] = s as u8;
                }
            } else {
                if count != 0 {
                    let all_busy = (0..=count).all(|column| {
                        self.map(t.skid_map(), row, column)
                            .is_ok_and(|slot| t.voices.wave_busy(t.slot_base + u32::from(slot)))
                    });
                    if all_busy {
                        return Err(SurfaceSoundError::RetryWouldSpin { surface: raw });
                    }
                    let mut slot = self.map(t.skid_map(), row, rpick(rng, count))?;
                    while t.voices.wave_busy(t.slot_base + u32::from(slot)) {
                        slot = self.map(t.skid_map(), row, rpick(rng, count))?;
                    }
                    t.play(slot, n, true);
                }
                self.last_surface[i] = s as u8;
            }
            let pct = t.pct_volume(raw, v)?;
            t.curve_volume(n, pct);
        }
        if count_2 == 0 && count_32 == 0 {
            if t.playing(3) {
                self.bed -= self.bed / 8;
                if self.bed > 0 {
                    t.curve_volume(3, self.bed);
                } else {
                    t.stop(3);
                    t.volume(3, 0);
                    self.bed = 0;
                }
            }
            return Ok(());
        }
        let k = if count_2 != 0 { 4 } else { 0 };
        let hud = hud_speed(input.speed_ec);
        let bed = if hud < 0x10000 {
            0
        } else {
            (((hud.min(0x64_0000) >> 16)
                .wrapping_mul(t.tables.surface_k)
                .wrapping_mul(k))
                / 110)
                .min(0x7fff)
        };
        let bed = t.pct_volume(2, bed)?;
        self.bed = bed.max(self.bed - self.bed / 8);
        if !t.playing(3) {
            let slot = match (count_2 != 0, t.class == 0) {
                (true, true) => self.maps.slot_2_i0,
                (true, false) => self.maps.slot_2_o0,
                (false, true) => self.maps.i[0x32][0],
                (false, false) => self.maps.o[0x32][0],
            };
            t.play(slot, 3, true);
        }
        t.curve_volume(3, self.bed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_mixer::Position3d;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Voices {
        /// channel -> wave
        playing: BTreeMap<u32, u32>,
        plays: Vec<(u32, u32, u32)>,
        stops: Vec<u32>,
        volumes: Vec<(u32, i32)>,
    }

    impl EngineVoices for Voices {
        fn playing(&self, channel: u32) -> bool {
            self.playing.contains_key(&channel)
        }
        fn play(&mut self, wave: u32, channel: u32, loops: u32) {
            self.playing.insert(channel, wave);
            self.plays.push((wave, channel, loops));
        }
        fn play_with_loop_start(&mut self, wave: u32, channel: u32, _loop_start: u32) {
            self.play(wave, channel, u32::MAX);
        }
        fn stop(&mut self, channel: u32) {
            self.playing.remove(&channel);
            self.stops.push(channel);
        }
        fn set_volume(&mut self, channel: u32, volume: i32) {
            self.volumes.push((channel, volume));
        }
        fn set_frequency(&mut self, _channel: u32, _frequency: u32) {}
        fn set_position(&mut self, _channel: u32, _position: Position3d) {}
        fn wave_busy(&self, wave: u32) -> bool {
            self.playing.values().any(|&playing| playing == wave)
        }
    }

    /// Synthetic tables: surface id = group; group 20 (gravel) has one roll and
    /// two skids per class, low ids 0..9 have one skid each, group 2 one roll.
    fn tables() -> SurfaceAudioTables {
        let mut counts = [[0u8; 4]; GROUPS];
        counts[20] = [2, 1, 2, 1];
        counts[21] = [0, 3, 0, 3];
        counts[10] = [1, 0, 2, 0];
        counts[2] = [1, 1, 1, 1];
        for low in [0, 1, 3, 5] {
            counts[low] = [1, 0, 1, 0];
        }
        SurfaceAudioTables {
            counts,
            class_percent: [[100, 100, 100]; GROUPS],
            surface_group: std::array::from_fn(|id| id as i32),
            wet_surfaces: [0x70; 15],
            surface_k: 0x100,
            initial_last_state: [NO_GROUP, NO_GROUP, -1, -1],
        }
    }

    fn input(surfaces: [i16; 4]) -> SurfaceTickInput {
        SurfaceTickInput {
            surfaces,
            contact: [0; 4],
            longitudinal: [0; 4],
            lateral: [0; 4],
            speed_ec: 0x2_0000,
            effects_off: false,
            centre_proximity: 1000,
            master_percent: 100,
        }
    }

    fn loaded(srf: &[u8]) -> (SurfaceAudioTables, SurfaceSoundState, SurfaceBank) {
        let tables = tables();
        let mut state = SurfaceSoundState::new(&tables);
        let bank = load_surface_bank(&tables, srf, &mut state).unwrap();
        (tables, state, bank)
    }

    #[test]
    fn the_bank_loads_present_groups_in_variant_order() {
        let (_, state, bank) = loaded(&[20, 2, 2]);
        assert_eq!(
            bank.waves,
            vec![
                "2_o0.wav",
                "2_i0.wav",
                "2_sko0.wav",
                "2_ski0.wav",
                "20_o0.wav",
                "20_i0.wav",
                "20_sko0.wav",
                "20_sko1.wav",
                "20_ski0.wav",
                "20_ski1.wav",
            ]
        );
        assert_eq!((state.maps.slot_2_o0, state.maps.slot_2_i0), (0, 1));
        assert_eq!(state.maps.o[20][0], 4);
        assert_eq!(
            state.maps.i[20][0], 5,
            "the interior wave replaces the fallback"
        );
        assert_eq!(state.maps.sko[20][1], 7);
        assert_eq!(state.maps.ski[20][1], 9);
        assert!(!bank.wet && state.loaded && state.last_surface == [0xff; 8]);
    }

    #[test]
    fn group_ten_aliases_fourteen_and_unchecked_bytes_read_on() {
        let (_, state, bank) = loaded(&[10, 14]);
        assert_eq!(
            bank.waves,
            vec!["10_sko0.wav", "10_sko1.wav", "10_ski0.wav"]
        );
        assert_eq!(state.maps.o[14][0], 1, "the last sko");
        assert_eq!(state.maps.i[14][0], 2, "the last ski");
        // Byte 120 reads the first wet id (0x70) as a group.
        let tables = tables();
        let mut state = SurfaceSoundState::new(&tables);
        let bank = load_surface_bank(&tables, &[120, 0x70], &mut state).unwrap();
        assert!(bank.wet);
        assert_eq!(
            load_surface_bank(&tables, &[140], &mut state),
            Err(SurfaceSoundError::SurfaceIdOutOfRange(140))
        );
        // Byte 135 reads K (0x100), which is no group.
        assert_eq!(
            load_surface_bank(&tables, &[135], &mut state),
            Err(SurfaceSoundError::GroupOutOfRange(0x100))
        );
    }

    #[test]
    fn the_tick_rewrites_the_surface_words() {
        let (tables, mut state, _) = loaded(&[20]);
        let mut voices = Voices::default();
        let mut rng = GameLcg::new(1);
        let out = state
            .tick(
                &mut voices,
                &tables,
                &mut rng,
                100,
                10,
                &input([0x5a, 4, 9, 20]),
            )
            .unwrap();
        assert_eq!(out.surfaces, [0, 2, 0xe, 20]);
    }

    #[test]
    fn a_shared_surface_plays_a_speed_roll_and_skids_on_slip() {
        let (tables, mut state, _) = loaded(&[20]);
        state.class = 1;
        let mut voices = Voices::default();
        let mut rng = GameLcg::new(1);
        let mut tick = input([20; 4]);
        state
            .tick(&mut voices, &tables, &mut rng, 100, 10, &tick)
            .unwrap();
        // One exterior roll variant: a loop on channel 0.
        let roll = 100 + u32::from(state.maps.o[20][0]);
        assert_eq!(voices.plays, vec![(roll, 10, u32::MAX)]);
        assert_eq!(state.last_group_0, 20);
        // speed term: hud = qmul(0x20000, 0x431168) >> 16 = 134; 134*K*4/110.
        let hud = (qmul(0x2_0000, 0x43_1168)).min(0x64_0000) >> 16;
        let v = hud * 0x100 * 4 / 110;
        let expected = ftol(f64::from(100 * v) * 0.8 * 0.01);
        assert_eq!(state.roll_primary, expected);
        assert_eq!(voices.volumes.last(), Some(&(10, expected)));
        tick.lateral = [0x8000; 4];
        state
            .tick(&mut voices, &tables, &mut rng, 100, 10, &tick)
            .unwrap();
        let skid = voices.plays.iter().find(|play| play.1 == 11).unwrap();
        let skids =
            [state.maps.sko[20][0], state.maps.sko[20][1]].map(|slot| 100 + u32::from(slot));
        assert!(skids.contains(&skid.0), "an exterior skid");
        assert!(state.skid_primary > 0);
    }

    #[test]
    fn everything_fades_when_every_wheel_is_airborne() {
        let (tables, mut state, _) = loaded(&[20]);
        let mut voices = Voices::default();
        let mut rng = GameLcg::new(1);
        state
            .tick(&mut voices, &tables, &mut rng, 100, 10, &input([20; 4]))
            .unwrap();
        let before = state.roll_primary;
        let mut air = input([20; 4]);
        air.contact = [1; 4];
        state
            .tick(&mut voices, &tables, &mut rng, 100, 10, &air)
            .unwrap();
        assert_eq!(state.roll_primary, before - before / 8);
    }

    #[test]
    fn low_surfaces_skid_per_wheel_and_surface_two_runs_the_bed() {
        let (tables, mut state, _) = loaded(&[2, 0, 1, 3, 5]);
        let mut voices = Voices::default();
        let mut rng = GameLcg::new(1);
        let mut tick = input([0, 1, 2, 3]);
        tick.lateral = [0x8000; 4];
        let out = state
            .tick(&mut voices, &tables, &mut rng, 100, 10, &tick)
            .unwrap();
        assert!(out.stopped_absolute_0x11);
        assert!(voices.stops.contains(&0x11));
        // Wheels 0 and 1 get skid loops; wheel 2 is on surface 2 (low, < 10);
        // channel 3 is the bed playing 2_o0 (class 0 -> 2_i0 here).
        let channels: Vec<u32> = voices.plays.iter().map(|play| play.1).collect();
        assert!(channels.contains(&10) && channels.contains(&11) && channels.contains(&12));
        assert!(
            voices
                .plays
                .contains(&(100 + u32::from(state.maps.slot_2_i0), 13, u32::MAX))
        );
        assert_eq!(state.last_surface[..3], [0, 1, 2]);
    }

    #[test]
    fn rpick_can_read_one_past_the_row() {
        let (_, mut state, _) = loaded(&[20]);
        state.maps.sko[21][0] = 42;
        assert_eq!(state.map(Map::Sko, 20, 6), Ok(42));
        state.maps.ski[0][0] = 7;
        assert_eq!(state.map(Map::Sko, 119, 6), Ok(7));
        assert_eq!(state.map(Map::O, 119, 6), Ok(1), "the loaded flag");
        assert!(state.map(Map::O, 119, 7).is_err());
        let mut rng = GameLcg::new(0);
        // Find a seed draw of 0x7fff: rpick(n) == n.
        let mut hit = false;
        for _ in 0..200_000 {
            let mut probe = rng;
            if rpick(&mut probe, 2) == 2 {
                hit = true;
                break;
            }
            rng.next_raw();
        }
        assert!(hit);
    }

    #[test]
    fn the_any_channel_retry_is_reported_when_it_would_spin() {
        let (tables, mut state, _) = loaded(&[0, 1, 3, 5]);
        let mut voices = Voices::default();
        // One skid variant: rpick(1) is 0 or 1, so columns 0 and 1 are the
        // only candidates.
        voices
            .playing
            .insert(30, 100 + u32::from(state.maps.ski[0][0]));
        voices
            .playing
            .insert(31, 100 + u32::from(state.maps.ski[0][1]));
        let mut rng = GameLcg::new(1);
        let mut tick = input([0, 0x40, 0x40, 0x40]);
        tick.lateral = [0x8000; 4];
        assert_eq!(
            state.tick(&mut voices, &tables, &mut rng, 100, 10, &tick),
            Err(SurfaceSoundError::RetryWouldSpin { surface: 0 })
        );
    }

    #[test]
    fn the_class_follows_the_camera_and_stops_the_channels() {
        let (_, mut state, _) = loaded(&[20]);
        let mut voices = Voices::default();
        voices.playing.insert(10, 1);
        set_surface_class(&mut state, &mut voices, 10, 0, 5);
        assert_eq!(state.class, 1);
        assert!(voices.stops.contains(&10));
        set_surface_class(&mut state, &mut voices, 10, 0, 7);
        assert_eq!(state.class, 2);
        set_surface_class(&mut state, &mut voices, 10, 1, 1);
        assert_eq!(state.class, 2, "player 1 is ignored");
        set_surface_class(&mut state, &mut voices, 10, 0, 10);
        assert_eq!(state.class, 0);
        let mut voices = Voices::default();
        stop_surface_channels(&mut voices, 10, true);
        assert_eq!(voices.stops, vec![10, 11, 12, 13, 14]);
    }
}
