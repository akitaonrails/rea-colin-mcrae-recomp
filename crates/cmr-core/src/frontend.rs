//! Platform-neutral contract for the first recovered frontend screen.
//!
//! Screen identifiers deliberately retain opaque legacy values: their user-facing
//! meanings are not established without consulting user-owned localization data.

use std::{error::Error, fmt};

use crate::config::LegacyConfig;

pub const ROOT_SCREEN_ID: ScreenId = ScreenId(0x00d4_8b80);
pub const CHAMPIONSHIP_SETUP_SCREEN_ID: ScreenId = ScreenId(0x00d4_0900);
/// Destination of root selection 1, whose matching root object is `Rally.C3D`.
pub const RALLY_SCREEN_ID: ScreenId = ScreenId(0x00d4_8230);
pub const CHAMPIONSHIP_NEXT_SCREEN_ID: ScreenId = ScreenId(0x00d4_5cf8);
/// Destination of root selection 2 (text 0x78), the time-trial setup.
pub const TIME_TRIAL_SCREEN_ID: ScreenId = ScreenId(0x00d4_b240);
pub const EXIT_SCREEN_ID: ScreenId = ScreenId(0x00d4_2058);
/// The rally name entry.
pub const RALLY_NAME_SCREEN_ID: ScreenId = ScreenId(0x00d4_7d90);
/// The time-trial name entry.
pub const TIME_TRIAL_NAME_SCREEN_ID: ScreenId = ScreenId(0x00d4_6fb0);
/// The time-trial stage select.
pub const STAGE_SELECT_SCREEN_ID: ScreenId = ScreenId(0x00d4_7450);
/// The Load screen (root entry 5).
pub const LOAD_SCREEN_ID: ScreenId = ScreenId(0x00d4_0da0);
/// The Options screen (root entry 4).
pub const OPTIONS_SCREEN_ID: ScreenId = ScreenId(0x00d4_53b8);
/// The network selection.
pub const NETWORK_SCREEN_ID: ScreenId = ScreenId(0x00d4_b6e0);
/// The empty "go to race" screen: a transition to it returns result 3
/// (`FUN_0044f6d0`), which leaves the front end for a race session.
pub const RACE_SENTINEL_SCREEN_ID: ScreenId = ScreenId(0x00d4_37c8);
/// The empty rally-school screen (root entry 3): a transition to it returns
/// result 4, which leaves the front end for a rally-school session.
pub const RALLY_SCHOOL_SENTINEL_SCREEN_ID: ScreenId = ScreenId(0x00d4_6b10);
pub const ROOT_FOOTER_TEXT_INDEX: usize = 253;

const ROOT_ENTRIES: [ScreenEntry; 7] = [
    ScreenEntry::new(0, CHAMPIONSHIP_SETUP_SCREEN_ID),
    ScreenEntry::new(1, RALLY_SCREEN_ID),
    ScreenEntry::new(120, TIME_TRIAL_SCREEN_ID),
    ScreenEntry::new(3, RALLY_SCHOOL_SENTINEL_SCREEN_ID),
    ScreenEntry::new(4, OPTIONS_SCREEN_ID),
    ScreenEntry::new(5, LOAD_SCREEN_ID),
    ScreenEntry::new(6, EXIT_SCREEN_ID),
];

pub const ROOT_SCREEN: ScreenSpec = ScreenSpec {
    id: ROOT_SCREEN_ID,
    title_text_index: None,
    parent: None,
    wrap_navigation: true,
    entries: &ROOT_ENTRIES,
    footer_text_index: Some(ROOT_FOOTER_TEXT_INDEX),
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ScreenId(u32);

impl ScreenId {
    /// The menu object at `address` (a recovered identifier).
    #[must_use]
    pub const fn from_legacy(address: u32) -> Self {
        Self(address)
    }

    /// Exposes the recovered identifier for evidence correlation and trace tests.
    #[must_use]
    pub const fn legacy_value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenEntry {
    pub text_index: usize,
    pub target: ScreenId,
    pub visible: bool,
    pub selectable: bool,
}

impl ScreenEntry {
    const fn new(text_index: usize, target: ScreenId) -> Self {
        Self {
            text_index,
            target,
            visible: true,
            selectable: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenSpec {
    pub id: ScreenId,
    pub title_text_index: Option<usize>,
    pub parent: Option<ScreenId>,
    pub wrap_navigation: bool,
    pub entries: &'static [ScreenEntry],
    pub footer_text_index: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidRootSelection {
    pub value: u8,
}

impl fmt::Display for InvalidRootSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "root selection must be in 0..=6, got {}",
            self.value
        )
    }
}

impl Error for InvalidRootSelection {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RootSelection(u8);

impl RootSelection {
    pub const fn new(value: u8) -> Result<Self, InvalidRootSelection> {
        if value < ROOT_ENTRIES.len() as u8 {
            Ok(Self(value))
        } else {
            Err(InvalidRootSelection { value })
        }
    }

    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }

    fn previous(self) -> Self {
        Self(if self.0 == 0 {
            (ROOT_ENTRIES.len() - 1) as u8
        } else {
            self.0 - 1
        })
    }

    fn next(self) -> Self {
        Self((usize::from(self.0) + 1).wrapping_rem(ROOT_ENTRIES.len()) as u8)
    }
}

/// Applies the only observed persisted-selection special case.
///
/// The shipped code maps `6` back to `0`, but does not safely handle values
/// above `6`. Rejecting those values is an explicit reconstruction safety rule.
pub fn root_selection_from_saved(value: u8) -> Result<RootSelection, InvalidRootSelection> {
    match value {
        0..=5 => RootSelection::new(value),
        6 => RootSelection::new(0),
        _ => Err(InvalidRootSelection { value }),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrontendPhase {
    Interactive = 1,
    AdvanceToPending = 2,
    AdvanceToInteractive = 3,
    BeginTransition = 4,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrontendInput {
    pub previous: bool,
    pub next: bool,
    pub confirm: bool,
    pub cancel: bool,
}

impl FrontendInput {
    #[must_use]
    pub const fn any(self) -> bool {
        self.previous || self.next || self.confirm || self.cancel
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionResult {
    Pending,
    Complete,
    StartRace,
    StartRallySchool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontendEffect {
    PersistRootSelection(u8),
    BeginTransition {
        from: ScreenId,
        to: ScreenId,
    },
    Activate(ScreenId),
    StartRace,
    /// Synchronizes config fields into runtime/profile state; this is not file I/O.
    ApplyConfigToProfileState,
    StartRallySchool,
    DisableConfigSave,
    StartAttract(ScreenId),
    SaveConfig,
    RequestExit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontendContinuation {
    Running,
    StartRace,
    StartRallySchool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontendState {
    continuation: FrontendContinuation,
    phase: FrontendPhase,
    active: ScreenId,
    pending: Option<ScreenId>,
    root_selection: RootSelection,
    idle_since: u32,
    exit_requested: bool,
}

impl FrontendState {
    pub fn from_config(config: &LegacyConfig, now: u32) -> Result<Self, InvalidRootSelection> {
        Self::after_startup_art(config.root_selection_byte(), now)
    }

    pub fn after_startup_art(saved_selection: u8, now: u32) -> Result<Self, InvalidRootSelection> {
        Ok(Self {
            continuation: FrontendContinuation::Running,
            phase: FrontendPhase::AdvanceToPending,
            active: ROOT_SCREEN_ID,
            pending: Some(ROOT_SCREEN_ID),
            root_selection: root_selection_from_saved(saved_selection)?,
            idle_since: now,
            exit_requested: false,
        })
    }

    #[must_use]
    pub const fn continuation(&self) -> FrontendContinuation {
        self.continuation
    }

    #[must_use]
    pub const fn frontend_active(&self) -> bool {
        matches!(self.continuation, FrontendContinuation::Running)
    }

    #[must_use]
    pub const fn phase(&self) -> FrontendPhase {
        self.phase
    }

    #[must_use]
    pub const fn active_screen(&self) -> ScreenId {
        self.active
    }

    #[must_use]
    pub const fn pending_screen(&self) -> Option<ScreenId> {
        self.pending
    }

    #[must_use]
    pub const fn root_selection(&self) -> RootSelection {
        self.root_selection
    }

    #[must_use]
    pub const fn idle_since(&self) -> u32 {
        self.idle_since
    }

    #[must_use]
    pub const fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// Advances the recovered frontend controller and returns ordered effects.
    ///
    /// `transition` is supplied by a platform/backend-neutral transition driver.
    /// `save_allowed` is the already-decoded legacy configuration save gate.
    #[must_use]
    pub fn step(
        &mut self,
        input: FrontendInput,
        now: u32,
        transition: TransitionResult,
        save_allowed: bool,
    ) -> Vec<FrontendEffect> {
        if self.continuation != FrontendContinuation::Running {
            return Vec::new();
        }

        match self.phase {
            FrontendPhase::Interactive => self.step_interactive(input, now),
            FrontendPhase::BeginTransition => {
                let Some(to) = self.pending else {
                    return Vec::new();
                };
                self.phase = FrontendPhase::AdvanceToPending;
                vec![FrontendEffect::BeginTransition {
                    from: self.active,
                    to,
                }]
            }
            FrontendPhase::AdvanceToPending => self.advance_to_pending(transition, save_allowed),
            FrontendPhase::AdvanceToInteractive => {
                if transition == TransitionResult::Complete {
                    self.phase = FrontendPhase::Interactive;
                }
                Vec::new()
            }
        }
    }

    /// A sub-screen's step left for `to` (`FUN_004af3a0` through the
    /// stepper's phase 1): the transition begins on the next step.
    pub fn begin_transition(&mut self, to: ScreenId) {
        if self.phase == FrontendPhase::Interactive {
            self.pending = Some(to);
            self.phase = FrontendPhase::BeginTransition;
        }
    }

    /// Re-enter the front end at the root menu after a race or rally-school
    /// session finishes or is abandoned (`FUN_004c1e00` -> `ROOT_SCREEN_ID`).
    pub fn resume_to_root(&mut self, now: u32) {
        self.continuation = FrontendContinuation::Running;
        self.phase = FrontendPhase::AdvanceToPending;
        self.active = ROOT_SCREEN_ID;
        self.pending = Some(ROOT_SCREEN_ID);
        self.idle_since = now;
        self.exit_requested = false;
    }

    fn step_interactive(&mut self, input: FrontendInput, now: u32) -> Vec<FrontendEffect> {
        if self.active != ROOT_SCREEN_ID {
            if input.any() {
                self.idle_since = now;
            }
            return Vec::new();
        }

        if input.any() {
            self.idle_since = now;
        } else if now.wrapping_sub(self.idle_since) > 59_999 {
            self.pending = Some(RACE_SENTINEL_SCREEN_ID);
            self.phase = FrontendPhase::BeginTransition;
            return vec![
                FrontendEffect::DisableConfigSave,
                FrontendEffect::StartAttract(RACE_SENTINEL_SCREEN_ID),
            ];
        }

        if input.previous {
            self.root_selection = self.root_selection.previous();
        } else if input.next {
            self.root_selection = self.root_selection.next();
        }

        let effects = vec![FrontendEffect::PersistRootSelection(
            self.root_selection.index(),
        )];
        if input.confirm && !input.cancel {
            let target = ROOT_ENTRIES[usize::from(self.root_selection.index())].target;
            self.pending = Some(target);
            self.phase = FrontendPhase::BeginTransition;
        }
        effects
    }

    fn advance_to_pending(
        &mut self,
        transition: TransitionResult,
        save_allowed: bool,
    ) -> Vec<FrontendEffect> {
        match transition {
            TransitionResult::Pending => Vec::new(),
            TransitionResult::StartRace => {
                self.continuation = FrontendContinuation::StartRace;
                vec![FrontendEffect::StartRace]
            }
            TransitionResult::StartRallySchool => {
                self.continuation = FrontendContinuation::StartRallySchool;
                vec![
                    FrontendEffect::ApplyConfigToProfileState,
                    FrontendEffect::StartRallySchool,
                ]
            }
            TransitionResult::Complete => {
                let Some(active) = self.pending.take() else {
                    return Vec::new();
                };
                self.active = active;
                self.phase = FrontendPhase::AdvanceToInteractive;
                let mut effects = vec![FrontendEffect::Activate(active)];
                if active == EXIT_SCREEN_ID {
                    if save_allowed {
                        effects.push(FrontendEffect::SaveConfig);
                    }
                    effects.push(FrontendEffect::RequestExit);
                    self.exit_requested = true;
                }
                effects
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interactive(selection: u8, idle_since: u32) -> FrontendState {
        FrontendState {
            continuation: FrontendContinuation::Running,
            phase: FrontendPhase::Interactive,
            active: ROOT_SCREEN_ID,
            pending: None,
            root_selection: RootSelection::new(selection).unwrap(),
            idle_since,
            exit_requested: false,
        }
    }

    #[test]
    fn root_spec_preserves_recovered_ids_and_text_indices() {
        assert_eq!(ROOT_SCREEN.id.legacy_value(), 0x00d4_8b80);
        assert_eq!(ROOT_SCREEN.title_text_index, None);
        assert_eq!(ROOT_SCREEN.parent, None);
        assert_eq!(ROOT_SCREEN.footer_text_index, Some(253));
        assert_eq!(
            ROOT_SCREEN
                .entries
                .iter()
                .map(|entry| (entry.text_index, entry.target.legacy_value()))
                .collect::<Vec<_>>(),
            vec![
                (0, 0x00d4_0900),
                (1, 0x00d4_8230),
                (120, 0x00d4_b240),
                (3, 0x00d4_6b10),
                (4, 0x00d4_53b8),
                (5, 0x00d4_0da0),
                (6, 0x00d4_2058),
            ]
        );
        assert!(
            ROOT_SCREEN
                .entries
                .iter()
                .all(|entry| entry.visible && entry.selectable)
        );
    }

    #[test]
    fn saved_selection_retains_zero_through_five_and_maps_six_to_zero() {
        for value in 0..=5 {
            assert_eq!(root_selection_from_saved(value).unwrap().index(), value);
        }
        assert_eq!(root_selection_from_saved(6).unwrap().index(), 0);
    }

    #[test]
    fn checked_and_saved_selection_reject_every_value_above_six_without_panicking() {
        for value in 7..=u8::MAX {
            assert_eq!(
                root_selection_from_saved(value),
                Err(InvalidRootSelection { value })
            );
            assert_eq!(
                RootSelection::new(value),
                Err(InvalidRootSelection { value })
            );
        }
        assert_eq!(
            InvalidRootSelection { value: 7 }.to_string(),
            "root selection must be in 0..=6, got 7"
        );
    }

    #[test]
    fn bootstrap_starts_the_observed_transition_with_safe_selection() {
        let state = FrontendState::after_startup_art(4, 123).unwrap();
        assert_eq!(state.phase, FrontendPhase::AdvanceToPending);
        assert!(state.frontend_active());
        assert_eq!(state.continuation(), FrontendContinuation::Running);
        assert_eq!(state.active, ROOT_SCREEN_ID);
        assert_eq!(state.pending, Some(ROOT_SCREEN_ID));
        assert_eq!(state.root_selection().index(), 4);
        assert_eq!(state.idle_since, 123);
        assert!(!state.exit_requested);
    }

    #[test]
    fn config_bridge_reads_only_the_recovered_selection_byte() {
        let mut bytes = [0xa5; crate::config::LEGACY_CONFIG_SIZE];
        bytes[0x24] = 6;
        let config = LegacyConfig::from_exact_bytes(&bytes).unwrap();
        let state = FrontendState::from_config(&config, 77).unwrap();
        assert_eq!(state.root_selection().index(), 0);
        assert_eq!(state.idle_since(), 77);

        bytes[0x24] = 7;
        let config = LegacyConfig::from_exact_bytes(&bytes).unwrap();
        assert_eq!(
            FrontendState::from_config(&config, 0).unwrap_err(),
            InvalidRootSelection { value: 7 }
        );
    }

    #[test]
    fn navigation_wraps_and_previous_precedes_next() {
        let mut state = interactive(0, 0);
        let _ = state.step(
            FrontendInput {
                previous: true,
                ..FrontendInput::default()
            },
            1,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.root_selection().index(), 6);

        let _ = state.step(
            FrontendInput {
                next: true,
                ..FrontendInput::default()
            },
            2,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.root_selection().index(), 0);

        let _ = state.step(
            FrontendInput {
                previous: true,
                next: true,
                ..FrontendInput::default()
            },
            3,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.root_selection().index(), 6);
    }

    #[test]
    fn root_cancel_is_a_no_op_but_selection_is_persisted() {
        let mut state = interactive(3, 0);
        let effects = state.step(
            FrontendInput {
                cancel: true,
                ..FrontendInput::default()
            },
            10,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.phase, FrontendPhase::Interactive);
        assert_eq!(state.pending, None);
        assert_eq!(state.idle_since, 10);
        assert_eq!(effects, vec![FrontendEffect::PersistRootSelection(3)]);
    }

    #[test]
    fn confirm_selects_every_exact_root_target() {
        for (selection, entry) in ROOT_ENTRIES.iter().enumerate() {
            let mut state = interactive(selection as u8, 0);
            let effects = state.step(
                FrontendInput {
                    confirm: true,
                    ..FrontendInput::default()
                },
                1,
                TransitionResult::Pending,
                true,
            );
            assert_eq!(state.phase, FrontendPhase::BeginTransition);
            assert_eq!(state.pending, Some(entry.target));
            assert_eq!(
                effects,
                vec![FrontendEffect::PersistRootSelection(selection as u8)]
            );
        }
    }

    #[test]
    fn cancel_suppresses_a_simultaneous_root_confirmation() {
        let mut state = interactive(2, 0);
        let _ = state.step(
            FrontendInput {
                confirm: true,
                cancel: true,
                ..FrontendInput::default()
            },
            1,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.phase, FrontendPhase::Interactive);
        assert_eq!(state.pending, None);
    }

    #[test]
    fn phase_trace_advances_four_to_two_to_three_to_one() {
        let target = ROOT_ENTRIES[0].target;
        let mut state = interactive(0, 0);
        state.pending = Some(target);
        state.phase = FrontendPhase::BeginTransition;

        assert_eq!(
            state.step(FrontendInput::default(), 0, TransitionResult::Pending, true),
            vec![FrontendEffect::BeginTransition {
                from: ROOT_SCREEN_ID,
                to: target
            }]
        );
        assert_eq!(state.phase, FrontendPhase::AdvanceToPending);
        assert_eq!(
            state.step(
                FrontendInput::default(),
                0,
                TransitionResult::Complete,
                true
            ),
            vec![FrontendEffect::Activate(target)]
        );
        assert_eq!(state.phase, FrontendPhase::AdvanceToInteractive);
        assert!(
            state
                .step(
                    FrontendInput::default(),
                    0,
                    TransitionResult::Complete,
                    true
                )
                .is_empty()
        );
        assert_eq!(state.phase, FrontendPhase::Interactive);
    }

    #[test]
    fn pending_transition_stays_in_its_current_phase() {
        for phase in [
            FrontendPhase::AdvanceToPending,
            FrontendPhase::AdvanceToInteractive,
        ] {
            let mut state = interactive(0, 0);
            state.phase = phase;
            state.pending = Some(ROOT_ENTRIES[0].target);
            assert!(
                state
                    .step(FrontendInput::default(), 0, TransitionResult::Pending, true)
                    .is_empty()
            );
            assert_eq!(state.phase, phase);
        }
    }

    #[test]
    fn phase_two_exposes_both_distinct_terminal_frontend_leave_results() {
        for (result, expected_effects, expected_continuation) in [
            (
                TransitionResult::StartRace,
                vec![FrontendEffect::StartRace],
                FrontendContinuation::StartRace,
            ),
            (
                TransitionResult::StartRallySchool,
                vec![
                    FrontendEffect::ApplyConfigToProfileState,
                    FrontendEffect::StartRallySchool,
                ],
                FrontendContinuation::StartRallySchool,
            ),
        ] {
            let mut state = FrontendState::after_startup_art(0, 0).unwrap();
            assert_eq!(
                state.step(FrontendInput::default(), 0, result, true),
                expected_effects
            );
            assert_eq!(state.phase, FrontendPhase::AdvanceToPending);
            assert!(!state.frontend_active());
            assert_eq!(state.continuation(), expected_continuation);
            let terminal_state = state.clone();
            assert!(
                state
                    .step(
                        FrontendInput {
                            previous: true,
                            next: true,
                            confirm: true,
                            cancel: true,
                        },
                        u32::MAX,
                        TransitionResult::Complete,
                        true,
                    )
                    .is_empty()
            );
            assert_eq!(state, terminal_state);
        }
    }

    #[test]
    fn phase_values_preserve_the_recovered_numeric_protocol() {
        assert_eq!(FrontendPhase::Interactive as u8, 1);
        assert_eq!(FrontendPhase::AdvanceToPending as u8, 2);
        assert_eq!(FrontendPhase::AdvanceToInteractive as u8, 3);
        assert_eq!(FrontendPhase::BeginTransition as u8, 4);
    }

    #[test]
    fn idle_attract_uses_the_strict_59999_boundary() {
        let mut state = interactive(0, 100);
        assert_eq!(
            state.step(
                FrontendInput::default(),
                60_099,
                TransitionResult::Pending,
                true
            ),
            vec![FrontendEffect::PersistRootSelection(0)]
        );
        assert_eq!(state.phase, FrontendPhase::Interactive);

        assert_eq!(
            state.step(
                FrontendInput::default(),
                60_100,
                TransitionResult::Pending,
                true
            ),
            vec![
                FrontendEffect::DisableConfigSave,
                FrontendEffect::StartAttract(RACE_SENTINEL_SCREEN_ID)
            ]
        );
        assert_eq!(state.phase, FrontendPhase::BeginTransition);
        assert_eq!(state.pending, Some(RACE_SENTINEL_SCREEN_ID));
    }

    #[test]
    fn any_input_resets_idle_baseline_before_the_threshold_check() {
        let mut state = interactive(0, 0);
        let _ = state.step(
            FrontendInput {
                next: true,
                ..FrontendInput::default()
            },
            60_000,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.idle_since, 60_000);
        assert_eq!(state.phase, FrontendPhase::Interactive);
    }

    #[test]
    fn idle_elapsed_time_uses_wrapping_tick_arithmetic() {
        let mut state = interactive(0, u32::MAX - 29_999);
        let _ = state.step(
            FrontendInput::default(),
            30_000,
            TransitionResult::Pending,
            true,
        );
        assert_eq!(state.phase, FrontendPhase::BeginTransition);
    }

    #[test]
    fn exit_activation_orders_optional_save_before_exit_request() {
        for (save_allowed, expected) in [
            (
                true,
                vec![
                    FrontendEffect::Activate(EXIT_SCREEN_ID),
                    FrontendEffect::SaveConfig,
                    FrontendEffect::RequestExit,
                ],
            ),
            (
                false,
                vec![
                    FrontendEffect::Activate(EXIT_SCREEN_ID),
                    FrontendEffect::RequestExit,
                ],
            ),
        ] {
            let mut state = FrontendState::after_startup_art(0, 0).unwrap();
            state.pending = Some(EXIT_SCREEN_ID);
            assert_eq!(
                state.step(
                    FrontendInput::default(),
                    0,
                    TransitionResult::Complete,
                    save_allowed
                ),
                expected
            );
            assert!(state.exit_requested);
        }
    }

    #[test]
    fn resume_to_root_restores_running_frontend_after_race_handoff() {
        let mut state = FrontendState::after_startup_art(2, 100).unwrap();
        state.pending = Some(RACE_SENTINEL_SCREEN_ID);
        assert_eq!(
            state.step(
                FrontendInput::default(),
                200,
                TransitionResult::StartRace,
                true,
            ),
            vec![FrontendEffect::StartRace]
        );
        assert!(!state.frontend_active());

        state.resume_to_root(5_000);
        assert!(state.frontend_active());
        assert_eq!(state.active_screen(), ROOT_SCREEN_ID);
        assert_eq!(state.pending_screen(), Some(ROOT_SCREEN_ID));
        assert_eq!(state.phase(), FrontendPhase::AdvanceToPending);
        assert_eq!(
            state.step(
                FrontendInput::default(),
                5_040,
                TransitionResult::Complete,
                true,
            ),
            vec![FrontendEffect::Activate(ROOT_SCREEN_ID)]
        );
        assert_eq!(
            state.step(
                FrontendInput::default(),
                5_080,
                TransitionResult::Complete,
                true,
            ),
            Vec::<FrontendEffect>::new()
        );
        assert_eq!(state.phase(), FrontendPhase::Interactive);
    }
}
