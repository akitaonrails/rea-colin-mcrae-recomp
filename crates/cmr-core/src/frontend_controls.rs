//! The options subtree's control screens: Control setup `0xd45858` (the
//! method and controller type of player `DAT_00d90504`), Analogue setup
//! `0xd41278` (which controls are analogue, force feedback) and Button setup
//! `0xd445d0` (the eight bindings). They edit the player block
//! `P(p) = img + 0x44 + 0x9c p` of the cmRally.cfg record: `+0` the analogue
//! and force-feedback bits, word `+2` the device, dword `+4` its type.
//!
//! All three slide their panel `0xd904f4` in from `-0x3333 - A` to `0x1999`
//! (`FUN_004ccd60`, mode 2, 15 steps, the slide cue) and out (mode 3), with
//! `A` the layout global `0x50aa8c` (0x8000 until the first enter writes
//! 0xb333). After the slide the commit (`FUN_004ccb50`) waits
//! `((DAT_00551af0 != 0) + 1) * 25 / 4` further steps, then enables moving,
//! adjusting and confirming but not going back (`aec50(m, 1, 1, 1, 0)`).
//! A description of disassembly, not original source. Evidence:
//! `rea.official-eu.frontend-controls`.

use crate::config::LEGACY_CONFIG_SIZE;
use crate::descriptor_menu::{ItemKind, Menu, MenuId, MenuItem, MenuNav, MenuSet};
use crate::frontend::ScreenId;
use crate::frontend_menu::ValueWidget;
use crate::scalar_tween::ScalarTween;

type Record = [u8; LEGACY_CONFIG_SIZE];

/// A control screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlScreen {
    Control,
    Analogue,
    Buttons,
}

impl ControlScreen {
    pub const ALL: [Self; 3] = [Self::Control, Self::Analogue, Self::Buttons];

    pub const fn screen(self) -> ScreenId {
        ScreenId::from_legacy(match self {
            Self::Control => 0x00d4_5858,
            Self::Analogue => 0x00d4_1278,
            Self::Buttons => 0x00d4_45d0,
        })
    }

    /// The camera target (argument 4 of the create call).
    pub const fn camera_target(self) -> u32 {
        match self {
            Self::Control => 0x0050_7ad8,
            Self::Analogue | Self::Buttons => 0x0050_7ae8,
        }
    }

    pub fn of(screen: ScreenId) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.screen() == screen)
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// Item keys.
pub mod key {
    pub const METHOD: i32 = 0x12;
    pub const TYPE: i32 = 0x13;
    pub const FORCE_FEEDBACK: i32 = 3;
    pub const STEERING: i32 = 0x1b;
    pub const BRAKE: i32 = 0x1c;
    pub const ACCELERATION: i32 = 0x1d;
    pub const OK: i32 = 9;
}

/// The control sub-state `DAT_00d90518`.
pub mod state {
    pub const CONTROL: u8 = 1;
    pub const ANALOGUE: u8 = 2;
    pub const BUTTONS: u8 = 3;
    pub const CAPTURE: u8 = 4;
}

/// The host's input devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputDevices {
    /// `FUN_0049d620`: bit `i` for each present device record (0 and 1 are
    /// the keyboards).
    pub present: u32,
    /// `FUN_0049d5c0`: one bit per keyboard, then one per joystick id that
    /// answers `joyGetPosEx`.
    pub polled: u32,
    /// `FUN_0049caf0`: a force-feedback device.
    pub force_feedback: bool,
    /// `FUN_0049d6a0(dev)`: each device record's button count (`+0xc`).
    pub buttons: [i32; 18],
    /// The rising-edge mask (`+8`) of each device record this frame.
    pub edges: [u32; 18],
}

impl InputDevices {
    /// Two keyboards and no joystick.
    pub const KEYBOARDS: Self = Self {
        present: 0b11,
        polled: 0b11,
        force_feedback: false,
        buttons: [0; 18],
        edges: [0; 18],
    };

    /// Two keyboards plus a modern 12-button dual-stick gamepad with rumble
    /// (`device = 2`, `method = 1`).
    pub const MODERN_GAMEPAD: Self = Self {
        present: 0b0111,
        polled: 0b1111,
        force_feedback: true,
        buttons: [0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        edges: [0; 18],
    };
}

/// A host request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlsEffect {
    /// `FUN_004c9a20`: the slide cue.
    SlideCue,
    /// `FUN_0049d670(dev, type)`: the device's type changed.
    DeviceType { device: u16, kind: i32 },
    /// The keyboard record of `player` changed (`FUN_004b9f70`,
    /// `FUN_0049d3f0`): the host refreshes its driving bindings from
    /// [`ControlScreens::keyboards`].
    Keyboard { player: u8 },
}

/// The default masks for binding slots 0..8 (the button setup's and the
/// reset `FUN_004b98b0`'s): left, right, up, down,
/// then joystick buttons 4, 5, 3 and 2 (keyboard: handbrake, gear up, gear
/// down, change view).
pub const DEFAULT_MASKS: [u16; 8] = [1, 2, 4, 8, 0x100, 0x200, 0x80, 0x40];

/// The number of significant bits of `mask`.
fn bit_length(mask: u32) -> i32 {
    32 - mask.leading_zeros() as i32
}

fn player_block(player: u8) -> usize {
    0x44 + 0x9c * usize::from(player)
}

fn value(menu: &Menu, key: i32) -> i32 {
    crate::frontend_options::value(menu, key)
}

fn item_mut(menu: &mut Menu, key: i32) -> Option<&mut MenuItem> {
    let index = menu.find(key)?;
    menu.items.get_mut(index)
}

/// Move the selection past disabled items (`+0x2c`), unbounded as the
/// native loop is (it stops at the menu's end here).
fn skip_disabled(menu: &mut Menu) {
    while menu
        .items
        .get(menu.selected)
        .is_some_and(|item| !item.enabled)
    {
        menu.selected += 1;
    }
}

fn value_item(text: i32, key: i32, count: i32, value: i32, wrap: bool) -> MenuItem {
    let mut item = MenuItem::new(ItemKind::Value, text, 0);
    item.key = key;
    item.value = ValueWidget {
        value,
        value_count: count,
        wrap,
    };
    item
}

/// The control screens' state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlScreens {
    ids: [MenuId; 3],
    /// The panel slide `0xd904f4` and its x `DAT_00d904fc`.
    tween: Option<ScalarTween>,
    pub panel_x: i32,
    /// `0x50aa8c`.
    amplitude: i32,
    /// `DAT_00d90548`: the steps since the slide ended.
    settle: i32,
    /// `DAT_00d90518`.
    pub state: u8,
    /// `DAT_00d9050c`: the type chosen is analogue.
    pub analogue: bool,
    /// `DAT_00d9050e`: capturing a binding.
    pub capturing: bool,
    /// `DAT_00d9050f`.
    keep_analogue: bool,
    /// `DAT_00d90521..d90524`: steering, accel, brake, force feedback.
    pub analogue_latch: [bool; 4],
    /// `0x50aaa0`: the device type chosen.
    pub device_type: i32,
    /// The layout `0x50aa94`/`0x50aa98`/`0x50aa9c`: label x, first row y,
    /// row gap.
    pub layout: (i32, i32, i32),
    /// The working copy `0x50aaa4` (masks) and `0x50aab4` (keys).
    pub masks: [u16; 8],
    pub keys: [u32; 8],
    /// `DAT_00d90514`: the device's button count.
    buttons: i32,
    /// `DAT_00d90508`: the slot being captured; `DAT_00d9051c` the state to
    /// return to; `DAT_00d9054c` the capture's ready latch.
    slot: usize,
    saved_state: u8,
    ready: bool,
    /// The keyboard device records 0 and 1 (`+0x78..`).
    pub keyboards: [crate::input_map::KeyBindings; 2],
    /// Whether modern gamepad labels and defaults are active.
    pub modern_mode: bool,
}

impl ControlScreens {
    /// Add the three menus to `set` below `settings`.
    pub fn build(set: &mut MenuSet, settings: MenuId) -> Self {
        let base = set.menus.len();
        let id = |screen: ControlScreen| MenuId(base + screen.index());
        let menu = |title: i32, parent: MenuId| {
            Menu::new(
                title,
                Some(parent),
                Some(crate::frontend_sounds::SETUP_SOUNDS),
            )
        };
        let mut control = menu(0xc1, settings);
        let mut pad = MenuItem::new(ItemKind::Action, 0xc5, 0);
        pad.activate = Some(0x004c_dbc0);
        let mut ok = MenuItem::new(ItemKind::Submenu(settings), 0xdf, 0);
        ok.activate = Some(0x004c_da80);
        control.items = vec![
            value_item(0xc2, key::METHOD, 0x12, 0, true),
            value_item(0xc3, key::TYPE, 3, 0, true),
            pad,
            ok,
        ];
        callbacks(
            &mut control,
            [
                0x004c_cbf0,
                0x004c_ee30,
                0x004c_dd80,
                0x004c_c6f0,
                0x004c_cb20,
            ],
        );
        let mut analogue = menu(0x38, settings);
        let toggle = |text, key, on: bool| value_item(text, key, 2, i32::from(on), true);
        analogue.items = vec![
            toggle(0x15, key::STEERING, true),
            toggle(0x16, key::ACCELERATION, false),
            toggle(0x17, key::BRAKE, false),
            toggle(0xc4, key::FORCE_FEEDBACK, false),
            MenuItem::new(ItemKind::Submenu(id(ControlScreen::Buttons)), 0xdf, 0),
        ];
        callbacks(
            &mut analogue,
            [
                0x004c_cdc0,
                0x004c_ee30,
                0x004c_e420,
                0x004c_c820,
                0x004c_cb20,
            ],
        );
        let mut buttons = menu(0x39, id(ControlScreen::Analogue));
        for (slot, text) in [0xb7, 0xb8, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0xcc]
            .into_iter()
            .enumerate()
        {
            let mut item = MenuItem::new(ItemKind::Action, text, 0);
            item.key = slot as i32;
            item.activate = Some(0x004c_dbe0);
            buttons.items.push(item);
        }
        let mut ok = MenuItem::new(ItemKind::Submenu(settings), 0xdf, 0);
        ok.key = key::OK;
        ok.activate = Some(0x004c_dc30);
        buttons.items.push(ok);
        callbacks(
            &mut buttons,
            [
                0x004c_cf50,
                0x004c_ee30,
                0x004c_e650,
                0x004c_c9b0,
                0x004c_cb20,
            ],
        );
        set.add(control);
        set.add(analogue);
        set.add(buttons);
        Self {
            ids: std::array::from_fn(|i| MenuId(base + i)),
            tween: None,
            panel_x: 0,
            amplitude: 0x8000,
            settle: 0,
            state: 0,
            analogue: false,
            capturing: false,
            keep_analogue: false,
            analogue_latch: [false; 4],
            device_type: 0,
            layout: (0xf5c, 0xccc, 0x7ae),
            masks: [0; 8],
            keys: [0; 8],
            buttons: 0,
            slot: 0,
            saved_state: 0,
            ready: false,
            keyboards: crate::input_map::KeyBindings::DEVICE_INIT,
            modern_mode: false,
        }
    }

    /// Enable modern gamepad labels (`GAMEPAD 1`, `MODERN GAMEPAD`, `RUMBLE`,
    /// and Xbox/PlayStation button prompts).
    pub fn enable_modern_mode(&mut self, set: &mut MenuSet) {
        self.modern_mode = true;
        let analogue_id = self.id(ControlScreen::Analogue);
        let analogue = set.menu_mut(analogue_id);
        if let Some(item) = item_mut(analogue, key::ACCELERATION) {
            item.value.value = 1;
        }
        if let Some(item) = item_mut(analogue, key::BRAKE) {
            item.value.value = 1;
        }
        if let Some(item) = item_mut(analogue, key::FORCE_FEEDBACK) {
            item.text_id = crate::frontend_options::modern_text::LABEL_RUMBLE;
            item.value.value = 1;
        }
    }

    /// `FUN_004ba060` at the config load: the record's bindings installed
    /// into both keyboard records.
    pub fn install_record(&mut self, img: &Record) {
        for (player, keyboard) in self.keyboards.iter_mut().enumerate() {
            *keyboard = keyboard.with_record(img, player);
        }
    }

    pub fn id(&self, screen: ControlScreen) -> MenuId {
        self.ids[screen.index()]
    }

    pub fn screen_of(&self, id: MenuId) -> Option<ControlScreen> {
        ControlScreen::ALL.into_iter().find(|&s| self.id(s) == id)
    }

    /// `FUN_004ccd60`: the slide in, the cue, the navigation off.
    fn slide_in(&mut self, menu: &mut Menu, no_animation: bool, effects: &mut Vec<ControlsEffect>) {
        let from = -0x3333 - self.amplitude;
        self.tween = Some(if no_animation {
            ScalarTween::ease_out(0x1999, 0x1999, 15, 0)
        } else {
            ScalarTween::ease_out(from, 0x1999, 15, 0)
        });
        effects.push(ControlsEffect::SlideCue);
        self.panel_x = -self.amplitude;
        menu.nav = MenuNav::NONE;
    }

    /// The posts' slide out (mode 3), the cue and the navigation off.
    fn slide_out(
        &mut self,
        menu: &mut Menu,
        no_animation: bool,
        effects: &mut Vec<ControlsEffect>,
    ) {
        let to = -0x3333 - self.amplitude;
        self.tween = Some(if no_animation {
            ScalarTween::ease_out(to, to, 15, 0)
        } else {
            ScalarTween::ease_in(0x1999, to, 15, 0)
        });
        effects.push(ControlsEffect::SlideCue);
        self.panel_x = 0x1999;
        menu.nav = MenuNav::NONE;
    }

    /// The enter callbacks (`0x4ccbf0`, `0x4ccdc0`, `0x4ccf50`).
    #[allow(clippy::too_many_arguments)]
    pub fn enter(
        &mut self,
        screen: ControlScreen,
        arg: u8,
        menu: &mut Menu,
        img: &mut Record,
        player: u8,
        devices: &InputDevices,
        no_animation: bool,
        effects: &mut Vec<ControlsEffect>,
    ) {
        self.slide_in(menu, no_animation, effects);
        self.amplitude = 0xb333;
        let p = player_block(player);
        match screen {
            ControlScreen::Control => {
                self.layout = (0xf5c, 0x1eb8, 0x7ae);
                self.state = state::CONTROL;
                if arg == 0 {
                    let length = bit_length(devices.present);
                    if let Some(item) = item_mut(menu, key::METHOD) {
                        item.value.value_count = length - 1;
                    }
                    let device = i32::from(u16::from_le_bytes([img[p + 2], img[p + 3]]));
                    if device < length {
                        let method = if device < 2 { 0 } else { device - 1 };
                        if let Some(item) = item_mut(menu, key::METHOD) {
                            item.value.value = method;
                        }
                    }
                    if value(menu, key::METHOD) != 0 {
                        let kind =
                            i32::from_le_bytes([img[p + 4], img[p + 5], img[p + 6], img[p + 7]]);
                        let index = match kind {
                            0 => Some(0),
                            2 => Some(1),
                            4 => Some(2),
                            _ => None,
                        };
                        if let (Some(index), Some(item)) = (index, item_mut(menu, key::TYPE)) {
                            item.value.value = index;
                        }
                    }
                    skip_disabled(menu);
                }
                self.capturing = false;
            }
            ControlScreen::Analogue => {
                self.layout = (0xf5c, 0x11eb, 0x7ae);
                self.state = state::ANALOGUE;
                if arg == 0 && !self.keep_analogue {
                    img[p] |= 4;
                    let bits = img[p];
                    let set = |menu: &mut Menu, key, v: u8| {
                        if let Some(item) = item_mut(menu, key) {
                            item.value.value = i32::from(v);
                        }
                    };
                    set(menu, key::STEERING, 1);
                    set(menu, key::ACCELERATION, (bits >> 1) & 1);
                    set(menu, key::BRAKE, bits & 1);
                    set(menu, key::FORCE_FEEDBACK, (bits >> 3) & 1);
                    skip_disabled(menu);
                    if let Some(item) = item_mut(menu, key::FORCE_FEEDBACK) {
                        item.enabled = devices.force_feedback;
                        if !devices.force_feedback {
                            item.value.value = 0;
                        }
                    }
                }
                self.keep_analogue = false;
                self.capturing = false;
            }
            ControlScreen::Buttons => {
                self.layout = (0x7333, 0xccc, 0x418);
                self.state = state::BUTTONS;
                if arg == 0 {
                    self.enter_buttons(menu, img, player, devices, effects);
                }
                self.capturing = false;
            }
        }
    }

    /// `0x4ccf50`'s body: the button count, the item gating, and the working
    /// copy from the player block (its type matches), the device's profile
    /// (`img + 0x1830 + 0x54 dev`, its type matches) or the defaults, then
    /// the masks a device cannot produce dropped.
    fn enter_buttons(
        &mut self,
        menu: &mut Menu,
        img: &mut Record,
        player: u8,
        devices: &InputDevices,
        effects: &mut Vec<ControlsEffect>,
    ) {
        let p = player_block(player);
        let device = usize::from(u16::from_le_bytes([img[p + 2], img[p + 3]]));
        let kind = self.device_type;
        self.buttons = if matches!(kind, 0 | 2 | 4) {
            devices.buttons.get(device).copied().unwrap_or(0)
        } else {
            0xc
        };
        if !self.analogue {
            self.analogue_latch[0] = false;
            self.analogue_latch[1] = false;
            self.analogue_latch[2] = false;
        }
        let label = if img[0x2e] & 8 != 0 {
            0x3b
        } else if img[0x2c] & 0x40 != 0 {
            0xcb
        } else {
            0x3d
        };
        if let Some(item) = item_mut(menu, 4) {
            item.text_id = label;
        }
        let (accel, brake) = (self.analogue_latch[1], self.analogue_latch[2]);
        if let Some(item) = item_mut(menu, 2) {
            item.enabled = !accel;
        }
        if let Some(item) = item_mut(menu, 3) {
            item.enabled = !brake;
        }
        if let Some(item) = item_mut(menu, 8) {
            item.enabled = false;
            item.visible = false;
        }
        for key in [0, 1] {
            if let Some(item) = item_mut(menu, key) {
                item.enabled = kind == 1;
                item.visible = kind == 1;
            }
        }
        self.keys = [0; 8];
        let word = |img: &Record, at: usize| u16::from_le_bytes([img[at], img[at + 1]]);
        let dword = |img: &Record, at: usize| {
            u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]])
        };
        let stored_type = dword(img, p + 4) as i32;
        let profile = 0x1830 + device * 0x54;
        let profile_matches = img
            .get(profile + 4..profile + 8)
            .is_some_and(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) == kind);
        let source = if kind == stored_type {
            Some(p)
        } else {
            profile_matches.then_some(profile)
        };
        if let Some(at) = source {
            for i in 0..8 {
                self.masks[i] = word(img, at + 0x20 + 2 * i);
                if kind == 1 {
                    self.keys[i] = dword(img, at + 0x34 + 4 * i);
                }
            }
        } else if kind == 1 {
            self.keyboards[0] = self.keyboards[0].with_button_defaults(0);
            self.keyboards[1] = self.keyboards[1].with_button_defaults(1);
            effects.push(ControlsEffect::Keyboard { player: 0 });
            effects.push(ControlsEffect::Keyboard { player: 1 });
            let keyboard = &self.keyboards[usize::from(player.min(1))];
            self.keys = DEFAULT_MASKS.map(|mask| keyboard.code_of(u32::from(mask)));
            self.masks = DEFAULT_MASKS;
        } else {
            self.masks = DEFAULT_MASKS;
        }
        let limit = self.buttons + 4;
        for i in 0..8 {
            if bit_length(u32::from(self.masks[i])) > limit {
                self.keys[i] = if kind != stored_type {
                    self.keyboards[usize::from(player.min(1))].code_of(u32::from(self.masks[i]))
                } else {
                    dword(img, p + 0x34 + 4 * i)
                };
                self.masks[i] = 0;
            }
        }
        for (latched, axis) in [(accel, 4u16), (brake, 8)] {
            if latched {
                for mask in &mut self.masks[4..8] {
                    if *mask == axis {
                        *mask = 0;
                    }
                }
            }
        }
        skip_disabled(menu);
    }

    /// `0x4cdbe0`: capture a binding for the slot keyed `slot`.
    pub fn start_capture(&mut self, menu: &mut Menu, slot: i32) {
        self.saved_state = self.state;
        self.state = state::CAPTURE;
        menu.nav = MenuNav::NONE;
        self.capturing = true;
        self.slot = slot.clamp(0, 7) as usize;
    }

    /// `FUN_004cec30`: one capture step over the typematic key `key` (0 for
    /// none) and the player's device's rising edges `edge`.
    fn capture(&mut self, menu: &mut Menu, img: &Record, player: u8, key: u32, edge: u32) {
        let other = player_block(u8::from(player == 0));
        let selected = menu.selected.min(7);
        let mut key = key;
        if key != 0 && !self.ready {
            // The native waits here, unpumped, until the key is released.
            key = 0;
        }
        self.ready = true;
        let finish = |this: &mut Self, menu: &mut Menu| {
            menu.nav = MenuNav {
                moves: true,
                adjusts: true,
                confirms: true,
                backs: false,
            };
            this.state = this.saved_state;
        };
        if self.device_type == 1 {
            if key == 0 {
                return;
            }
            let old = self.keys[selected];
            for slot in &mut self.keys {
                if *slot == key {
                    *slot = old;
                }
            }
            self.keys[self.slot] = key;
            self.ready = false;
            finish(self, menu);
            return;
        }
        if key != 0 {
            let at = other + 0x34 + 4 * selected;
            let rejected = u32::from_le_bytes([img[at], img[at + 1], img[at + 2], img[at + 3]]);
            for i in 0..8 {
                if key == self.keys[i] {
                    self.keys[i] = 0;
                    continue;
                }
                if key == rejected {
                    return;
                }
            }
            self.keys[self.slot] = key;
            self.masks[self.slot] = 0;
            self.ready = false;
            finish(self, menu);
        }
        let edge = edge & 0xffff;
        if edge & 3 != 0 || edge.count_ones() != 1 {
            return;
        }
        let slot = self.slot;
        if let Some(i) = (0..8).find(|&i| u32::from(self.masks[i]) == edge && i != slot) {
            self.masks[i] = 0;
        }
        self.masks[slot] = edge as u16;
        self.keys[slot] = 0;
        finish(self, menu);
    }

    /// `FUN_004cdd30`: the default for an empty mask — the first of
    /// `4..0x80` (without 4 and 8 on a device of six buttons or more) that
    /// masks 1..7 and the low word of key 0 do not hold, else `0x100`.
    fn default_mask(&self) -> u16 {
        let words: Vec<u16> = self.masks[1..]
            .iter()
            .copied()
            .chain(std::iter::once(self.keys[0] as u16))
            .collect();
        for c in 2..8 {
            if self.buttons >= 6 && (c == 2 || c == 3) {
                continue;
            }
            if !words.contains(&(1 << c)) {
                return 1 << c;
            }
        }
        0x100
    }

    /// `0x4cdc30`, the button setup's OK: every empty mask defaulted, the
    /// keys and masks and type into the player block, the bindings into the
    /// keyboard record of the player, and the block copied to the device's
    /// profile.
    pub fn buttons_ok(&mut self, img: &mut Record, player: u8, effects: &mut Vec<ControlsEffect>) {
        let p = player_block(player);
        for i in 0..8 {
            if self.masks[i] == 0 {
                self.masks[i] = self.default_mask();
            }
            img[p + 0x34 + 4 * i..p + 0x38 + 4 * i].copy_from_slice(&self.keys[i].to_le_bytes());
            let keyboard = &mut self.keyboards[usize::from(player.min(1))];
            keyboard.install(self.masks[i], self.keys[i]);
        }
        for i in 0..8 {
            img[p + 0x20 + 2 * i..p + 0x22 + 2 * i].copy_from_slice(&self.masks[i].to_le_bytes());
        }
        img[p + 4..p + 8].copy_from_slice(&self.device_type.to_le_bytes());
        let device = usize::from(u16::from_le_bytes([img[p + 2], img[p + 3]]));
        let profile = 0x1830 + device * 0x54;
        if profile + 0x54 <= img.len() {
            img.copy_within(p..p + 0x54, profile);
        }
        effects.push(ControlsEffect::Keyboard { player });
    }

    /// `0x4cee30` (all three): the method count from the polled devices,
    /// the panel and the settle (`FUN_004ccb50`), then the control setup's
    /// type gating.
    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        &mut self,
        menu: &mut Menu,
        img: &Record,
        player: u8,
        devices: &InputDevices,
        key: u32,
        slow_motion: bool,
    ) {
        let mut count = bit_length(devices.polled);
        if count > 2 {
            count -= 1;
        }
        if let Some(item) = item_mut(menu, key::METHOD) {
            item.value.value_count = count - 1;
            if count - 2 < item.value.value {
                item.value.value = count - 2;
            }
        }
        if let Some(tween) = self.tween.as_mut()
            && !tween.is_finished()
        {
            self.panel_x = tween.evaluate_and_advance();
            self.settle = 0;
        } else if self.settle >= (i32::from(slow_motion) + 1) * 25 / 4 {
            if !self.capturing {
                menu.nav = MenuNav {
                    moves: true,
                    adjusts: true,
                    confirms: true,
                    backs: false,
                };
            }
        } else {
            self.settle += 1;
        }
        if self.state == state::CAPTURE {
            let p = player_block(player);
            let device = usize::from(u16::from_le_bytes([img[p + 2], img[p + 3]]));
            let edge = devices.edges.get(device).copied().unwrap_or(0);
            self.capture(menu, img, player, key, edge);
            return;
        }
        if self.state == state::CONTROL {
            if value(menu, key::METHOD) == 0 {
                if let Some(item) = item_mut(menu, key::TYPE) {
                    item.enabled = false;
                    item.value.value = -1;
                }
            } else if let Some(item) = item_mut(menu, key::TYPE) {
                item.enabled = true;
                if item.value.value < 0 {
                    item.value.value = 0;
                }
            }
        }
    }

    /// `0x4cda80`, the control setup's OK: the device and its type into the
    /// player block, then on to the analogue setup (types 2 and 4) or the
    /// button setup, whose parent (`0xd445e4`) becomes the screen it is
    /// reached from. Returns the menu to open and the button setup's parent.
    pub fn control_ok(
        &mut self,
        menu: &Menu,
        img: &mut Record,
        player: u8,
        effects: &mut Vec<ControlsEffect>,
    ) -> (MenuId, MenuId) {
        let p = player_block(player);
        let kind = value(menu, key::TYPE);
        let method = value(menu, key::METHOD);
        let (device, device_type) = if method == 0 {
            (u16::from(player), 1)
        } else {
            match kind {
                0 => ((method + 1) as u16, 0),
                1 => ((method + 1) as u16, 2),
                2 => ((method + 1) as u16, 4),
                _ => (0, 1),
            }
        };
        img[p + 2..p + 4].copy_from_slice(&device.to_le_bytes());
        self.device_type = device_type;
        effects.push(ControlsEffect::DeviceType {
            device,
            kind: device_type,
        });
        if matches!(device_type, 2 | 4) {
            self.analogue = true;
            self.state = state::ANALOGUE;
            let analogue = self.id(ControlScreen::Analogue);
            (analogue, analogue)
        } else {
            self.analogue = false;
            self.state = state::BUTTONS;
            (
                self.id(ControlScreen::Buttons),
                self.id(ControlScreen::Control),
            )
        }
    }

    /// The posts: `0x4cc6f0` (a digital or keyboard type clears the analogue
    /// and force-feedback bits), `0x4cc820` (the analogue bits latched and
    /// written) and `0x4cc9b0`.
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &mut self,
        screen: ControlScreen,
        back: bool,
        menu: &mut Menu,
        img: &mut Record,
        player: u8,
        no_animation: bool,
        effects: &mut Vec<ControlsEffect>,
    ) {
        self.slide_out(menu, no_animation, effects);
        let p = player_block(player);
        match screen {
            ControlScreen::Control if !back => {
                if matches!(self.device_type, 0 | 1) {
                    img[p] &= !0x0f;
                }
                skip_disabled(menu);
            }
            ControlScreen::Analogue => {
                let latch = [
                    value(menu, key::STEERING) != 0,
                    value(menu, key::ACCELERATION) != 0,
                    value(menu, key::BRAKE) != 0,
                    value(menu, key::FORCE_FEEDBACK) != 0,
                ];
                self.analogue_latch = latch;
                img[p] = (img[p] & !4) | u8::from(latch[0]) << 2;
                img[p] = (img[p] & !2) | u8::from(latch[1]) << 1;
                img[p] = (img[p] & !1) | u8::from(latch[2]);
                img[p] = (img[p] & !8) | u8::from(latch[3]) << 3;
            }
            _ => {}
        }
        menu.nav = MenuNav::NONE;
    }
}

fn callbacks(menu: &mut Menu, [enter, commit, predraw, post, leave]: [u32; 5]) {
    menu.on_enter = Some(enter);
    menu.on_commit = Some(commit);
    menu.on_predraw = Some(predraw);
    menu.on_post = Some(post);
    menu.on_leave = Some(leave);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build() -> (MenuSet, ControlScreens, Box<Record>) {
        let mut set = MenuSet::default();
        set.add(Menu::new(-1, None, None));
        let controls = ControlScreens::build(&mut set, MenuId(0));
        (set, controls, Box::new([0; LEGACY_CONFIG_SIZE]))
    }

    #[test]
    fn the_control_setup_reads_the_device_and_gates_the_type() {
        let (mut set, mut c, mut img) = build();
        let p = player_block(0);
        img[p + 2] = 3; // device 3
        img[p + 4] = 4; // analogue type 4
        let devices = InputDevices {
            present: 0b1111,
            polled: 0b1111,
            ..InputDevices::KEYBOARDS
        };
        let m = set.menu_mut(c.id(ControlScreen::Control));
        let mut effects = Vec::new();
        c.enter(
            ControlScreen::Control,
            0,
            m,
            &mut img,
            0,
            &devices,
            false,
            &mut effects,
        );
        assert_eq!(effects, vec![ControlsEffect::SlideCue]);
        assert_eq!(value(m, key::METHOD), 2);
        assert_eq!(value(m, key::TYPE), 2);
        assert_eq!(m.nav, MenuNav::NONE);
        // The slide, then six settle steps, then no back.
        for _ in 0..15 + 6 {
            c.commit(m, &img, 0, &devices, 0, false);
            assert!(!m.nav.moves);
        }
        c.commit(m, &img, 0, &devices, 0, false);
        assert!(m.nav.moves && !m.nav.backs);
        // Keyboard (method 0): the type is disabled at -1.
        m.items[0].value.value = 0;
        c.commit(m, &img, 0, &devices, 0, false);
        assert!(!m.items[1].enabled);
        assert_eq!(value(m, key::TYPE), -1);
        let (target, parent) = c.control_ok(m, &mut img, 0, &mut effects);
        assert_eq!(target, c.id(ControlScreen::Buttons));
        assert_eq!(parent, c.id(ControlScreen::Control));
        assert_eq!((img[p + 2], c.device_type), (0, 1));
    }

    #[test]
    fn an_analogue_type_opens_the_analogue_setup_which_writes_the_bits() {
        let (mut set, mut c, mut img) = build();
        let p = player_block(1);
        let mut effects = Vec::new();
        let control = c.id(ControlScreen::Control);
        set.menu_mut(control).items[0].value.value = 1;
        set.menu_mut(control).items[1].value.value = 1;
        let m = set.menu(control).clone();
        assert_eq!(
            c.control_ok(&m, &mut img, 1, &mut effects).0,
            c.id(ControlScreen::Analogue)
        );
        assert_eq!(img[p + 2], 2);
        img[p] = 0b1011;
        let a = set.menu_mut(c.id(ControlScreen::Analogue));
        c.enter(
            ControlScreen::Analogue,
            0,
            a,
            &mut img,
            1,
            &InputDevices::KEYBOARDS,
            true,
            &mut effects,
        );
        assert_eq!(img[p] & 4, 4, "steering forced on");
        assert_eq!(value(a, key::STEERING), 1);
        assert_eq!(value(a, key::ACCELERATION), 1);
        assert_eq!(value(a, key::BRAKE), 1);
        assert_eq!(value(a, key::FORCE_FEEDBACK), 0, "no force feedback device");
        a.items[2].value.value = 0;
        c.post(
            ControlScreen::Analogue,
            false,
            a,
            &mut img,
            1,
            true,
            &mut effects,
        );
        assert_eq!(img[p] & 0xf, 0b0110);
        // A digital type clears the bits on the control setup's post.
        c.device_type = 0;
        let m = set.menu_mut(control);
        c.post(
            ControlScreen::Control,
            false,
            m,
            &mut img,
            1,
            true,
            &mut effects,
        );
        assert_eq!(img[p] & 0xf, 0);
    }

    #[test]
    fn the_button_setup_loads_defaults_captures_a_key_and_saves() {
        let (mut set, mut c, mut img) = build();
        let p = player_block(0);
        // A keyboard type with nothing stored: the defaults.
        c.device_type = 1;
        img[p + 4] = 0xff; // stored type differs
        let b = c.id(ControlScreen::Buttons);
        let mut effects = Vec::new();
        let m = set.menu_mut(b);
        c.enter(
            ControlScreen::Buttons,
            0,
            m,
            &mut img,
            0,
            &InputDevices::KEYBOARDS,
            true,
            &mut effects,
        );
        assert_eq!(c.masks, DEFAULT_MASKS);
        assert_eq!(c.keys, [0x25, 0x27, 0x26, 0x28, 0x64, 0x68, 0x62, 0x66]);
        assert!(effects.contains(&ControlsEffect::Keyboard { player: 1 }));
        assert!(!m.items[8].visible && m.items[0].visible);
        // Capture slot 2 (accelerate): the first key event only arms it.
        m.selected = 2;
        c.start_capture(m, 2);
        c.commit(m, &img, 0, &InputDevices::KEYBOARDS, 0x41, false);
        assert_eq!(c.state, state::CAPTURE, "the first event is waited out");
        c.commit(m, &img, 0, &InputDevices::KEYBOARDS, 0x27, false);
        // Right was on slot 1: it takes slot 2's old key (Up).
        assert_eq!((c.keys[1], c.keys[2]), (0x26, 0x27));
        assert_eq!(c.state, state::BUTTONS);
        assert!(m.nav.confirms && !m.nav.backs);
        // OK writes the record, the profile and the keyboard record.
        c.buttons_ok(&mut img, 0, &mut effects);
        assert_eq!(img[p + 0x34 + 8], 0x27);
        assert_eq!(img[p + 0x20 + 4], 4);
        assert_eq!(img[p + 4], 1);
        assert_eq!(img[0x1830..0x1830 + 0x54], img[p..p + 0x54]);
        assert_eq!(c.keyboards[0].code_of(4), 0x27, "accelerate is now Right");
    }

    #[test]
    fn a_joystick_capture_takes_one_new_button_and_ok_fills_empty_masks() {
        let (mut set, mut c, mut img) = build();
        let p = player_block(0);
        img[p + 2] = 2; // device 2
        img[p + 4] = 0xff; // nothing stored for this type
        img[0x1830 + 2 * 0x54 + 4] = 0xff; // nor in device 2's profile
        c.device_type = 0;
        let mut devices = InputDevices::KEYBOARDS;
        devices.buttons[2] = 4;
        let b = c.id(ControlScreen::Buttons);
        let mut effects = Vec::new();
        let m = set.menu_mut(b);
        c.enter(
            ControlScreen::Buttons,
            0,
            m,
            &mut img,
            0,
            &devices,
            true,
            &mut effects,
        );
        // Four buttons: 0x100 and 0x200 (buttons 4 and 5) are out of range.
        assert_eq!(c.masks, [1, 2, 4, 8, 0, 0, 0x80, 0x40]);
        m.selected = 4;
        c.start_capture(m, 4);
        devices.edges[2] = 0x20; // button 1
        c.commit(m, &img, 0, &devices, 0, false);
        assert_eq!(c.masks[4], 0x20);
        assert_eq!(c.state, state::BUTTONS);
        // Two bits, or an X-axis bit, are ignored.
        c.start_capture(m, 5);
        devices.edges[2] = 0x30;
        c.commit(m, &img, 0, &devices, 0, false);
        devices.edges[2] = 0x1;
        c.commit(m, &img, 0, &devices, 0, false);
        assert_eq!(c.state, state::CAPTURE);
        c.state = state::BUTTONS;
        // OK: the empty slot 5 gets the first free default (0x10).
        c.buttons_ok(&mut img, 0, &mut effects);
        assert_eq!(c.masks[5], 0x10);
    }

    #[test]
    fn modern_gamepad_mode_exposes_gamepad_and_enables_analogue_and_force_feedback() {
        let (mut set, mut c, mut img) = build();
        c.enable_modern_mode(&mut set);
        assert!(c.modern_mode);
        crate::modern_settings::ModernSettings::default().apply_to_legacy_record(&mut img, true);

        let id = c.id(ControlScreen::Control);
        let mut effects = Vec::new();
        let m = set.menu_mut(id);
        c.enter(
            ControlScreen::Control,
            0,
            m,
            &mut img,
            0,
            &InputDevices::MODERN_GAMEPAD,
            true,
            &mut effects,
        );
        // Method item has value_count = 2 (Keyboard = 0, Gamepad 1 = 1) and defaults to Gamepad 1 (1).
        assert_eq!(m.items[0].value.value_count, 2);
        assert_eq!(m.items[0].value.value, 1);
        // Commit preserves value_count = 2 so the user can toggle between Keyboard and Gamepad 1.
        c.commit(m, &img, 0, &InputDevices::MODERN_GAMEPAD, 0, false);
        assert_eq!(m.items[0].value.value_count, 2);

        // Enter ControlScreen::Analogue and verify Steering, Acceleration, Brake, and Force Feedback are ON (1).
        let analogue_id = c.id(ControlScreen::Analogue);
        let a = set.menu_mut(analogue_id);
        c.enter(
            ControlScreen::Analogue,
            0,
            a,
            &mut img,
            0,
            &InputDevices::MODERN_GAMEPAD,
            true,
            &mut effects,
        );
        for row in 0..4 {
            assert!(
                a.items[row].enabled,
                "analogue row {row} enabled for modern gamepad"
            );
            assert_eq!(
                a.items[row].value.value, 1,
                "analogue row {row} defaults to ON"
            );
        }
    }
}
