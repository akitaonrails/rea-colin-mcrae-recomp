//! The front end's setup, name-entry and stage-select screens as the
//! descriptor-menu tree `FUN_004be540` builds, driven the way the stepper
//! `FUN_004c20f0` drives them.
//!
//! Each frame the stepper steps the current menu (`FUN_004aee60`) and then
//! the frame draws it; the draw callbacks have side effects the next step
//! reads (the setups' PLAYERS item hides the second car, the car and country
//! items gate Continue; the stage select's predraw routes OK). So a frame here
//! is [`FrontendScreens::step`] then [`FrontendScreens::draw_effects`]. The
//! name entries get no menu bits (`0x4c2258`): their commit runs the key
//! handler. A screen accepts input only once its slide-in has ended: the
//! setups' seven row channels (`FUN_004c4410`, 15-step ease-outs after each
//! row's delay from the screen's layout descriptor), the name entries' two
//! slides and the stage select's slide (15 steps each).
//!
//! The layout descriptors are static data of the user's executable; without
//! them every row delay is taken as zero (harness). A description of
//! disassembly, not original source. Evidence:
//! `rea.official-eu.frontend-flow`.

use crate::config::LEGACY_CONFIG_SIZE;
use crate::descriptor_menu::{Menu, MenuEvent, MenuId, MenuNav, MenuSet};
use crate::display::DisplayExtent;
use crate::frontend::{
    CHAMPIONSHIP_NEXT_SCREEN_ID, CHAMPIONSHIP_SETUP_SCREEN_ID, LOAD_SCREEN_ID, NETWORK_SCREEN_ID,
    RACE_SENTINEL_SCREEN_ID, RALLY_NAME_SCREEN_ID, RALLY_SCREEN_ID, ROOT_SCREEN_ID,
    STAGE_SELECT_SCREEN_ID, ScreenId, TIME_TRIAL_NAME_SCREEN_ID, TIME_TRIAL_SCREEN_ID,
};
use crate::frontend_controls::{ControlScreen, ControlScreens, ControlsEffect, InputDevices};
use crate::frontend_load::{LoadScreen, SlotFile};
use crate::frontend_name_entry::{NameEntry, NameKey, NameOutcome, NameScreen};
use crate::frontend_options::{DisplayCaps, OptionScreen, OptionsEffect, OptionsScreens};
use crate::frontend_setup::{SetupKind, SetupScreen, key};
use crate::frontend_stage_select::{PostOutcome, StageSelect};
use crate::scalar_tween::ScalarTween;

type Record = [u8; LEGACY_CONFIG_SIZE];

/// One row channel of a setup layout descriptor (8 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlideChannel {
    /// The angles the row swings through: in `a0 -> a1`, out `a1 -> a2`.
    pub angles: [i16; 3],
    pub delay_in: u8,
    pub delay_out: u8,
}

/// A setup screen's layout descriptor (`0x5092e8`/`0x509338`/`0x509388`,
/// 0x50 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetupLayout {
    pub channels: [SlideChannel; 7],
    /// `+0x38`/`+0x3c`: the info box anchor (Q16).
    pub info: [i32; 2],
    /// `+0x40`.
    pub info_flags: u32,
    /// `+0x48`: the item text flags.
    pub item_flags: u32,
}

impl SetupLayout {
    /// Parse the 0x50-byte descriptor; `None` when it is shorter.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 0x50 {
            return None;
        }
        let i16_at = |at: usize| i16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let u32_at = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let channels = std::array::from_fn(|k| {
            let at = 8 * k;
            SlideChannel {
                angles: [i16_at(at), i16_at(at + 2), i16_at(at + 4)],
                delay_in: bytes[at + 6],
                delay_out: bytes[at + 7],
            }
        });
        Some(Self {
            channels,
            info: [u32_at(0x38) as i32, u32_at(0x3c) as i32],
            info_flags: u32_at(0x40),
            item_flags: u32_at(0x48),
        })
    }
}

/// What a frame asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenEffect {
    /// `FUN_004b93f0`: commit the record to the live block.
    CommitRecord,
    /// A menu sound-table slot.
    Sound(MenuEvent),
    /// A direct cue of the front-end bank (`FUN_004c9a20` and the root's
    /// table): slot, success byte.
    Cue { slot: u32, ok: bool },
    /// The Load screen loaded `slot` into the record (`0x4c7b40`): commit
    /// it, then apply [`crate::frontend_load::apply_to_live`] with the slot's
    /// record ([`FrontendScreens::load_screen`]).
    LoadGame { slot: u8 },
    /// The sound options' front-end volumes (`FUN_004c9860`/`FUN_004c97f0`,
    /// percent).
    Volumes { effects: i32, music: i32 },
    /// A keyboard device record changed: the host refreshes the driving
    /// bindings of `player` from [`FrontendScreens::controls`].
    Keyboard { player: u8 },
    /// The name entry took a code or cheat and is entered again.
    NameCue(crate::frontend_name_entry::NameCue),
    /// Modern display/graphics settings were adjusted in the Options menus.
    ModernChanged(crate::modern_settings::ModernSettings),
}

/// The control screens' cues (the device type change is the host's input
/// layer, not modelled here).
fn control_effects(changes: Vec<ControlsEffect>) -> impl Iterator<Item = ScreenEffect> {
    changes.into_iter().filter_map(|change| match change {
        ControlsEffect::SlideCue => Some(ScreenEffect::Cue {
            slot: crate::frontend_sounds::SLIDE_CUE,
            ok: true,
        }),
        ControlsEffect::DeviceType { .. } => None,
        ControlsEffect::Keyboard { player } => Some(ScreenEffect::Keyboard { player }),
    })
}

/// The host's reader of the twelve save-slot files (`FUN_0044ed90`).
pub type SlotReader = Box<dyn FnMut() -> Vec<SlotFile>>;

/// The tweens a screen's commit waits on.
#[derive(Clone, Debug, Default)]
struct Slides {
    tweens: Vec<ScalarTween>,
}

impl Slides {
    fn start(&mut self, tweens: Vec<ScalarTween>) {
        self.tweens = tweens;
    }

    /// Evaluate every active tween, reporting each active one's value;
    /// true when none was active.
    fn tick(&mut self, mut each: impl FnMut(usize, i32)) -> bool {
        let mut idle = true;
        for (index, tween) in self.tweens.iter_mut().enumerate() {
            if !tween.is_finished() {
                each(index, tween.evaluate_and_advance());
                idle = false;
            }
        }
        idle
    }

    fn idle(&self) -> bool {
        self.tweens.iter().all(ScalarTween::is_finished)
    }

    fn levels(&self) -> Vec<i32> {
        self.tweens.iter().map(ScalarTween::level).collect()
    }
}

const SETUPS: [SetupKind; 3] = [
    SetupKind::Championship,
    SetupKind::Rally,
    SetupKind::TimeTrial,
];
const NAMES: [NameScreen; 3] = [
    NameScreen::Championship,
    NameScreen::Rally,
    NameScreen::TimeTrial,
];

/// The menu tree of the setup flow.
pub struct FrontendScreens {
    set: MenuSet,
    setups: [SetupScreen; 3],
    stage: StageSelect,
    name: NameEntry,
    load: LoadScreen,
    slot_reader: Option<SlotReader>,
    options: OptionsScreens,
    controls: ControlScreens,
    /// The host's display hardware for the advanced graphics.
    display: DisplayCaps,
    /// The host's input devices for the control setup.
    devices: InputDevices,
    /// A capture began in this step (its key event is zeroed).
    capture_started: bool,
    layouts: [SetupLayout; 3],
    slides: Slides,
    /// `0x508fe8`: the name entry is leaving.
    name_leaving: bool,
    /// `DAT_00bbabec`: the "no animation" option (`!(record[0x10] & 0x20)`).
    no_animation: bool,
    /// How many times a setup screen was entered (the draw's caches reset).
    setup_entries: u32,
    /// The row arc, the sine table and the display extent, when known.
    geometry: Option<Geometry>,
    /// `DAT_00d8ffe0`: the seven row channels' Q16 positions.
    channels: [(i32, i32); 7],
    /// `DAT_00d90038/c`: the info anchor, and its slide `0xd90040`.
    info: (i32, i32),
    info_slide: Option<ScalarTween>,
}

struct Geometry {
    arc: crate::frontend_camera::Arc,
    sine: Box<[i32; crate::fixed_point::TRIG_TABLE_LEN]>,
    width: i32,
    height: i32,
}

/// Menu ids in the set, in build order.
mod id {
    use crate::descriptor_menu::MenuId;
    pub const ROOT: MenuId = MenuId(0);
    pub const SETUP: [MenuId; 3] = [MenuId(1), MenuId(2), MenuId(3)];
    pub const NAME: [MenuId; 3] = [MenuId(4), MenuId(5), MenuId(6)];
    pub const STAGE: MenuId = MenuId(7);
    pub const RACE: MenuId = MenuId(8);
    pub const NETWORK: MenuId = MenuId(9);
    pub const LOAD: MenuId = MenuId(10);
}

const SCREENS: [(MenuId, ScreenId); 11] = [
    (id::ROOT, ROOT_SCREEN_ID),
    (id::SETUP[0], CHAMPIONSHIP_SETUP_SCREEN_ID),
    (id::SETUP[1], RALLY_SCREEN_ID),
    (id::SETUP[2], TIME_TRIAL_SCREEN_ID),
    (id::NAME[0], CHAMPIONSHIP_NEXT_SCREEN_ID),
    (id::NAME[1], RALLY_NAME_SCREEN_ID),
    (id::NAME[2], TIME_TRIAL_NAME_SCREEN_ID),
    (id::STAGE, STAGE_SELECT_SCREEN_ID),
    (id::RACE, RACE_SENTINEL_SCREEN_ID),
    (id::NETWORK, NETWORK_SCREEN_ID),
    (id::LOAD, LOAD_SCREEN_ID),
];

/// The options subtree's menus follow the Load screen, in
/// [`OptionScreen::ALL`] order; the control setup's menu after them.
const OPTIONS_BASE: usize = 11;
const CONTROLS: MenuId = MenuId(OPTIONS_BASE + OptionScreen::ALL.len());

fn menu_of(screen: ScreenId) -> Option<MenuId> {
    SCREENS
        .iter()
        .find(|(_, s)| *s == screen)
        .map(|(m, _)| *m)
        .or_else(|| {
            let index = OptionScreen::ALL
                .iter()
                .position(|&o| o.screen() == screen)?;
            Some(MenuId(OPTIONS_BASE + index))
        })
        .or_else(|| {
            let index = ControlScreen::ALL
                .iter()
                .position(|&c| c.screen() == screen)?;
            Some(MenuId(CONTROLS.0 + index))
        })
}

fn screen_of(menu: MenuId) -> Option<ScreenId> {
    SCREENS
        .iter()
        .find(|(m, _)| *m == menu)
        .map(|(_, s)| *s)
        .or_else(|| {
            let index = menu.0.checked_sub(OPTIONS_BASE)?;
            OptionScreen::ALL.get(index).map(|o| o.screen())
        })
        .or_else(|| {
            let index = menu.0.checked_sub(CONTROLS.0)?;
            ControlScreen::ALL.get(index).map(|c| c.screen())
        })
}

enum Kind {
    Setup(usize),
    Name(usize),
    Stage,
    Load,
    Option(OptionScreen),
    Control(ControlScreen),
}

fn kind_of(menu: MenuId) -> Option<Kind> {
    if let Some(i) = id::SETUP.iter().position(|&m| m == menu) {
        Some(Kind::Setup(i))
    } else if let Some(i) = id::NAME.iter().position(|&m| m == menu) {
        Some(Kind::Name(i))
    } else if menu == id::STAGE {
        Some(Kind::Stage)
    } else if menu == id::LOAD {
        Some(Kind::Load)
    } else {
        let index = menu.0.checked_sub(OPTIONS_BASE)?;
        OptionScreen::ALL
            .get(index)
            .map(|&o| Kind::Option(o))
            .or_else(|| {
                let index = menu.0.checked_sub(CONTROLS.0)?;
                ControlScreen::ALL.get(index).map(|&c| Kind::Control(c))
            })
    }
}

impl FrontendScreens {
    /// The tree, with the setups' layout descriptors when the host has them.
    pub fn new(layouts: Option<[SetupLayout; 3]>) -> Self {
        let mut set = MenuSet::default();
        set.add(Menu::new(-1, None, None));
        // The setups' commit `FUN_004c9040` (`0x4c90b0`) and post `0x4c90c0`.
        let setups: [SetupScreen; 3] = std::array::from_fn(|i| {
            let mut setup = SetupScreen::build(SETUPS[i], id::ROOT, id::NAME[i]);
            setup.menu.on_commit = Some(0x004c_9040);
            setup.menu.on_post = Some(0x004c_90c0);
            setup
        });
        for setup in &setups {
            set.add(setup.menu.clone());
        }
        // The name entries' commit `0x4c5e30` and post `0x4c5ec0`.
        for (i, _) in NAMES.iter().enumerate() {
            let mut menu = Menu::new(0xed, Some(id::SETUP[i]), Some(0x00d9_0050));
            menu.nav = MenuNav::NONE;
            menu.on_commit = Some(0x004c_5e30);
            menu.on_post = Some(0x004c_5ec0);
            set.add(menu);
        }
        // The stage select's commit `0x4c8320` and post `0x4c84d0`.
        let mut stage = StageSelect::build(id::NAME[2], id::RACE, id::NETWORK);
        stage.menu.on_commit = Some(0x004c_8320);
        stage.menu.on_post = Some(0x004c_84d0);
        set.add(stage.menu.clone());
        set.add(Menu::new(-1, None, None));
        set.add(Menu::new(-1, None, None));
        let load = LoadScreen::build(id::ROOT);
        set.add(load.menu.clone());
        let options = OptionsScreens::build(&mut set, id::ROOT, CONTROLS, &[0; LEGACY_CONFIG_SIZE]);
        debug_assert_eq!(options.id(OptionScreen::Options), MenuId(OPTIONS_BASE));
        let controls = ControlScreens::build(&mut set, options.id(OptionScreen::Settings));
        debug_assert_eq!(controls.id(ControlScreen::Control), CONTROLS);
        Self {
            set,
            setups,
            stage,
            name: NameEntry::default(),
            load,
            slot_reader: None,
            options,
            controls,
            display: DisplayCaps::DEFAULT,
            devices: InputDevices::KEYBOARDS,
            capture_started: false,
            layouts: layouts.unwrap_or_default(),
            slides: Slides::default(),
            name_leaving: false,
            no_animation: false,
            setup_entries: 0,
            geometry: None,
            channels: [(0, 0); 7],
            info: (0, 0),
            info_slide: None,
        }
    }

    /// The host's save-slot reader the Load screen's enter calls.
    pub fn set_slot_reader(&mut self, reader: SlotReader) {
        self.slot_reader = Some(reader);
    }

    /// The host's display hardware the advanced graphics ask about.
    pub fn set_display_caps(&mut self, caps: DisplayCaps) {
        self.display = caps;
    }

    /// The options row screens' slide templates.
    pub fn set_option_templates(&mut self, templates: crate::frontend_options::RowTemplates) {
        self.options.set_templates(templates);
    }

    /// The host's input devices the control setup asks about.
    pub fn set_input_devices(&mut self, devices: InputDevices) {
        self.devices = devices;
    }

    /// Enable the modern graphics (`ENHANCED`), high-resolution `ADVANCED`,
    /// and `GAMEPAD 1` / `MODERN GAMEPAD` menus and defaults.
    pub fn enable_modern_mode(&mut self, settings: crate::modern_settings::ModernSettings) {
        self.options.enable_modern_mode(&mut self.set, settings);
        self.controls.enable_modern_mode(&mut self.set);
        self.devices = InputDevices::MODERN_GAMEPAD;
    }

    /// Current modern settings in the Options subtree.
    pub const fn modern_settings(&self) -> crate::modern_settings::ModernSettings {
        self.options.modern
    }

    /// `FUN_004ba060` at the config load: the record's bindings into the
    /// keyboard device records.
    pub fn install_keyboard_record(&mut self, record: &Record) {
        self.controls.install_record(record);
    }

    /// The name entry's encoded cheat names, from the user's executable.
    pub fn set_name_cheats(&mut self, cheats: Vec<Vec<u8>>) {
        self.name.set_cheats(cheats);
    }

    /// The control screens' state.
    pub const fn controls(&self) -> &ControlScreens {
        &self.controls
    }

    /// The options subtree's state.
    pub const fn options(&self) -> &OptionsScreens {
        &self.options
    }

    /// The Load screen's state, for the draw and the host's load.
    pub const fn load_screen(&self) -> &LoadScreen {
        &self.load
    }

    /// The setups' layout descriptors.
    pub fn set_layouts(&mut self, layouts: [SetupLayout; 3]) {
        self.layouts = layouts;
    }

    /// The row arc and the display extent the row positions use.
    pub fn set_geometry(&mut self, arc: crate::frontend_camera::Arc, extent: DisplayExtent) {
        self.geometry = Some(Geometry {
            arc,
            sine: Box::new(crate::frontend_camera::sine_table()),
            width: i32::from(extent.width),
            height: i32::from(extent.height),
        });
    }

    /// The row channels' positions (`DAT_00d8ffe0`).
    pub const fn channels(&self) -> &[(i32, i32); 7] {
        &self.channels
    }

    /// The info anchor (`DAT_00d90038/c`).
    pub const fn info_anchor(&self) -> (i32, i32) {
        self.info
    }

    /// The layout descriptor of a setup screen.
    pub fn layout(&self, kind: SetupKind) -> &SetupLayout {
        &self.layouts[SETUPS.iter().position(|&k| k == kind).unwrap_or(0)]
    }

    /// Whether `screen` is one of these.
    pub fn handles(screen: ScreenId) -> bool {
        menu_of(screen).and_then(kind_of).is_some()
    }

    /// The setup screen's state, for the draw.
    pub fn setup(&self, kind: SetupKind) -> &SetupScreen {
        &self.setups[SETUPS.iter().position(|&k| k == kind).unwrap_or(0)]
    }

    /// The name entry's state, for the draw.
    pub const fn name_entry(&self) -> &NameEntry {
        &self.name
    }

    /// The stage select's state, for the draw.
    pub const fn stage_select(&self) -> &StageSelect {
        &self.stage
    }

    /// A 15-step ease-out, or its "no animation" form.
    fn slide(&self, from: i32, to: i32, delay: u8) -> ScalarTween {
        if self.no_animation {
            ScalarTween::ease_out(to, to, 15, 0)
        } else {
            ScalarTween::ease_out(from, to, 15, i32::from(delay))
        }
    }

    /// The name entry's and stage select's two slide-outs: 15-step ease-ins
    /// from 0 to `0x10000`.
    fn slide_outs(&self) -> Vec<ScalarTween> {
        let tween = if self.no_animation {
            ScalarTween::ease_out(0x10000, 0x10000, 15, 0)
        } else {
            ScalarTween::ease_in(0, 0x10000, 15, 0)
        };
        vec![tween, tween]
    }

    /// `FUN_004aed50(menu, went_back)`: activate `screen` and run its enter
    /// callback, then the stepper's step with no input in the same frame.
    pub fn activate(&mut self, screen: ScreenId, record: &mut Record) -> Vec<ScreenEffect> {
        let Some(menu) = menu_of(screen) else {
            return Vec::new();
        };
        self.no_animation = record[0x10] & 0x20 == 0;
        let mut cues = Vec::new();
        let went_back = self.set.went_back;
        let _ = self.set.activate(menu, None, u8::from(went_back));
        match kind_of(menu) {
            Some(Kind::Setup(i)) => {
                let layout = self.layouts[i];
                self.setups[i].menu = self.set.menu(menu).clone();
                self.setups[i].enter(record);
                self.setups[i].apply_players();
                *self.set.menu_mut(menu) = self.setups[i].menu.clone();
                let tweens = layout
                    .channels
                    .iter()
                    .map(|c| self.slide(i32::from(c.angles[0]), i32::from(c.angles[1]), c.delay_in))
                    .collect();
                self.slides.start(tweens);
                self.info_slide = Some(self.slide(0x10000, 0, 0));
                self.setup_entries = self.setup_entries.wrapping_add(1);
            }
            Some(Kind::Name(i)) => {
                cues.push(ScreenEffect::Cue {
                    slot: crate::frontend_sounds::SLIDE_CUE,
                    ok: true,
                });
                self.name.enter(NAMES[i], record);
                self.name_leaving = false;
                let tweens = vec![self.slide(0x10000, 0, 0), self.slide(0x10000, 0, 0)];
                self.slides.start(tweens);
            }
            Some(Kind::Option(screen)) => {
                let arg = u8::from(went_back);
                let display = self.display;
                let no_animation = self.no_animation;
                let slides =
                    self.options
                        .enter_presentation(screen, self.set.menu_mut(menu), no_animation);
                cues.extend(
                    slides
                        .into_iter()
                        .map(|slot| ScreenEffect::Cue { slot, ok: true }),
                );
                self.options
                    .enter(screen, arg, self.set.menu_mut(menu), record, &display);
            }
            Some(Kind::Control(screen)) => {
                let mut changes = Vec::new();
                let (player, devices, no_animation) =
                    (self.options.player, self.devices, self.no_animation);
                self.controls.enter(
                    screen,
                    u8::from(went_back),
                    self.set.menu_mut(menu),
                    record,
                    player,
                    &devices,
                    no_animation,
                    &mut changes,
                );
                cues.extend(control_effects(changes));
            }
            Some(Kind::Load) => {
                let files = self
                    .slot_reader
                    .as_mut()
                    .map_or_else(Vec::new, |read| read());
                self.load.menu = self.set.menu(menu).clone();
                self.load.enter(&files, self.no_animation);
                *self.set.menu_mut(menu) = self.load.menu.clone();
            }
            Some(Kind::Stage) => {
                cues.push(ScreenEffect::Cue {
                    slot: crate::frontend_sounds::SLIDE_CUE,
                    ok: true,
                });
                self.stage.menu = self.set.menu(menu).clone();
                self.stage.enter(record);
                *self.set.menu_mut(menu) = self.stage.menu.clone();
                let tweens = vec![self.slide(0x10000, 0, 0), self.slide(0x10000, 0, 0)];
                self.slides.start(tweens);
            }
            None => {}
        }
        let (_, effects) = self.step(screen, 0, NameKey::default(), record);
        cues.extend(effects);
        cues
    }

    /// One stepper step of `screen` with the menu `bits` (the name entries
    /// read `key` instead). Returns the screen the step left for, if any.
    pub fn step(
        &mut self,
        screen: ScreenId,
        bits: u32,
        key: NameKey,
        record: &mut Record,
    ) -> (Option<ScreenId>, Vec<ScreenEffect>) {
        let Some(menu) = menu_of(screen) else {
            return (None, Vec::new());
        };
        let Some(kind) = kind_of(menu) else {
            return (None, Vec::new());
        };
        let bits = if matches!(kind, Kind::Name(_)) {
            0
        } else {
            bits
        };
        let mut effects = Vec::new();
        let (pending, events) = self.set.step(menu, bits);
        let mut pending = pending;
        for event in events {
            match event {
                // The Load screen's enter clears the table's move entry
                // while no slot is occupied.
                MenuEvent::Sound {
                    sound: crate::descriptor_menu::MenuSound::Move,
                    ..
                } if matches!(kind, Kind::Load) && !self.load.move_sound => {}
                // The sound options' enter clears the value entry `0xd90064`.
                MenuEvent::Sound {
                    sound: crate::descriptor_menu::MenuSound::ValueChange,
                    ..
                } if matches!(kind, Kind::Option(OptionScreen::Sound))
                    && self.options.value_cue_muted => {}
                MenuEvent::Sound { .. } => effects.push(ScreenEffect::Sound(event)),
                // `0x4c7b40`: an occupied slot loads and starts the race.
                MenuEvent::Activate { item, .. } if matches!(kind, Kind::Option(_)) => {
                    let Kind::Option(screen) = kind else {
                        unreachable!()
                    };
                    let item = self.set.menu(menu).items[item].clone();
                    if let Some(target) = self.options.activate(screen, &item, record) {
                        self.set.pending = Some(target);
                        pending = Some(target);
                    }
                }
                // `0x4cda80`: the control setup's OK picks the next screen.
                MenuEvent::Activate {
                    item,
                    callback: 0x004c_da80,
                    ..
                } => {
                    let mut changes = Vec::new();
                    let player = self.options.player;
                    let (target, parent) =
                        self.controls
                            .control_ok(self.set.menu(menu), record, player, &mut changes);
                    effects.extend(control_effects(changes));
                    self.set.menu_mut(menu).items[item].kind =
                        crate::descriptor_menu::ItemKind::Submenu(target);
                    self.set
                        .menu_mut(self.controls.id(ControlScreen::Buttons))
                        .parent = Some(parent);
                    self.set.pending = Some(target);
                    pending = Some(target);
                }
                // `0x4cdbe0`: capture the selected slot's binding.
                MenuEvent::Activate {
                    item,
                    callback: 0x004c_dbe0,
                    ..
                } => {
                    let slot = self.set.menu(menu).items[item].key;
                    self.controls.start_capture(self.set.menu_mut(menu), slot);
                    self.capture_started = true;
                }
                // `0x4cdc30`: the button setup's OK writes the bindings and
                // returns to its parent's parent.
                MenuEvent::Activate {
                    callback: 0x004c_dc30,
                    ..
                } => {
                    let mut changes = Vec::new();
                    let player = self.options.player;
                    self.controls.buttons_ok(record, player, &mut changes);
                    effects.extend(control_effects(changes));
                    let parent = self.set.menu(menu).parent;
                    if let Some(target) = parent.and_then(|p| self.set.menu(p).parent) {
                        self.set.pending = Some(target);
                        pending = Some(target);
                    }
                }
                MenuEvent::Activate { item, .. } if matches!(kind, Kind::Load) => {
                    if self.load.activate(item, record) {
                        effects.push(ScreenEffect::LoadGame { slot: item as u8 });
                        self.set.pending = Some(id::RACE);
                        pending = Some(id::RACE);
                    }
                }
                MenuEvent::Commit { .. } => {
                    if let Some(target) = self.commit(&kind, menu, key, record, &mut effects) {
                        pending = Some(target);
                    }
                }
                MenuEvent::Post { back, .. } => self.post(&kind, menu, back, record, &mut effects),
                _ => {}
            }
        }
        // A commit that set the pending menu completes the step's tail.
        if pending.is_some() && self.set.pending.is_some() {
            let (resolved, events) = self.set.resolve_pending(menu);
            pending = resolved.or(pending);
            for event in events {
                if let MenuEvent::Post { back, .. } = event {
                    self.post(&kind, menu, back, record, &mut effects);
                }
            }
        }
        (pending.and_then(screen_of), effects)
    }

    /// The commit callbacks.
    fn commit(
        &mut self,
        kind: &Kind,
        menu: MenuId,
        key: NameKey,
        record: &mut Record,
        effects: &mut Vec<ScreenEffect>,
    ) -> Option<MenuId> {
        let mut values = [None; 7];
        let idle = self.slides.tick(|index, value| {
            if let Some(slot) = values.get_mut(index) {
                *slot = Some(value);
            }
        });
        match kind {
            // `FUN_004c9040`: the rows move on the arc (`FUN_004c43b0`), the
            // info anchor slides in, and the navigation returns once no row
            // channel is active.
            Kind::Setup(i) => {
                if let Some(geometry) = &self.geometry {
                    for (channel, value) in values.iter().enumerate() {
                        if let Some(angle) = value {
                            self.channels[channel] = crate::frontend_camera::arc_position(
                                geometry.arc,
                                *angle,
                                &geometry.sine,
                                geometry.width,
                                geometry.height,
                            );
                        }
                    }
                }
                let layout = self.layouts[*i];
                if let Some(slide) = self.info_slide.as_mut()
                    && !slide.is_finished()
                {
                    let value = slide.evaluate_and_advance();
                    self.info = (value.wrapping_add(layout.info[0]), layout.info[1]);
                }
                if idle {
                    self.set.menu_mut(menu).nav = MenuNav::ALL;
                }
                None
            }
            // `0x4c5e30`.
            Kind::Name(_) => {
                if !idle || self.name_leaving {
                    return None;
                }
                match self.name.key(key, record) {
                    NameOutcome::Editing => None,
                    NameOutcome::Again { cue } => {
                        effects.push(ScreenEffect::NameCue(cue));
                        self.set.pending = Some(menu);
                        Some(menu)
                    }
                    NameOutcome::Leave { target, commit } => {
                        if commit {
                            effects.push(ScreenEffect::CommitRecord);
                        }
                        let target = menu_of(target)?;
                        self.set.pending = Some(target);
                        Some(target)
                    }
                }
            }
            Kind::Option(screen) => {
                let geometry = self.geometry.as_ref();
                let place = |angle: i32| {
                    geometry.map_or((0, 0), |g| {
                        crate::frontend_camera::arc_position(
                            g.arc, angle, &g.sine, g.width, g.height,
                        )
                    })
                };
                self.options
                    .commit_presentation(*screen, self.set.menu_mut(menu), &place);
                let display = self.display;
                let mut changes = Vec::new();
                self.options.commit(
                    *screen,
                    self.set.menu_mut(menu),
                    record,
                    &display,
                    &mut changes,
                );
                // `FUN_004ad710` from the graphics commit.
                self.no_animation = record[0x10] & 0x20 == 0;
                for change in changes {
                    effects.push(match change {
                        OptionsEffect::Volumes { effects, music } => {
                            ScreenEffect::Volumes { effects, music }
                        }
                        OptionsEffect::PreviewCue => ScreenEffect::Cue { slot: 2, ok: true },
                        OptionsEffect::ModernChanged(settings) => {
                            ScreenEffect::ModernChanged(settings)
                        }
                    });
                }
                None
            }
            // `0x4cee30`.
            Kind::Control(_) => {
                let devices = self.devices;
                let player = self.options.player;
                // `0x4cdbe0` zeroed the key event for its own step.
                let key = if std::mem::take(&mut self.capture_started) {
                    0
                } else {
                    u32::from(key.vk)
                };
                self.controls.commit(
                    self.set.menu_mut(menu),
                    record,
                    player,
                    &devices,
                    key,
                    false,
                );
                None
            }
            // `0x4c76c0`.
            Kind::Load => {
                self.load.menu = self.set.menu(menu).clone();
                self.load.commit();
                *self.set.menu_mut(menu) = self.load.menu.clone();
                None
            }
            // `0x4c8320`.
            Kind::Stage => {
                self.stage.menu = self.set.menu(menu).clone();
                self.stage.commit(record, idle);
                *self.set.menu_mut(menu) = self.stage.menu.clone();
                None
            }
        }
    }

    /// The post callbacks: the slide-outs and the record writes.
    fn post(
        &mut self,
        kind: &Kind,
        menu: MenuId,
        back: bool,
        record: &mut Record,
        effects: &mut Vec<ScreenEffect>,
    ) {
        match kind {
            // `0x4c90c0`.
            Kind::Setup(i) => {
                let i = *i;
                self.setups[i].menu = self.set.menu(menu).clone();
                self.setups[i].post(record);
                self.set.menu_mut(menu).nav = MenuNav::NONE;
                let layout = self.layouts[i];
                let tweens = layout
                    .channels
                    .iter()
                    .map(|c| {
                        self.slide(i32::from(c.angles[1]), i32::from(c.angles[2]), c.delay_out)
                    })
                    .collect();
                self.slides.start(tweens);
                self.info_slide = Some(if self.no_animation {
                    ScalarTween::ease_out(0x10000, 0x10000, 15, 0)
                } else {
                    ScalarTween::ease_in(0, 0x10000, 15, 0)
                });
                self.name.next_player = 0;
            }
            // `0x4c5ec0`.
            Kind::Name(_) => {
                effects.push(ScreenEffect::Cue {
                    slot: crate::frontend_sounds::SLIDE_CUE,
                    ok: true,
                });
                self.slides.start(self.slide_outs());
                self.name_leaving = true;
            }
            Kind::Option(screen) => {
                let no_animation = self.no_animation;
                let slides =
                    self.options
                        .post_presentation(*screen, self.set.menu_mut(menu), no_animation);
                effects.extend(
                    slides
                        .into_iter()
                        .map(|slot| ScreenEffect::Cue { slot, ok: true }),
                );
            }
            Kind::Control(screen) => {
                let mut changes = Vec::new();
                let (player, no_animation) = (self.options.player, self.no_animation);
                self.controls.post(
                    *screen,
                    back,
                    self.set.menu_mut(menu),
                    record,
                    player,
                    no_animation,
                    &mut changes,
                );
                effects.extend(control_effects(changes));
            }
            // `0x4c7770`.
            Kind::Load => {
                self.load.menu = self.set.menu(menu).clone();
                self.load.post(self.no_animation);
                *self.set.menu_mut(menu) = self.load.menu.clone();
            }
            // `0x4c84d0`.
            Kind::Stage => {
                effects.push(ScreenEffect::Cue {
                    slot: crate::frontend_sounds::SLIDE_CUE,
                    ok: true,
                });
                self.stage.menu = self.set.menu(menu).clone();
                match self.stage.post(record, back) {
                    PostOutcome::Back { next_player } => self.name.next_player = next_player,
                    PostOutcome::Commit => effects.push(ScreenEffect::CommitRecord),
                }
                self.slides.start(self.slide_outs());
            }
        }
    }

    /// The frame's draw callbacks' side effects on `screen`.
    pub fn draw_effects(&mut self, screen: ScreenId, record: &Record) {
        let Some(menu) = menu_of(screen) else {
            return;
        };
        match kind_of(menu) {
            Some(Kind::Setup(i)) => {
                let setup = &mut self.setups[i];
                setup.menu = self.set.menu(menu).clone();
                setup.apply_players();
                let gate = setup.continue_enabled(record);
                if let Some(index) = setup.menu.find(key::CONTINUE) {
                    setup.menu.items[index].enabled = gate;
                }
                *self.set.menu_mut(menu) = setup.menu.clone();
            }
            Some(Kind::Stage) => {
                self.stage.menu = self.set.menu(menu).clone();
                self.stage.predraw(record);
                *self.set.menu_mut(menu) = self.stage.menu.clone();
            }
            Some(Kind::Load) => {
                self.load.menu = self.set.menu(menu).clone();
                self.load.draw_effects();
                *self.set.menu_mut(menu) = self.load.menu.clone();
            }
            _ => {}
        }
    }

    /// The current menu of `screen`, for the draw.
    pub fn menu(&self, screen: ScreenId) -> Option<&Menu> {
        menu_of(screen).map(|menu| self.set.menu(menu))
    }

    /// How many times a setup screen was entered: the draw resets its
    /// preview caches on each entry (`FUN_004c8c20`).
    pub fn setup_entries(&self) -> u32 {
        self.setup_entries
    }

    /// Whether the slides have ended.
    pub fn slides_idle(&self) -> bool {
        self.slides.idle()
    }

    /// The current slides' last values (the name entry's `NX` and anchor
    /// slides, the stage select's `SX` and anchor slides).
    pub fn slide_values(&self) -> Vec<i32> {
        self.slides.levels()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor_menu::bits;

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

    /// Step until the slides end (no input).
    fn settle(screens: &mut FrontendScreens, screen: ScreenId, record: &mut Record) {
        for _ in 0..40 {
            screens.step(screen, 0, NameKey::default(), record);
            screens.draw_effects(screen, record);
        }
    }

    #[test]
    fn a_championship_runs_setup_name_and_race() {
        let mut record = record();
        let mut screens = FrontendScreens::new(None);
        screens.activate(CHAMPIONSHIP_SETUP_SCREEN_ID, &mut record);
        // Input is ignored during the slide-in.
        let (left, _) = screens.step(
            CHAMPIONSHIP_SETUP_SCREEN_ID,
            bits::CONFIRM,
            NameKey::default(),
            &mut record,
        );
        assert_eq!(left, None);
        settle(&mut screens, CHAMPIONSHIP_SETUP_SCREEN_ID, &mut record);
        // Continue (selected first) leads to the name entry.
        let (left, _) = screens.step(
            CHAMPIONSHIP_SETUP_SCREEN_ID,
            bits::CONFIRM,
            NameKey::default(),
            &mut record,
        );
        assert_eq!(left, Some(CHAMPIONSHIP_NEXT_SCREEN_ID));
        assert_eq!(record[0x180] & 3, 0);
        screens.activate(CHAMPIONSHIP_NEXT_SCREEN_ID, &mut record);
        settle(&mut screens, CHAMPIONSHIP_NEXT_SCREEN_ID, &mut record);
        let mut effects = Vec::new();
        let mut left = None;
        for vk in [b'A', 0, b'B', 0, 0x0d] {
            let (target, more) = screens.step(
                CHAMPIONSHIP_NEXT_SCREEN_ID,
                0,
                NameKey { vk, pad: 0 },
                &mut record,
            );
            effects.extend(more);
            left = left.or(target);
        }
        assert_eq!(left, Some(RACE_SENTINEL_SCREEN_ID));
        assert!(effects.contains(&ScreenEffect::CommitRecord));
        assert_eq!(&record[0x34..0x37], b"AB\0");
    }

    #[test]
    fn a_cac_code_enters_the_name_screen_again() {
        let mut record = record();
        let mut screens = FrontendScreens::new(None);
        screens.activate(CHAMPIONSHIP_NEXT_SCREEN_ID, &mut record);
        settle(&mut screens, CHAMPIONSHIP_NEXT_SCREEN_ID, &mut record);
        let mut effects = Vec::new();
        let mut left = None;
        for &vk in b"CAC12340\r" {
            for vk in [vk, 0] {
                let (target, more) = screens.step(
                    CHAMPIONSHIP_NEXT_SCREEN_ID,
                    0,
                    NameKey { vk, pad: 0 },
                    &mut record,
                );
                effects.extend(more);
                left = left.or(target);
            }
        }
        assert_eq!(left, Some(CHAMPIONSHIP_NEXT_SCREEN_ID));
        assert!(effects.contains(&ScreenEffect::NameCue(
            crate::frontend_name_entry::NameCue::Code
        )));
        assert!(!effects.contains(&ScreenEffect::CommitRecord));
        assert_eq!(u16::from_le_bytes([record[0x28], record[0x29]]), 1234);
        screens.activate(CHAMPIONSHIP_NEXT_SCREEN_ID, &mut record);
        assert_eq!(screens.name_entry().name(), b"Player 1");
    }

    #[test]
    fn back_from_a_setup_returns_to_the_root() {
        let mut record = record();
        let mut screens = FrontendScreens::new(None);
        screens.activate(RALLY_SCREEN_ID, &mut record);
        settle(&mut screens, RALLY_SCREEN_ID, &mut record);
        let (left, _) = screens.step(RALLY_SCREEN_ID, bits::BACK, NameKey::default(), &mut record);
        assert_eq!(left, Some(ROOT_SCREEN_ID));
        assert_eq!(record[0x180] & 3, 1);
    }

    #[test]
    fn a_locked_country_disables_continue() {
        let mut record = record();
        record[0x181] = 3; // locked by default for a rally
        let mut screens = FrontendScreens::new(None);
        screens.activate(RALLY_SCREEN_ID, &mut record);
        settle(&mut screens, RALLY_SCREEN_ID, &mut record);
        let menu = screens.menu(RALLY_SCREEN_ID).unwrap();
        let continue_item = menu.find(key::CONTINUE).unwrap();
        assert!(!menu.items[continue_item].enabled);
    }

    #[test]
    fn a_time_trial_ends_on_the_stage_select() {
        let mut record = record();
        let mut screens = FrontendScreens::new(None);
        screens.activate(TIME_TRIAL_SCREEN_ID, &mut record);
        settle(&mut screens, TIME_TRIAL_SCREEN_ID, &mut record);
        let (left, _) = screens.step(
            TIME_TRIAL_SCREEN_ID,
            bits::CONFIRM,
            NameKey::default(),
            &mut record,
        );
        assert_eq!(left, Some(TIME_TRIAL_NAME_SCREEN_ID));
        screens.activate(TIME_TRIAL_NAME_SCREEN_ID, &mut record);
        settle(&mut screens, TIME_TRIAL_NAME_SCREEN_ID, &mut record);
        let (left, effects) = screens.step(
            TIME_TRIAL_NAME_SCREEN_ID,
            0,
            NameKey { vk: 0x0d, pad: 0 },
            &mut record,
        );
        assert_eq!(left, Some(STAGE_SELECT_SCREEN_ID));
        assert!(effects.contains(&ScreenEffect::CommitRecord));
        screens.activate(STAGE_SELECT_SCREEN_ID, &mut record);
        settle(&mut screens, STAGE_SELECT_SCREEN_ID, &mut record);
        // Turn the first stage on, then up (wrapping) to OK and confirm.
        let mut left = None;
        let mut effects = Vec::new();
        screens.step(
            STAGE_SELECT_SCREEN_ID,
            bits::CONFIRM,
            NameKey::default(),
            &mut record,
        );
        screens.draw_effects(STAGE_SELECT_SCREEN_ID, &record);
        for _ in 0..8 {
            let (target, more) = screens.step(
                STAGE_SELECT_SCREEN_ID,
                bits::UP,
                NameKey::default(),
                &mut record,
            );
            screens.draw_effects(STAGE_SELECT_SCREEN_ID, &record);
            left = left.or(target);
            effects.extend(more);
            let menu = screens.menu(STAGE_SELECT_SCREEN_ID).unwrap();
            if menu.selected == crate::frontend_stage_select::OK_ITEM {
                break;
            }
        }
        let (target, more) = screens.step(
            STAGE_SELECT_SCREEN_ID,
            bits::CONFIRM,
            NameKey::default(),
            &mut record,
        );
        left = left.or(target);
        effects.extend(more);
        assert_eq!(left, Some(RACE_SENTINEL_SCREEN_ID));
        assert!(effects.contains(&ScreenEffect::CommitRecord));
        let country = usize::from(record[0x181] & 0xf);
        assert_ne!(record[0x16c + 2 * country], 0);
    }

    #[test]
    fn the_layout_descriptor_parses() {
        let mut bytes = vec![0u8; 0x50];
        bytes[6] = 3;
        bytes[0x38..0x3c].copy_from_slice(&0x8000i32.to_le_bytes());
        let layout = SetupLayout::parse(&bytes).unwrap();
        assert_eq!(layout.channels[0].delay_in, 3);
        assert_eq!(layout.info[0], 0x8000);
        assert!(SetupLayout::parse(&bytes[..0x4f]).is_none());
    }
}
