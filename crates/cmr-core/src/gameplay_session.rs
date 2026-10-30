//! The driven top-level session: [`GameSession`] plus the fixed-step race sim,
//! advanced from **injected per-frame controls** and reporting the per-frame
//! render route.
//!
//! [`GameSession`] already runs the recovered FSM from boot to quit, deferring
//! the gameplay band (`5..=0xb`) to a caller handler. This driver supplies that
//! handler: in state `5` (the main driving tick) it steps the deterministic
//! [`RaceState`] with the frame's [`Controls`] and feeds the car position to a
//! caller-owned [`StageProgress`] (the recovered checkpoint tracker) that raises
//! the finish; every in-race transition is the recovered
//! [`race_flow_next`] table. The same driver backs both the headless path (fixed
//! controls each frame) and the windowed path (live controls from the recovered
//! keyboard chain), so they drive identical logic.
//!
//! Presentation is not performed here: each [`advance`](GameplaySession::advance)
//! reports the [`RenderRoute`] of the state whose render handler ran this frame
//! ([`FramePlan::render`]), and the backend presents accordingly. The in-race
//! sequencer flags that are not the player's doing (replay/flyby/fade/quit) are a
//! documented [`InRaceScript`] harness stand-in until those subsystems are wired;
//! only `stage_finished` comes from the live tracker.

use crate::car_reset::ResetDispatchInput;
use crate::event_flow::EventState;
use crate::fixed_point::{Fixed16, Vec3};
use crate::game_menu_flow::MenuFlowInputs;
use crate::game_race_flow::{InRaceInputs, race_flow_next};
use crate::game_session::{GameSession, SessionBand};
use crate::game_state::GameState;
use crate::host_loop::HostClock;
use crate::live_static_collision_car_frame::PreviousCarXz;
use crate::moving_object_collision::MovingObjectHitCache;
use crate::moving_objects::{CompiledObjectGround, FlatGround, MovingObjects};
use crate::race_state::{
    CarState, Controls, FourWheelIterationParams, LiveStaticCollisionStepError, RaceState,
    ResetOutcome, SimParams, WheelPostprocessError,
};
use crate::render_route::{RenderRoute, render_route};
use crate::results_director::{DirectorContext, DirectorEffect, DirectorInput, ResultsDirector};
use crate::screen_fade::ScreenFade;
use crate::stage_checkpoints::StageProgress;
use crate::stage_triggers::{RuntimeTrigger, initialize_runtime_triggers};
use crate::static_collision_response::StaticDamageOwner;
use crate::static_collision_scene_builder::ParsedStaticCollisionScene;
use crate::trigger_walk::{
    TriggerCellOutOfRange, TriggerCells, TriggerEvent, TriggerWalkConfig, link_trigger_cells,
};

/// The in-race sequencer inputs that are not the player's live controls: the
/// replay/flyby/fade/quit flags and the world-ready gate the recovered
/// in-race graph branches on. A **harness stand-in** for the real subsystems
/// (`race_flow` documents each); only `stage_finished` is supplied live, from
/// the checkpoint tracker, and the session kind comes from the
/// [`crate::event_flow::EventState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InRaceScript {
    /// Harness: the front end starts a rally-school session instead of a race
    /// (the session's initial [`EventState`] unless one is supplied).
    pub school_session: bool,
    /// The stage world has finished loading (`stage_world_ready`).
    pub stage_world_ready: bool,
    /// The replay camera is ready (`replay_ready`).
    pub replay_ready: bool,
    /// The replay intro has played out (`replay_intro_done`).
    pub replay_intro_done: bool,
    /// The finish fly-by cinematic has played out (`flyby_cinematic_done`).
    pub flyby_cinematic_done: bool,
    /// A quit was requested from the post-race hub (`quit_requested`).
    pub quit_requested: bool,
    /// The quit fade has completed (`fade_done`).
    pub fade_done: bool,
}

impl InRaceScript {
    /// A player's race session: the stage world is ready, the stage-boundary
    /// director and the results page are treated as done and the fade as
    /// complete, and nothing is quit. Harness stand-ins; the hub's event
    /// advance and welcome screen are the recovered [`EventState`] logic.
    #[must_use]
    pub const fn gameplay_harness() -> Self {
        InRaceScript {
            school_session: false,
            stage_world_ready: true,
            replay_ready: false,
            replay_intro_done: false,
            flyby_cinematic_done: false,
            quit_requested: false,
            fade_done: true,
        }
    }

    /// A rally-school session with every sequencer gate satisfied, so the
    /// school path (`4 -> 7 -> 5 -> 9 -> 0xb -> 0xc`) runs to completion once
    /// the tracker raises the finish. Matches the headless game session.
    #[must_use]
    pub const fn school_harness() -> Self {
        InRaceScript {
            school_session: true,
            stage_world_ready: true,
            replay_ready: true,
            replay_intro_done: true,
            flyby_cinematic_done: true,
            quit_requested: true,
            fade_done: true,
        }
    }

    /// The session the front end starts for this script: a rally-school
    /// session, or a race over the default (championship) config block.
    #[must_use]
    pub fn initial_event(self) -> EventState {
        let mut event = EventState::default();
        if self.school_session {
            event.start_school_session();
        } else {
            event.start_race_session();
        }
        event
    }

    fn with_finish(self, stage_finished: bool, school_session: bool) -> InRaceInputs {
        InRaceInputs {
            school_session,
            stage_world_ready: self.stage_world_ready,
            stage_finished,
            replay_ready: self.replay_ready,
            replay_intro_done: self.replay_intro_done,
            flyby_cinematic_done: self.flyby_cinematic_done,
            quit_requested: self.quit_requested,
            fade_done: self.fade_done,
            ..InRaceInputs::default()
        }
    }
}

/// The car and control words the surface and landing sounds read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceSoundWords {
    /// `ctl +0x42a + 2i`.
    pub surfaces: [i16; 4],
    /// `car +0x73a..+0x741`.
    pub contact: [u8; 8],
    /// `ctl +0x3b0 + 4i` / `ctl +0x3c0 + 4i`.
    pub longitudinal: [i32; 4],
    pub lateral: [i32; 4],
    /// `car +0xec`.
    pub speed_ec: i32,
    /// `ctl +0x404`.
    pub upright_404: bool,
    /// `car +0x680`.
    pub out_of_control_680: bool,
}

/// The pause words the in-race handlers read on the next frame, from the
/// host's [`crate::pause_menu::PauseController`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PauseFlow {
    /// `FUN_00468200` hit in a driving state: the handler returns `0xa`
    /// without running Mechanics.
    pub enter_pause: bool,
    /// State `0xa`: Shift+Q or `DAT_0057e678`.
    pub quit: bool,
    /// State `0xa`: `DAT_0057e680` in real play.
    pub restart: bool,
    /// State `0xa`: `DAT_0057e67c`.
    pub resume_alt: bool,
    /// State `0xa`: `DAT_0057e670` consumed.
    pub resume: bool,
    /// `DAT_00551b28 == 7`.
    pub prepause_was_7: bool,
}

impl PauseFlow {
    fn apply(self, inputs: InRaceInputs) -> InRaceInputs {
        InRaceInputs {
            pause_requested: self.enter_pause,
            quit_requested: inputs.quit_requested || self.quit,
            restart_requested: inputs.restart_requested || self.restart,
            resume_alt: self.resume_alt,
            resume: self.resume,
            prepause_was_7: self.prepause_was_7,
            ..inputs
        }
    }
}

/// What one [`GameplaySession::advance`] produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameplayFrame {
    /// The FSM band the frame dispatched from.
    pub band: SessionBand,
    /// The render route of the state whose render handler ran this frame, or
    /// `None` when the render-skip gate dropped this frame's render.
    pub route: Option<RenderRoute>,
    /// Whether the driving sim stepped this frame (the live state was `5`).
    pub stepped: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaticGameplayAdvanceError<E> {
    Wheel(WheelPostprocessError),
    Collision(LiveStaticCollisionStepError<E>),
}

/// The driven session: the recovered FSM plus the deterministic race sim.
pub struct GameplaySession<'a> {
    session: GameSession,
    race: RaceState,
    params: SimParams,
    script: InRaceScript,
    wheel: Option<FourWheelIterationParams<'a>>,
    ground_geometry: Option<&'a crate::stage_compiled_geometry::StageCompiledGeometry>,
    reset_environment: ResetEnvironment,
    last_reset: ResetOutcome,
    stage_triggers: Option<StageTriggers<'a>>,
    last_trigger_events: Vec<TriggerEvent>,
    /// `ctl +0x304` as the last four-wheel tick read it.
    last_throttle_304: i32,
    /// The static scenery impacts of the last stepped tick.
    last_static_impacts: Vec<crate::race_state::StaticMaterialImpact>,
    /// The pause words for the next frame ([`Self::set_pause_flow`]).
    pause_flow: PauseFlow,
    /// The post-stage results director (state 8).
    results: ResultsHold,
    /// The car as the stage setup left it, restored when a new stage starts.
    stage_start: CarState,
    /// The sim has stepped since the stage started.
    race_started: bool,
    /// The FSM left the gameplay band after a race; the next drive re-inits.
    stage_dirty: bool,
    sim_ticks: u32,
    /// The screen fade as the race uses it.
    fade: RaceFade,
    /// The state whose render handler the last frame planned (`None` when
    /// the render gate skipped it).
    rendered_state: Option<u8>,
    /// The same, kept for the frame's draws.
    render_state: Option<u8>,
    /// The player's setup block (`0x50d950`) with its damage fractions,
    /// the wet-brake flag `+0xbb4`, the gearbox's unscaled ratio copies
    /// (`ctl +0x210/+0x230`), and a finish awaiting the damage write-back
    /// (`FUN_00420db0` at `0x437cd9`).
    car_setup: crate::car_setup::CarSetupBlock,
    wet_brakes: bool,
    gear_copies: Option<([i32; 8], [i32; 8])>,
    damage_writeback: bool,
    /// The player car's crash damage (parts and setup-block words), the
    /// impacts' breakage cues for the host, and whether the live meshes
    /// changed since the host last copied them.
    car_damage: Option<crate::car_crash_damage::CarDamage>,
    breakage_cues: Vec<(crate::car_crash_damage::BreakageKind, [i32; 3])>,
    damage_mesh_dirty: bool,
    /// `DAT_005fbe8c`: the last impact's severity (the school reads it).
    last_severity_5fbe8c: i32,
    /// Car `+0x760`: a penalty reset ran this stage (only ever set), which
    /// turns the crash damage off.
    car_760: bool,
    /// The rally school's task (`FUN_004716b0` at the stage load) and the
    /// words that outlive it.
    school: Option<SchoolTask>,
    school_statics: crate::school_race::SchoolStatics,
    /// Dynamic trackside objects (`.obj` cones/signs/markers, `FUN_00461090`),
    /// their stage-start snapshot, and their per-car collision hit cache.
    moving_objects: Option<MovingObjects>,
    moving_objects_initial: Option<MovingObjects>,
    object_hit_cache: MovingObjectHitCache,
    /// `DAT_00552f68`: whether `FUN_0043f790` recorded an object contact this tick.
    last_object_contact_552f68: bool,
    /// `DAT_00558fb0`: the last static or moving-object hit's surface material byte.
    last_contact_material_558fb0: u8,
    /// The game's `rand()` (`DAT_0050b690`) as the session draws on it; the
    /// host copies its generator in before an advance and back after.
    rng: crate::game_rng::GameLcg,
}

/// The rally school's task in a session: the in-race logic, the intro
/// (state 7) and exit (state 9) phases, and what the host plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchoolTask {
    pub race: crate::school_race::SchoolRace,
    /// 0 not started, 1 requested, 2 playing, 3 done.
    intro: u8,
    /// 0 waiting for the fade (`5ef458`), 1 speech due (`5ef45c`), 2
    /// requested, 3 playing, 4 done.
    exit: u8,
    /// The host's instructor channel is playing.
    pub speaking: bool,
    cues: Vec<SchoolHostCue>,
    marks_out: Option<[u8; 4]>,
}

/// What the school asks the host to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchoolHostCue {
    /// `FUN_00474cb0`: the intro wav chosen at load.
    IntroSpeech,
    /// `FUN_00474cd0`: the exit wav by the marks' total.
    ExitSpeech(u8),
    /// A line's or the task's cue.
    Task(crate::school_race::SchoolCue),
}

impl SchoolTask {
    /// `FUN_00474cd0`'s pick: the total of the eight nibbles.
    fn exit_wav(marks: [u8; 4], rng: &mut crate::game_rng::GameLcg) -> u8 {
        let total: u32 = marks
            .iter()
            .map(|&b| u32::from(b & 0xf) + u32::from(b >> 4))
            .sum();
        if total > 75 {
            7
        } else if total > 70 {
            4 + (rng.next_raw() % 3) as u8
        } else if total > 40 {
            1 + (rng.next_raw() % 3) as u8
        } else {
            0
        }
    }
}

/// The static pass's damage owner (`FUN_0043d760` -> `FUN_0041f630`):
/// each new contact dents the car in pass order and hands the detach's
/// kind back as the effect handle the pass cues with. With the damage off
/// the kind is not written (the caller's zero).
struct CrashDamageOwner<'a, D> {
    inner: &'a mut D,
    school: bool,
    damage: Option<&'a mut crate::car_crash_damage::CarDamage>,
    /// Per committed contact: its impact table (with the damage on) and its
    /// severity.
    impacts: Vec<(Option<[i32; crate::car_crash_damage::MAX_PARTS]>, i32)>,
}

impl<D: StaticDamageOwner> StaticDamageOwner for CrashDamageOwner<'_, D> {
    type Error = D::Error;

    fn type_one_effect_gate_blocked(&mut self) -> Result<bool, Self::Error> {
        self.inner.type_one_effect_gate_blocked()
    }

    fn capture_debug_material(&self) -> bool {
        self.school || self.inner.capture_debug_material()
    }

    fn commit_contact(
        &mut self,
        request: crate::static_collision_response::StaticDamageRequest,
    ) -> Result<crate::static_collision_response::DamageResult, Self::Error> {
        let result = self.inner.commit_contact(request)?;
        let severity = request.severity.raw();
        let Some(damage) = self.damage.as_deref_mut() else {
            self.impacts.push((None, severity));
            return Ok(result);
        };
        let c = request.contact_3f0;
        damage.dent([c.x.raw(), c.y.raw(), c.z.raw()], severity);
        let kind = damage.detach();
        self.impacts.push((Some(damage.impact_864), severity));
        Ok(crate::static_collision_response::DamageResult {
            effect_handle: kind.map_or(0, |kind| kind as i32),
        })
    }
}

/// The fade channel `0x82b37c` as the race drives it
/// (`.work/race-fade-notes.md`): the stage load leaves a one-frame fade out,
/// state 8 cuts its finish shots in through black, the exits to the hub
/// fade out, the hub waits for the fade and then for the display shutdown
/// its render performs. The school's intro and replay fades (states 7, 9)
/// are not reproduced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaceFade {
    pub fade: ScreenFade,
    /// `DAT_00bbabec`: every fade is instant.
    pub instant: bool,
    /// `DAT_00551af0`: the fades take 25 frames instead of 12.
    pub flag_551af0: bool,
    /// `ctx+0x428 && ctx+0x10`: the race display is up (the load creates
    /// it, state `0xb`'s render shuts it down).
    display_up: bool,
    /// Harness: state 8's finish shots started (the camera sequence
    /// `DAT_006e2d5c` is taken to play on from then; its player is not
    /// reconstructed, so later shot changes do not fade).
    shots_started: bool,
}

/// What the compositor does with the fade on one rendered frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeComposite {
    /// No fade in progress: the frame as composed.
    Clear,
    /// The black quad of this alpha over the frame (`FUN_00423170`).
    Quad(u8),
    /// State `0xb`'s idle frame: the display is shut down and nothing is
    /// composited (`FUN_00423420` -> `FUN_004992e0`).
    Shutdown,
}

impl RaceFade {
    fn count(&self) -> u32 {
        crate::screen_fade::race_fade_count(self.flag_551af0)
    }

    fn fade_out(&mut self) {
        self.fade.fade_out(self.count(), self.instant);
    }

    /// `FUN_0048df80(N)`.
    fn fade_in_race(&mut self) {
        self.fade.fade_in(self.count(), self.instant);
    }

    /// State 4's load (`FUN_00421da0` tail): `FUN_0044e980`'s fade in is
    /// replaced at once by a one-frame fade out (`0x422724`), so the first
    /// four composited frames are black; the display is up.
    fn load(&mut self) {
        self.fade.fade_out(1, self.instant);
        self.display_up = true;
        self.shots_started = false;
    }

    /// State 8's cut through black (`0x43822d`): with the fade idle and no
    /// shot playing, a visible screen fades out and a black one starts the
    /// shots and fades in.
    fn finish_shots(&mut self) {
        if !self.fade.idle() || self.shots_started {
            return;
        }
        if self.fade.evaluate() != 0 {
            self.fade_out();
        } else {
            self.shots_started = true;
            self.fade.fade_in(self.count(), self.instant);
        }
    }

    /// The fades a transition starts: out to the hub from 6, 8 and 0xa (its
    /// quit and restart; SHIFT+Q is not modelled), and the head-to-head
    /// reload from the hub.
    fn transition(&mut self, from: u8, to: u8) {
        match (from, to) {
            (6 | 8 | 0xA, 0xB) => self.fade_out(),
            (0xB, 5) => self.load(),
            _ => {}
        }
    }
}

/// The stage's `.dat` records with their per-player state and cell lists, and
/// the cell centres `FUN_0048fd20` selects the car's cell from.
struct StageTriggers<'a> {
    runtime: Vec<RuntimeTrigger>,
    /// The freshly loaded state (`FUN_00469630` clears `+0x2e` at load).
    initial: Vec<RuntimeTrigger>,
    cells: TriggerCells,
    cell_centers: &'a [Vec3],
    config: TriggerWalkConfig,
}

/// The words `FUN_00467210` reads besides the car and the `.cat` cursor.
///
/// [`ResetEnvironment::single_player_harness`] is the harness choice: the
/// race-start init `FUN_00437510` sets `DAT_0057b0ac[player] = 100` on both
/// of its branches, so a running race passes the gate; the mode bits
/// `DAT_00d3ca80 +0x166/+0x168` and the loop offset `DAT_0057b030` are not
/// recovered and are zero (the plain mode); the comparison car is car record
/// 1, which the setup clear at `0x404431` zeroes and a single-player race is
/// inferred to leave at the origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetEnvironment {
    pub race_state_57b0ac: i32,
    pub replay_d3ca88: bool,
    pub config_166: u8,
    pub config_168: u8,
    pub loop_offset_57b030: i32,
    pub comparison_xz: [i32; 2],
}

impl ResetEnvironment {
    /// The harness defaults described on the type.
    #[must_use]
    pub const fn single_player_harness() -> Self {
        Self {
            race_state_57b0ac: crate::car_reset::RACE_RUNNING_57B0AC,
            replay_d3ca88: false,
            config_166: 0,
            config_168: 0,
            loop_offset_57b030: 0,
            comparison_xz: [0, 0],
        }
    }
}

/// The post-stage results director and the host's input for its next tick.
#[derive(Clone, Debug, Default)]
struct ResultsHold {
    director: Option<ResultsDirector>,
    input: DirectorInput,
    effects: Vec<DirectorEffect>,
    /// The save dialog between directors (its cursor is a process global).
    save: crate::save_dialog::SaveDialog,
    /// The record pages' globals between directors.
    record_globals: crate::results_director::RecordGlobals,
    /// The record image a finished director handed back, until the host
    /// takes it.
    records_out: Option<crate::results_director::RecordRun>,
}

impl ResultsHold {
    /// State 8's update `FUN_00438100` before its exit test: the special
    /// event skips the results (`DAT_00514478`, clearing `DAT_0055001c`);
    /// otherwise the director ticks (`FUN_00430770`, which runs its init
    /// lazily when the finish did not), and the state holds while it runs.
    fn boundary(&mut self, event: &mut EventState, inputs: &mut InRaceInputs) {
        if inputs.school_session {
            return;
        }
        if event.globals.special_514478 {
            event.globals.special_55001c = false;
            inputs.boundary_skip = true;
            return;
        }
        let input = std::mem::take(&mut self.input);
        if self.director.is_none() {
            let director = ResultsDirector::start(event, DirectorContext::default());
            self.director = Some(self.carried(director));
        }
        let director = self.director.as_mut().expect("just started");
        self.effects.extend(director.tick(event, input));
        inputs.boundary_active = event.globals.results_550024;
        if !inputs.boundary_active {
            // Continue, restart and quit clear the init flag `DAT_00550020`.
            if let Some(mut director) = self.director.take() {
                self.keep(&mut director);
            }
        }
    }

    /// A new director with what the last one left (the save dialog and the
    /// record globals).
    fn carried(&mut self, director: ResultsDirector) -> ResultsDirector {
        director
            .with_save_dialog(std::mem::take(&mut self.save))
            .with_record_globals(std::mem::take(&mut self.record_globals))
    }

    /// Keep what a finished director leaves for the next one and the host.
    fn keep(&mut self, director: &mut ResultsDirector) {
        self.save = director.take_save_dialog();
        let (records, globals) = director.take_records();
        self.record_globals = globals;
        if records.is_some() {
            self.records_out = records;
        }
    }
}

/// The school's gates in states 5, 7 and 9. The intro and exit
/// demonstration replays (`.rsp`) are not reconstructed, so the binary's
/// path without them runs: state 7 plays the intro wav and waits for it
/// (`0x437f3a`); state 9 waits out the task end's fade, fades in at black,
/// plays the exit wav by the total and waits for it (`0x4384f2`).
fn school_gates(
    state: u8,
    task: &mut SchoolTask,
    fade: &mut RaceFade,
    rng: &mut crate::game_rng::GameLcg,
    inputs: &mut InRaceInputs,
) {
    match state {
        5 => {
            // The ordinary race start runs the clock of the tasks that do
            // not wait for the throttle.
            if task.race.fsm == 1 {
                task.race.race_start();
            }
            inputs.stage_finished = task.race.done;
            inputs.replay_ready = task.race.done;
        }
        7 => {
            match task.intro {
                0 => {
                    task.cues.push(SchoolHostCue::IntroSpeech);
                    task.intro = 1;
                }
                1 => task.intro = 2,
                2 if !task.speaking => task.intro = 3,
                _ => {}
            }
            inputs.replay_intro_done = task.intro == 3;
        }
        9 => {
            match task.exit {
                0 if fade.fade.idle() => {
                    if fade.fade.evaluate() == 0 {
                        task.race.hud = false;
                        fade.fade_in_race();
                    } else {
                        task.exit = 1;
                    }
                }
                1 => {
                    let wav = SchoolTask::exit_wav(task.race.marks(), rng);
                    task.cues.push(SchoolHostCue::ExitSpeech(wav));
                    task.exit = 2;
                }
                2 => task.exit = 3,
                3 if !task.speaking => task.exit = 4,
                _ => {}
            }
            inputs.flyby_cinematic_done = task.exit == 4;
        }
        _ => {}
    }
}

/// One gameplay-band handler: the recovered race-flow table over the script,
/// the pause flow, the results director and the live finish. A restart
/// chosen in the pause menu sets `DAT_00550018` on its way to the hub
/// (`0x4389fa`).
#[allow(clippy::too_many_arguments)]
fn in_race_next(
    state: GameState,
    event: &mut EventState,
    pause: PauseFlow,
    script: InRaceScript,
    finished: bool,
    results: &mut ResultsHold,
    fade: &mut RaceFade,
    finish_writeback: &mut bool,
    school: Option<&mut SchoolTask>,
    rng: &mut crate::game_rng::GameLcg,
) -> GameState {
    let mut inputs = pause.apply(script.with_finish(finished, event.globals.school_d3ca88));
    if let Some(task) = school.filter(|_| inputs.school_session) {
        school_gates(state.id(), task, fade, rng, &mut inputs);
    }
    inputs.fade_done = fade.fade.idle();
    inputs.results_page_up = fade.display_up;
    if state.id() == 0xA
        && !inputs.quit_requested
        && !inputs.school_session
        && inputs.restart_requested
    {
        event.globals.restart_550018 = true;
    }
    if state.id() == 8 {
        results.boundary(event, &mut inputs);
        if !inputs.school_session && !event.globals.special_514478 {
            fade.finish_shots();
        }
    }
    let next = race_flow_next(state, inputs, event).unwrap_or(state);
    // State 5's finish (`0x437cd9`): the damage goes back to the record.
    if !inputs.school_session && state.id() == 5 && next.id() == 8 {
        *finish_writeback = true;
    }
    // The school's task ends in state 9 (`0x4385f7`): its results screen
    // comes up when state 2 runs again.
    if inputs.school_session && state.id() == 9 && next.id() == 0xB {
        event.cfg.school_6c0 = (event.cfg.school_6c0 & 0xfc) | 4;
    }
    fade.transition(state.id(), next.id());
    next
}

impl<'a> GameplaySession<'a> {
    /// Replace the session's config block and globals (the front end's
    /// commit), before the first frame.
    #[must_use]
    pub fn with_event(mut self, event: EventState) -> Self {
        self.session = GameSession::with_event(event);
        self
    }

    /// The event bookkeeping and session flags.
    #[must_use]
    pub const fn event(&self) -> &EventState {
        self.session.event()
    }

    /// The event bookkeeping and session flags.
    pub fn event_mut(&mut self) -> &mut EventState {
        self.session.event_mut()
    }

    /// A fresh simplified session at the boot state with the car at rest.
    #[must_use]
    pub fn new(params: SimParams, script: InRaceScript) -> Self {
        Self::with_optional_wheel(params, script, None)
    }

    /// A simplified session with an explicit stage start.
    #[must_use]
    pub fn new_with_car(params: SimParams, script: InRaceScript, car: CarState) -> Self {
        let mut session = Self::new(params, script);
        session.race = RaceState::new(car);
        session.stage_start = car;
        session
    }

    /// A fresh session driven by the complete four-wheel producer.
    #[must_use]
    pub fn new_with_four_wheel(
        params: SimParams,
        script: InRaceScript,
        wheel: FourWheelIterationParams<'a>,
    ) -> Self {
        Self::with_optional_wheel(params, script, Some(wheel))
    }

    /// A complete four-wheel session with an explicit recovered/harness start.
    #[must_use]
    pub fn new_with_four_wheel_and_car(
        params: SimParams,
        script: InRaceScript,
        wheel: FourWheelIterationParams<'a>,
        car: CarState,
    ) -> Self {
        let mut session = Self::with_optional_wheel(params, script, Some(wheel));
        session.race = RaceState::new(car);
        session.stage_start = car;
        session
    }

    fn with_optional_wheel(
        params: SimParams,
        script: InRaceScript,
        wheel: Option<FourWheelIterationParams<'a>>,
    ) -> Self {
        GameplaySession {
            session: GameSession::with_event(script.initial_event()),
            race: RaceState::new(CarState::at_rest()),
            params,
            script,
            wheel,
            ground_geometry: None,
            reset_environment: ResetEnvironment::single_player_harness(),
            last_reset: ResetOutcome::default(),
            stage_triggers: None,
            last_trigger_events: Vec::new(),
            last_throttle_304: 0,
            last_static_impacts: Vec::new(),
            pause_flow: PauseFlow::default(),
            results: ResultsHold::default(),
            stage_start: CarState::at_rest(),
            race_started: false,
            stage_dirty: false,
            sim_ticks: 0,
            fade: RaceFade::default(),
            rendered_state: None,
            render_state: None,
            car_setup: crate::car_setup::CarSetupBlock::RESET,
            wet_brakes: false,
            gear_copies: None,
            damage_writeback: false,
            car_damage: None,
            breakage_cues: Vec::new(),
            damage_mesh_dirty: false,
            last_severity_5fbe8c: 0,
            car_760: false,
            school: None,
            school_statics: crate::school_race::SchoolStatics::default(),
            moving_objects: None,
            moving_objects_initial: None,
            object_hit_cache: MovingObjectHitCache::default(),
            last_object_contact_552f68: false,
            last_contact_material_558fb0: 0,
            rng: crate::game_rng::GameLcg::new(1),
        }
    }

    /// Whether this session steps the complete four-wheel producer, whose car
    /// pose lives in `MotionState::basis` rather than the planar heading.
    #[must_use]
    pub fn is_four_wheel(&self) -> bool {
        self.wheel.is_some()
    }

    /// Run the per-car ground pass `FUN_00410a60` against this stage geometry
    /// after every four-wheel step whose car carries live ground contact.
    #[must_use]
    pub fn with_ground_geometry(
        mut self,
        geometry: &'a crate::stage_compiled_geometry::StageCompiledGeometry,
    ) -> Self {
        self.ground_geometry = Some(geometry);
        self
    }

    /// Replace the reset dispatcher's environment words.
    #[must_use]
    pub fn with_reset_environment(mut self, environment: ResetEnvironment) -> Self {
        self.reset_environment = environment;
        self
    }

    /// The HUD words `FUN_00464130` prepares from the four-wheel car, or
    /// `None` for the planar harness (which has no engine or gearbox).
    #[must_use]
    pub fn prepared_hud_values(
        &self,
        metric_units: bool,
        classic_gauge: bool,
    ) -> Option<crate::race_hud_prepare::PreparedHudValues> {
        let wheel = self.wheel.as_ref()?;
        let car = self.race.car();
        Some(crate::race_hud_prepare::prepare_hud_values(
            crate::race_hud_prepare::HudPrepareInput {
                engine_speed_2e8: car.four_wheel.engine_speed_2e8,
                redline_15c: wheel.engine_smoothing.redline_15c,
                speed_ec: car.four_wheel.speed_ec,
                gear_43a: car.four_wheel.gearbox.gear_43a,
                metric_units,
                classic_gauge,
            },
        ))
    }

    /// The stage's compiled ground (`.hpc`), when attached.
    #[must_use]
    pub fn ground_geometry(
        &self,
    ) -> Option<&'a crate::stage_compiled_geometry::StageCompiledGeometry> {
        self.ground_geometry
    }

    /// The four-wheel car's setup wheel words `+0x800..+0x80c`, or `None`
    /// for the planar harness.
    #[must_use]
    pub fn wheel_setup_800(&self) -> Option<[i32; 4]> {
        self.wheel.as_ref().map(|wheel| wheel.wheel_setup_800)
    }

    /// What the reset dispatcher did on the last four-wheel tick.
    #[must_use]
    pub fn last_reset(&self) -> ResetOutcome {
        self.last_reset
    }

    /// Attach the stage's dynamic trackside objects (`FUN_00461090`).
    pub fn set_moving_objects(&mut self, objects: MovingObjects) {
        self.moving_objects_initial = Some(objects.clone());
        self.moving_objects = Some(objects);
        self.object_hit_cache.clear();
        self.last_object_contact_552f68 = false;
    }

    /// The live dynamic trackside objects (`FUN_00461090`), if attached.
    #[must_use]
    pub const fn moving_objects(&self) -> Option<&MovingObjects> {
        self.moving_objects.as_ref()
    }

    /// The `.obj` scene-graph `(node_index, local_matrix_q16)` overrides from
    /// any dynamic trackside objects that have moved (`FUN_00452b70`).
    #[must_use]
    pub fn moving_object_node_locals(&self) -> Vec<(usize, [i32; 16])> {
        self.moving_objects
            .as_ref()
            .map_or_else(Vec::new, |o| o.node_locals().collect())
    }

    /// Run the trigger walk `FUN_00403a00` each four-wheel tick over these
    /// `.dat` records (linked into cells by `FUN_004904e0`) with the harness
    /// [`TriggerWalkConfig::single_player_harness`] words.
    ///
    /// # Errors
    ///
    /// A record names a cell outside `cell_centers`.
    pub fn with_stage_triggers(
        mut self,
        volumes: &[cmr_formats::stage::TriggerVolume],
        cell_centers: &'a [Vec3],
    ) -> Result<Self, TriggerCellOutOfRange> {
        let runtime = initialize_runtime_triggers(volumes);
        self.stage_triggers = Some(StageTriggers {
            initial: runtime.clone(),
            runtime,
            cells: link_trigger_cells(volumes, cell_centers.len())?,
            cell_centers,
            config: TriggerWalkConfig::single_player_harness(),
        });
        Ok(self)
    }

    /// The car and control words the engine sound `FUN_00488d60` reads after
    /// this tick's Mechanics, or `None` for the planar harness. `car +0x764`
    /// (the reset-branch ignition flag) has no recovered writer in this
    /// session and reads as zero.
    #[must_use]
    pub fn engine_sound_car(&self) -> Option<crate::engine_sound::EngineSoundCar> {
        let wheel = self.wheel.as_ref()?;
        let car = self.race.car();
        let four = &car.four_wheel;
        Some(crate::engine_sound::EngineSoundCar {
            engine_speed_2e8: four.engine_speed_2e8,
            redline_reciprocal_160: crate::drivetrain::reciprocal_or_minus_one(
                wheel.engine_smoothing.redline_15c,
            ),
            speed_ec: four.speed_ec,
            gear_43a: four.gearbox.gear_43a,
            throttle_response_2d8: four.pedals.response_2d8,
            throttle_pedal_304: self.last_throttle_304,
            reset_timer_75c: four.trigger_walk.penalty_timer_75c,
            idle_6e8: four.idle_6e8,
            idle_countdown_6ec: four.idle_countdown_6ec,
            ignition_764: 0,
        })
    }

    /// The pause words for the next frame only.
    pub fn set_pause_flow(&mut self, flow: PauseFlow) {
        self.pause_flow = flow;
    }

    /// The results init `FUN_0042f860`, which the finish handoff
    /// `FUN_00443c80` runs at the finish line (single player, not the
    /// school, a demo or a network session), with the standings computed
    /// there.
    /// `records` is the host's record image for the record checks (none:
    /// they are skipped).
    pub fn start_results(
        &mut self,
        context: DirectorContext,
        records: Option<crate::results_director::RecordRun>,
    ) {
        if let Some(mut last) = self.results.director.take() {
            self.results.keep(&mut last);
        }
        let mut director = ResultsDirector::start(self.session.event_mut(), context);
        if let Some(records) = records {
            director = director.with_records(records);
        }
        self.results.director = Some(self.results.carried(director));
    }

    /// The record image the last finished director handed back (with any
    /// record its checks wrote), once.
    pub fn take_record_image(&mut self) -> Option<crate::results_director::RecordRun> {
        self.results.records_out.take()
    }

    /// The host's answer to the save dialog's slot read.
    pub fn provide_save_slots(&mut self, files: &[Option<Vec<u8>>]) {
        if let Some(director) = &mut self.results.director {
            director.provide_save_slots(files);
        }
    }

    /// The results director's input for its next tick (`FUN_00430980`).
    pub fn set_results_input(&mut self, input: DirectorInput) {
        self.results.input = input;
    }

    /// The running (or last) results director.
    #[must_use]
    pub fn results(&self) -> Option<&ResultsDirector> {
        self.results.director.as_ref()
    }

    /// The results director's cues since the last call.
    pub fn take_results_effects(&mut self) -> Vec<DirectorEffect> {
        std::mem::take(&mut self.results.effects)
    }

    /// The words the surface and landing sounds (`FUN_00489890`,
    /// `FUN_0046a030`) read after Mechanics, or `None` without ground contact.
    #[must_use]
    pub fn surface_sound_words(&self) -> Option<SurfaceSoundWords> {
        let car = self.race.car();
        let ground = car.ground?;
        let four = &car.four_wheel;
        Some(SurfaceSoundWords {
            surfaces: std::array::from_fn(|wheel| ground.lanes.type_words[wheel] as i16),
            contact: ground.thresholds.flags_73a,
            longitudinal: std::array::from_fn(|wheel| four.second[wheel].ratio_at_3b0),
            lateral: four.lateral_dot_3c0,
            speed_ec: four.speed_ec,
            upright_404: four.upright_404,
            out_of_control_680: car.collision.response_state_680,
        })
    }

    /// The car and control words the wheel particles read this tick
    /// (`FUN_00428360`/`FUN_00425280`), or `None` before the car has a
    /// ground state.
    pub fn wheel_particle_car(&self) -> Option<crate::car_particles::WheelCar> {
        let car = self.race.car();
        let ground = car.ground?;
        let four = &car.four_wheel;
        let raw = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        Some(crate::car_particles::WheelCar {
            velocity: raw(car.velocity),
            speed: four.speed_ec,
            probes: ground.probes,
            up: raw(car.motion.basis[1]),
            contact: car.motion.contact_distance_flags,
            flags_73a: std::array::from_fn(|w| ground.thresholds.flags_73a[w]),
            wheel_speed: std::array::from_fn(|w| four.first[w].wheel_speed),
            head: four.head_vectors_380.map(raw),
            longitudinal: std::array::from_fn(|w| four.second[w].ratio_at_3b0),
            lateral: four.lateral_dot_3c0,
            surfaces: std::array::from_fn(|w| ground.lanes.type_words[w] as i16),
            gear: four.gearbox.gear_43a,
            upright: four.upright_404,
            normals: std::array::from_fn(|w| ground.lanes.output_vectors[w]),
        })
    }

    /// Write back the surface words the surface sound rewrites into the
    /// control record (`ctl +0x42a`).
    pub fn set_surface_words(&mut self, surfaces: [i16; 4]) {
        self.race.set_surface_words(surfaces);
    }

    /// Write back the engine sound's `car +0x6ec`.
    pub fn set_idle_countdown_6ec(&mut self, countdown: i16) {
        self.race.set_idle_countdown_6ec(countdown);
    }

    /// The tumbling impact of the last stepped tick's ground pass
    /// (`FUN_00412860`).
    #[must_use]
    pub fn last_tumble_impact(&self) -> Option<crate::race_state::TumbleImpact> {
        self.race.last_tumble_impact()
    }

    /// The static scenery impacts of the last stepped tick (`FUN_0043d760`).
    #[must_use]
    pub fn last_static_impacts(&self) -> &[crate::race_state::StaticMaterialImpact] {
        &self.last_static_impacts
    }

    /// The trigger walk's effects on the last four-wheel tick.
    #[must_use]
    pub fn last_trigger_events(&self) -> &[TriggerEvent] {
        &self.last_trigger_events
    }

    /// The driver and integrator of one four-wheel mechanics tick.
    fn four_wheel_drive(
        &mut self,
        controls: &Controls,
    ) -> Result<crate::race_state::TraceSample, WheelPostprocessError> {
        let wheel = self.wheel.as_ref().expect("a four-wheel session");
        self.race.step_with_all_four_wheels(controls, wheel)
    }

    /// The rest of the mechanics tick after the driver and the static-scene
    /// collision: the trigger walk `FUN_00403a00` (`0x43b327`), then per car
    /// the ground pass `FUN_00410a60` and the reset `FUN_00467210`
    /// (`0x43b50a`, `0x43b517`).
    fn four_wheel_tail(
        &mut self,
        controls: &Controls,
        progress: &mut StageProgress,
        sample: &mut crate::race_state::TraceSample,
    ) -> Result<(), WheelPostprocessError> {
        let wheel = self.wheel.as_ref().expect("a four-wheel session");
        self.last_reset = ResetOutcome::default();
        self.last_trigger_events.clear();
        self.last_throttle_304 = controls.raw_pedals.throttle_304;
        if let Some(triggers) = self.stage_triggers.as_mut() {
            let events = self
                .race
                .run_trigger_walk(
                    &mut triggers.runtime,
                    &triggers.cells,
                    triggers.cell_centers,
                    &triggers.config,
                )
                .map_err(WheelPostprocessError::TriggerWalk)?;
            for event in &events {
                if let TriggerEvent::NodeCursor { payload, .. } = *event
                    && let Ok(cursor) = usize::try_from(payload)
                {
                    progress.set_cursor(cursor);
                }
            }
            self.last_trigger_events = events;
        }
        // Dynamic trackside object physics (`FUN_00462330` -> `FUN_00450d40`,
        // `0x43b358`) runs after the trigger walk and before the per-car ground
        // pass.
        if let Some(objects) = self.moving_objects.as_mut() {
            let dt = wheel.lane_time_step.raw();
            let trig = self.race.trig();
            if let Some(geometry) = self.ground_geometry {
                let mut ground = CompiledObjectGround(geometry.provider());
                objects.update(&mut ground, dt, trig);
            } else {
                let mut ground = FlatGround(0);
                objects.update(&mut ground, dt, trig);
            }
        }
        // The render-spin block (`0x43b3a4`) runs before the per-car loop,
        // whose order is `FUN_00417e20`, the ground pass (ending in
        // `FUN_00410350`), `FUN_00415df0` and the reset.
        self.race.animate_wheel_spin(controls, wheel);
        self.race.animate_wheel_wobble(wheel);
        if let Some(geometry) = self.ground_geometry {
            let mut config = |_lane: usize| Ok::<u8, std::convert::Infallible>(0);
            self.race.run_ground_pass(
                wheel.lane_time_step,
                &mut geometry.eight_lane_providers(),
                &mut config,
            )?;
            self.race.animate_suspension(wheel);
            let environment = self.reset_environment;
            let dispatch = ResetDispatchInput {
                nodes: progress.nodes(),
                cursor: i32::try_from(progress.cursor()).unwrap_or(i32::MAX),
                loop_offset_57b030: environment.loop_offset_57b030,
                race_state_57b0ac: environment.race_state_57b0ac,
                replay_d3ca88: environment.replay_d3ca88,
                config_166: environment.config_166,
                config_168: environment.config_168,
                comparison_xz: environment.comparison_xz,
                position: Vec3::ZERO,
                row0: Vec3::ZERO,
            };
            let outcome = self.race.run_reset(
                &dispatch,
                &mut geometry.eight_lane_providers(),
                &mut config,
            )?;
            if let Some(cursor) = outcome.cursor.and_then(|c| usize::try_from(c).ok()) {
                progress.set_cursor(cursor);
            }
            self.last_reset = outcome;
        }
        // The barrier walls' and the ground pass's `FUN_0041f630` calls ran
        // before this tick's reset could set `+0x760`.
        self.apply_tail_impacts();
        self.car_760 |= self.last_reset.penalty_done_760;
        sample.car = self.race.car();
        Ok(())
    }

    /// Advance a simplified session. Four-wheel sessions use [`Self::try_advance`]
    /// to preserve checked producer failures.
    pub fn advance<C: HostClock>(
        &mut self,
        clock: &C,
        menu_inputs: MenuFlowInputs,
        controls: Controls,
        progress: &mut StageProgress,
    ) -> GameplayFrame {
        assert!(
            self.wheel.is_none(),
            "four-wheel sessions must use try_advance"
        );
        self.try_advance(clock, menu_inputs, controls, progress)
            .expect("the simplified race step is infallible")
    }

    /// A new stage starts when driving resumes after the FSM left the gameplay
    /// band: the stage load re-creates the car from its setup, reloads the
    /// trigger records and restarts the checkpoint tracker.
    fn begin_stage_if_reentered(&mut self, progress: &mut StageProgress) {
        if self.session.current().id() != 5 || !self.stage_dirty {
            return;
        }
        self.race = RaceState::new(self.stage_start);
        if let Some(triggers) = self.stage_triggers.as_mut() {
            triggers.runtime.clone_from(&triggers.initial);
        }
        if let Some(initial) = self.moving_objects_initial.as_ref() {
            self.moving_objects = Some(initial.clone());
        }
        self.object_hit_cache.clear();
        self.last_object_contact_552f68 = false;
        self.last_contact_material_558fb0 = 0;
        progress.restart();
        self.last_reset = ResetOutcome::default();
        self.last_trigger_events.clear();
        self.car_760 = false;
        self.stage_dirty = false;
    }

    /// Note when a race hands back to the menu band.
    fn track_stage_end(&mut self, stepped: bool) {
        self.race_started |= stepped;
        if self.race_started && (1..=4).contains(&self.session.current().id()) {
            self.stage_dirty = true;
            self.race_started = false;
        }
    }

    /// Advance one frame, preserving checked four-wheel producer failures. When
    /// already in state 5, the driving step completes before the FSM frame, so an
    /// error does not advance the FSM, stage progress, or simulation tick count.
    ///
    /// # Errors
    ///
    /// Returns contact-index, candidate lookup, or geometric postprocess failures.
    pub fn try_advance<C: HostClock>(
        &mut self,
        clock: &C,
        menu_inputs: MenuFlowInputs,
        controls: Controls,
        progress: &mut StageProgress,
    ) -> Result<GameplayFrame, WheelPostprocessError> {
        self.begin_stage_if_reentered(progress);
        let mut controls = controls;
        if self.school.is_some() && self.session.current().id() == 5 {
            self.school_pre_drive(&mut controls);
        }
        let pause = std::mem::take(&mut self.pause_flow);
        let driving_sample = if self.session.current().id() == 5 && !pause.enter_pause {
            Some(if self.wheel.is_some() {
                let race_before = self.race.clone();
                let mut sample = self.four_wheel_drive(&controls)?;
                if let Err(error) = self.four_wheel_tail(&controls, progress, &mut sample) {
                    self.race = race_before;
                    return Err(error);
                }
                sample
            } else {
                self.race.step(&controls, &self.params)
            })
        } else {
            None
        };
        if driving_sample.is_some() {
            self.wet_tick(&controls);
            self.school_after_drive(&controls, progress);
        }
        let script = self.script;
        let results = &mut self.results;
        let sim_ticks = &mut self.sim_ticks;
        let fade = &mut self.fade;
        let writeback = &mut self.damage_writeback;
        let school = &mut self.school;
        let rng = &mut self.rng;
        let before = self.session.current().id();
        let mut stepped = false;

        let frame = self.session.run_frame(clock, menu_inputs, |state, event| {
            if state.id() == 5
                && let Some(sample) = driving_sample
            {
                *sim_ticks += 1;
                progress.update(sample.car.position, sample.centiseconds);
                stepped = true;
            }
            in_race_next(
                state,
                event,
                pause,
                script,
                progress.finished(),
                results,
                fade,
                writeback,
                school.as_mut(),
                rng,
            )
        });
        if before == 4 && self.session.current().id() != 4 {
            self.fade.load();
        }
        self.rendered_state = frame
            .plan
            .and_then(|plan| plan.render)
            .map(|state| state.id());
        self.render_state = self.rendered_state;
        self.track_stage_end(stepped);

        Ok(GameplayFrame {
            band: frame.band,
            route: frame.plan.and_then(|plan| plan.render).map(render_route),
            stepped,
        })
    }

    /// Advance state 5 through integration and the recovered static-scene pass
    /// before checkpoint progress observes the car. A collision failure restores
    /// the complete pre-step race snapshot and leaves the FSM/progress/tick count
    /// unchanged.
    pub fn try_advance_with_static_collision<C: HostClock, D: StaticDamageOwner>(
        &mut self,
        clock: &C,
        menu_inputs: MenuFlowInputs,
        controls: Controls,
        progress: &mut StageProgress,
        parsed: &ParsedStaticCollisionScene,
        damage_owner: &mut D,
    ) -> Result<GameplayFrame, StaticGameplayAdvanceError<D::Error>> {
        self.begin_stage_if_reentered(progress);
        let mut controls = controls;
        if self.school.is_some() && self.session.current().id() == 5 {
            self.school_pre_drive(&mut controls);
        }
        let pause = std::mem::take(&mut self.pause_flow);
        let driving_sample = if self.session.current().id() == 5 && !pause.enter_pause {
            let race_before = self.race.clone();
            let previous_center_xz = PreviousCarXz::capture(race_before.car().position);
            let four_wheel = self.wheel.is_some();
            let mut sample = if four_wheel {
                self.four_wheel_drive(&controls)
                    .map_err(StaticGameplayAdvanceError::Wheel)?
            } else {
                self.race.step(&controls, &self.params)
            };
            let enabled = self.damage_enabled();
            if let Some(objects) = self.moving_objects.as_mut() {
                objects.rebucket(&parsed.grid);
            }
            let school_material_capture = self.session.event().globals.school_d3ca88;
            let mut owner = CrashDamageOwner {
                inner: damage_owner,
                school: school_material_capture,
                damage: self.car_damage.as_mut().filter(|_| enabled),
                impacts: Vec::new(),
            };
            let result = self.race.resolve_static_collision_after_step(
                previous_center_xz,
                parsed,
                &mut owner,
            );
            let committed = std::mem::take(&mut owner.impacts);
            match result {
                Ok(step) => {
                    self.commit_static_damage(&committed, &step.contact_effects);
                    self.last_static_impacts = step.impacts;
                    if let Some(mat) = step.debug_material_558fb0 {
                        self.last_contact_material_558fb0 = mat;
                    }
                }
                Err(error) => {
                    self.race = race_before;
                    return Err(StaticGameplayAdvanceError::Collision(error));
                }
            }
            self.last_object_contact_552f68 = false;
            if let Some(mut objects) = self.moving_objects.take() {
                let mass_48 = self
                    .wheel
                    .as_ref()
                    .map_or(Fixed16::from_raw(1200 << 16), |w| {
                        w.angular.momentum.moment_scale
                    });
                let double_rate = self.fade.flag_551af0;
                let mut hit_cache = std::mem::take(&mut self.object_hit_cache);
                let mut obj_owner = CrashDamageOwner {
                    inner: damage_owner,
                    school: school_material_capture,
                    damage: self.car_damage.as_mut().filter(|_| enabled),
                    impacts: Vec::new(),
                };
                let obj_result = self.race.resolve_moving_object_collision_after_step(
                    previous_center_xz,
                    parsed,
                    &mut objects,
                    &mut hit_cache,
                    double_rate,
                    mass_48,
                    &mut obj_owner,
                );
                let obj_committed = std::mem::take(&mut obj_owner.impacts);
                self.moving_objects = Some(objects);
                self.object_hit_cache = hit_cache;
                match obj_result {
                    Ok((obj_step, object_contact)) => {
                        self.commit_static_damage(&obj_committed, &obj_step.contact_effects);
                        self.last_static_impacts.extend(obj_step.impacts);
                        if let Some(mat) = obj_step.debug_material_558fb0 {
                            self.last_contact_material_558fb0 = mat;
                        }
                        self.last_object_contact_552f68 = object_contact;
                    }
                    Err(error) => {
                        self.race = race_before;
                        return Err(StaticGameplayAdvanceError::Collision(error));
                    }
                }
            }
            sample.car = self.race.car();
            if four_wheel && let Err(error) = self.four_wheel_tail(&controls, progress, &mut sample)
            {
                self.race = race_before;
                return Err(StaticGameplayAdvanceError::Wheel(error));
            }
            Some(sample)
        } else {
            None
        };
        if driving_sample.is_some() {
            self.wet_tick(&controls);
            self.school_after_drive(&controls, progress);
        }
        let script = self.script;
        let results = &mut self.results;
        let sim_ticks = &mut self.sim_ticks;
        let fade = &mut self.fade;
        let writeback = &mut self.damage_writeback;
        let school = &mut self.school;
        let rng = &mut self.rng;
        let before = self.session.current().id();
        let mut stepped = false;

        let frame = self.session.run_frame(clock, menu_inputs, |state, event| {
            if state.id() == 5
                && let Some(sample) = driving_sample
            {
                *sim_ticks += 1;
                progress.update(sample.car.position, sample.centiseconds);
                stepped = true;
            }
            in_race_next(
                state,
                event,
                pause,
                script,
                progress.finished(),
                results,
                fade,
                writeback,
                school.as_mut(),
                rng,
            )
        });
        if before == 4 && self.session.current().id() != 4 {
            self.fade.load();
        }
        self.rendered_state = frame
            .plan
            .and_then(|plan| plan.render)
            .map(|state| state.id());
        self.render_state = self.rendered_state;
        self.track_stage_end(stepped);

        Ok(GameplayFrame {
            band: frame.band,
            route: frame.plan.and_then(|plan| plan.render).map(render_route),
            stepped,
        })
    }

    /// `FUN_00420c70` at the stage load: the player's carried damage (the
    /// record from `+0x9a`, whose repaired parts are cleared in place) into
    /// the setup block, the handling and the gear ratios of the four-wheel
    /// car. The planar harness has nothing to set up.
    ///
    /// # Errors
    ///
    /// [`crate::car_setup::CarSetupError`] when the ratio factor has no
    /// reciprocal.
    pub fn race_start_damage(
        &mut self,
        record: &mut crate::car_setup::SetupConfigBytes,
    ) -> Result<(), crate::car_setup::CarSetupError> {
        let Some(wheel) = self.wheel.as_mut() else {
            return Ok(());
        };
        let Some(gearbox) = wheel.gearbox.as_mut() else {
            return Ok(());
        };
        let (ratios, inverses) = *self
            .gear_copies
            .get_or_insert((gearbox.tables.ratio_1d0, gearbox.tables.inverse_1f0));
        let (block, gears) = crate::car_damage::race_start(
            record,
            wheel.pedal_response.throttle_max_148,
            ratios,
            inverses,
        )?;
        gearbox.tables.ratio_1d0 = gears.ratio_1d0;
        gearbox.tables.inverse_1f0 = gears.inverse_1f0;
        crate::race_state::populate_car_setup(wheel, &block);
        self.car_setup = block;
        self.wet_brakes = false;
        Ok(())
    }

    /// The setup block with this race's damage fractions.
    #[must_use]
    pub const fn car_setup(&self) -> &crate::car_setup::CarSetupBlock {
        &self.car_setup
    }

    /// The stage load's part setup (`FUN_0041efe0`) for the player car:
    /// fresh parts, no deformation.
    pub fn attach_car_damage(&mut self, mut damage: crate::car_crash_damage::CarDamage) {
        damage.clear_impacts();
        self.car_damage = Some(damage);
        self.damage_mesh_dirty = false;
        self.breakage_cues.clear();
    }

    /// The player car's crash damage.
    #[must_use]
    pub const fn car_damage(&self) -> Option<&crate::car_crash_damage::CarDamage> {
        self.car_damage.as_ref()
    }

    /// Whether the live part meshes changed since the last call (the host
    /// then writes them into the model it draws).
    pub fn take_damage_mesh_dirty(&mut self) -> bool {
        std::mem::take(&mut self.damage_mesh_dirty)
    }

    /// The breakage cues (`FUN_0046a2a0`'s kind and the world point) since
    /// the last call.
    pub fn take_breakage_cues(&mut self) -> Vec<(crate::car_crash_damage::BreakageKind, [i32; 3])> {
        std::mem::take(&mut self.breakage_cues)
    }

    /// `FUN_00474da0`'s damage total: the parts' deformation but part 12.
    #[must_use]
    pub fn damage_total(&self) -> i32 {
        self.car_damage.as_ref().map_or(0, |damage| {
            damage.deform_788[..damage.count]
                .iter()
                .enumerate()
                .filter(|&(i, _)| i != 0xc)
                .fold(0i32, |sum, (_, &d)| sum.wrapping_add(d))
        })
    }

    /// Whether `FUN_0041f630` runs the damage: `cfg[0x26] & 0x11` clear and
    /// no penalty run (`car +0x760`).
    fn damage_enabled(&self) -> bool {
        self.session.event().cfg.effects_026 & 0x11 == 0 && !self.car_760
    }

    /// `FUN_0041f630`'s tail for one impact of `severity` at the car-local
    /// `contact_3f0`: with the damage on, the dents, the detach and its cue
    /// at `point`, and the handling re-derived (`FUN_00420740`); then the
    /// school's severity hook.
    pub fn crash_impact(&mut self, contact_3f0: [i32; 3], severity: i32, point: [i32; 3]) {
        if self.damage_enabled()
            && let Some(damage) = self.car_damage.as_mut()
        {
            damage.dent(contact_3f0, severity);
            if let Some(kind) = damage.detach() {
                self.breakage_cues.push((kind, point));
            }
            self.damage_mesh_dirty = true;
            let table = damage.impact_864;
            let indices = damage.layout.handling_cc1;
            self.rederive_handling(&table, indices);
        }
        if self.session.event().globals.school_d3ca88 {
            self.last_severity_5fbe8c = severity;
        }
    }

    /// The barrier walls' impacts (`0x402558`, gated by `+0x6e0` where the
    /// walk builds them) then the ground pass's landing or tumbling impact
    /// (`0x411fb2`/`0x413990`, skipped with `cfg[0x26] & 2`), in that order.
    fn apply_tail_impacts(&mut self) {
        let barriers: Vec<_> = self
            .last_trigger_events
            .iter()
            .filter_map(|event| match *event {
                TriggerEvent::Barrier {
                    impact: Some(impact),
                    ..
                } => Some((impact.contact_3f0, impact.severity_vector, impact.point)),
                _ => None,
            })
            .collect();
        let ground = self
            .race
            .last_tumble_impact()
            .filter(|_| self.session.event().cfg.effects_026 & 2 == 0)
            .map(|impact| (impact.contact_3f0, impact.severity_vector, impact.point));
        for (contact, vector, point) in barriers.into_iter().chain(ground) {
            let severity = crate::prang_sound::impact_severity(vector);
            self.crash_impact(contact, severity, point);
        }
    }

    /// The static pass's committed contacts in order: the handling for each
    /// (`FUN_00420740`), the school's severity hook, and the pass's
    /// breakage cues.
    fn commit_static_damage(
        &mut self,
        impacts: &[(Option<[i32; crate::car_crash_damage::MAX_PARTS]>, i32)],
        contact_effects: &[(i32, [i32; 3])],
    ) {
        let indices = self
            .car_damage
            .as_ref()
            .map(|damage| damage.layout.handling_cc1);
        for (table, severity) in impacts {
            if let (Some(table), Some(indices)) = (table, indices) {
                self.damage_mesh_dirty = true;
                self.rederive_handling(table, indices);
            }
            if self.session.event().globals.school_d3ca88 {
                self.last_severity_5fbe8c = *severity;
            }
        }
        for &(handle, point) in contact_effects {
            let kind = match handle {
                1 => crate::car_crash_damage::BreakageKind::Headlight,
                2 => crate::car_crash_damage::BreakageKind::Screen,
                3 => crate::car_crash_damage::BreakageKind::Mirror,
                _ => continue,
            };
            self.breakage_cues.push((kind, point));
        }
    }

    /// `FUN_00420740` after an impact: the fractions raised by the impact
    /// table, the handling and gear ratios re-derived.
    fn rederive_handling(&mut self, table: &[i32], indices: [u8; 6]) {
        let Some(wheel) = self.wheel.as_mut() else {
            return;
        };
        let Some(gearbox) = wheel.gearbox.as_mut() else {
            return;
        };
        let (ratios, inverses) = *self
            .gear_copies
            .get_or_insert((gearbox.tables.ratio_1d0, gearbox.tables.inverse_1f0));
        let Ok((block, gears)) = crate::car_setup::derive_car_setup(
            self.car_setup,
            crate::car_setup::SetupDamage {
                table_864: table,
                indices_cc1: indices,
            },
            wheel.pedal_response.throttle_max_148,
            ratios,
            inverses,
        ) else {
            return;
        };
        gearbox.tables.ratio_1d0 = gears.ratio_1d0;
        gearbox.tables.inverse_1f0 = gears.inverse_1f0;
        crate::race_state::populate_car_setup(wheel, &block);
        self.car_setup = block;
    }

    /// A finish since the last call: the host writes the damage back to the
    /// record (`FUN_00420db0`).
    pub fn take_damage_writeback(&mut self) -> bool {
        std::mem::take(&mut self.damage_writeback)
    }

    /// The game's `rand()` the session draws on.
    pub fn rng_mut(&mut self) -> &mut crate::game_rng::GameLcg {
        &mut self.rng
    }

    /// `FUN_004716b0`: the school task the stage load starts.
    pub fn start_school_task(&mut self, race: crate::school_race::SchoolRace) {
        // `FUN_00473540`'s tail: the advice words rearmed.
        self.school_statics
            .instructor
            .start_task(race.level, race.task);
        self.school = Some(SchoolTask {
            race,
            intro: 0,
            exit: 0,
            speaking: false,
            cues: Vec::new(),
            marks_out: None,
        });
    }

    /// `DAT_005fbe18`: the loaded stage's school lines (their numbers are
    /// `0..count`).
    #[must_use]
    pub fn school_line_count(&self) -> i32 {
        self.stage_triggers.as_ref().map_or(0, |triggers| {
            triggers
                .runtime
                .iter()
                .filter(|t| t.volume().raw_type == 3)
                .map(|t| t.volume().payload + 1)
                .max()
                .unwrap_or(0)
        })
    }

    /// The school task, if one runs.
    #[must_use]
    pub const fn school_task(&self) -> Option<&SchoolTask> {
        self.school.as_ref()
    }

    /// Whether the host's instructor channel is playing (states 7 and 9
    /// wait on it).
    pub fn set_school_speaking(&mut self, speaking: bool) {
        if let Some(task) = self.school.as_mut() {
            task.speaking = speaking;
        }
    }

    /// The speech and cues the school asked for since the last call.
    pub fn take_school_cues(&mut self) -> Vec<SchoolHostCue> {
        self.school
            .as_mut()
            .map(|task| std::mem::take(&mut task.cues))
            .unwrap_or_default()
    }

    /// The marks of a task that just ended, for `cfg[0x6bc..0x6bf]`.
    pub fn take_school_marks(&mut self) -> Option<[u8; 4]> {
        self.school.as_mut().and_then(|task| task.marks_out.take())
    }

    /// The car words the school reads, including moving-object contact
    /// (`DAT_00552f68`) and the last contact surface byte (`DAT_00558fb0`).
    fn school_car(&self, progress: Option<&StageProgress>) -> crate::school_race::SchoolCar {
        let car = self.race.car();
        let four = &car.four_wheel;
        let raw = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        let surfaces = car.ground.map_or([0; 4], |ground| {
            std::array::from_fn(|w| ground.lanes.type_words[w] as i16)
        });
        crate::school_race::SchoolCar {
            position: raw(car.position),
            velocity: raw(car.velocity),
            speed: four.speed_ec,
            forward_xz: [car.motion.basis[0].x.raw(), car.motion.basis[0].z.raw()],
            lateral: raw(car.motion.basis[2]),
            word_3e0: four.steering.angle_3e0,
            brake: four.pedals.brake_response_2ec,
            handbrake: four.pedals.handbrake_torque_2f4,
            throttle: 0,
            gear: four.gearbox.gear_43a as u8,
            object_contact: self.last_object_contact_552f68,
            contact_surface: self.last_contact_material_558fb0 as i8,
            brake_demand: 0,
            handbrake_demand: 0,
            analog_throttle: false,
            surfaces,
            crashed: car.collision.response_state_680,
            damage: self.damage_total(),
            severity: self.last_severity_5fbe8c,
            cat_remaining: progress
                .map_or(i32::MAX, |p| p.nodes().len() as i32 - p.cursor() as i32),
            reverse_count_5: progress.is_some_and(|p| p.reverse_crossings() == 5),
        }
    }

    /// Before a state-5 drive: the walk's school words and the finish
    /// settle's control words (`FUN_004718d0` in place of the control map).
    fn school_pre_drive(&mut self, controls: &mut Controls) {
        let school = self.session.event().globals.school_d3ca88;
        let armed = self.school.as_ref().is_some_and(|t| t.race.armed());
        if let Some(triggers) = self.stage_triggers.as_mut() {
            triggers.config.school = school;
            triggers.config.school_lines_armed = armed;
        }
        let car = self.school_car(None);
        if let Some(settle) = self
            .school
            .as_ref()
            .and_then(|t| t.race.settle_controls(&car))
        {
            if settle.throttle_zero {
                controls.raw_pedals.throttle_304 = 0;
                controls.forward_force = Fixed16::ZERO;
            }
            if settle.brake_one {
                controls.raw_pedals.brake_308 = 1;
                controls.handbrake = true;
            }
        }
    }

    /// After a state-5 drive: the crossings the walk saw, then the task.
    fn school_after_drive(&mut self, controls: &Controls, progress: &StageProgress) {
        let mut car = self.school_car(Some(progress));
        car.throttle = controls.raw_pedals.throttle_304;
        car.brake_demand = controls.raw_pedals.brake_308;
        car.handbrake_demand = i32::from(controls.handbrake);
        car.analog_throttle = controls.raw_pedals.throttle_analog_414;
        let lines: Vec<i32> = self
            .last_trigger_events
            .iter()
            .filter_map(|e| match *e {
                TriggerEvent::SchoolLine { line, .. } => Some(line),
                _ => None,
            })
            .collect();
        let Some(task) = self.school.as_mut() else {
            return;
        };
        let was_done = task.race.done;
        self.school_statics.instructor.begin_frame(task.speaking);
        for line in lines {
            let cues = task
                .race
                .cross(line, &car, &mut self.school_statics, &mut self.rng);
            task.cues.extend(cues.into_iter().map(SchoolHostCue::Task));
        }
        let cues = task
            .race
            .tick(&car, false, &mut self.rng, &mut self.school_statics);
        task.cues.extend(cues.into_iter().map(SchoolHostCue::Task));
        if task.race.done && !was_done {
            task.marks_out = Some(task.race.marks());
            // `FUN_00471810`'s fade out (12 frames).
            self.fade.fade_out();
        }
    }

    /// `FUN_00420260` after a four-wheel tick: water wets the brakes and
    /// damages the electrics; the brake scales go back to the sim.
    fn wet_tick(&mut self, controls: &Controls) {
        let Some(wheel) = self.wheel.as_mut() else {
            return;
        };
        let car = self.race.car();
        let Some(ground) = car.ground else {
            return;
        };
        let raw = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        let input = crate::car_damage::WetInputs {
            super_special: !self.session.event().globals.school_d3ca88
                && self.session.event().super_special(),
            initialised: true,
            contact_count: car.motion.contact_count_674,
            surfaces: std::array::from_fn(|w| ground.lanes.type_words[w] as i16),
            up: raw(car.motion.basis[1]),
            normal: raw(car.motion.projection_axis),
            velocity: raw(car.velocity),
            brake_308: controls.raw_pedals.brake_308,
        };
        crate::car_damage::wet_tick(&mut self.car_setup, &mut self.wet_brakes, &input);
        wheel.brake_balance_e1a4 = [self.car_setup.words_84c[2], self.car_setup.words_84c[3]];
    }

    /// The race's fade channel and its options.
    pub fn race_fade_mut(&mut self) -> &mut RaceFade {
        &mut self.fade
    }

    /// The state whose render handler this frame's advance planned.
    #[must_use]
    pub const fn render_state(&self) -> Option<u8> {
        self.render_state
    }

    /// The compositor's fade step for this frame's render (`FUN_00422ca0`
    /// runs `FUN_00423170` first): when the planned render is one of states
    /// 5..=0xb a fade in progress advances and draws its quad; state `0xb`'s
    /// idle frame shuts the display down instead. Call once per frame, after
    /// the advance.
    pub fn composite_fade(&mut self) -> FadeComposite {
        let Some(state) = self.rendered_state.take() else {
            return FadeComposite::Clear;
        };
        if !(5..=0xB).contains(&state) {
            return FadeComposite::Clear;
        }
        if self.fade.fade.idle() {
            if state == 0xB && self.fade.display_up {
                self.fade.display_up = false;
                return FadeComposite::Shutdown;
            }
            return FadeComposite::Clear;
        }
        FadeComposite::Quad(crate::screen_fade::overlay_alpha(self.fade.fade.evaluate()))
    }

    /// Report the planned render's overrun to the driver's render-skip gate.
    pub fn report_render(&mut self, overran: bool) {
        self.session.report_render(overran);
    }

    /// The top-level FSM session (driver, event state, session flags).
    #[must_use]
    pub const fn game_session(&self) -> &GameSession {
        &self.session
    }

    /// Carry an FSM session over from the previous stage's driving session,
    /// as the stage load replaces only the stage and the car.
    pub fn replace_game_session(&mut self, session: GameSession) {
        self.session = session;
    }

    /// Whether the session has drained to the terminal stopped state.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.session.is_stopped()
    }

    /// The live FSM state id.
    #[must_use]
    pub fn state(&self) -> GameState {
        self.session.current()
    }

    /// The current car state (position/heading/velocity), for a follow camera.
    #[must_use]
    pub fn car(&self) -> CarState {
        self.race.car()
    }

    /// The stage clock (Q16.16 seconds) the race sim has accumulated.
    #[must_use]
    pub fn stage_clock(&self) -> Fixed16 {
        self.race.clock()
    }

    /// Total driving-sim ticks stepped so far.
    #[must_use]
    pub fn sim_ticks(&self) -> u32 {
        self.sim_ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c3d_terrain_cull::CellGrid;
    use crate::fixed_point::Vec3;
    use crate::host_loop::HostClock;
    use crate::live_static_collision_car_frame::ActiveCellRecordBuildError;
    use crate::static_collision_kernel::NoStaticDamageOwner;
    use crate::static_world_collision::{StaticCollisionScene, StaticNarrowObb};
    use cmr_formats::stage::CatNode;

    struct StationaryClock;
    impl HostClock for StationaryClock {
        fn millis(&self) -> u32 {
            0
        }
    }

    fn line() -> Vec<CatNode> {
        [10i32, 20, 40]
            .into_iter()
            .map(|x| CatNode {
                position: [x << 16, 0, 0],
                direction: [1 << 16, 0, 0],
                split_flag: 0b1,
                reserved: 0,
                seq_index: 0,
            })
            .collect()
    }

    fn empty_collision_scene(cell_count: i32) -> ParsedStaticCollisionScene {
        ParsedStaticCollisionScene {
            grid: CellGrid {
                width: cell_count.max(1),
                height: 1,
                cell_count,
                cell_size: 0x20_0000,
                min_x: 0,
                half_cell: 0x10_0000,
                max_z: 0,
            },
            main_cell_centers: vec![Vec3::ZERO],
            scene: StaticCollisionScene::<StaticNarrowObb>::new(Vec::new(), []).unwrap(),
        }
    }

    fn params() -> SimParams {
        SimParams {
            inverse_mass: Fixed16::ONE,
            gravity: Vec3::ZERO,
        }
    }

    /// Every pre-race wait satisfied: the welcome screen and the school menu
    /// are done.
    fn frontend_menu() -> MenuFlowInputs {
        MenuFlowInputs {
            welcome_done: true,
            school_menu_done: true,
            ..MenuFlowInputs::default()
        }
    }

    fn forward() -> Controls {
        Controls {
            steer_rate: 0,
            forward_force: Fixed16::from_raw(8 << 16),
            handbrake: false,
            raw_steer: Default::default(),
            raw_pedals: Default::default(),
            raw_gear_command_314: 0,
        }
    }

    fn four_wheel_params(trig: &[i32]) -> FourWheelIterationParams<'_> {
        let inactive = crate::vehicle_dynamics::GeometricPostprocessLane {
            active: false,
            vector_a: Vec3::ZERO,
            vector_b: Vec3::ZERO,
            vector_c: Vec3::ZERO,
            accumulator: Vec3::ZERO,
        };
        FourWheelIterationParams {
            first: [crate::race_state::FirstWheelParams {
                split: 0x10000,
                wheel_constant: 0,
                wheel_vector: [0; 3],
                state_vector: [0; 3],
                scale_c600: 0x10000,
                limit: 0x10000,
                shift: 0,
                factor_c5c0: 0x10000,
                gear: 1,
                config_40c: 0,
                flag_43b: 0,
                config_158: 1,
                gear_ratio: 1,
                is_front_wheel: true,
            }; 4],
            second: std::array::from_fn(|_| crate::race_state::SecondWheelParams {
                axis_dot: 0,
                local_68: [0x10000, 0, 0],
                local_98: [0; 3],
                wheel_e8: 0,
                scale_c5c4: 0x10000,
                residual_flag_147a: false,
                config_2ec: 1,
                gear_ratio: 1,
                coefficient_34: 0x10000,
                arctan_table: &crate::legacy_math::SECOND_WHEEL_ARCTAN_TABLE,
                cos_table: trig,
                config_scale: 1,
                transition_gain: 0,
                transition_base: 0,
                alternate_gain: 0,
                alternate_base: 0,
                correction_gate: 1,
                accumulation_limit: 0x10000,
                correction_alternate: 0x10000,
                surface_vector: [0; 3],
                table_factor: None,
            }),
            contact_vector_indices: [2, 3, 6, 7],
            contact_offsets: [Vec3::ZERO; 4],
            contact_update_inputs: [crate::vehicle_dynamics::ContactVectorUpdateInput {
                update_gate: 0,
                enabled: false,
                vector: Vec3::ZERO,
                basis: Vec3::ZERO,
                scale_528: Fixed16::ONE,
                extra_vector: Vec3::ZERO,
                limit: Fixed16::ONE,
                threshold: Fixed16::ONE,
            }; 8],
            lane_threshold_inputs: crate::compiled_car_initializer::CompiledLaneThresholdInputs {
                lane_words: [0; 8],
                source_y: [0x199a; 8],
            },
            pedal_response: crate::pedal_response::PedalResponseParams::default(),
            lane_time_step: Fixed16::ZERO,
            lane_direction_angle: 0,
            steering: crate::steering_response::SteeringParams::default(),
            steering_car_factor_e188: 0x1_0000,
            steering_surface_factors: [0; 2],
            gearbox: None,
            brake_balance_e1a4: [0x1_0000; 2],
            tyre: None,
            hull_contact_offsets: [Vec3::ZERO; 4],
            // Harness value: never crest.
            crest_128_12c: [i32::MAX, 0],
            suspension: crate::wheel_animation::SuspensionConstants::default(),
            wheel_setup_800: [0; 4],
            lane_rebuild: false,
            lane_exchange: false,
            postprocess: crate::race_state::FourWheelPostprocessParams {
                lanes: [inactive; 4],
                current_lane: 0,
                argument_scale: Fixed16::ONE,
                base_force: Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
                non_wheel_forces: &[],
                wheel_contacts: [true; 4],
            },
            inverse_mass_4c: Fixed16::ONE,
            angular: crate::vehicle_dynamics::AngularOrientationInput {
                momentum: crate::vehicle_dynamics::AngularMomentumStepInput {
                    prior_moment: Vec3::ZERO,
                    contact_moment: Vec3::ZERO,
                    moment_scale: Fixed16::ONE,
                    inertia_scale: Vec3::new(Fixed16::ONE, Fixed16::ONE, Fixed16::ONE),
                    angular_velocity: Vec3::ZERO,
                    time_step: Fixed16::ZERO,
                },
                clamp_enabled: false,
                control: crate::vehicle_dynamics::ControlAxisFlags {
                    mode_3ec: 0,
                    axis_3f4: 0,
                    axis_3f8: 0,
                },
                basis: [Vec3::ZERO; 3],
            },
            engine_smoothing: crate::drivetrain::SmoothedEngineSpeedInput {
                wheel_speeds: [0; 4],
                config_158: 1,
                gear_ratio: 0x10000,
                engine_2e0: 0,
                redline_15c: 0x10000,
                slip_gain: 0,
                engine_load: 0x10000,
            },
            driven_smoothing: crate::race_state::DrivenWheelUpdateParams::default(),
            engine_force: crate::drivetrain::EngineForceInput {
                limiter_444: false,
                cut_448: 0,
                engine_2e0: 0,
                redline_15c: 0x10000,
                gear: 1,
                config_40c: 0,
                response_2d8: 0,
                response_e1ac: 0,
                engine_brake_144: 0,
                throttle_304: 0,
            },
        }
    }

    #[test]
    fn four_wheel_session_steps_and_preserves_checked_errors() {
        let trig = crate::fixed_point::build_sine_table();
        let wheel = four_wheel_params(&trig);
        let mut session =
            GameplaySession::new_with_four_wheel(params(), InRaceScript::school_harness(), wheel);
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        for _ in 0..1000 {
            session
                .try_advance(&StationaryClock, frontend_menu(), forward(), &mut progress)
                .unwrap();
            if session.sim_ticks() != 0 {
                break;
            }
        }
        assert_eq!(session.sim_ticks(), 1);

        let mut invalid = four_wheel_params(&trig);
        invalid.contact_vector_indices[0] = 8;
        let mut session =
            GameplaySession::new_with_four_wheel(params(), InRaceScript::school_harness(), invalid);
        let mut progress = StageProgress::new(&nodes, 0);
        let mut failure = None;
        for _ in 0..1000 {
            if let Err(error) =
                session.try_advance(&StationaryClock, frontend_menu(), forward(), &mut progress)
            {
                failure = Some(error);
                break;
            }
        }
        assert_eq!(
            failure,
            Some(WheelPostprocessError::ContactVectorIndex {
                lane: 0,
                vector_index: 8,
            })
        );
        assert_eq!(session.sim_ticks(), 0);
        assert!(!progress.finished());
    }

    #[test]
    fn static_collision_advance_is_atomic_before_progress_and_fsm_commit() {
        let mut session = GameplaySession::new(params(), InRaceScript::school_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let invalid = empty_collision_scene(2);
        let mut failure = None;
        for _ in 0..1000 {
            let before = session.car();
            match session.try_advance_with_static_collision(
                &StationaryClock,
                frontend_menu(),
                forward(),
                &mut progress,
                &invalid,
                &mut NoStaticDamageOwner,
            ) {
                Ok(_) => {}
                Err(error) => {
                    assert_eq!(session.car(), before);
                    failure = Some(error);
                    break;
                }
            }
        }
        assert!(matches!(
            failure,
            Some(StaticGameplayAdvanceError::Collision(
                LiveStaticCollisionStepError::ActiveCell(
                    ActiveCellRecordBuildError::CellCountMismatch { .. }
                )
            ))
        ));
        assert_eq!(session.sim_ticks(), 0);
        assert!(!progress.finished());

        let mut session = GameplaySession::new(params(), InRaceScript::school_harness());
        let mut progress = StageProgress::new(&nodes, 0);
        let valid = empty_collision_scene(1);
        for _ in 0..1000 {
            let frame = session
                .try_advance_with_static_collision(
                    &StationaryClock,
                    frontend_menu(),
                    forward(),
                    &mut progress,
                    &valid,
                    &mut NoStaticDamageOwner,
                )
                .unwrap();
            if frame.stepped {
                break;
            }
        }
        assert_eq!(session.sim_ticks(), 1);
    }

    #[test]
    fn compiled_constants_fill_mass_inertia_pedals_and_mode() {
        let trig = crate::fixed_point::build_sine_table();
        let mut wheel = four_wheel_params(&trig);
        let constants = crate::compiled_car_initializer::CompiledFourWheelConstants {
            state_word_48: 0x7a_0000,
            state_word_4c: 0x218,
            inertia_50_54_58: [1, 2, 1],
            control_word_148: 0x3_0000,
            control_word_16c: 0x2_0000,
            control_word_158: 1,
            control_word_150: 0x0ccc,
            control_word_174: 0x0ccd,
            control_word_17c: 0x0cce,
            control_word_184: 0x8000,
            control_word_408: 1,
            control_word_188: 0x3333,
            control_u16_3e4: 0x02aa,
            control_byte_440: 3,
            control_word_15c: 0x5_0000,
            gravity_c5fc: 0x9999,
            control_word_420: 1,
        };
        crate::race_state::populate_compiled_four_wheel_constants(&mut wheel, &constants);
        assert_eq!(wheel.inverse_mass_4c.raw(), 0x218);
        assert_eq!(wheel.angular.momentum.moment_scale.raw(), 0x7a_0000);
        assert_eq!(
            [
                wheel.angular.momentum.inertia_scale.x.raw(),
                wheel.angular.momentum.inertia_scale.y.raw(),
                wheel.angular.momentum.inertia_scale.z.raw(),
            ],
            [1, 2, 1]
        );
        assert_eq!(
            wheel.pedal_response,
            crate::pedal_response::PedalResponseParams {
                throttle_max_148: 0x3_0000,
                throttle_rate_150: 0x0ccc,
                brake_max_16c: 0x2_0000,
                brake_rate_174: 0x0ccd,
                handbrake_rate_17c: 0x0cce,
                handbrake_max_184: 0x8000,
            }
        );
        assert_eq!(wheel.driven_smoothing.mode_408, 1);
        assert_eq!(
            (
                wheel.steering.centering_188,
                wheel.steering.max_angle_3e4,
                wheel.steering.flags_440,
            ),
            (0x3333, 0x02aa, 3)
        );
        assert_eq!(wheel.engine_smoothing.redline_15c, 0x5_0000);
        assert_eq!(wheel.engine_force.redline_15c, 0x5_0000);
        assert_eq!(
            wheel.postprocess.base_force,
            crate::race_state::gravity_force_51c(
                crate::fixed_point::Fixed16::from_raw(0x7a_0000),
                crate::fixed_point::Fixed16::from_raw(0x9999),
            )
        );
        assert!(
            wheel
                .second
                .iter()
                .all(|second| second.correction_gate == 1)
        );
    }

    #[test]
    fn setup_block_fills_engine_steering_wheel_and_gearbox_words() {
        let trig = crate::fixed_point::build_sine_table();
        let mut wheel = four_wheel_params(&trig);
        wheel.gearbox = Some(crate::gearbox::GearboxParams::default());
        let mut setup = crate::car_setup::CarSetupBlock::RESET;
        setup.engine_response_85c = 0xe000;
        setup.steering_factor_838 = 0xd000;
        setup.wheel_constants_83c = [1, 2, 3, 4];
        setup.base_angle_bac = -5;
        setup.setup_800[8] = 0xb334;
        crate::race_state::populate_car_setup(&mut wheel, &setup);
        assert_eq!(wheel.engine_force.response_e1ac, 0xe000);
        assert_eq!(wheel.steering_car_factor_e188, 0xd000);
        assert_eq!(wheel.lane_direction_angle, -5);
        assert_eq!(wheel.first.map(|first| first.wheel_constant), [1, 2, 3, 4]);
        assert!(wheel.gearbox.unwrap().gear_two_skip);
    }

    #[test]
    fn four_wheel_base_force_produces_world_motion() {
        let trig = crate::fixed_point::build_sine_table();
        let wheel = four_wheel_params(&trig);
        let mut session =
            GameplaySession::new_with_four_wheel(params(), InRaceScript::school_harness(), wheel);
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        for _ in 0..100 {
            session
                .try_advance(&StationaryClock, frontend_menu(), forward(), &mut progress)
                .unwrap();
        }
        assert!(session.sim_ticks() > 0);
        assert!(session.car().position.x.raw() > 0);
    }

    /// One frame and its render's compositor step (the fade only advances
    /// on rendered frames).
    fn rendered(session: &mut GameplaySession, progress: &mut StageProgress) -> GameplayFrame {
        let frame = session.advance(&StationaryClock, frontend_menu(), forward(), progress);
        session.composite_fade();
        frame
    }

    #[test]
    fn a_forward_drive_runs_the_school_session_back_to_the_front_end() {
        let mut session = GameplaySession::new(params(), InRaceScript::school_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);

        let mut frames = 0u32;
        while session.event().in_session() && frames < 100_000 {
            frames += 1;
            rendered(&mut session, &mut progress);
        }
        assert!(session.event().globals.frontend_506228);
        assert_eq!(session.state().id(), 0xc);
        assert!(progress.finished());
        assert!(session.sim_ticks() > 0);
        // The car advanced in +x under the forward force.
        assert!(session.car().position.x.raw() > 0);
    }

    #[test]
    fn the_in_race_band_presents_the_stage_world_route() {
        let mut session = GameplaySession::new(params(), InRaceScript::school_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);

        // Run until the driving tick (state 5) has stepped, and confirm that
        // frame's render route is the in-race stage-world composition.
        let mut saw_stage_world = false;
        let mut frames = 0u32;
        while !session.is_stopped() && frames < 100_000 {
            frames += 1;
            let frame = rendered(&mut session, &mut progress);
            if frame.stepped {
                assert_eq!(frame.band, SessionBand::Gameplay);
                if matches!(frame.route, Some(RenderRoute::StageWorld(_))) {
                    saw_stage_world = true;
                }
            }
        }
        assert!(saw_stage_world, "the driving tick presents the stage world");
    }

    #[test]
    fn a_finished_race_returns_to_the_menu_and_the_next_stage_starts_fresh() {
        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let mut frames = 0u32;
        let mut left_gameplay = false;
        let mut finish_position = Vec3::ZERO;
        let mut held_in_results = 0u32;
        let mut cues = Vec::new();
        while frames < 100_000 {
            frames += 1;
            // The results pages hold state 8 until Return reaches Continue.
            if session.state().id() == 8 {
                held_in_results += 1;
                // Return held, and pressed anew every other frame for the
                // save dialog's change filter.
                session.set_results_input(DirectorInput {
                    confirm: held_in_results > 20,
                    keys: if held_in_results > 20 && held_in_results.is_multiple_of(2) {
                        0x10
                    } else {
                        0
                    },
                    ..DirectorInput::default()
                });
            }
            let frame = rendered(&mut session, &mut progress);
            cues.extend(session.take_results_effects());
            if progress.finished() && !frame.stepped && session.state().id() <= 4 {
                left_gameplay = true;
                finish_position = session.car().position;
                break;
            }
        }
        assert!(
            left_gameplay,
            "a normal finish runs 5 -> 8 -> 0xb back to the menu band"
        );
        assert!(held_in_results > 20, "state 8 held for the results");
        assert!(session.results().is_none(), "Continue cleared the director");
        assert!(
            cues.contains(&DirectorEffect::PageCue),
            "each page change cues the menu sound"
        );
        assert!(!session.is_stopped());
        assert!(finish_position.x.raw() > 0);
        // The menu flow starts the stage again; the first drive starts over.
        let mut restarted = false;
        while frames < 200_000 {
            frames += 1;
            let frame = rendered(&mut session, &mut progress);
            if frame.stepped {
                restarted = true;
                assert!(session.car().position.x.raw() < finish_position.x.raw());
                assert!(!progress.finished());
                break;
            }
        }
        assert!(restarted);
    }

    fn drive_to_state_five(session: &mut GameplaySession, progress: &mut StageProgress) {
        for _ in 0..100 {
            if session.state().id() == 5 {
                return;
            }
            rendered(session, progress);
        }
        panic!("the menu flow never reached driving");
    }

    #[test]
    fn a_pause_freezes_the_sim_until_resume() {
        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        drive_to_state_five(&mut session, &mut progress);
        rendered(&mut session, &mut progress);
        let before = session.car();
        session.set_pause_flow(PauseFlow {
            enter_pause: true,
            ..PauseFlow::default()
        });
        let frame = rendered(&mut session, &mut progress);
        assert!(!frame.stepped, "the pausing tick runs no Mechanics");
        assert_eq!(session.state().id(), 0xa);
        for _ in 0..10 {
            let frame = rendered(&mut session, &mut progress);
            assert!(!frame.stepped);
        }
        assert_eq!(session.car(), before);
        assert_eq!(session.state().id(), 0xa);
        session.set_pause_flow(PauseFlow {
            resume: true,
            ..PauseFlow::default()
        });
        rendered(&mut session, &mut progress);
        assert_eq!(session.state().id(), 5);
        let frame = rendered(&mut session, &mut progress);
        assert!(frame.stepped);
        assert_ne!(session.car().position, before.position);
    }

    #[test]
    fn a_pause_restart_returns_to_the_menu_band_and_quit_ends_the_session() {
        let nodes = line();
        for (flow, expected) in [
            (
                PauseFlow {
                    restart: true,
                    ..PauseFlow::default()
                },
                2,
            ),
            (
                PauseFlow {
                    quit: true,
                    ..PauseFlow::default()
                },
                0xc,
            ),
        ] {
            let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
            let mut progress = StageProgress::new(&nodes, 0);
            drive_to_state_five(&mut session, &mut progress);
            session.set_pause_flow(PauseFlow {
                enter_pause: true,
                ..PauseFlow::default()
            });
            rendered(&mut session, &mut progress);
            assert_eq!(session.state().id(), 0xa);
            // The words persist from the pause into the hub.
            // The fade out to the hub, then the display shutdown.
            for _ in 0..40 {
                session.set_pause_flow(flow);
                rendered(&mut session, &mut progress);
                if !(0xa..=0xc).contains(&session.state().id()) {
                    break;
                }
            }
            assert_eq!(session.state().id(), expected);
        }
    }

    #[test]
    fn the_race_fades_follow_the_load_the_finish_and_the_hub() {
        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let mut log: Vec<(u8, FadeComposite)> = Vec::new();
        let mut wrote_back = 0u32;
        let mut held_in_results = 0u32;
        for _ in 0..100_000 {
            if session.state().id() == 8 {
                held_in_results += 1;
                session.set_results_input(DirectorInput {
                    confirm: held_in_results > 20,
                    keys: if held_in_results > 20 && held_in_results.is_multiple_of(2) {
                        0x10
                    } else {
                        0
                    },
                    ..DirectorInput::default()
                });
            }
            session.advance(&StationaryClock, frontend_menu(), forward(), &mut progress);
            wrote_back += u32::from(session.take_damage_writeback());
            let state = session.rendered_state.unwrap_or(0);
            log.push((state, session.composite_fade()));
            if progress.finished() && session.state().id() <= 4 {
                break;
            }
        }
        let of = |state: u8| -> Vec<FadeComposite> {
            log.iter()
                .filter(|(s, _)| *s == state)
                .map(|(_, f)| *f)
                .collect()
        };
        // The load's one-frame fade out: four black frames, then a hard cut.
        let driving = of(5);
        assert_eq!(
            driving[..5],
            [
                FadeComposite::Quad(255),
                FadeComposite::Quad(255),
                FadeComposite::Quad(255),
                FadeComposite::Quad(255),
                FadeComposite::Clear,
            ]
        );
        // The finish cuts in through black over 12 frames.
        let results = of(8);
        assert_eq!(results[0], FadeComposite::Quad(255));
        assert_eq!(results[1], FadeComposite::Quad(234));
        assert_eq!(
            results[11..16],
            [
                FadeComposite::Quad(0),
                FadeComposite::Quad(0),
                FadeComposite::Quad(0),
                FadeComposite::Quad(0),
                FadeComposite::Clear,
            ]
        );
        // The exit fades out in the hub, which then shuts the display down.
        let hub = of(0xB);
        // State 8's render showed the exit's first step (alpha 0).
        assert_eq!(hub[0], FadeComposite::Quad(22));
        assert_eq!(
            hub.iter()
                .filter(|f| **f == FadeComposite::Shutdown)
                .count(),
            1
        );
        assert_eq!(*hub.last().unwrap(), FadeComposite::Clear);
        assert_eq!(wrote_back, 1, "the finish hands the damage back once");
    }

    #[test]
    fn an_impact_dents_the_car_unless_the_damage_is_off() {
        use crate::car_crash_damage::{BreakageKind, CarDamage, DamagePart, DamagePrim};
        let prim = |x: i32, material: i16| DamagePrim {
            positions: vec![[x, 0, 0]],
            uvs: vec![[0, 0]],
            material,
            entries: vec![0],
            faces: 0..1,
        };
        // Node 7 is part 1, the high-detail mirror.
        let part = |id| {
            DamagePart::new(
                id,
                0,
                vec![prim(0x8000, 1)],
                vec![prim(0x4000, 2)],
                vec![prim(0x8000, 1)],
            )
            .unwrap()
        };
        let damage = CarDamage::new(vec![part(6), part(7)], false, 0);
        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        session.attach_car_damage(damage.clone());
        session.crash_impact([0x8000, 0, 0], 0x1_0000, [1, 2, 3]);
        assert!(session.take_damage_mesh_dirty());
        assert!(!session.take_damage_mesh_dirty(), "taken once");
        assert!(session.damage_total() > 0);
        assert_eq!(
            session.take_breakage_cues(),
            [(BreakageKind::Mirror, [1, 2, 3])]
        );
        // `cfg[0x26] & 1` turns the damage off.
        let mut off = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        off.event_mut().cfg.effects_026 = 1;
        off.attach_car_damage(damage);
        off.crash_impact([0x8000, 0, 0], 0x1_0000, [0; 3]);
        assert!(!off.take_damage_mesh_dirty());
        assert_eq!(off.damage_total(), 0);
    }

    #[test]
    fn a_school_task_runs_its_intro_task_and_exit() {
        use crate::school_race::{SchoolCue, SchoolRace, SchoolScoring, advice_path};
        let script = InRaceScript {
            quit_requested: false,
            ..InRaceScript::school_harness()
        };
        let mut session = GameplaySession::new(params(), script);
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        for _ in 0..50 {
            if session.state().id() == 4 {
                break;
            }
            rendered(&mut session, &mut progress);
        }
        assert_eq!(session.state().id(), 4);
        let reference = cmr_formats::school_rsf::SchoolReference {
            time_cs: 100,
            records: vec![cmr_formats::school_rsf::LineRecord::default(); 200],
            trailing: Vec::new(),
        };
        let scoring = SchoolScoring {
            offsets: [[0; 6]; 12],
            scales: [[1; 6]; 12],
            damage_sum: 0,
            setup: [0; 5],
        };
        session.start_school_task(SchoolRace::start(0, 2, 3, reference, scoring));
        let mut seen = Vec::new();
        let mut states = Vec::new();
        for _ in 0..3000 {
            rendered(&mut session, &mut progress);
            seen.extend(session.take_school_cues());
            if states.last() != Some(&session.state().id()) {
                states.push(session.state().id());
            }
            if session.state().id() == 2 {
                break;
            }
        }
        assert_eq!(seen.first(), Some(&SchoolHostCue::IntroSpeech));
        assert!(
            seen.iter()
                .any(|c| matches!(c, SchoolHostCue::ExitSpeech(_)))
        );
        assert!(seen.iter().all(|c| match c {
            SchoolHostCue::Task(SchoolCue::Advice(i)) => advice_path(*i).is_some(),
            _ => true,
        }));
        assert_eq!(
            states,
            [7, 5, 9, 0xb, 2],
            "the task timed out, then the exit"
        );
        let marks = session.take_school_marks().expect("the scorer ran");
        assert_eq!(marks[3] & 0xf, 7, "gears");
        assert_eq!(session.event().cfg.school_6c0 & 7, 4, "the results come up");
    }

    #[test]
    fn a_still_car_never_reaches_the_finish_line() {
        // With no forward force the car never crosses a gate, so the FSM stays in
        // the gameplay band (never funnels through the finish).
        let mut session = GameplaySession::new(params(), InRaceScript::school_harness());
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let still = Controls {
            steer_rate: 0,
            forward_force: Fixed16::ZERO,
            handbrake: false,
            raw_steer: Default::default(),
            raw_pedals: Default::default(),
            raw_gear_command_314: 0,
        };
        for _ in 0..1_000 {
            session.advance(&StationaryClock, frontend_menu(), still, &mut progress);
        }
        assert!(!progress.finished());
        assert!(!session.is_stopped());
    }

    #[test]
    fn moving_object_collision_steps_object_and_feeds_school_car() {
        let trig = crate::fixed_point::build_sine_table();
        let mut wheel = four_wheel_params(&trig);
        let identity_basis = [
            Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
            Vec3::new(Fixed16::ZERO, Fixed16::ONE, Fixed16::ZERO),
            Vec3::new(Fixed16::ZERO, Fixed16::ZERO, Fixed16::ONE),
        ];
        wheel.angular.basis = identity_basis;
        wheel.angular.momentum.moment_scale = Fixed16::from_raw(1200 << 16);
        wheel.postprocess.base_force = Vec3::ZERO;
        let mut car = CarState::at_rest();
        car.motion.basis = identity_basis;
        car.motion.compiled_half_words = Vec3::new(
            Fixed16::from_raw(1 << 16),
            Fixed16::from_raw(0x8000),
            Fixed16::from_raw(2 << 16),
        );
        car.position = Vec3::new(
            Fixed16::from_raw(4 << 16),
            Fixed16::from_raw(0x8000),
            Fixed16::from_raw(2 << 16),
        );
        car.velocity = Vec3::new(Fixed16::ZERO, Fixed16::ZERO, Fixed16::from_raw(1 << 16));
        car.four_wheel.speed_ec = 1 << 16;
        let mut session = GameplaySession::new_with_four_wheel_and_car(
            params(),
            InRaceScript::school_harness(),
            wheel,
            car,
        );
        let mut parsed = empty_collision_scene(1);
        parsed.grid.max_z = 0x20_0000;
        let cone = crate::moving_objects::MovingObject {
            node: 7,
            position: [4 << 16, 0x8000, 4 << 16],
            half: [0x8000, 0x8000, 0x8000],
            velocity: [0; 3],
            angular: [0; 3],
            torque: [0; 3],
            impulse: [0; 3],
            inverse_inertia: [0x8000, 0x8000, 0x8000],
            rows: [[0x1_0000, 0, 0], [0, 0x1_0000, 0], [0, 0, 0x1_0000]],
            base: [4 << 16, 0, 4 << 16],
            radius: 0xe000,
            ground_height: 0,
            id_material: (1 << 5) | 5,
            ground_handle: -1,
            cell: 0,
            flags: 0,
            rest_axes: 7,
            row_scale: [0x1_0000, 0x1_0000, 0x1_0000],
            ground_normal: [0, 0x1_0000, 0],
            next: None,
            prev: None,
        };
        let objects = MovingObjects::from_objects_for_test(vec![cone], &parsed.grid);
        session.set_moving_objects(objects);

        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        for _ in 0..1000 {
            let frame = session
                .try_advance_with_static_collision(
                    &StationaryClock,
                    frontend_menu(),
                    forward(),
                    &mut progress,
                    &parsed,
                    &mut NoStaticDamageOwner,
                )
                .unwrap();
            if frame.stepped {
                break;
            }
        }
        assert_eq!(session.sim_ticks(), 1);
        let school_car = session.school_car(Some(&progress));
        assert!(school_car.object_contact);
        assert_eq!(school_car.contact_surface, 5);
        assert_eq!(
            session.moving_objects().unwrap().objects[0].state(),
            crate::moving_objects::state::MOVING
        );
        assert_eq!(session.moving_object_node_locals().len(), 1);
        assert_eq!(session.moving_object_node_locals()[0].0, 7);
    }
}
