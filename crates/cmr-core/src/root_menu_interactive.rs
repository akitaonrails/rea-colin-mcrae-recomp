//! Pure interactive controller for the recovered front end: the root menu,
//! the setup, name-entry and stage-select screens ([`FrontendScreens`]), and
//! the screen-to-screen camera transition.
//!
//! This composes the recovered [`FrontendState`] controller with the
//! stepper's transition (`FUN_004c20f0` phases 2-4): a confirmed screen
//! change starts the 13-step camera ease `FUN_0044f6d0`; each following step
//! evaluates it (`FUN_0044f600`), which on its sixth step returns the hand-off
//! code (3 for the race sentinel `0xd437c8`, 4 for the rally-school sentinel
//! `0xd46b10`), leaving the front end, and once it has ended activates the
//! destination. While the camera moves, the screen being left keeps stepping
//! with no input (its slide-out); after the activation the new screen steps
//! once more with no input before it takes input. The sub-screens read the
//! front end's repeated key (`FUN_004c2970`) and its menu bits
//! (`FUN_004c25f0`). Destinations that are not reconstructed (options, load,
//! network) wait. The root's own input stays the host's [`FrontendInput`].
//! The live selection's chrome is snapped to its recovered endpoint via
//! [`steady_root_render_input`].

use crate::{
    config::LEGACY_CONFIG_SIZE,
    display::DisplayExtent,
    frontend::{
        EXIT_SCREEN_ID, FrontendEffect, FrontendInput, FrontendPhase, FrontendState,
        InvalidRootSelection, RACE_SENTINEL_SCREEN_ID, RALLY_SCHOOL_SENTINEL_SCREEN_ID,
        ROOT_SCREEN_ID, ScreenId, TransitionResult,
    },
    frontend_camera::{Camera, CameraBlock},
    frontend_flow::{FrontendScreens, ScreenEffect, SetupLayout},
    frontend_keys::{KeyRepeat, menu_bits},
    frontend_name_entry::NameKey,
    localization::Language,
    root_menu_visual::{RootRenderInput, steady_root_render_input},
    scalar_tween::ScalarTween,
};

type Record = [u8; LEGACY_CONFIG_SIZE];

/// Host-visible outcome of one interactive front-end tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootTick {
    /// Steady render input while the interactive root screen is showing, or
    /// `None` once a confirmed selection has begun leaving it (or the frontend
    /// has stopped running).
    pub render_input: Option<RootRenderInput>,
    /// Recovered controller effects emitted by this tick, in order.
    pub effects: Vec<FrontendEffect>,
    /// The sub-screens' effects (the record commit, menu sounds).
    pub screen_effects: Vec<ScreenEffect>,
    /// Whether the recovered controller has requested process exit.
    pub exit_requested: bool,
}

/// The keyboard and pad state the sub-screens read each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontendKeys {
    /// `FUN_0049d1c0`'s held key.
    pub held: Option<u8>,
    /// The device's pad bits.
    pub pad: u32,
}

/// The camera transition `FUN_0044f6d0`/`FUN_0044f600`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CameraTransition {
    tween: ScalarTween,
    /// `DAT_0055d76c`.
    steps: u32,
    /// `DAT_0055d768`.
    latch: u32,
    /// `DAT_004f1310`.
    code: u32,
}

/// `DAT_0055d770`: the camera ease's steps.
const CAMERA_STEPS: i32 = 13;

impl CameraTransition {
    fn start(to: ScreenId) -> Self {
        let code = if to == RACE_SENTINEL_SCREEN_ID {
            3
        } else if to == RALLY_SCHOOL_SENTINEL_SCREEN_ID {
            4
        } else {
            2
        };
        Self {
            tween: ScalarTween::ease_out(0, 0x10000, CAMERA_STEPS, 0),
            steps: 0,
            latch: 1,
            code,
        }
    }

    /// `FUN_0044f600`: 1 while the ease runs, the hand-off code from its
    /// sixth step, 0 once it has ended.
    fn step(&mut self, camera: &mut Camera) -> u32 {
        if self.tween.is_finished() {
            self.latch = 0;
            return 0;
        }
        camera.place(self.tween.evaluate_and_advance());
        self.steps += 1;
        if self.steps == (CAMERA_STEPS / 2) as u32 {
            self.latch = self.code;
        }
        self.latch
    }
}

/// Interactive front-end driver over the recovered [`FrontendState`].
pub struct RootInteraction {
    state: FrontendState,
    language: Language,
    viewport_extent: DisplayExtent,
    screens: FrontendScreens,
    camera: CameraTransition,
    /// `DAT_004f1300/4`.
    position: Camera,
    targets: Option<CameraBlock>,
    repeat: KeyRepeat,
}

impl RootInteraction {
    /// Starts the driver from the recovered saved-selection byte, in the same
    /// post-startup-art state the recovered frontend bootstraps into (its
    /// camera transition to the root already started, `0x4c0b32`).
    pub fn from_saved_selection(
        saved_selection: u8,
        language: Language,
        viewport_extent: DisplayExtent,
        now: u32,
    ) -> Result<Self, InvalidRootSelection> {
        Ok(Self {
            state: FrontendState::after_startup_art(saved_selection, now)?,
            language,
            viewport_extent,
            screens: FrontendScreens::new(None),
            camera: CameraTransition::start(ROOT_SCREEN_ID),
            position: Camera::at(
                crate::root_menu_visual::ROOT_INITIAL_HORIZONTAL_TRANSFORM_Q16,
                crate::root_menu_visual::ROOT_INITIAL_VERTICAL_TRANSFORM_Q16,
            ),
            targets: None,
            repeat: KeyRepeat::default(),
        })
    }

    /// Use the camera targets and the row arc of the user's executable
    /// (block `0x507a78`); the boot camera is the root's target.
    #[must_use]
    pub fn with_camera_block(mut self, block: CameraBlock) -> Self {
        if let Some((x, y)) = block.target(ROOT_SCREEN_ID) {
            self.position = Camera::at(x, y);
        }
        self.screens.set_geometry(block.arc(), self.viewport_extent);
        self.targets = Some(block);
        self
    }

    /// The options row screens' slide templates from the user's executable.
    #[must_use]
    pub fn with_option_templates(
        mut self,
        templates: crate::frontend_options::RowTemplates,
    ) -> Self {
        self.screens.set_option_templates(templates);
        self
    }

    /// `FUN_004ba060` at the config load: the record's bindings into the
    /// keyboard device records.
    pub fn install_keyboard_record(&mut self, record: &Record) {
        self.screens.install_keyboard_record(record);
    }

    /// Enable the modern graphics (`ENHANCED`), high-resolution `ADVANCED`,
    /// and `GAMEPAD 1` / `MODERN GAMEPAD` menus and defaults.
    pub fn enable_modern_mode(&mut self, settings: crate::modern_settings::ModernSettings) {
        self.screens.enable_modern_mode(settings);
    }

    /// The name entry's encoded cheat names (`FUN_004c6880`'s strings).
    #[must_use]
    pub fn with_name_cheats(mut self, cheats: Vec<Vec<u8>>) -> Self {
        self.screens.set_name_cheats(cheats);
        self
    }

    /// The host's save-slot reader the Load screen calls on entry.
    #[must_use]
    pub fn with_save_slots(mut self, reader: crate::frontend_flow::SlotReader) -> Self {
        self.screens.set_slot_reader(reader);
        self
    }

    /// The camera position `DAT_004f1300/4`.
    #[must_use]
    pub const fn camera(&self) -> Camera {
        self.position
    }

    /// Use the setups' layout descriptors from the user's executable.
    #[must_use]
    pub fn with_setup_layouts(mut self, layouts: [SetupLayout; 3]) -> Self {
        self.screens.set_layouts(layouts);
        self
    }

    /// Borrows the underlying recovered controller for inspection.
    #[must_use]
    pub const fn state(&self) -> &FrontendState {
        &self.state
    }

    /// The sub-screens, for the draw.
    #[must_use]
    pub const fn screens(&self) -> &FrontendScreens {
        &self.screens
    }

    /// Re-enter the front end at the root menu after a race or rally-school
    /// session finishes or is abandoned (`FUN_004c1e00`).
    pub fn resume_to_root(&mut self, now: u32) {
        self.state.resume_to_root(now);
        self.camera = CameraTransition::start(ROOT_SCREEN_ID);
        self.position.aim(
            self.targets
                .as_ref()
                .and_then(|block| block.target(ROOT_SCREEN_ID)),
        );
        self.repeat = KeyRepeat::default();
    }

    /// Advances the root alone by one tick (no record: the sub-screens are
    /// not entered).
    #[must_use]
    pub fn tick(&mut self, input: FrontendInput, now: u32, save_allowed: bool) -> RootTick {
        self.advance(input, FrontendKeys::default(), now, save_allowed, None)
    }

    /// Advances the whole front end by one tick over the cmRally.cfg
    /// `record` the screens edit.
    #[must_use]
    pub fn tick_for_game_session(
        &mut self,
        input: FrontendInput,
        keys: FrontendKeys,
        now: u32,
        save_allowed: bool,
        record: &mut Record,
    ) -> RootTick {
        self.advance(input, keys, now, save_allowed, Some(record))
    }

    fn advance(
        &mut self,
        input: FrontendInput,
        keys: FrontendKeys,
        now: u32,
        save_allowed: bool,
        mut record: Option<&mut Record>,
    ) -> RootTick {
        let phase = self.state.phase();
        let active = self.state.active_screen();
        let mut screen_effects = Vec::new();
        let sub_screen = FrontendScreens::handles(active);
        // Phase 1 reads the repeated key every frame (`0x4c2249`).
        let key = if phase == FrontendPhase::Interactive {
            self.repeat.poll(keys.held, now)
        } else {
            0
        };
        if let Some(record) = record.as_deref_mut()
            && sub_screen
        {
            let bits = if phase == FrontendPhase::Interactive {
                menu_bits(key, keys.pad)
            } else {
                0
            };
            let name_key = if phase == FrontendPhase::Interactive {
                NameKey {
                    vk: key,
                    pad: keys.pad,
                }
            } else {
                NameKey::default()
            };
            // Phase 3 steps the new screen after the camera check; phases 2
            // and 4 step the old one; phase 1 steps it with the input.
            let (target, effects) = self.screens.step(active, bits, name_key, record);
            screen_effects.extend(effects);
            self.screens.draw_effects(active, record);
            if phase == FrontendPhase::Interactive {
                if let Some(target) = target {
                    self.state.begin_transition(target);
                }
                return RootTick {
                    render_input: None,
                    effects: Vec::new(),
                    screen_effects,
                    exit_requested: self.state.exit_requested(),
                };
            }
        }
        // The root's sound table `0xd90070`: a move (it wraps, so it always
        // moves) and the confirmed entry.
        if phase == FrontendPhase::Interactive && active == ROOT_SCREEN_ID {
            if input.previous || input.next {
                screen_effects.push(ScreenEffect::Cue { slot: 0, ok: true });
            }
            if input.confirm && !input.cancel {
                screen_effects.push(ScreenEffect::Cue { slot: 3, ok: true });
            }
        }
        let transition = self.resolve_transition(record.is_some());
        let effects = self.state.step(input, now, transition, save_allowed);
        for effect in &effects {
            match *effect {
                FrontendEffect::BeginTransition { to, .. } => {
                    self.camera = CameraTransition::start(to);
                    self.position
                        .aim(self.targets.as_ref().and_then(|block| block.target(to)));
                }
                FrontendEffect::Activate(screen) if FrontendScreens::handles(screen) => {
                    if let Some(record) = record.as_deref_mut() {
                        screen_effects.extend(self.screens.activate(screen, record));
                        self.screens.draw_effects(screen, record);
                    }
                }
                _ => {}
            }
        }
        RootTick {
            render_input: self.current_render_input(),
            exit_requested: self.state.exit_requested(),
            effects,
            screen_effects,
        }
    }

    /// The camera step of phases 2 and 3: the hand-off codes leave the
    /// front end; the end of the ease activates a reconstructed destination.
    fn resolve_transition(&mut self, with_record: bool) -> TransitionResult {
        match self.state.phase() {
            FrontendPhase::AdvanceToPending => {
                let target = self.state.pending_screen();
                match self.camera.step(&mut self.position) {
                    3 => TransitionResult::StartRace,
                    4 => TransitionResult::StartRallySchool,
                    0 if target.is_some_and(|screen| is_reconstructed(screen, with_record)) => {
                        TransitionResult::Complete
                    }
                    _ => TransitionResult::Pending,
                }
            }
            FrontendPhase::AdvanceToInteractive => {
                self.camera.step(&mut self.position);
                TransitionResult::Complete
            }
            FrontendPhase::Interactive | FrontendPhase::BeginTransition => {
                TransitionResult::Pending
            }
        }
    }

    /// The root is drawn while it is the current menu: interactive, and
    /// while the camera leaves it (its chrome is gated on the phase).
    fn current_render_input(&self) -> Option<RootRenderInput> {
        let phase = self.state.phase();
        if self.state.frontend_active()
            && self.state.active_screen() == ROOT_SCREEN_ID
            && matches!(
                phase,
                FrontendPhase::Interactive
                    | FrontendPhase::BeginTransition
                    | FrontendPhase::AdvanceToPending
            )
            && (phase == FrontendPhase::Interactive
                || self
                    .state
                    .pending_screen()
                    .is_some_and(|pending| pending != ROOT_SCREEN_ID))
        {
            let mut input = steady_root_render_input(
                self.state.root_selection(),
                self.language,
                self.viewport_extent,
                false,
            );
            input.phase = phase;
            input.horizontal_transform_q16 = self.position.x;
            input.vertical_transform_q16 = self.position.y;
            Some(input)
        } else {
            None
        }
    }
}

fn is_reconstructed(screen: ScreenId, with_record: bool) -> bool {
    screen == ROOT_SCREEN_ID
        || screen == EXIT_SCREEN_ID
        || (with_record && FrontendScreens::handles(screen))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{CHAMPIONSHIP_NEXT_SCREEN_ID, CHAMPIONSHIP_SETUP_SCREEN_ID};
    use crate::root_menu_visual::steady_root_render_input;

    const EXTENT: DisplayExtent = DisplayExtent {
        width: 640,
        height: 480,
    };

    fn driver(saved: u8) -> RootInteraction {
        RootInteraction::from_saved_selection(saved, Language::English, EXTENT, 0).unwrap()
    }

    fn press(input: FrontendInput) -> FrontendInput {
        input
    }

    const CONFIRM: FrontendInput = FrontendInput {
        previous: false,
        next: false,
        confirm: true,
        cancel: false,
    };

    /// Tick with no input until the front end is interactive again.
    fn settle(driver: &mut RootInteraction, now: &mut u32) -> RootTick {
        loop {
            *now += 1;
            let tick = driver.tick(FrontendInput::default(), *now, true);
            if driver.state().phase() == FrontendPhase::Interactive || *now > 1000 {
                return tick;
            }
        }
    }

    #[test]
    fn the_camera_takes_thirteen_steps_then_the_root_is_interactive() {
        let mut driver = driver(0);
        let mut ticks = 0;
        let mut now = 0;
        let settled = loop {
            now += 1;
            ticks += 1;
            let tick = driver.tick(FrontendInput::default(), now, true);
            if tick.render_input.is_some() {
                break tick;
            }
            assert!(ticks < 100);
        };
        // 13 eased steps, the activating step, then one more (phase 3).
        assert_eq!(ticks, 15);
        assert_eq!(
            settled.render_input,
            Some(steady_root_render_input(
                driver.state().root_selection(),
                Language::English,
                EXTENT,
                false
            ))
        );
        assert!(!settled.exit_requested);
    }

    #[test]
    fn a_next_pulse_advances_the_live_selection_render_input() {
        let mut driver = driver(0);
        let mut now = 0;
        let _ = settle(&mut driver, &mut now);
        let advanced = driver.tick(
            press(FrontendInput {
                next: true,
                ..FrontendInput::default()
            }),
            now + 1,
            true,
        );
        assert_eq!(
            advanced.render_input.map(|input| input.selection.index()),
            Some(1)
        );
        assert_eq!(
            advanced.effects,
            vec![FrontendEffect::PersistRootSelection(1)]
        );
    }

    #[test]
    fn confirming_exit_drives_through_to_the_recovered_exit_effects() {
        for (save_allowed, expect_save) in [(true, true), (false, false)] {
            let mut driver = driver(6);
            let mut now = 0;
            let _ = settle(&mut driver, &mut now);
            let last = (crate::frontend::ROOT_SCREEN.entries.len() - 1) as u8;
            while driver.state().root_selection().index() != last {
                now += 1;
                let _ = driver.tick(
                    FrontendInput {
                        previous: true,
                        ..FrontendInput::default()
                    },
                    now,
                    save_allowed,
                );
            }
            now += 1;
            let confirm = driver.tick(CONFIRM, now, save_allowed);
            assert!(!confirm.exit_requested, "confirm begins the transition");
            let mut exit_tick = None;
            for _ in 0..20 {
                now += 1;
                let tick = driver.tick(FrontendInput::default(), now, save_allowed);
                if tick.exit_requested {
                    exit_tick = Some(tick);
                    break;
                }
            }
            let exit = exit_tick.expect("exit must be reached");
            assert!(exit.render_input.is_none());
            assert!(exit.effects.contains(&FrontendEffect::RequestExit));
            assert_eq!(
                exit.effects.contains(&FrontendEffect::SaveConfig),
                expect_save
            );
        }
    }

    #[test]
    fn without_a_record_the_setup_screens_wait() {
        let mut driver = driver(0);
        let mut now = 0;
        let _ = settle(&mut driver, &mut now);
        now += 1;
        let _ = driver.tick(CONFIRM, now, true);
        for _ in 0..40 {
            now += 1;
            let tick = driver.tick(FrontendInput::default(), now, true);
            assert!(!tick.exit_requested);
            // The root stays drawn (without its chrome) while it is left.
            assert!(
                tick.render_input
                    .is_none_or(|input| input.phase != FrontendPhase::Interactive)
            );
        }
        assert_ne!(driver.state().phase(), FrontendPhase::Interactive);
    }

    fn record() -> Box<Record> {
        let mut record = Box::new([0u8; LEGACY_CONFIG_SIZE]);
        crate::config_commit::apply_defaults(
            &mut record,
            crate::config_commit::DefaultsHost {
                token: 0,
                language: 0,
                large_memory: false,
                computer_name: None,
                player_text: b"Player",
            },
            None,
        );
        record
    }

    #[test]
    fn a_championship_goes_from_the_root_through_its_screens_to_the_race() {
        let mut driver = driver(0);
        let mut record = record();
        let mut now = 0;
        let mut tick = |driver: &mut RootInteraction,
                        input: FrontendInput,
                        held: Option<u8>,
                        record: &mut Record| {
            now += 50;
            driver.tick_for_game_session(input, FrontendKeys { held, pad: 0 }, now, true, record)
        };
        while driver.state().phase() != FrontendPhase::Interactive {
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        }
        let _ = tick(&mut driver, CONFIRM, None, &mut record);
        // The camera, the setup's activation and its slide-in.
        let mut frames = 0;
        while !(driver.state().active_screen() == CHAMPIONSHIP_SETUP_SCREEN_ID
            && driver.state().phase() == FrontendPhase::Interactive
            && driver.screens().slides_idle())
        {
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
            frames += 1;
            assert!(frames < 100);
        }
        // The commit re-enables the navigation after the step that read the
        // input, so one more frame passes before Return is taken.
        let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        assert!(
            driver
                .screens()
                .menu(CHAMPIONSHIP_SETUP_SCREEN_ID)
                .is_some_and(|menu| menu.nav.confirms)
        );
        // Return on Continue; release; the name entry.
        let _ = tick(
            &mut driver,
            FrontendInput::default(),
            Some(0x0d),
            &mut record,
        );
        let mut frames = 0;
        while !(driver.state().active_screen() == CHAMPIONSHIP_NEXT_SCREEN_ID
            && driver.state().phase() == FrontendPhase::Interactive
            && driver.screens().slides_idle())
        {
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
            frames += 1;
            assert!(frames < 100);
        }
        let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        let enter = tick(
            &mut driver,
            FrontendInput::default(),
            Some(0x0d),
            &mut record,
        );
        assert!(enter.screen_effects.contains(&ScreenEffect::CommitRecord));
        // The camera's sixth step hands off to the race.
        let mut started = false;
        for _ in 0..20 {
            let tick = tick(&mut driver, FrontendInput::default(), None, &mut record);
            if tick.effects.contains(&FrontendEffect::StartRace) {
                started = true;
                break;
            }
        }
        assert!(started);
        assert_eq!(record[0x180] & 3, 0, "a championship");
    }

    #[test]
    fn the_load_screen_loads_a_saved_slot_and_starts_the_race() {
        use crate::frontend_load::SlotFile;
        let mut live = vec![0u8; 0x6c4];
        live[0x2c..0x31].copy_from_slice(b"COLIN");
        let mut saved = crate::save_dialog::build_save_record(&live, 0xaa00_0003);
        saved[0x148] = 0x12;
        let mut driver = driver(5).with_save_slots(Box::new(move || {
            let mut files = vec![SlotFile::default(); 12];
            files[0].bytes = Some(saved.clone());
            files
        }));
        let mut record = record();
        let mut now = 0;
        let mut tick = |driver: &mut RootInteraction,
                        input: FrontendInput,
                        held: Option<u8>,
                        record: &mut Record| {
            now += 50;
            driver.tick_for_game_session(input, FrontendKeys { held, pad: 0 }, now, true, record)
        };
        while driver.state().phase() != FrontendPhase::Interactive {
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        }
        let _ = tick(&mut driver, CONFIRM, None, &mut record);
        let mut frames = 0;
        while !(driver.state().active_screen() == crate::frontend::LOAD_SCREEN_ID
            && driver.state().phase() == FrontendPhase::Interactive)
        {
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
            frames += 1;
            assert!(frames < 100, "the Load screen is reached");
        }
        let screen = driver.screens().load_screen();
        assert!(screen.slot(0).is_some() && screen.slot(1).is_none());
        assert!(screen.menu.items[0].enabled && !screen.menu.items[1].enabled);
        let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        let load = tick(
            &mut driver,
            FrontendInput::default(),
            Some(0x0d),
            &mut record,
        );
        assert!(
            load.screen_effects
                .contains(&ScreenEffect::LoadGame { slot: 0 })
        );
        assert_eq!(&record[0x34..0x39], b"COLIN");
        assert_eq!(record[0x181], 0x12);
        let mut started = false;
        for _ in 0..20 {
            if tick(&mut driver, FrontendInput::default(), None, &mut record)
                .effects
                .contains(&FrontendEffect::StartRace)
            {
                started = true;
                break;
            }
        }
        assert!(started, "the load starts the race");
    }

    #[test]
    fn the_options_screens_are_reached_and_write_the_record() {
        use crate::frontend_options::OptionScreen;
        let mut driver = driver(4);
        let mut record = record();
        record[0x20] = (record[0x20] & !0x18) | 0x08;
        let now = std::cell::Cell::new(0);
        let tick = |driver: &mut RootInteraction,
                    input: FrontendInput,
                    held: Option<u8>,
                    record: &mut Record| {
            now.set(now.get() + 50);
            driver.tick_for_game_session(
                input,
                FrontendKeys { held, pad: 0 },
                now.get(),
                true,
                record,
            )
        };
        let wait_for = |driver: &mut RootInteraction, screen: ScreenId, record: &mut Record| {
            let mut frames = 0;
            while !(driver.state().active_screen() == screen
                && driver.state().phase() == FrontendPhase::Interactive)
            {
                let _ = tick(driver, FrontendInput::default(), None, record);
                frames += 1;
                assert!(frames < 100, "{screen:?} is reached");
            }
            let _ = tick(driver, FrontendInput::default(), None, record);
        };
        wait_for(&mut driver, ROOT_SCREEN_ID, &mut record);
        let _ = tick(&mut driver, CONFIRM, None, &mut record);
        wait_for(&mut driver, OptionScreen::Options.screen(), &mut record);
        // Down twice to Difficulty, Return.
        for vk in [0x28, 0x28, 0x0d] {
            let _ = tick(&mut driver, FrontendInput::default(), Some(vk), &mut record);
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        }
        wait_for(&mut driver, OptionScreen::Difficulty.screen(), &mut record);
        let menu = driver
            .screens()
            .menu(OptionScreen::Difficulty.screen())
            .unwrap();
        assert_eq!(menu.selected, 1, "intermediate selected");
        // Up to Novice, Return: written, and back to Options.
        for vk in [0x26, 0x0d] {
            let _ = tick(&mut driver, FrontendInput::default(), Some(vk), &mut record);
            let _ = tick(&mut driver, FrontendInput::default(), None, &mut record);
        }
        assert_eq!(record[0x20] & 0x18, 0);
        wait_for(&mut driver, OptionScreen::Options.screen(), &mut record);
    }

    #[test]
    fn the_settings_open_the_control_setup() {
        use crate::frontend_controls::ControlScreen;
        assert!(FrontendScreens::handles(ControlScreen::Control.screen()));
        let mut driver = driver(4);
        let mut record = record();
        let now = std::cell::Cell::new(0);
        let tick = |driver: &mut RootInteraction, held: Option<u8>, record: &mut Record| {
            now.set(now.get() + 50);
            driver.tick_for_game_session(
                FrontendInput::default(),
                FrontendKeys { held, pad: 0 },
                now.get(),
                true,
                record,
            )
        };
        let press = |driver: &mut RootInteraction, vk: u8, record: &mut Record| {
            let mut frames = 0;
            loop {
                let active = driver.state().active_screen();
                if driver.state().phase() == FrontendPhase::Interactive
                    && driver
                        .screens()
                        .menu(active)
                        .is_some_and(|m| m.nav.confirms)
                {
                    break;
                }
                let _ = tick(driver, None, record);
                frames += 1;
                assert!(frames < 200, "{active:?} takes input");
            }
            let _ = tick(driver, Some(vk), record);
            let _ = tick(driver, None, record);
        };
        while driver.state().phase() != FrontendPhase::Interactive {
            let _ = tick(&mut driver, None, &mut record);
        }
        now.set(now.get() + 50);
        let _ = driver.tick_for_game_session(
            CONFIRM,
            FrontendKeys::default(),
            now.get(),
            true,
            &mut record,
        );
        press(&mut driver, 0x0d, &mut record);
        for vk in [0x28, 0x28, 0x0d] {
            press(&mut driver, vk, &mut record);
        }
        let mut frames = 0;
        while driver.state().active_screen() != ControlScreen::Control.screen() {
            let _ = tick(&mut driver, None, &mut record);
            frames += 1;
            assert!(
                frames < 100,
                "the control setup opens (at {:?}, pending {:?}, phase {:?})",
                driver.state().active_screen(),
                driver.state().pending_screen(),
                driver.state().phase()
            );
        }
        for _ in 0..30 {
            let _ = tick(&mut driver, None, &mut record);
        }
        assert_eq!(
            driver.screens().controls().panel_x,
            0x1999,
            "the panel slid in"
        );
        let menu = driver
            .screens()
            .menu(ControlScreen::Control.screen())
            .unwrap();
        assert!(menu.nav.confirms && !menu.nav.backs);
    }

    #[test]
    fn resume_to_root_recovers_root_menu_after_race_handoff() {
        let mut driver = RootInteraction::from_saved_selection(
            0,
            Language::English,
            DisplayExtent {
                width: 640,
                height: 480,
            },
            0,
        )
        .unwrap();
        let mut record = [0u8; crate::config::LEGACY_CONFIG_SIZE];
        while driver.state().phase() != FrontendPhase::Interactive {
            let _ = driver.tick_for_game_session(
                FrontendInput::default(),
                FrontendKeys::default(),
                0,
                true,
                &mut record,
            );
        }
        driver.resume_to_root(1_000);
        for step in 0..20 {
            let _ = driver.tick_for_game_session(
                FrontendInput::default(),
                FrontendKeys::default(),
                1_000 + step * 40,
                true,
                &mut record,
            );
            if driver.state().phase() == FrontendPhase::Interactive {
                break;
            }
        }
        assert!(driver.state().frontend_active());
        assert_eq!(driver.state().active_screen(), ROOT_SCREEN_ID);
        assert_eq!(driver.state().phase(), FrontendPhase::Interactive);
        assert!(driver.current_render_input().is_some());
    }
}
