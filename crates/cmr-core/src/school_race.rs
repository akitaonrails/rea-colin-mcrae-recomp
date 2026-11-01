//! The rally school's in-race logic (state 5 in a school session): the
//! task's lines (`.dat` triggers of type 3, numbered at load), each crossing
//! captured and compared with the instructor's reference run (`.rsf`), the
//! task's timer and fail conditions (`FUN_0046e230`), the finish settle
//! (`FUN_004718d0`), the task end (`FUN_00471810`) and the scorer
//! (`FUN_004725e0`) that writes the results marks into `cfg[0x6bc..0x6bf]`.
//!
//! The level `L` and task `T` select the row `row(L, T)` of the scorer's
//! offset and scale tables (`0x4f4f80`, `0x4f50a0`, static data). The loop
//! tasks (`DAT_005f9b60`: L0 T1, L1/L2 T0..1) drive four laps; the others
//! one pass. A description of disassembly, not original source. Evidence:
//! `rea.official-eu.school-race`.

use cmr_formats::school_rsf::{LineRecord, RSF_RECORDS, SchoolReference};

use crate::game_rng::GameLcg;
use crate::legacy_math::{signed_ratio_angle, sqrt_q16};

mod advice;
pub use advice::{Instructor, advice_path};

/// The scorer's row for a task (`FUN_00472c60`).
pub const fn score_row(level: u8, task: u8) -> usize {
    (match level {
        0 => task,
        1 => task + 3,
        _ => task + 7,
    }) as usize
}

/// `FUN_004752e0`'s slack over the reference time, seconds by row.
pub const TIME_SLACK: [i32; 12] = [10, 30, 40, 30, 30, 30, 30, 25, 25, 30, 30, 40];

/// `FUN_00475230`'s contact-frame limit by level.
pub const CONE_LIMIT: [i32; 3] = [10, 8, 3];

/// `FUN_00472fa0`'s set-up grades (stack tables): the tyre for tasks 3 and
/// 4, then the four forced setup options.
const SETUP_TYRES_T3: [i32; 5] = [3, 4, 10, 7, -1];
const SETUP_TYRES_T4: [i32; 5] = [3, 10, 7, 6, 0];
const SETUP_93: [i32; 5] = [5, 10, 5, -1, -1];
const SETUP_95: [i32; 5] = [10, 6, 2, -1, -1];
const SETUP_97: [i32; 5] = [2, 6, 10, -1, -1];
const SETUP_99: [i32; 5] = [2, 3, 6, 10, 7];

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

/// The words the school reads from the player car each tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchoolCar {
    /// `car +0xc8` position, `+0xd4` velocity, `+0xec` speed.
    pub position: [i32; 3],
    pub velocity: [i32; 3],
    pub speed: i32,
    /// `car +0x258/+0x260`: the forward row's x and z; `+0x270`: the
    /// lateral row.
    pub forward_xz: [i32; 2],
    pub lateral: [i32; 3],
    /// Control `+0x3e0`, `+0x2ec` (brake), `+0x2f4` (handbrake), `+0x304`
    /// (throttle demand), `+0x43a` (gear).
    pub word_3e0: i16,
    pub brake: i32,
    pub handbrake: i32,
    pub throttle: i32,
    pub gear: u8,
    /// `DAT_00552f68`: an object contact this frame, and `FUN_00443980`
    /// its object's surface.
    pub object_contact: bool,
    pub contact_surface: i8,
    /// Control `+0x308`/`+0x30c` (brake and handbrake demands) and
    /// `+0x414` (analog throttle).
    pub brake_demand: i32,
    pub handbrake_demand: i32,
    pub analog_throttle: bool,
    /// Control `+0x42a`: the wheel surfaces.
    pub surfaces: [i16; 4],
    /// Car `+0x680`: out of control.
    pub crashed: bool,
    /// `FUN_00476030`'s damage total (`DAT_0050e0d8[i]`, `i != 0xc`) and
    /// the last impact's severity (`DAT_005fbe8c`, from `FUN_0041f630`).
    pub damage: i32,
    pub severity: i32,
    /// `DAT_0057c448 - DAT_0057ae70`: `.cat` nodes left; `FUN_004671f0(0)`:
    /// five reverse crossings in a row.
    pub cat_remaining: i32,
    pub reverse_count_5: bool,
}

/// The state that outlives a task (`0x4f51c4..0x4f51d8`, only written by
/// the crossing handler).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchoolStatics {
    pub ring: [i32; 5],
    pub lap_latch: i32,
    /// The instructor's speech words, also never reset by a task.
    pub instructor: Instructor,
}

impl Default for SchoolStatics {
    fn default() -> Self {
        Self {
            ring: [-1; 5],
            lap_latch: 999,
            instructor: Instructor::default(),
        }
    }
}

/// What the task asks of the host this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchoolCue {
    /// `FUN_00475e90`: play this advice-bank wav ([`advice_path`]) on the
    /// advice channel.
    Advice(u8),
}

/// The control words the finish settle forces (`FUN_004719f0`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettleControls {
    pub throttle_zero: bool,
    /// The literal demand 1 on the brake and handbrake.
    pub brake_one: bool,
    pub engine_zero: bool,
    pub gear_neutral: bool,
}

/// One task in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchoolRace {
    pub level: u8,
    pub task: u8,
    /// `DAT_005f9b60` and `DAT_00551ae4` (the HUD).
    pub loop_task: bool,
    pub hud: bool,
    reference: SchoolReference,
    /// `DAT_005fbe18`: the stage's lines; `DAT_005f9e94`: crossings armed.
    pub lines: i32,
    armed: bool,
    /// `DAT_005f9b70`: the lines not yet crossed.
    uncrossed: Vec<bool>,
    /// `DAT_005fbe28` (laps done), `DAT_005fbe3c` (+2 penalty),
    /// `DAT_005fbe48..0x5fbe60` (the crossing history, newest first; the
    /// last word doubles as the lap probe latch).
    lap: i32,
    lap_penalty: bool,
    history: [i32; 7],
    /// `DAT_005fbdf0` (crossing in reverse), `DAT_005f9ea0` (order kept),
    /// `DAT_005fbe30` (four laps done).
    pub wrong_way: bool,
    order_ok: bool,
    laps_done: bool,
    /// `DAT_005fbe14` (finished, settling) and `DAT_005fbdec` (ended).
    pub finishing: bool,
    pub done: bool,
    /// `DAT_005fbe2c`, `DAT_005fbe20`, `DAT_005fbe1c`, `DAT_005fbe24`.
    last_line: i32,
    index: i32,
    prev_index: i32,
    missed: i32,
    captures: Vec<LineRecord>,
    /// `DAT_005f9ea8`: the run's time (cs).
    run_time: i32,
    /// `DAT_005f9b38..0x5f9b54`.
    pub penalties: [i32; 8],
    /// The race clock slot 0 (`DAT_005ffc00`/`0x5ffa48`/`0x5ffc0c`), Q16
    /// seconds.
    clock_running: bool,
    clock_start: i32,
    now: i32,
    /// `DAT_005ef43c` (task FSM), `DAT_005ef470` (start wait),
    /// `DAT_005ef474` (the wait's idle nag armed).
    pub fsm: u8,
    wait: i32,
    wait_nag_armed: bool,
    /// `DAT_005fc070` (contact frames), `DAT_005fc054` (failed by time or
    /// cones), `DAT_005fbe78` (abandoned).
    pub cones: i32,
    contact_latch: bool,
    pub failed: bool,
    pub abandoned: bool,
    cones_spoken: bool,
    /// `DAT_005fbe38`, `DAT_005f9b5c`, `DAT_005f9b58`: the settle window.
    settle_started: bool,
    settle_mark: i32,
    settle_window: i32,
    /// `DAT_005fc074`/`DAT_005fc03c`: the reference seconds, latched.
    ref_seconds: Option<i32>,
    /// `DAT_005ef458`: the task end armed the exit.
    pub exit_pending: bool,
    scoring: SchoolScoring,
    marks: [u8; 4],
}

fn ticks_per_second(flag_551af0: bool) -> i32 {
    ((1i64 << 32) / (if flag_551af0 { 2 } else { 1 } * 0x19_0000)) as i32
}

impl SchoolRace {
    /// `FUN_004716b0` (after the stage's lines are counted) and
    /// `FUN_00473390`: the task's start.
    pub fn start(
        level: u8,
        task: u8,
        lines: i32,
        reference: SchoolReference,
        scoring: SchoolScoring,
    ) -> Self {
        let loop_task = if level == 0 { task == 1 } else { task <= 1 };
        let hud = !(loop_task || level == 0 && task == 0);
        Self {
            level,
            task,
            loop_task,
            hud,
            reference,
            lines,
            armed: true,
            uncrossed: vec![lines > 0; RSF_RECORDS],
            lap: 0,
            lap_penalty: false,
            history: [0; 7],
            wrong_way: false,
            order_ok: true,
            laps_done: false,
            finishing: false,
            done: false,
            last_line: 0,
            index: 0,
            prev_index: -1,
            missed: 0,
            captures: vec![LineRecord::default(); RSF_RECORDS],
            run_time: 0,
            penalties: [0; 8],
            clock_running: false,
            clock_start: 0,
            now: 0,
            fsm: u8::from(hud),
            wait: 0,
            wait_nag_armed: false,
            cones: 0,
            contact_latch: false,
            failed: false,
            abandoned: false,
            cones_spoken: false,
            settle_started: false,
            settle_mark: 0,
            settle_window: 0,
            ref_seconds: None,
            exit_pending: false,
            scoring,
            marks: [0; 4],
        }
    }

    /// The loops done (`DAT_005fbe28`).
    pub const fn laps(&self) -> i32 {
        self.lap
    }

    /// The lines are armed (`DAT_005f9e94`).
    pub const fn armed(&self) -> bool {
        self.armed
    }

    /// The race clock's elapsed centiseconds (`FUN_0047fd20(0, 0)`).
    fn elapsed_cs(&self) -> i32 {
        if !self.clock_running {
            return 0;
        }
        let e = self.now - self.clock_start;
        (((e & 0xffff) * 100) >> 16) + (e >> 16) * 100
    }

    /// The normal race start (tasks that do not wait for the throttle).
    pub fn race_start(&mut self) {
        if !self.clock_running {
            self.clock_running = true;
            self.clock_start = self.now;
        }
    }

    /// `FUN_00471ec0(id)`: the reference index of the line on this lap.
    fn reference_index(&self, id: i32) -> i32 {
        let span = (self.lap + 1) * self.lines;
        let mut seen = 0;
        for k in 0..span.max(0) {
            let line = self
                .reference
                .records
                .get(k as usize)
                .map_or(0, |r| r.line as i8);
            if i32::from(line) == id {
                seen += 1;
                if seen == self.lap + 1 {
                    return k;
                }
            }
        }
        span.max(0)
    }

    /// `FUN_00473220`: the crossing is the task's last line.
    fn last_line_reached(&self, k: i32) -> bool {
        if self.lines == 0 {
            return false;
        }
        if self.loop_task {
            k == (self.lap + 1) * (self.lines - 1) - 1
        } else {
            k >= self.lines - 1
        }
    }

    /// `FUN_00471b30(id)`: a line crossed (from the trigger walk).
    pub fn cross(
        &mut self,
        id: i32,
        car: &SchoolCar,
        statics: &mut SchoolStatics,
        rng: &mut GameLcg,
    ) -> Vec<SchoolCue> {
        let mut cues = Vec::new();
        if !self.armed {
            return cues;
        }
        let n = self.lines;
        self.last_line = id;
        let k = self.reference_index(id);
        let clock = self.clock_running;
        if n > 0
            && (k / (self.lap + 1)) % n == 0
            && k > 0
            && self.loop_task
            && !self.laps_done
            && clock
        {
            if self.history[6] == 0 {
                self.wrong_way = true;
                self.history[6] = 1;
                self.lap_penalty = true;
            }
            return cues;
        }
        self.history[6] = 0;
        if self.loop_task {
            if self.laps_done && self.uncrossed.get(id as usize).copied().unwrap_or(false) {
                let in_reference = (1..n.max(1)).any(|i| {
                    self.reference
                        .records
                        .get(i as usize)
                        .is_some_and(|r| i32::from(r.line as i8) == id)
                });
                if in_reference {
                    return cues;
                }
                self.stop_marker();
                self.laps_done = false;
                self.finishing = true;
                return cues;
            }
            if id != statics.lap_latch && clock {
                statics.lap_latch = id;
                self.history.copy_within(0..6, 1);
                self.history[0] = self.reference_index(id);
                self.wrong_way = true;
                if (0..5).any(|j| self.history[j] >= self.history[j + 1]) {
                    self.wrong_way = false;
                } else {
                    self.order_ok = false;
                }
            }
        }
        if !self.uncrossed.get(id as usize).copied().unwrap_or(false) || statics.ring.contains(&id)
        {
            return cues;
        }
        statics.ring.copy_within(0..4, 1);
        statics.ring[0] = id;
        let first = i32::from(self.reference.records.first().map_or(0, |r| r.line as i8));
        if !clock && id != -1 && id != first {
            return cues;
        }
        self.index = self.reference_index(id);
        self.capture(id, car);
        if let Some(flag) = self.uncrossed.get_mut(id as usize) {
            *flag = false;
        }
        if self.last_line_reached(self.index) {
            if self.loop_task {
                if !self.order_ok {
                    self.order_ok = true;
                    self.uncrossed.fill(true);
                    return cues;
                }
                self.lap += 1;
                if self.lap != 4 {
                    self.laps_done = false;
                    self.uncrossed.fill(true);
                    return cues;
                }
                self.near_end_speech(&mut statics.instructor, car, &mut cues);
                self.laps_done = true;
                return cues;
            }
            self.stop_marker();
            self.finishing = true;
        }
        if !self.finishing {
            let errors = self.line_error(self.index);
            if (self.level != 2 || self.task <= 1)
                && clock
                && id != first
                && !(self.level == 0 && self.task == 0)
            {
                self.line_advice(errors, &mut statics.instructor, car, rng, &mut cues);
            }
        }
        cues
    }

    /// `FUN_00472050(id)`: the crossing into the runtime record.
    fn capture(&mut self, id: i32, car: &SchoolCar) {
        let k = self.index.clamp(0, RSF_RECORDS as i32 - 1) as usize;
        let mut heading = signed_ratio_angle(car.forward_xz[0], car.forward_xz[1]).unwrap_or(0);
        heading = heading.rem_euclid(0x800);
        let slide = (qmul(car.velocity[0], car.lateral[0])
            .wrapping_add(qmul(car.velocity[1], car.lateral[1]))
            .wrapping_add(qmul(car.velocity[2], car.lateral[2])))
        .unsigned_abs();
        self.captures[k] = LineRecord {
            position: car.position,
            heading,
            word_3e0: car.word_3e0,
            slide,
            speed: car.speed,
            brake: car.brake,
            handbrake: car.handbrake,
            cones: 0,
            gear: car.gear,
            line: id as u8,
        };
        self.missed = 0;
        let next = self.prev_index + 1;
        if next < self.index {
            for j in next..self.index {
                let j = j as usize;
                self.captures[j] = LineRecord::default();
                if let Some(r) = self.reference.records.get(j)
                    && let Some(flag) = self.uncrossed.get_mut(usize::from(r.line))
                {
                    *flag = false;
                }
                self.missed += 1;
            }
        }
        self.prev_index = self.index;
    }

    /// `FUN_004723b0(k)`: the crossing against the reference, into the
    /// scorer's sums; returns `{speed, direction, sliding, distance, brake,
    /// handbrake, cones}` for the advice.
    fn line_error(&mut self, k: i32) -> [i32; 7] {
        let k = k.clamp(0, RSF_RECORDS as i32 - 1) as usize;
        let a = self.captures[k];
        let b = self.reference.records.get(k).copied().unwrap_or_default();
        let d: [i32; 3] = std::array::from_fn(|i| a.position[i].wrapping_sub(b.position[i]));
        let sq = (0..3).fold(0i32, |s, i| s.wrapping_add(qmul(d[i], d[i])));
        let dist = sqrt_q16(sq as u32) as i32;
        let position = if self.loop_task {
            a.cones - b.cones
        } else {
            dist
        };
        self.penalties[0] = self.penalties[0].wrapping_add(position);
        let mut h = b.heading;
        if (a.heading < 0x200 && h > 0x600) || (a.heading > 0x600 && h < 0x200) {
            h = 0x800 - h;
        }
        let direction = i32::from(a.heading) - i32::from(h);
        self.penalties[1] = self.penalties[1].wrapping_add(direction.abs());
        let speed = a.speed.wrapping_sub(b.speed);
        self.penalties[4] = self.penalties[4].wrapping_add(speed);
        let slide = (a.slide as i32).wrapping_abs() - (b.slide as i32).wrapping_abs();
        self.penalties[2] = self.penalties[2].wrapping_add(slide.abs());
        let (brake, handbrake) = (a.brake - b.brake, a.handbrake - b.handbrake);
        self.penalties[5] = self.penalties[5].wrapping_add(brake.abs() + handbrake.abs());
        [speed, direction, slide, dist, brake, handbrake, position]
    }

    /// `FUN_00473430`: disarm the lines and stop the run's time.
    fn stop_marker(&mut self) {
        self.armed = false;
        self.run_time = self.elapsed_cs();
    }

    /// The controls the finish settle forces this tick (`FUN_004718d0`
    /// before the control map), or `None` while driving.
    pub fn settle_controls(&self, car: &SchoolCar) -> Option<SettleControls> {
        if !self.finishing {
            return None;
        }
        if self.level == 0 && self.task == 0 && !self.settle_started {
            return Some(SettleControls {
                throttle_zero: car.gear == 0,
                engine_zero: car.gear == 0,
                ..SettleControls::default()
            });
        }
        Some(SettleControls {
            throttle_zero: true,
            brake_one: car.speed > 0,
            engine_zero: car.speed <= 0 || car.gear == 7 || car.gear == 0,
            gear_neutral: car.gear == 7,
        })
    }

    /// One state-5 tick after Mechanics: the clock, the settle, the task
    /// FSM `FUN_0046e230` with the instructor's speech.
    pub fn tick(
        &mut self,
        car: &SchoolCar,
        flag_551af0: bool,
        rng: &mut GameLcg,
        statics: &mut SchoolStatics,
    ) -> Vec<SchoolCue> {
        let mut cues = Vec::new();
        let step = ticks_per_second(flag_551af0);
        self.now = self.now.wrapping_add(step);
        if self.finishing && !self.done {
            self.settle(car);
        }
        if self.done {
            return cues;
        }
        let ins = &mut statics.instructor;
        // `FUN_004752e0` (its reference latch is idempotent).
        let over_time = self.over_time();
        let entry = self.fsm;
        match entry {
            0 | 1 | 4 => {
                if entry == 0 {
                    // The wait for the throttle, with the idle nag timed in
                    // its seconds; a timeout nags with `t = 1000`.
                    let limit = match self.level {
                        0 => 0x14_0000,
                        1 => 0xa_0000,
                        _ => 0x2_0000,
                    };
                    if !self.wait_nag_armed {
                        self.idle_nag(ins, 0, true, car, rng, &mut cues);
                        self.wait_nag_armed = true;
                    }
                    self.idle_nag(ins, self.wait >> 16, false, car, rng, &mut cues);
                    self.wait = self.wait.wrapping_add(step);
                    if !self.clock_running {
                        let timed_out = self.wait > limit;
                        if timed_out {
                            self.idle_nag(ins, 1000, false, car, rng, &mut cues);
                            self.idle_nag(ins, 0, true, car, rng, &mut cues);
                        }
                        if timed_out || car.throttle != 0 {
                            self.wait = 0;
                            self.race_start();
                            self.wait_nag_armed = false;
                            self.fsm = 1;
                        }
                    }
                }
                if over_time {
                    self.fsm = 2;
                }
                if entry != 4 && self.laps_done {
                    self.fsm = 4;
                }
                // `FUN_00474c50` latches the contact flag and speaks;
                // `FUN_00475230` counts the latched frames.
                if (self.level != 2 || self.task <= 1) && !self.finishing {
                    self.contact_latch = car.object_contact;
                    self.speech_frame(ins, car, rng, &mut cues);
                }
                let counted = self.loop_task || (self.level == 0 && self.task == 0);
                if entry != 4 && counted {
                    if self.contact_latch {
                        self.cones += 1;
                    }
                    let limit = CONE_LIMIT
                        .get(usize::from(self.level))
                        .copied()
                        .unwrap_or(0);
                    if self.cones > limit {
                        // `5fc078 = 0`: the cone speech is due.
                        self.cones_spoken = false;
                        self.fsm = 3;
                    }
                }
            }
            2 => {
                self.failed = true;
                self.stop_marker();
                self.end(rng);
                // A failed task's end also clears `DAT_005fbef4`.
                ins.fail_phase2 = false;
            }
            3 => {
                self.failed = true;
                if !self.cones_spoken {
                    self.cones_speech(ins, car, rng, &mut cues);
                    self.cones_spoken = true;
                }
                if !Self::instructor_speaking(ins, car) {
                    self.stop_marker();
                    self.end(rng);
                    ins.fail_phase2 = false;
                }
            }
            _ => {}
        }
        cues
    }

    /// `FUN_004752e0`: past the reference time plus the task's slack.
    fn over_time(&mut self) -> bool {
        let reference = *self.ref_seconds.get_or_insert(self.reference.time_cs / 100);
        let slack = TIME_SLACK[score_row(self.level, self.task).min(11)];
        let elapsed = if self.clock_running {
            (self.now - self.clock_start) >> 16
        } else {
            0
        };
        reference - elapsed < -slack
    }

    /// `FUN_004718d0`'s timing: once settled long enough (0.5 s, 1 s from
    /// task 2), end; the stop-in-the-box task first waits to stop.
    fn settle(&mut self, car: &SchoolCar) {
        let now = self.elapsed_cs();
        if !self.settle_started {
            self.settle_mark = now;
            self.settle_started = true;
            self.settle_window = if self.task >= 2 { 100 } else { 50 };
            return;
        }
        if (now - self.settle_mark).unsigned_abs() <= self.settle_window as u32 {
            return;
        }
        if self.task != 0 || self.level != 0 {
            self.end(&mut GameLcg::new(0));
            return;
        }
        if car.speed > 1000 {
            return;
        }
        self.captures[0].position = car.position;
        self.run_time = now;
        self.end(&mut GameLcg::new(0));
    }

    /// The pause menu's abandon (`FUN_00468c70`): every mark 0.
    pub fn abandon(&mut self, rng: &mut GameLcg) {
        self.abandoned = true;
        self.finishing = true;
        self.end(rng);
    }

    /// `FUN_00471810`: the task end (the scorer, then state 9).
    fn end(&mut self, rng: &mut GameLcg) {
        self.score(rng);
        self.done = true;
        if self.loop_task {
            self.lap = 0;
        }
        if self.failed {
            self.abandoned = false;
            self.failed = false;
        }
        self.exit_pending = true;
        self.settle_started = false;
        self.clock_running = false;
        self.laps_done = false;
    }

    /// The marks the scorer left (`cfg[0x6bc..0x6bf]`).
    pub const fn marks(&self) -> [u8; 4] {
        self.marks
    }

    /// `FUN_004725e0`: the penalties (0 best, 10 worst) and the marks
    /// `(10 - penalty) & 0xf`, low nibble first.
    fn score(&mut self, rng: &mut GameLcg) {
        let n = self.lines;
        let mut passed = if self.loop_task { n * 4 - 4 } else { n };
        let setup_grade = (self.level == 2 && self.task > 2).then(|| self.setup_grade());
        let p = &mut self.penalties;
        let row = score_row(self.level, self.task).min(11);
        let novice_box = self.level == 0 && self.task == 0;
        if self.abandoned {
            *p = [10; 8];
        } else {
            p[3] = self.scoring.damage_sum;
            p[6] = 3;
            let missed = if self.loop_task {
                n * 4 - self.index - 5
            } else {
                n - self.index - 1
            };
            passed -= missed;
            let div = |x: i32| if passed == 0 { x } else { x / passed };
            if novice_box {
                let r0 = self.reference.records.first().copied().unwrap_or_default();
                let d: [i32; 3] = std::array::from_fn(|i| {
                    self.captures[0].position[i].wrapping_sub(r0.position[i])
                });
                let sq = (0..3).fold(0i32, |s, i| s.wrapping_add(qmul(d[i], d[i])));
                p[0] = sqrt_q16(sq as u32) as i32;
                let over = (self.run_time - self.reference.time_cs).max(0) / 50;
                let line = self.last_line.clamp(0, RSF_RECORDS as i32 - 1) as usize;
                let faster = (self.captures[line].speed
                    - self.reference.records.get(line).map_or(0, |r| r.speed))
                .max(0)
                    / 10000;
                p[4] = faster + over;
                p[5] =
                    ((p[0] as u32) / (2 * self.scoring.scales[0][0].max(1) as u32)) as i32 + over;
                p[2] = (p[5] + p[4]) / 2;
            } else {
                p[0] = div(p[0]);
                p[4] = div(p[4]);
                p[5] = div(p[5]);
                p[2] = div(p[2]);
            }
            p[1] = div(p[1]);
            // `FUN_00472c60`: past the row's offset, in the row's units.
            for (i, x) in p.iter_mut().take(6).enumerate() {
                let mut v = x.wrapping_abs() - self.scoring.offsets[row][i];
                if v < 0 {
                    v = 0;
                }
                if !(novice_box && (i == 4 || i == 5)) {
                    v /= self.scoring.scales[row][i].max(1);
                }
                *x = v;
            }
            if self.loop_task {
                let c = (self.cones / if self.level == 0 { 2 } else { 1 }).min(5);
                p[0] += c;
                if c > 2 {
                    p[1] += 1;
                }
                p[3] += c;
                if self.lap_penalty {
                    for i in [0, 1, 4, 3, 2, 5] {
                        p[i] += 2;
                    }
                    self.lap_penalty = false;
                }
            }
            if passed < 5 && missed > 0 {
                // Fewer than five lines: random penalties (the native
                // divides by zero when none were missed).
                for i in [0, 1, 2, 4, 5] {
                    p[i] += rng.next_raw() as i32 % missed;
                }
            }
            for x in p.iter_mut() {
                *x = (*x).min(10);
            }
            p[7] = (p[0] + p[1] + p[2] + p[3] + p[4] + p[5] + p[7]) / 7;
            if let Some(grade) = setup_grade {
                p[7] = grade;
            }
        }
        if self.failed {
            for i in [0, 1, 2, 3, 4, 5, 7] {
                if p[i] < 7 {
                    p[i] += 3;
                }
            }
        }
        let mark = |x: i32| (10i32.wrapping_sub(x) & 0xf) as u8;
        self.marks = std::array::from_fn(|b| mark(p[2 * b]) | (mark(p[2 * b + 1]) << 4));
    }

    /// `FUN_00472fa0`: the expert stages' set-up grade from the tyre and
    /// the four forced options (`cfg[0x91..0x99]`).
    fn setup_grade(&self) -> i32 {
        let [tyre, o93, o95, o97, o99] = self.scoring.setup;
        let pick = |t: &[i32; 5], v: u8| t.get(usize::from(v & 7)).copied().unwrap_or(0);
        let mut p = match self.task {
            3 => pick(&SETUP_TYRES_T3, tyre),
            4 => pick(&SETUP_TYRES_T4, tyre),
            _ => return 10,
        };
        p += pick(&SETUP_93, o93)
            + pick(&SETUP_95, o95)
            + pick(&SETUP_97, o97)
            + pick(&SETUP_99, o99);
        10 - p / 5
    }
}

/// What the scorer reads besides the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchoolScoring {
    /// `0x4f4f80`/`0x4f50a0`: six offsets and scales per row.
    pub offsets: [[i32; 6]; 12],
    pub scales: [[i32; 6]; 12],
    /// `Σ 0x50e0d8[0..29]`: the car's damage (sympathy).
    pub damage_sum: i32,
    /// `cfg[0x91]`, `[0x93]`, `[0x95]`, `[0x97]`, `[0x99]`.
    pub setup: [u8; 5],
}

impl SchoolScoring {
    /// The two tables from their static-table blocks (288 bytes each).
    pub fn from_blocks(offsets: &[u8], scales: &[u8]) -> Option<Self> {
        let table = |b: &[u8]| -> Option<[[i32; 6]; 12]> {
            (b.len() >= 288).then(|| {
                std::array::from_fn(|r| {
                    std::array::from_fn(|i| {
                        let o = (r * 6 + i) * 4;
                        i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
                    })
                })
            })
        };
        Some(Self {
            offsets: table(offsets)?,
            scales: table(scales)?,
            damage_sum: 0,
            setup: [0; 5],
        })
    }
}

/// `FUN_0046ee70` (state 4, every school task): the setup options the
/// school forces in the live block, by level and task.
pub fn force_setup(live: &mut [u8], level: u8, task: u8) {
    if (level == 1 && task == 3) || (level == 2 && task == 2) {
        live[0x95] &= 0xf8;
    } else {
        live[0x95] = (live[0x95] & 0xf9) | 1;
    }
    live[0x93] = (live[0x93] & 0xf9) | 1;
    live[0x97] = (live[0x97] & 0xf9) | 1;
    if level == 2 && task == 2 {
        live[0x99] = (live[0x99] & 0xfb) | 3;
    } else {
        live[0x99] = (live[0x99] & 0xfa) | 2;
    }
}

/// `FUN_00474600`'s intro wav: `Intro<k>\<n>.wav` with `k = min(task, 2)`
/// and `n` drawn from 3, 3 or 4 at the load.
pub fn intro_wav(task: u8, rng: &mut GameLcg) -> (u8, u8) {
    let k = task.min(2);
    let count = [3, 3, 4][usize::from(k)];
    (k, (rng.next_raw() % count) as u8)
}

/// `FUN_004732a0`: number the stage's school lines (type 3) in record order;
/// a record repeating the previous one's segment (a line split across
/// cells) takes its number. Returns the line count (`DAT_005fbe18`).
pub fn number_lines(records: &mut [cmr_formats::stage::TriggerVolume]) -> i32 {
    let key = |r: &cmr_formats::stage::TriggerVolume| {
        (r.point_a[0], r.point_a[2], r.point_b[0], r.point_b[2])
    };
    let mut count = 0;
    let mut r = 0;
    while r < records.len() {
        let head = key(&records[r]);
        let mut id = records[r].payload;
        if records[r].raw_type == 3 {
            id = count;
            records[r].payload = id;
            count += 1;
        }
        r += 1;
        while r < records.len() && key(&records[r]) == head {
            records[r].payload = id;
            r += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(lines: &[(u8, i32)], time_cs: i32) -> SchoolReference {
        let mut records = vec![LineRecord::default(); RSF_RECORDS];
        for (k, &(id, x)) in lines.iter().enumerate() {
            records[k] = LineRecord {
                position: [x, 0, 0],
                speed: 0x8_0000,
                line: id,
                ..LineRecord::default()
            };
        }
        SchoolReference {
            time_cs,
            records,
            trailing: Vec::new(),
        }
    }

    fn scoring() -> SchoolScoring {
        SchoolScoring {
            offsets: [[0; 6]; 12],
            scales: [[0x1_0000, 40, 1000, 6500, 2000, 3000]; 12],
            damage_sum: 0,
            setup: [0; 5],
        }
    }

    fn at(x: i32, speed: i32) -> SchoolCar {
        SchoolCar {
            position: [x, 0, 0],
            speed,
            forward_xz: [0, 0x1_0000],
            ..SchoolCar::default()
        }
    }

    fn run(race: &mut SchoolRace, xs: &[(i32, i32)], statics: &mut SchoolStatics) {
        let mut rng = GameLcg::new(1);
        for &(id, x) in xs {
            statics.instructor.begin_frame(false);
            race.cross(id, &at(x, 0x8_0000), statics, &mut rng);
            race.tick(&at(x, 0x8_0000), false, &mut rng, statics);
        }
        for _ in 0..200 {
            if race.done {
                break;
            }
            statics.instructor.begin_frame(false);
            race.tick(&at(xs.last().unwrap().1, 0), false, &mut rng, statics);
        }
    }

    #[test]
    fn a_run_matching_the_reference_scores_full_marks_but_gears() {
        let lines = [(0, 0), (1, 0x10_0000), (2, 0x20_0000)];
        let mut race = SchoolRace::start(0, 2, 3, reference(&lines, 1000), scoring());
        race.race_start();
        let mut statics = SchoolStatics::default();
        run(
            &mut race,
            &[(0, 0), (1, 0x10_0000), (2, 0x20_0000)],
            &mut statics,
        );
        assert!(race.done && race.exit_pending);
        // Every shown mark 10; the hidden gears mark is 7.
        assert_eq!(race.marks(), [0xaa, 0xaa, 0xaa, 0xa7]);
    }

    #[test]
    fn a_line_off_the_reference_costs_position() {
        let lines = [(0, 0), (1, 0x10_0000), (2, 0x20_0000)];
        let mut race = SchoolRace::start(0, 2, 3, reference(&lines, 1000), scoring());
        race.race_start();
        let mut statics = SchoolStatics::default();
        run(
            &mut race,
            &[(0, 0), (1, 0x14_0000), (2, 0x20_0000)],
            &mut statics,
        );
        // Line 1 is 4.0 off: the mean over the lines passed, in whole units.
        assert!(race.marks()[0] & 0xf < 10);
    }

    #[test]
    fn running_out_of_time_fails_with_three_more_penalty_points() {
        let lines = [(0, 0), (1, 0x10_0000), (2, 0x20_0000)];
        let mut race = SchoolRace::start(0, 2, 3, reference(&lines, 100), scoring());
        race.race_start();
        let mut rng = GameLcg::new(1);
        let mut statics = SchoolStatics::default();
        statics.instructor.begin_frame(false);
        race.cross(0, &at(0, 0x8_0000), &mut statics, &mut rng);
        // 1 s of reference plus 40 s of slack.
        for _ in 0..(44 * 25) {
            statics.instructor.begin_frame(false);
            race.tick(&at(0, 0), false, &mut rng, &mut statics);
        }
        assert!(race.done);
        assert_eq!(race.fsm, 2);
        // Every shown penalty below 7 gained 3; the gears mark is left at 7.
        assert!(race.marks()[0] & 0xf <= 7);
        assert_eq!(race.marks()[3] & 0xf, 7);
    }

    #[test]
    fn a_loop_task_waits_for_the_throttle_and_counts_cones() {
        let lines = [(0, 0), (1, 0x10_0000), (2, 0x20_0000)];
        let mut race = SchoolRace::start(1, 0, 3, reference(&lines, 1000), scoring());
        assert!(race.loop_task && !race.hud && race.fsm == 0);
        let mut rng = GameLcg::new(1);
        let mut statics = SchoolStatics::default();
        statics.instructor.begin_frame(false);
        race.tick(&at(0, 0), false, &mut rng, &mut statics);
        assert_eq!(race.fsm, 0, "no throttle yet");
        let mut go = at(0, 0);
        go.throttle = 0xfc00;
        statics.instructor.begin_frame(false);
        race.tick(&go, false, &mut rng, &mut statics);
        assert_eq!(race.fsm, 1);
        let mut hit = go;
        hit.object_contact = true;
        for _ in 0..9 {
            statics.instructor.begin_frame(false);
            race.tick(&hit, false, &mut rng, &mut statics);
        }
        assert_eq!(race.fsm, 3, "more than 8 contact frames");
        statics.instructor.begin_frame(false);
        let cues = race.tick(&hit, false, &mut rng, &mut statics);
        assert!(matches!(cues[..], [SchoolCue::Advice(80..=82)]));
        statics.instructor.begin_frame(false);
        race.tick(&hit, false, &mut rng, &mut statics);
        assert!(race.done, "the speech is over");
    }

    #[test]
    fn abandoning_scores_nothing() {
        let lines = [(0, 0), (1, 0x10_0000)];
        let mut race = SchoolRace::start(0, 2, 2, reference(&lines, 1000), scoring());
        race.abandon(&mut GameLcg::new(1));
        assert_eq!(race.marks(), [0; 4]);
    }

    #[test]
    fn lines_number_in_order_and_split_segments_share_a_number() {
        let line = |x: i32, kind: u8| cmr_formats::stage::TriggerVolume {
            point_a: [x, 0, 0],
            point_b: [x, 0, 0x10_0000],
            normal_q16: [0; 3],
            payload: 77,
            spatial_node_index: 0,
            raw_type: kind,
            event_flag: 0,
            raw_state: 0,
            unknown_2f: 0,
        };
        let mut records = vec![line(0, 3), line(0, 3), line(5, 0), line(9, 3)];
        assert_eq!(number_lines(&mut records), 2);
        let ids: Vec<i32> = records.iter().map(|r| r.payload).collect();
        assert_eq!(ids, [0, 0, 77, 1]);
    }

    #[test]
    fn the_expert_set_up_mark_reads_the_tyre() {
        let mut scoring = scoring();
        // The forced options: 0x93 = 1, 0x95 = 1, 0x97 = 1, 0x99 = 2; tyre 2.
        scoring.setup = [2, 1, 1, 1, 2];
        let race = SchoolRace::start(2, 3, 2, reference(&[(0, 0)], 1000), scoring);
        // (A3[2] + 10 + 6 + 6 + 6) / 5 = 7.
        assert_eq!(10 - race.setup_grade(), 7);
    }
}
