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

/// Camera transitions triggered by the State 5 race-start director (`FUN_00439b50`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartCameraEvent {
    /// Enter Mode 8 (`FUN_00467630` / `FUN_004678c0` preset 0) for the 75-tick
    /// start-line camera orbit around the car (`0x439be2..0x439bff`).
    BeginOrbit,
    /// Crossfade from the end of the 75-tick Mode 8 orbit into the active
    /// driving camera over 25 ticks (`FUN_004360a0(0, 1)` at `0x439c88`).
    CrossfadeToChase,
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
    /// `ctl +0x304` and `ctl +0x308` as the last four-wheel tick read them.
    last_throttle_304: i32,
    last_brake_308: i32,
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
    /// Bound `.moj` kinematic scene objects (`FUN_0045bde0`..`FUN_0045c9c0`) and
    /// their stage-start snapshot.
    moj_slots: Vec<crate::stage_moj_motion::MojRuntimeSlot>,
    moj_slots_initial: Vec<crate::stage_moj_motion::MojRuntimeSlot>,
    /// `DAT_00552f68`: whether `FUN_0043f790` recorded an object contact this tick.
    last_object_contact_552f68: bool,
    /// `DAT_00558fb0`: the last static or moving-object hit's surface material byte.
    last_contact_material_558fb0: u8,
    /// Whether the State 5 start-line orbit and 5-4-3-2-1-GO! director
    /// (`FUN_00439b50`) is enabled for non-school stages.
    start_sequence_enabled: bool,
    /// `DAT_0057b0ac[0]` start-sequence state (`0` = uninitialised, `1` = arm
    /// orbit, `2` = 75-tick orbit, `4` = 100-tick countdown, `5` = 19-tick
    /// `"GO!"` banner, `100` = normal race running).
    start_phase_57b0ac: u8,
    /// `DAT_0057b048[0]` per-phase tick countdown.
    start_timer_57b048: u8,
    /// Pending camera mode transition emitted by `FUN_00439b50`.
    pending_start_camera_event: Option<StartCameraEvent>,
    /// Pending countdown speech cues (`5.wav`..`1.wav`, `go!.wav`) emitted by
    /// `FUN_00439b50` (`0x439d08`, `0x439d3b`).
    pending_countdown_cues: Vec<crate::audio_music_cues::CountdownCue>,
    /// The game's `rand()` (`DAT_0050b690`) as the session draws on it; the
    /// host copies its generator in before an advance and back after.
    rng: crate::game_rng::GameLcg,
}

/// A loaded `.rsp` / `.ssr` replay track (`DAT_005ef444` intro or `DAT_005ef440` exit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchoolReplayTrack {
    region_a: Box<[u8; crate::replay::REPLAY_INITIAL_STATE_BYTES]>,
    records: Vec<crate::replay::ReplayControlRecord>,
    stepper: crate::replay::ReplayPlaybackStepper,
    started: bool,
}

impl SchoolReplayTrack {
    /// Parse a single-frame `.rsp` or `.ssr` buffer (`FUN_0046ddc0` + `FUN_0046de30`).
    #[must_use]
    pub fn from_replay_bytes(bytes: &[u8]) -> Option<Self> {
        let view = crate::replay::ReplayFileView::from_bytes(bytes)?;
        let region_a_view = view.region_a(0)?;
        let (records, stepper) = view.frame_playback(0)?;
        let mut region_a = Box::new([0u8; crate::replay::REPLAY_INITIAL_STATE_BYTES]);
        region_a.copy_from_slice(region_a_view.as_bytes());
        Some(Self {
            region_a,
            records,
            stepper,
            started: false,
        })
    }

    #[must_use]
    pub fn region_a_view(&self) -> crate::replay::ReplayRegionAView<'_> {
        crate::replay::ReplayRegionAView::new(&self.region_a)
    }

    #[must_use]
    pub const fn is_started(&self) -> bool {
        self.started
    }

    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.stepper.is_finished()
    }

    /// Step one physics tick of RLE replay input (`FUN_0046dc20`).
    pub fn step_controls(&mut self) -> Option<Controls> {
        let decoded = self.stepper.step_tick_46dc20(&self.records)?;
        Some(Controls::from_decoded_replay_control(&decoded))
    }

    /// Stop playback early (`FUN_0046dd90`, player skip `FUN_0046ee40`).
    pub fn stop_playback_46dd90(&mut self) {
        self.stepper.cursor = usize::from(self.stepper.end_cursor);
    }
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
    /// Optional loaded `intro.rsp` pan (`DAT_005ef444` / `DAT_004f4b34`).
    pub intro_rsp: Option<SchoolReplayTrack>,
    /// Optional loaded `exit.rsp` demonstration (`DAT_005ef440` / `DAT_004f4b38`).
    pub exit_rsp: Option<SchoolReplayTrack>,
    /// `DAT_005ef454`: State 7 post-pan fade-out / restore phase.
    intro_fading_5ef454: bool,
    /// `DAT_005ef450`: State 7 intro `.rsp` pan completed; ready for intro speech.
    intro_pan_done_5ef450: bool,
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

/// The school's gates in states 5, 7 and 9 (`FUN_00437ef0` / `FUN_0046e0f0` /
/// `FUN_0046edd0` for state 7; `FUN_004384a0` / `FUN_0046e180` for state 9).
///
/// When `intro_rsp` (`DAT_005ef444`) is attached, state 7 seeds `race` from
/// `intro.rsp` Region A (`FUN_0046db30`), plays its RLE control stream until
/// completion or skip (`FUN_0046e0f0`), fades out to black, restores `race` to
/// `stage_start` in neutral gear (`FUN_0046edd0`), fades back in, plays the
/// instructor intro speech (`FUN_00474cb0`), and shifts to 1st gear (`0x438087`).
/// When `exit_rsp` (`DAT_005ef440`) is attached, state 9 seeds `race` from
/// `exit.rsp` Region A at the black fade boundary (`0x438545`), fades in, starts
/// the instructor exit speech (`FUN_00474cd0`), and waits for both the speech
/// and `exit.rsp` playback (`FUN_0046e180`) to finish.
#[allow(clippy::too_many_arguments)]
fn school_gates(
    state: u8,
    task: &mut SchoolTask,
    fade: &mut RaceFade,
    race: &mut RaceState,
    stage_start: CarState,
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
            if !task.intro_pan_done_5ef450
                && let Some(track) = task.intro_rsp.as_mut()
            {
                if !task.intro_fading_5ef454 {
                    if !track.started {
                        track.started = true;
                        race.seed_from_replay_region_a_46d460(track.region_a_view());
                    }
                    if track.is_finished() {
                        fade.fade_out();
                        task.intro_fading_5ef454 = true;
                    }
                } else if fade.fade.idle() {
                    if fade.fade.evaluate() == 0 {
                        // `FUN_0046edd0`: restore car 0 from pre-pan backup in neutral gear.
                        race.set_car(stage_start);
                        race.set_gear_43a(0);
                        task.race.hud = true;
                        fade.fade_in_race();
                    } else {
                        task.intro_fading_5ef454 = false;
                        task.intro_pan_done_5ef450 = true;
                    }
                }
            } else {
                match task.intro {
                    0 => {
                        task.cues.push(SchoolHostCue::IntroSpeech);
                        task.intro = 1;
                    }
                    1 => task.intro = 2,
                    2 if !task.speaking => {
                        if task.intro_pan_done_5ef450 {
                            race.set_gear_43a(1);
                        }
                        task.intro = 3;
                    }
                    _ => {}
                }
            }
            inputs.replay_intro_done = task.intro == 3;
        }
        9 => {
            match task.exit {
                0 if fade.fade.idle() => {
                    if fade.fade.evaluate() == 0 {
                        if let Some(track) = task.exit_rsp.as_mut() {
                            track.started = true;
                            race.seed_from_replay_region_a_46d460(track.region_a_view());
                        }
                        task.race.hud = false;
                        fade.fade_in_race();
                    } else {
                        task.exit = 1;
                    }
                }
                1 => {
                    let wav = SchoolTask::exit_wav(task.race.marks(), rng);
                    task.cues.push(SchoolHostCue::ExitSpeech(wav));
                    if task.exit_rsp.is_some() {
                        race.set_gear_43a(1);
                    }
                    task.exit = 2;
                }
                2 => task.exit = 3,
                3 if !task.speaking
                    && task
                        .exit_rsp
                        .as_ref()
                        .is_none_or(SchoolReplayTrack::is_finished) =>
                {
                    task.exit = 4;
                }
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
    race: &mut RaceState,
    stage_start: CarState,
    finish_writeback: &mut bool,
    school: Option<&mut SchoolTask>,
    rng: &mut crate::game_rng::GameLcg,
) -> GameState {
    let mut inputs = pause.apply(script.with_finish(finished, event.globals.school_d3ca88));
    if let Some(task) = school.filter(|_| inputs.school_session) {
        school_gates(state.id(), task, fade, race, stage_start, rng, &mut inputs);
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
        self.reset_start_sequence();
        self
    }

    /// Enable or disable the State 5 race-start sequence (`FUN_00439b50`:
    /// 75-tick Mode 8 start-line camera orbit followed by the 5-4-3-2-1-GO!
    /// countdown and `FUN_00407590` launch).
    #[must_use]
    pub fn with_start_sequence(mut self, enabled: bool) -> Self {
        self.start_sequence_enabled = enabled;
        self.reset_start_sequence();
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
            last_brake_308: 0,
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
            moj_slots: Vec::new(),
            moj_slots_initial: Vec::new(),
            last_object_contact_552f68: false,
            last_contact_material_558fb0: 0,
            start_sequence_enabled: false,
            start_phase_57b0ac: 100,
            start_timer_57b048: 0,
            pending_start_camera_event: None,
            pending_countdown_cues: Vec::new(),
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

    /// Attach the stage's bound `.moj` kinematic scene-object slots
    /// (`FUN_0045bde0`..`FUN_0045c160`).
    pub fn set_moj_slots(&mut self, slots: Vec<crate::stage_moj_motion::MojRuntimeSlot>) {
        self.moj_slots_initial.clone_from(&slots);
        self.moj_slots = slots;
    }

    /// The live bound `.moj` kinematic scene-object slots (`DAT_00573a88`).
    #[must_use]
    pub fn moj_slots(&self) -> &[crate::stage_moj_motion::MojRuntimeSlot] {
        &self.moj_slots
    }

    /// The `.obj` scene-graph `(node_index, local_matrix_q16)` overrides from
    /// any dynamic trackside objects (`FUN_00452b70`) and bound `.moj` kinematic
    /// scene objects (`FUN_0045c9c0`).
    #[must_use]
    pub fn moving_object_node_locals(&self) -> Vec<(usize, [i32; 16])> {
        let mut locals = self
            .moving_objects
            .as_ref()
            .map_or_else(Vec::new, |o| o.node_locals().collect());
        for slot in &self.moj_slots {
            let ground_y = if slot.record.terrain_snap_flag != 0
                && let Some(geometry) = self.ground_geometry
            {
                use crate::moving_objects::ObjectGround as _;
                let mut ground = CompiledObjectGround(geometry.provider());
                let mut handle = -1;
                let (y, _) = ground.query(
                    [
                        slot.curr_tf_48[12],
                        slot.curr_tf_48[13],
                        slot.curr_tf_48[14],
                    ],
                    &mut handle,
                    slot.curr_tf_48[13],
                );
                Some(y)
            } else {
                None
            };
            let tf = crate::stage_moj_motion::interpolate_moj_slot_45c9c0(
                slot,
                crate::stage_moj_motion::ONE_Q16,
                ground_y,
            );
            locals.push((slot.scene_node_index, tf));
        }
        locals
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
        self.last_brake_308 = controls.raw_pedals.brake_308;
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
        // Kinematic `.moj` object motion (`FUN_0045c1b0`, `0x43b388..0x43b397`)
        // runs immediately after dynamic `.obj` physics in `FUN_0043afd0`.
        for slot in &mut self.moj_slots {
            crate::stage_moj_motion::step_moj_slot_45c1b0(slot);
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
        self.moj_slots.clone_from(&self.moj_slots_initial);
        self.object_hit_cache.clear();
        self.last_object_contact_552f68 = false;
        self.last_contact_material_558fb0 = 0;
        progress.restart();
        self.last_reset = ResetOutcome::default();
        self.last_trigger_events.clear();
        self.car_760 = false;
        self.stage_dirty = false;
        self.reset_start_sequence();
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
        let mut driving_sample = if self.session.current().id() == 5 && !pause.enter_pause {
            self.start_sequence_pre_drive(&mut controls);
            Some(if self.wheel.is_some() {
                let race_before = self.race.clone();
                let mut sample = self.four_wheel_drive(&controls)?;
                if let Err(error) = self.four_wheel_tail(&controls, progress, &mut sample) {
                    self.race = race_before;
                    return Err(error);
                }
                sample
            } else {
                for slot in &mut self.moj_slots {
                    crate::stage_moj_motion::step_moj_slot_45c1b0(slot);
                }
                self.race.step(&controls, &self.params)
            })
        } else {
            None
        };
        if let Some(sample) = driving_sample.as_mut() {
            self.start_sequence_after_drive(sample);
            self.wet_tick(&controls);
            self.school_after_drive(&controls, progress);
        } else if !pause.enter_pause {
            self.step_active_school_replay(progress)?;
        }
        let script = self.script;
        let results = &mut self.results;
        let sim_ticks = &mut self.sim_ticks;
        let fade = &mut self.fade;
        let race = &mut self.race;
        let stage_start = self.stage_start;
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
                race,
                stage_start,
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
        let mut driving_sample = if self.session.current().id() == 5 && !pause.enter_pause {
            self.start_sequence_pre_drive(&mut controls);
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
            if four_wheel {
                if let Err(error) = self.four_wheel_tail(&controls, progress, &mut sample) {
                    self.race = race_before;
                    return Err(StaticGameplayAdvanceError::Wheel(error));
                }
            } else {
                for slot in &mut self.moj_slots {
                    crate::stage_moj_motion::step_moj_slot_45c1b0(slot);
                }
            }
            Some(sample)
        } else {
            None
        };
        if let Some(sample) = driving_sample.as_mut() {
            self.start_sequence_after_drive(sample);
            self.wet_tick(&controls);
            self.school_after_drive(&controls, progress);
        } else if !pause.enter_pause {
            self.step_active_school_replay(progress)
                .map_err(StaticGameplayAdvanceError::Wheel)?;
        }
        let script = self.script;
        let results = &mut self.results;
        let sim_ticks = &mut self.sim_ticks;
        let fade = &mut self.fade;
        let race = &mut self.race;
        let stage_start = self.stage_start;
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
                race,
                stage_start,
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
            intro_rsp: None,
            exit_rsp: None,
            intro_fading_5ef454: false,
            intro_pan_done_5ef450: false,
            cues: Vec::new(),
            marks_out: None,
        });
        self.reset_start_sequence();
    }

    fn reset_start_sequence(&mut self) {
        if self.start_sequence_enabled
            && !self.session.event().globals.school_d3ca88
            && self.school.is_none()
        {
            self.start_phase_57b0ac = 1;
            self.start_timer_57b048 = 0;
        } else {
            self.start_phase_57b0ac = 100;
            self.start_timer_57b048 = 0;
        }
        self.pending_start_camera_event = None;
        self.pending_countdown_cues.clear();
    }

    /// Whether the State 5 race-start sequence (`FUN_00439b50`) is enabled.
    #[must_use]
    pub const fn is_start_sequence_enabled(&self) -> bool {
        self.start_sequence_enabled
    }

    /// Take any pending camera transition emitted by the State 5 race-start
    /// sequence (`FUN_00439b50`).
    pub fn take_start_camera_event(&mut self) -> Option<StartCameraEvent> {
        self.pending_start_camera_event.take()
    }

    /// Take any pending countdown speech cues (`5.wav`..`1.wav`, `go!.wav`)
    /// emitted by the State 5 race-start sequence (`FUN_00439b50`).
    pub fn take_countdown_cues(&mut self) -> Vec<crate::audio_music_cues::CountdownCue> {
        std::mem::take(&mut self.pending_countdown_cues)
    }

    /// Active HUD countdown / `"GO!"` banner state (`FUN_00423570` /
    /// `FUN_00423700`) for the current State 5 frame.
    #[must_use]
    pub const fn hud_countdown(&self) -> Option<crate::race_hud_plan::RaceHudCountdown> {
        if self.start_phase_57b0ac == 4 && self.start_timer_57b048 < 100 {
            let digit = (self.start_timer_57b048 / 20) + 1;
            let ticks_left = self.start_timer_57b048 % 20;
            Some(crate::race_hud_plan::RaceHudCountdown::Digit { digit, ticks_left })
        } else if self.start_phase_57b0ac == 5 {
            Some(crate::race_hud_plan::RaceHudCountdown::Go {
                ticks_left: self.start_timer_57b048,
            })
        } else {
            None
        }
    }

    /// `FUN_00439b50` + `FUN_0040e1f0` pre-step control gate: while the car is
    /// on the start line (`DAT_0057b0ac[0]` in `1..=4`), only throttle is read
    /// from the player so the engine can be revved in Neutral while the
    /// handbrake holds the car at the line.
    fn start_sequence_pre_drive(&mut self, controls: &mut Controls) {
        if !self.start_sequence_enabled
            || self.session.event().globals.school_d3ca88
            || self.school.is_some()
        {
            return;
        }
        if self.start_phase_57b0ac == 1 {
            self.pending_start_camera_event = Some(StartCameraEvent::BeginOrbit);
            self.start_phase_57b0ac = 2;
            self.start_timer_57b048 = crate::gameplay_camera::START_ORBIT_TICKS as u8;
        }
        if self.start_phase_57b0ac == 2 || self.start_phase_57b0ac == 4 {
            controls.raw_steer = crate::vehicle_dynamics::RawSteer::default();
            controls.steer_rate = 0;
            controls.raw_pedals.brake_308 = 0;
            controls.handbrake = true;
            controls.raw_gear_command_314 = 0;
            controls.forward_force = Fixed16::ZERO;
            self.race.hold_start_line_neutral();
        }
    }

    /// `FUN_00439b50` post-step state machine: advances the 75-tick Mode 8
    /// camera orbit (`Phase 2`), the 100-tick `5..1` countdown (`Phase 4`),
    /// fires `FUN_00407590` launch at `timer == 0`, and runs the 19-tick
    /// `"GO!"` banner (`Phase 5`).
    fn start_sequence_after_drive(&mut self, sample: &mut crate::race_state::TraceSample) {
        if !self.start_sequence_enabled
            || self.session.event().globals.school_d3ca88
            || self.school.is_some()
        {
            return;
        }
        match self.start_phase_57b0ac {
            2 => {
                self.race.hold_start_line_neutral();
                sample.centiseconds = 0;
                sample.car = self.race.car();
                self.start_timer_57b048 = self.start_timer_57b048.saturating_sub(1);
                if self.start_timer_57b048 == 0 {
                    self.pending_start_camera_event = Some(StartCameraEvent::CrossfadeToChase);
                    self.start_phase_57b0ac = 4;
                    self.start_timer_57b048 = 100;
                }
            }
            4 => {
                self.race.hold_start_line_neutral();
                sample.centiseconds = 0;
                self.start_timer_57b048 = self.start_timer_57b048.saturating_sub(1);
                let timer = self.start_timer_57b048;
                let digit = (timer / 20) + 1;
                let rem = timer % 20;
                if rem == 19 {
                    let cue = match digit {
                        5 => Some(crate::audio_music_cues::CountdownCue::Five),
                        4 => Some(crate::audio_music_cues::CountdownCue::Four),
                        3 => Some(crate::audio_music_cues::CountdownCue::Three),
                        2 => Some(crate::audio_music_cues::CountdownCue::Two),
                        1 => Some(crate::audio_music_cues::CountdownCue::One),
                        _ => None,
                    };
                    if let Some(cue) = cue {
                        self.pending_countdown_cues.push(cue);
                    }
                }
                if timer == 0 {
                    self.pending_countdown_cues
                        .push(crate::audio_music_cues::CountdownCue::Go);
                    if let Some(wheel) = self.wheel.as_ref() {
                        self.race.launch_from_start_line_407590(wheel);
                    } else {
                        self.race.set_gear_43a(1);
                    }
                    self.start_phase_57b0ac = 5;
                    self.start_timer_57b048 = 19;
                }
                sample.car = self.race.car();
            }
            5 => {
                self.start_timer_57b048 = self.start_timer_57b048.saturating_sub(1);
                if self.start_timer_57b048 == 0 {
                    self.start_phase_57b0ac = 100;
                }
            }
            _ => {}
        }
    }

    /// Attach loaded `intro.rsp` (`DAT_005ef444`) and/or `exit.rsp` (`DAT_005ef440`)
    /// tracks (`FUN_0046de70`) to the active school task.
    pub fn set_school_replays(
        &mut self,
        intro_rsp: Option<SchoolReplayTrack>,
        exit_rsp: Option<SchoolReplayTrack>,
    ) {
        if let Some(task) = self.school.as_mut() {
            task.intro_rsp = intro_rsp;
            task.exit_rsp = exit_rsp;
        }
    }

    /// Stop the currently playing school `.rsp` pan (`FUN_0046ee40` -> `FUN_0046dd90`).
    pub fn skip_school_replay(&mut self) {
        let state = self.session.current().id();
        let Some(task) = self.school.as_mut() else {
            return;
        };
        if state == 7
            && let Some(track) = task.intro_rsp.as_mut()
        {
            track.stop_playback_46dd90();
        } else if state == 9
            && let Some(track) = task.exit_rsp.as_mut()
        {
            track.stop_playback_46dd90();
        }
    }

    /// Step an active `.rsp` replay track during State 7 (`intro.rsp`) or State 9
    /// (`exit.rsp`) (`FUN_0043afd0` -> `FUN_0046dd50` -> `FUN_0046dc20`).
    fn step_active_school_replay(
        &mut self,
        progress: &mut StageProgress,
    ) -> Result<(), WheelPostprocessError> {
        let state = self.session.current().id();
        let replay_controls = {
            let Some(task) = self.school.as_mut() else {
                return Ok(());
            };
            match state {
                7 if !task.intro_fading_5ef454 && !task.intro_pan_done_5ef450 => {
                    let Some(track) = task.intro_rsp.as_mut() else {
                        return Ok(());
                    };
                    if !track.started {
                        track.started = true;
                        self.race
                            .seed_from_replay_region_a_46d460(track.region_a_view());
                    }
                    track.step_controls()
                }
                9 => {
                    let Some(track) = task.exit_rsp.as_mut() else {
                        return Ok(());
                    };
                    if track.started {
                        track.step_controls()
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        let Some(controls) = replay_controls else {
            return Ok(());
        };
        if self.wheel.is_some() {
            let race_before = self.race.clone();
            let mut sample = self.four_wheel_drive(&controls)?;
            if let Err(error) = self.four_wheel_tail(&controls, progress, &mut sample) {
                self.race = race_before;
                return Err(error);
            }
        } else {
            self.race.step(&controls, &self.params);
        }
        self.sim_ticks += 1;
        Ok(())
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

    /// Build the car pose and state input for [`crate::car_headlights`]
    /// (`FUN_0041b5b0`, `FUN_0044a950`, `FUN_0044bbc0`).
    #[must_use]
    pub fn lamp_pose_input(
        &self,
        body_local_94: [i32; 16],
        body_world_d4: [i32; 16],
        container_matrix_94: [i32; 16],
        lights_on_6f8: bool,
        night_stage: bool,
    ) -> crate::car_headlights::CarLampPoseInput {
        let car = self.race.car();
        let four = &car.four_wheel;
        let event = self.session.event();
        let raw = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        let mode_408 = self.wheel.as_ref().is_some_and(|w| {
            if w.gearbox.is_some() {
                four.gearbox.reverse_mode_408
            } else {
                w.driven_smoothing.mode_408 != 0
            }
        });
        let delorean = event.cfg.effects_026 & 2 != 0;
        let braking_6f4 = delorean
            || if mode_408 {
                self.last_throttle_304 != 0
            } else {
                self.last_brake_308 != 0
            };
        let reverse_6fc = four.gearbox.gear_43a < 0;
        let (detached_lights_lr, broken_pod_lamps_ca8) = self
            .car_damage
            .as_ref()
            .map_or(([false; 2], [false; 6]), |d| {
                (d.detached_lights_lr(), d.broken_pod_lamps_ca8)
            });
        crate::car_headlights::CarLampPoseInput {
            body_local_94,
            body_world_d4,
            container_matrix_94,
            live_forward_258: raw(car.motion.basis[0]),
            live_up_264: raw(car.motion.basis[1]),
            frozen_up_228: raw(self.stage_start.motion.basis[1]),
            braking_6f4,
            lights_on_6f8,
            reverse_6fc,
            detached_lights_lr,
            broken_pod_lamps_ca8,
            electrics_damage_810: self.car_setup.setup_800[4],
            night_stage,
            low_detail_828e28: false,
            borrowers: event.cfg.options_025 & 4 != 0,
            aliengoo: event.cfg.effects_026 & 1 != 0,
            delorean,
            out_of_bounds_75c: four.trigger_walk.penalty_timer_75c != 0,
            penalty_760: self.car_760,
            special_event_514478: event.globals.special_514478,
        }
    }

    /// The car's out-of-bounds timer `car + 0x75c` (`FUN_004853e0`).
    #[must_use]
    pub fn out_of_bounds_timer_75c(&self) -> i32 {
        i32::from(self.race.car().four_wheel.trigger_walk.penalty_timer_75c)
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
            control_word_144: 0x83,
            control_word_148: 0x3_0000,
            control_word_160: 0x2000,
            control_word_16c: 0x2_0000,
            control_word_170: 0x8000,
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

    #[test]
    fn school_task_with_intro_and_exit_rsp_tracks_seeds_steps_and_restores_car() {
        use crate::replay::{
            REPLAY_INITIAL_STATE_BYTES, REPLAY_OBJECT_HEADER_BYTES, REPLAY_PHYSICS_OFFSET,
            REPLAY_TICK_RECORD_BYTES, ReplayObjectHeader, replay_object_size,
        };
        use crate::school_race::{SchoolRace, SchoolScoring};

        let make_rsp_bytes = |seed_x: i32, n_records: u16| -> Vec<u8> {
            let n_ticks = 8u16;
            let size = replay_object_size(u32::from(n_ticks), 1);
            let mut buf = vec![0u8; size];
            let hdr = ReplayObjectHeader {
                n_frames_30: 1,
                n_ticks_32: n_ticks as i16,
                recorded_frames_34: 1,
                ..ReplayObjectHeader::default()
            };
            buf[..REPLAY_OBJECT_HEADER_BYTES].copy_from_slice(&hdr.to_bytes());
            let pos_off = REPLAY_OBJECT_HEADER_BYTES + REPLAY_PHYSICS_OFFSET + 0xc8;
            buf[pos_off..pos_off + 4].copy_from_slice(&seed_x.to_le_bytes());
            let rb = REPLAY_OBJECT_HEADER_BYTES + REPLAY_INITIAL_STATE_BYTES;
            for i in 0..usize::from(n_records) {
                let off = rb + i * REPLAY_TICK_RECORD_BYTES;
                buf[off..off + 4].copy_from_slice(&[0x04, 0x00, 0x01, 0x00]);
            }
            let rc = rb + usize::from(n_ticks) * REPLAY_TICK_RECORD_BYTES;
            buf[rc..rc + 2].copy_from_slice(&n_records.to_le_bytes());
            buf
        };

        let intro_track =
            SchoolReplayTrack::from_replay_bytes(&make_rsp_bytes(0x77_0000, 2)).unwrap();
        let exit_track =
            SchoolReplayTrack::from_replay_bytes(&make_rsp_bytes(0x88_0000, 2)).unwrap();

        let script = InRaceScript {
            replay_ready: false,
            replay_intro_done: false,
            flyby_cinematic_done: false,
            quit_requested: false,
            ..InRaceScript::school_harness()
        };
        let mut session = GameplaySession::new(params(), script);
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        for _ in 0..100 {
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
        session.start_school_task(SchoolRace::start(0, 0, 3, reference, scoring));
        session.set_school_replays(Some(intro_track), Some(exit_track));

        let mut saw_intro_seed = false;
        let mut saw_restored_start = false;
        let mut saw_exit_seed = false;
        let mut states = Vec::new();
        for _ in 0..3500 {
            rendered(&mut session, &mut progress);
            let st = session.state().id();
            if states.last() != Some(&st) {
                states.push(st);
            }
            if st == 7 && session.car().position.x == Fixed16::from_raw(0x77_0000) {
                saw_intro_seed = true;
            }
            if st == 7 && saw_intro_seed && session.car().position.x == Fixed16::ZERO {
                saw_restored_start = true;
            }
            if st == 9 && session.car().position.x == Fixed16::from_raw(0x88_0000) {
                saw_exit_seed = true;
            }
            if st == 2 {
                break;
            }
        }
        assert!(
            saw_intro_seed,
            "State 7 seeded car from intro.rsp Region A (0x77_0000)"
        );
        assert!(
            saw_restored_start,
            "State 7 restored car to stage_start (0) via FUN_0046edd0 after intro.rsp"
        );
        assert!(
            saw_exit_seed,
            "State 9 seeded car from exit.rsp Region A (0x88_0000)"
        );
        assert_eq!(states, [7, 5, 9, 0xb, 2]);
    }

    #[test]
    fn moj_slots_step_on_drive_tick_and_emit_node_locals() {
        use crate::stage_moj_motion::{IDENTITY_4X4_Q16, bind_moj_scene_node_45be90};
        use cmr_formats::stage::MojRecord;

        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness());
        let record = MojRecord {
            terrain_snap_flag: 0,
            initial_angle_bam: 0,
            angle_padding: 0,
            orbit_diameter_q16: 0x20_0000,
            reserved_0c: 0,
            position_or_target_q16: [0, 0x10_0000, 0],
            translation_period_seconds: 1,
            rotation_period_seconds: [1, 0, 0],
            motion_mode: 4,
        };
        let mut node_tf = IDENTITY_4X4_Q16;
        node_tf[13] = 0x10_0000;
        let slot = bind_moj_scene_node_45be90(record, 42, &node_tf, false).unwrap();
        session.set_moj_slots(vec![slot]);

        let initial_locals = session.moving_object_node_locals();
        assert_eq!(initial_locals.len(), 1);
        assert_eq!(initial_locals[0].0, 42);
        assert_eq!(initial_locals[0].1[14], 0);

        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let mut steps = 0;
        for _ in 0..1000 {
            let frame = session
                .try_advance(&StationaryClock, frontend_menu(), forward(), &mut progress)
                .unwrap();
            if frame.stepped {
                steps += 1;
                if steps == 2 {
                    break;
                }
            }
        }
        assert_eq!(session.sim_ticks(), 2);
        assert_eq!(session.moj_slots()[0].tick_counters_d4[0], 2);
        let stepped_locals = session.moving_object_node_locals();
        assert_eq!(stepped_locals.len(), 1);
        assert_eq!(stepped_locals[0].0, 42);
        assert!(stepped_locals[0].1[14] > 0);
    }

    #[test]
    fn start_sequence_orbits_counts_down_and_launches_car() {
        use crate::audio_music_cues::CountdownCue;
        use crate::race_hud_plan::RaceHudCountdown;

        let mut session = GameplaySession::new(params(), InRaceScript::gameplay_harness())
            .with_start_sequence(true);
        let nodes = line();
        let mut progress = StageProgress::new(&nodes, 0);
        let mut camera_events = Vec::new();
        let mut cues = Vec::new();
        let mut saw_digit_5 = false;
        let mut saw_digit_1 = false;
        let mut saw_go = false;

        // Advance through the 75-tick orbit + 100-tick countdown + 19-tick GO! banner.
        let mut state5_ticks = 0u32;
        for _ in 0..400 {
            let frame = session
                .try_advance(&StationaryClock, frontend_menu(), forward(), &mut progress)
                .unwrap();
            if let Some(ev) = session.take_start_camera_event() {
                camera_events.push((state5_ticks, ev));
            }
            cues.extend(session.take_countdown_cues());
            match session.hud_countdown() {
                Some(RaceHudCountdown::Digit { digit: 5, .. }) => saw_digit_5 = true,
                Some(RaceHudCountdown::Digit { digit: 1, .. }) => saw_digit_1 = true,
                Some(RaceHudCountdown::Go { .. }) => saw_go = true,
                _ => {}
            }
            if frame.stepped {
                state5_ticks += 1;
                if state5_ticks == 75 {
                    assert_eq!(
                        session.car().position,
                        CarState::at_rest().position,
                        "car stays held at start line during 75-tick orbit"
                    );
                    assert_eq!(session.stage_clock(), Fixed16::ZERO);
                }
                if state5_ticks == 174 {
                    assert_eq!(
                        session.car().position,
                        CarState::at_rest().position,
                        "car stays held at start line until countdown hits zero"
                    );
                }
                if state5_ticks == 180 {
                    break;
                }
            }
        }
        assert_eq!(
            camera_events,
            vec![
                (0, StartCameraEvent::BeginOrbit),
                (74, StartCameraEvent::CrossfadeToChase),
            ]
        );
        assert_eq!(
            cues,
            vec![
                CountdownCue::Five,
                CountdownCue::Four,
                CountdownCue::Three,
                CountdownCue::Two,
                CountdownCue::One,
                CountdownCue::Go,
            ]
        );
        assert!(saw_digit_5 && saw_digit_1 && saw_go);
        assert_ne!(
            session.car().position,
            CarState::at_rest().position,
            "car launches and moves forward after GO!"
        );
    }
}
