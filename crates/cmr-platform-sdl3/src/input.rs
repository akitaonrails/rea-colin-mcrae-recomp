//! Host input translation into the recovered frontend input model.
//!
//! SDL keyboard and gamepad signals are mapped onto the recovered
//! [`cmr_core::frontend::FrontendInput`] contract. The recovered frontend is
//! edge-triggered: one call to `FrontendState::step` with `next` set advances
//! the selection exactly once. This mapper therefore emits one pulse per
//! press transition and never auto-repeats while an input is held; key/axis
//! auto-repeat is deliberately a host input policy, not a recovered behavior.
//!
//! The specific key and button bindings below are host policy. The shipped
//! game used DirectInput and its exact device bindings are not recovered, so
//! these choices are not a compatibility claim.

use std::collections::{BTreeMap, BTreeSet};

use cmr_core::frontend::FrontendInput;
use sdl3::gamepad::{Axis, Button};
use sdl3::keyboard::Keycode;

/// Minimum absolute analog-axis magnitude treated as a held direction.
///
/// SDL analog axes report `i16` values; half of the positive range gives a
/// generous dead zone so a centered stick never latches a direction.
pub const AXIS_DEAD_ZONE: i16 = 16_384;

/// One recovered frontend action a host device can drive.
///
/// The four actions correspond exactly to the fields of [`FrontendInput`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrontendAction {
    Previous,
    Next,
    Confirm,
    Cancel,
}

impl FrontendAction {
    const COUNT: usize = 4;

    const fn index(self) -> usize {
        self as usize
    }
}

/// Resolves a keyboard key to a frontend action, or `None` when unbound.
///
/// `Escape` is intentionally absent: the presenter uses it as a host
/// window-close shortcut, which is distinct from the recovered cancel action.
#[must_use]
pub fn action_for_key(keycode: Keycode) -> Option<FrontendAction> {
    match keycode {
        Keycode::Left | Keycode::Up => Some(FrontendAction::Previous),
        Keycode::Right | Keycode::Down => Some(FrontendAction::Next),
        Keycode::Return | Keycode::KpEnter | Keycode::Space => Some(FrontendAction::Confirm),
        Keycode::Backspace => Some(FrontendAction::Cancel),
        _ => None,
    }
}

/// Resolves a gamepad button to a frontend action, or `None` when unbound.
#[must_use]
pub fn action_for_button(button: Button) -> Option<FrontendAction> {
    match button {
        Button::DPadLeft | Button::DPadUp => Some(FrontendAction::Previous),
        Button::DPadRight | Button::DPadDown => Some(FrontendAction::Next),
        Button::South => Some(FrontendAction::Confirm),
        Button::East => Some(FrontendAction::Cancel),
        _ => None,
    }
}

/// Resolves a left-stick axis position to a direction, or `None` inside the
/// dead zone or for any non-navigation axis.
///
/// The vertical axis follows SDL's convention: negative is up (previous) and
/// positive is down (next).
#[must_use]
pub fn axis_action(axis: Axis, value: i16) -> Option<FrontendAction> {
    if !matches!(axis, Axis::LeftX | Axis::LeftY) {
        None
    } else if value <= -AXIS_DEAD_ZONE {
        Some(FrontendAction::Previous)
    } else if value >= AXIS_DEAD_ZONE {
        Some(FrontendAction::Next)
    } else {
        None
    }
}

/// Translates an SDL keyboard key to the Windows **virtual-key code** the
/// recovered in-race keyboard bindings ([`cmr_core::input_map::KeyBindings`])
/// expect, or `None` when the key has no bound VK code.
///
/// The recovered control map reads keys by VK code (`GetAsyncKeyState`), so the
/// SDL host must present the same VK numbering. This covers both default player
/// tables: player 1 (arrows + numpad + escape) and player 2 (A/S/R/D, Q, T, Tab,
/// escape). The one recovered player-2 binding with no clean SDL equivalent
/// (reset at raw VK `0x5c`) is left unmapped; that is host policy, not a
/// compatibility claim, exactly as the frontend bindings above.
#[must_use]
pub fn sdl_keycode_to_vk(keycode: Keycode) -> Option<u8> {
    let vk = match keycode {
        // Player 1: arrows + numpad.
        Keycode::Left => 0x25,
        Keycode::Up => 0x26,
        Keycode::Right => 0x27,
        Keycode::Down => 0x28,
        Keycode::Kp2 => 0x62,
        Keycode::Kp4 => 0x64,
        Keycode::Kp6 => 0x66,
        Keycode::Kp8 => 0x68,
        // Shared pause, and the pause menu's own GetAsyncKeyState reads:
        // VK_RETURN covers both Enter keys, VK_SHIFT either Shift key.
        Keycode::Escape => 0x1b,
        Keycode::Return | Keycode::KpEnter => 0x0d,
        Keycode::LShift | Keycode::RShift => 0x10,
        // Player 2: letters + tab.
        Keycode::Tab => 0x09,
        Keycode::A => 0x41,
        Keycode::D => 0x44,
        Keycode::Q => 0x51,
        Keycode::R => 0x52,
        Keycode::S => 0x53,
        Keycode::T => 0x54,
        // The rest of the keys the front end's name entry reads
        // (`FUN_004c64a0`): letters, digits, space, '.', Backspace, Delete.
        Keycode::B => 0x42,
        Keycode::C => 0x43,
        Keycode::E => 0x45,
        Keycode::F => 0x46,
        Keycode::G => 0x47,
        Keycode::H => 0x48,
        Keycode::I => 0x49,
        Keycode::J => 0x4a,
        Keycode::K => 0x4b,
        Keycode::L => 0x4c,
        Keycode::M => 0x4d,
        Keycode::N => 0x4e,
        Keycode::O => 0x4f,
        Keycode::P => 0x50,
        Keycode::U => 0x55,
        Keycode::V => 0x56,
        Keycode::W => 0x57,
        Keycode::X => 0x58,
        Keycode::Y => 0x59,
        Keycode::Z => 0x5a,
        Keycode::_0 => 0x30,
        Keycode::_1 => 0x31,
        Keycode::_2 => 0x32,
        Keycode::_3 => 0x33,
        Keycode::_4 => 0x34,
        Keycode::_5 => 0x35,
        Keycode::_6 => 0x36,
        Keycode::_7 => 0x37,
        Keycode::_8 => 0x38,
        Keycode::_9 => 0x39,
        Keycode::Space => 0x20,
        Keycode::Period => 0xbe,
        Keycode::Backspace => 0x08,
        Keycode::Delete => 0x2e,
        _ => return None,
    };
    Some(vk)
}

/// Translates a gamepad button into the menu/UI virtual-key code it drives
/// (`DPad` -> arrow VKs `0x25..=0x28`, `South` -> `VK_RETURN` `0x0d`,
/// `East`/`Back`/`Start` -> `VK_ESCAPE` `0x1b`), or `None` when unbound.
#[must_use]
pub fn gamepad_button_to_menu_vk(button: Button) -> Option<u8> {
    let vk = match button {
        Button::DPadLeft => 0x25,
        Button::DPadUp => 0x26,
        Button::DPadRight => 0x27,
        Button::DPadDown => 0x28,
        Button::South => 0x0d,
        Button::East | Button::Back | Button::Start => 0x1b,
        _ => return None,
    };
    Some(vk)
}

/// Translates a gamepad left-stick axis deflection outside [`AXIS_DEAD_ZONE`]
/// into the corresponding arrow virtual-key code (`LeftX` -> `0x25`/`0x27`,
/// `LeftY` -> `0x26`/`0x28`), or `None` inside the dead zone or for other axes.
#[must_use]
pub fn gamepad_axis_to_menu_vk(axis: Axis, value: i16) -> Option<u8> {
    match axis {
        Axis::LeftX if value <= -AXIS_DEAD_ZONE => Some(0x25),
        Axis::LeftX if value >= AXIS_DEAD_ZONE => Some(0x27),
        Axis::LeftY if value <= -AXIS_DEAD_ZONE => Some(0x26),
        Axis::LeftY if value >= AXIS_DEAD_ZONE => Some(0x28),
        _ => None,
    }
}

/// Tracks which recovered VK codes are currently held, so the in-race keyboard
/// poll ([`cmr_core::input_poll::DevicePoll::poll_keyboard`]) and all menu VK
/// polls (`front_end_frame`, `pause`, `service`, `school`, `results`) can be
/// driven from SDL keyboard and gamepad events with a level (`is_down`) probe.
///
/// Keyboard keys and gamepad navigation sources (`DPad`, left stick, `South`,
/// `East`, `Back`, `Start`) are tracked independently by source so releasing a
/// stick or D-pad button never clears a held keyboard key (or vice versa), while
/// [`Self::is_keyboard_down`] exposes the keyboard-only level for the in-race
/// keyboard driving poll.
#[derive(Clone, Debug, Default)]
pub struct KeyboardVkState {
    held: BTreeSet<u8>,
    gamepad_sources: BTreeMap<InputSource, u8>,
}

impl KeyboardVkState {
    /// A fresh tracker (nothing held).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a key-down (`pressed`) or key-up event; keys with no bound VK code
    /// are ignored.
    pub fn apply_key(&mut self, keycode: Keycode, pressed: bool) {
        let Some(vk) = sdl_keycode_to_vk(keycode) else {
            return;
        };
        self.apply_vk(vk, pressed);
    }

    /// Apply a key-down or key-up event already translated to its VK code.
    pub fn apply_vk(&mut self, vk: u8, pressed: bool) {
        if pressed {
            self.held.insert(vk);
        } else {
            self.held.remove(&vk);
        }
    }

    /// Apply a gamepad button-down (`pressed`) or button-up event for menu and
    /// pause navigation.
    pub fn apply_gamepad_button(&mut self, button: Button, pressed: bool) {
        let source = InputSource::button(button);
        match (gamepad_button_to_menu_vk(button), pressed) {
            (Some(vk), true) => {
                self.gamepad_sources.insert(source, vk);
            }
            _ => {
                self.gamepad_sources.remove(&source);
            }
        }
    }

    /// Apply a gamepad analog-axis motion for menu navigation (`LeftX` ->
    /// Left/Right VKs, `LeftY` -> Up/Down VKs). Non-navigation axes are ignored.
    pub fn apply_gamepad_axis(&mut self, axis: Axis, value: i16) {
        if !matches!(axis, Axis::LeftX | Axis::LeftY) {
            return;
        }
        let source = InputSource::axis(axis);
        if let Some(vk) = gamepad_axis_to_menu_vk(axis, value) {
            self.gamepad_sources.insert(source, vk);
        } else {
            self.gamepad_sources.remove(&source);
        }
    }

    /// Whether the given VK code is currently held on either the keyboard or a
    /// mapped gamepad navigation control.
    #[must_use]
    pub fn is_down(&self, vk: u8) -> bool {
        self.held.contains(&vk) || self.gamepad_sources.values().any(|&held_vk| held_vk == vk)
    }

    /// Whether the given VK code is held on the physical keyboard only (used by
    /// the in-race keyboard driving poll so gamepad navigation buttons do not
    /// alias into keyboard driving bindings).
    #[must_use]
    pub fn is_keyboard_down(&self, vk: u8) -> bool {
        self.held.contains(&vk)
    }

    /// Whether an in-race pause trigger (`Escape` on the keyboard, or `Start` /
    /// `Back` on the gamepad) is currently held. Excludes `Button::East` so
    /// pressing B/Circle during a race does not pause driving.
    #[must_use]
    pub fn is_pause_down(&self) -> bool {
        self.held.contains(&0x1b)
            || self
                .gamepad_sources
                .contains_key(&InputSource::button(Button::Start))
            || self
                .gamepad_sources
                .contains_key(&InputSource::button(Button::Back))
    }
}

/// Maps an SDL analog axis value (`i16`, centre `0`) onto the WinMM range the
/// recovered analog poll expects (`0..=0xffff`, centre `0x8000`).
///
/// The recovered `FUN_0049cdf0` reads `joyGetPosEx` axes in the WinMM range; an
/// SDL host presents its `i16` stick the same way so the recovered dead-zone and
/// axis split apply unchanged.
#[must_use]
pub fn sdl_axis_to_winmm(value: i16) -> u32 {
    (i32::from(value) + 0x8000).clamp(0, 0xffff) as u32
}

/// Combines a gamepad's two one-sided triggers (`0..=i16::MAX`) into the single
/// WinMM pedal axis the recovered analog path reads: the throttle trigger pulls
/// below centre (the recovered throttle half) and the brake trigger pulls above
/// it (the brake half), centred at `0x8000` when both are released.
///
/// Splitting the triggers onto one recovered pedal axis is host policy (the
/// shipped analog path has one pedal axis), not a compatibility claim.
#[must_use]
pub fn gamepad_trigger_pedal(throttle: i16, brake: i16) -> u32 {
    let throttle = i32::from(throttle.max(0));
    let brake = i32::from(brake.max(0));
    (0x8000 + brake - throttle).clamp(0, 0xffff) as u32
}

/// Maps a gamepad button to the recovered driving action bit it drives in the
/// control map's button mask, or `None` when unbound. Host policy, like the
/// frontend bindings: handbrake `0x100`, gear-up `0x200`, gear-down `0x80`,
/// reset `0x40`.
#[must_use]
pub fn gamepad_action_bit(button: Button) -> Option<u32> {
    let bit = match button {
        Button::South => 0x100,         // handbrake
        Button::RightShoulder => 0x200, // gear up
        Button::LeftShoulder => 0x80,   // gear down
        Button::North => 0x40,          // reset / rescue
        _ => return None,
    };
    Some(bit)
}

/// Tracks a gamepad's live driving state - the steering axis, the two pedal
/// triggers, and the held driving-action button bits - so the recovered analog
/// driver ([`cmr_core::gameplay_input::AnalogDriver`]) can be fed the WinMM-range
/// axes and the packed button mask each frame.
#[derive(Clone, Debug, Default)]
pub struct GamepadDrivingState {
    steer: i16,
    throttle: i16,
    brake: i16,
    buttons: u32,
    dpad_left: bool,
    dpad_right: bool,
    dpad_up: bool,
    dpad_down: bool,
}

impl GamepadDrivingState {
    /// A fresh state (centred axes, nothing held).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a gamepad axis motion: the left-X stick steers, the triggers are the
    /// pedals; other axes are ignored.
    pub fn apply_axis(&mut self, axis: Axis, value: i16) {
        match axis {
            Axis::LeftX => self.steer = value,
            Axis::TriggerRight => self.throttle = value,
            Axis::TriggerLeft => self.brake = value,
            _ => {}
        }
    }

    /// Apply a gamepad button-down (`pressed`) or button-up event, setting or
    /// clearing its recovered action bit and D-pad steering/pedal fallback.
    pub fn apply_button(&mut self, button: Button, pressed: bool) {
        match button {
            Button::DPadLeft => self.dpad_left = pressed,
            Button::DPadRight => self.dpad_right = pressed,
            Button::DPadUp => self.dpad_up = pressed,
            Button::DPadDown => self.dpad_down = pressed,
            _ => {}
        }
        let Some(bit) = gamepad_action_bit(button) else {
            return;
        };
        if pressed {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
    }

    /// The steering axis in the recovered WinMM range. When the analog stick is
    /// inside [`AXIS_DEAD_ZONE`], D-pad Left/Right provide full digital lock.
    #[must_use]
    pub fn steer_axis(&self) -> u32 {
        if self.steer.saturating_abs() < AXIS_DEAD_ZONE {
            match (self.dpad_left, self.dpad_right) {
                (true, false) => return 0,
                (false, true) => return 0xffff,
                _ => {}
            }
        }
        sdl_axis_to_winmm(self.steer)
    }

    /// The combined pedal axis in the recovered WinMM range. When both analog
    /// triggers are released, D-pad Up/Down provide digital throttle/brake.
    #[must_use]
    pub fn pedal_axis(&self) -> u32 {
        if self.throttle <= 0 && self.brake <= 0 {
            let throttle = if self.dpad_up { i16::MAX } else { 0 };
            let brake = if self.dpad_down { i16::MAX } else { 0 };
            if throttle > 0 || brake > 0 {
                return gamepad_trigger_pedal(throttle, brake);
            }
        }
        gamepad_trigger_pedal(self.throttle, self.brake)
    }

    /// Whether any driving axis or action is non-neutral.
    #[must_use]
    pub fn has_driving_input(&self) -> bool {
        self.steer_axis() != 0x8000 || self.pedal_axis() != 0x8000 || self.buttons != 0
    }

    /// The held driving-action button bits.
    #[must_use]
    pub fn buttons(&self) -> u32 {
        self.buttons
    }
}

/// A distinct physical input whose held state the mapper tracks.
///
/// Several sources may resolve to the same action (for example, the D-pad and
/// a keyboard arrow both request `Previous`); each is tracked independently so
/// releasing one never clears an action another source still holds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct InputSource {
    kind: u8,
    code: i64,
}

impl InputSource {
    const KEY: u8 = 0;
    const BUTTON: u8 = 1;
    const AXIS: u8 = 2;

    const fn key(keycode: Keycode) -> Self {
        Self {
            kind: Self::KEY,
            code: keycode as u32 as i64,
        }
    }

    const fn button(button: Button) -> Self {
        Self {
            kind: Self::BUTTON,
            code: button as i64,
        }
    }

    const fn axis(axis: Axis) -> Self {
        Self {
            kind: Self::AXIS,
            code: axis as i64,
        }
    }
}

/// Edge-triggered accumulator from host device signals to [`FrontendInput`].
///
/// Feed raw SDL events with [`apply_key`](Self::apply_key),
/// [`apply_button`](Self::apply_button), and [`apply_axis`](Self::apply_axis),
/// then drain one frame's worth of pulses with [`poll`](Self::poll).
#[derive(Clone, Debug, Default)]
pub struct FrontendInputMapper {
    active: BTreeMap<InputSource, FrontendAction>,
    pressed_edges: [bool; FrontendAction::COUNT],
}

impl FrontendInputMapper {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn is_held(&self, action: FrontendAction) -> bool {
        self.active.values().any(|&held| held == action)
    }

    fn press(&mut self, source: InputSource, action: FrontendAction) {
        if !self.is_held(action) {
            self.pressed_edges[action.index()] = true;
        }
        // Overwriting re-maps a source that changed direction (for example, an
        // axis swinging across center); the previous action is dropped so it is
        // no longer considered held unless another source still provides it.
        self.active.insert(source, action);
    }

    fn release(&mut self, source: InputSource) {
        self.active.remove(&source);
    }

    /// Applies a keyboard key-down (`pressed`) or key-up event.
    pub fn apply_key(&mut self, keycode: Keycode, pressed: bool) {
        self.apply(InputSource::key(keycode), action_for_key(keycode), pressed);
    }

    /// Applies a gamepad button-down (`pressed`) or button-up event.
    pub fn apply_button(&mut self, button: Button, pressed: bool) {
        self.apply(
            InputSource::button(button),
            action_for_button(button),
            pressed,
        );
    }

    /// Applies a gamepad analog-axis motion, latching or clearing a direction.
    pub fn apply_axis(&mut self, axis: Axis, value: i16) {
        let source = InputSource::axis(axis);
        match axis_action(axis, value) {
            Some(action) => self.press(source, action),
            None => self.release(source),
        }
    }

    fn apply(&mut self, source: InputSource, action: Option<FrontendAction>, pressed: bool) {
        match (action, pressed) {
            (Some(action), true) => self.press(source, action),
            (Some(_), false) => self.release(source),
            // An unbound key still clears any stale mapping for its source.
            (None, _) => self.release(source),
        }
    }

    /// Drains the press edges accumulated since the last poll into one
    /// [`FrontendInput`] suitable for a single `FrontendState::step` call.
    #[must_use]
    pub fn poll(&mut self) -> FrontendInput {
        let input = FrontendInput {
            previous: self.pressed_edges[FrontendAction::Previous.index()],
            next: self.pressed_edges[FrontendAction::Next.index()],
            confirm: self.pressed_edges[FrontendAction::Confirm.index()],
            cancel: self.pressed_edges[FrontendAction::Cancel.index()],
        };
        self.pressed_edges = [false; FrontendAction::COUNT];
        input
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmr_core::frontend::{FrontendEffect, FrontendState, ROOT_SCREEN, TransitionResult};

    #[test]
    fn sdl_axis_maps_onto_the_winmm_range() {
        assert_eq!(sdl_axis_to_winmm(0), 0x8000);
        assert_eq!(sdl_axis_to_winmm(i16::MAX), 0xffff);
        assert_eq!(sdl_axis_to_winmm(i16::MIN), 0);
    }

    #[test]
    fn gamepad_triggers_split_onto_the_recovered_pedal_axis() {
        // Released: centre.
        assert_eq!(gamepad_trigger_pedal(0, 0), 0x8000);
        // Full throttle pulls below centre (the recovered throttle half).
        assert!(gamepad_trigger_pedal(i16::MAX, 0) < 0x8000);
        // Full brake pulls to the top (the brake half).
        assert_eq!(gamepad_trigger_pedal(0, i16::MAX), 0xffff);
    }

    #[test]
    fn gamepad_buttons_map_to_recovered_action_bits() {
        assert_eq!(gamepad_action_bit(Button::South), Some(0x100));
        assert_eq!(gamepad_action_bit(Button::RightShoulder), Some(0x200));
        assert_eq!(gamepad_action_bit(Button::LeftShoulder), Some(0x80));
        assert_eq!(gamepad_action_bit(Button::North), Some(0x40));
        assert_eq!(gamepad_action_bit(Button::Start), None);
    }

    #[test]
    fn gamepad_driving_state_tracks_axes_and_buttons() {
        let mut state = GamepadDrivingState::new();
        assert_eq!(state.steer_axis(), 0x8000);
        assert_eq!(state.pedal_axis(), 0x8000);
        assert_eq!(state.buttons(), 0);

        state.apply_axis(Axis::LeftX, i16::MAX);
        state.apply_axis(Axis::TriggerRight, i16::MAX);
        assert_eq!(state.steer_axis(), 0xffff);
        assert!(state.pedal_axis() < 0x8000); // throttle half
        // A non-driving axis is ignored.
        state.apply_axis(Axis::RightY, i16::MAX);

        state.apply_button(Button::South, true);
        assert_eq!(state.buttons() & 0x100, 0x100);
        state.apply_button(Button::South, false);
        assert_eq!(state.buttons() & 0x100, 0);
    }

    #[test]
    fn gamepad_driving_activity_requires_a_nonneutral_axis_or_button() {
        let mut state = GamepadDrivingState::new();
        assert!(!state.has_driving_input());
        state.apply_axis(Axis::TriggerRight, 0x4000);
        assert!(state.has_driving_input());
    }

    #[test]
    fn gamepad_state_feeds_the_recovered_analog_driver() {
        use cmr_core::drive_controls::DriveResponse;
        use cmr_core::fixed_point::Fixed16;
        use cmr_core::gameplay_input::AnalogDriver;

        let mut state = GamepadDrivingState::new();
        let mut driver = AnalogDriver::new();
        let response = DriveResponse {
            max_forward_force: Fixed16::from_raw(8 << 16),
            max_steer_rate: 0x200,
        };

        // Full throttle trigger drives forward through the whole recovered chain.
        state.apply_axis(Axis::TriggerRight, i16::MAX);
        let frame = driver.poll(
            state.steer_axis(),
            state.pedal_axis(),
            state.buttons(),
            &response,
            true,
        );
        assert!(frame.car.throttle > 0);
        assert!(frame.controls.forward_force > Fixed16::ZERO);
    }

    #[test]
    fn nominal_sdl_stick_directions_match_keyboard_steering_semantics() {
        use cmr_core::drive_controls::DriveResponse;
        use cmr_core::fixed_point::Fixed16;
        use cmr_core::gameplay_input::{AnalogDriver, KeyboardDriver};

        let response = DriveResponse {
            max_forward_force: Fixed16::from_raw(8 << 16),
            max_steer_rate: 0x200,
        };
        let mut state = GamepadDrivingState::new();
        let mut analog = AnalogDriver::new();

        // SDL's standard LeftX convention is negative for a physical left
        // deflection and positive for right.  Pin the full adapter -> recovered
        // analog-driver chain to the same heading signs as the independently
        // recovered keyboard path. The explicit planar harness preserves the
        // recovered right-minus-left command.
        state.apply_axis(Axis::LeftX, i16::MIN);
        let stick_left = analog.poll(
            state.steer_axis(),
            state.pedal_axis(),
            state.buttons(),
            &response,
            true,
        );
        state.apply_axis(Axis::LeftX, i16::MAX);
        let stick_right = analog.poll(
            state.steer_axis(),
            state.pedal_axis(),
            state.buttons(),
            &response,
            true,
        );

        let mut keyboard = KeyboardDriver::player_one();
        let keyboard_left = keyboard.poll(|vk| vk == 0x25, &response, true);
        let keyboard_right = keyboard.poll(|vk| vk == 0x27, &response, true);

        assert!(stick_left.car.steer_left > 0);
        assert_eq!(stick_left.car.steer_right, 0);
        assert!(stick_left.controls.steer_rate < 0);
        assert_eq!(
            stick_left.controls.steer_rate.signum(),
            keyboard_left.controls.steer_rate.signum()
        );
        let stick_left_heading = 0_u16.wrapping_add(stick_left.controls.steer_rate as u16);
        let keyboard_left_heading = 0_u16.wrapping_add(keyboard_left.controls.steer_rate as u16);
        assert!((stick_left_heading as i16) < 0);
        assert!((keyboard_left_heading as i16) < 0);

        assert!(stick_right.car.steer_right > 0);
        assert_eq!(stick_right.car.steer_left, 0);
        assert!(stick_right.controls.steer_rate > 0);
        assert_eq!(
            stick_right.controls.steer_rate.signum(),
            keyboard_right.controls.steer_rate.signum()
        );
        let stick_right_heading = 0_u16.wrapping_add(stick_right.controls.steer_rate as u16);
        let keyboard_right_heading = 0_u16.wrapping_add(keyboard_right.controls.steer_rate as u16);
        assert!((stick_right_heading as i16) > 0);
        assert!((keyboard_right_heading as i16) > 0);
    }

    #[test]
    fn sdl_keys_translate_to_the_recovered_vk_codes() {
        // Player 1 arrows + numpad.
        assert_eq!(sdl_keycode_to_vk(Keycode::Left), Some(0x25));
        assert_eq!(sdl_keycode_to_vk(Keycode::Up), Some(0x26));
        assert_eq!(sdl_keycode_to_vk(Keycode::Right), Some(0x27));
        assert_eq!(sdl_keycode_to_vk(Keycode::Down), Some(0x28));
        assert_eq!(sdl_keycode_to_vk(Keycode::Kp8), Some(0x68));
        assert_eq!(sdl_keycode_to_vk(Keycode::Escape), Some(0x1b));
        assert_eq!(sdl_keycode_to_vk(Keycode::Return), Some(0x0d));
        assert_eq!(sdl_keycode_to_vk(Keycode::KpEnter), Some(0x0d));
        assert_eq!(sdl_keycode_to_vk(Keycode::RShift), Some(0x10));
        // Numpad 6 is player one's change-view action.
        assert_eq!(sdl_keycode_to_vk(Keycode::Kp6), Some(0x66));
        // Player 2 letters + tab.
        assert_eq!(sdl_keycode_to_vk(Keycode::A), Some(0x41));
        assert_eq!(sdl_keycode_to_vk(Keycode::S), Some(0x53));
        assert_eq!(sdl_keycode_to_vk(Keycode::Tab), Some(0x09));
        // An unbound key.
        assert_eq!(sdl_keycode_to_vk(Keycode::F1), None);
    }

    #[test]
    fn held_vk_state_tracks_press_and_release_and_ignores_unbound_keys() {
        let mut keys = KeyboardVkState::new();
        assert!(!keys.is_down(0x26));
        keys.apply_key(Keycode::Up, true);
        assert!(keys.is_down(0x26));
        // An unbound key changes nothing.
        keys.apply_key(Keycode::F1, true);
        assert!(keys.is_down(0x26));
        keys.apply_key(Keycode::Up, false);
        assert!(!keys.is_down(0x26));
    }

    #[test]
    fn held_vk_state_feeds_the_recovered_driving_chain() {
        use cmr_core::drive_controls::DriveResponse;
        use cmr_core::fixed_point::Fixed16;
        use cmr_core::gameplay_input::KeyboardDriver;

        let mut keys = KeyboardVkState::new();
        let mut driver = KeyboardDriver::player_one();
        let response = DriveResponse {
            max_forward_force: Fixed16::from_raw(8 << 16),
            max_steer_rate: 0x200,
        };

        // Holding Up (SDL) drives the throttle through the whole recovered chain.
        keys.apply_key(Keycode::Up, true);
        let frame = driver.poll(|vk| keys.is_down(vk), &response, true);
        assert!(frame.car.throttle > 0);
        assert!(frame.controls.forward_force > Fixed16::ZERO);

        // Releasing returns to neutral.
        keys.apply_key(Keycode::Up, false);
        let neutral = driver.poll(|vk| keys.is_down(vk), &response, true);
        assert_eq!(neutral.controls.forward_force, Fixed16::ZERO);
    }

    #[test]
    fn keyboard_and_dpad_bindings_resolve_to_the_recovered_actions() {
        assert_eq!(
            action_for_key(Keycode::Left),
            Some(FrontendAction::Previous)
        );
        assert_eq!(action_for_key(Keycode::Up), Some(FrontendAction::Previous));
        assert_eq!(action_for_key(Keycode::Right), Some(FrontendAction::Next));
        assert_eq!(action_for_key(Keycode::Down), Some(FrontendAction::Next));
        assert_eq!(
            action_for_key(Keycode::Return),
            Some(FrontendAction::Confirm)
        );
        assert_eq!(
            action_for_key(Keycode::Space),
            Some(FrontendAction::Confirm)
        );
        assert_eq!(
            action_for_key(Keycode::Backspace),
            Some(FrontendAction::Cancel)
        );
        // Escape stays a window-close shortcut, never the recovered cancel.
        assert_eq!(action_for_key(Keycode::Escape), None);

        assert_eq!(
            action_for_button(Button::DPadLeft),
            Some(FrontendAction::Previous)
        );
        assert_eq!(
            action_for_button(Button::DPadRight),
            Some(FrontendAction::Next)
        );
        assert_eq!(
            action_for_button(Button::South),
            Some(FrontendAction::Confirm)
        );
        assert_eq!(
            action_for_button(Button::East),
            Some(FrontendAction::Cancel)
        );
        assert_eq!(action_for_button(Button::Start), None);
    }

    #[test]
    fn axis_latches_direction_only_outside_the_dead_zone() {
        assert_eq!(axis_action(Axis::LeftX, 0), None);
        assert_eq!(axis_action(Axis::LeftX, AXIS_DEAD_ZONE - 1), None);
        assert_eq!(
            axis_action(Axis::LeftX, AXIS_DEAD_ZONE),
            Some(FrontendAction::Next)
        );
        assert_eq!(
            axis_action(Axis::LeftX, -AXIS_DEAD_ZONE),
            Some(FrontendAction::Previous)
        );
        assert_eq!(
            axis_action(Axis::LeftY, i16::MIN),
            Some(FrontendAction::Previous)
        );
        assert_eq!(axis_action(Axis::RightX, i16::MAX), None);
        assert_eq!(axis_action(Axis::TriggerLeft, i16::MAX), None);
    }

    #[test]
    fn a_held_key_pulses_once_until_released_and_pressed_again() {
        let mut mapper = FrontendInputMapper::new();

        mapper.apply_key(Keycode::Right, true);
        assert!(mapper.poll().next);
        // Still held, but no new press transition: no further pulse.
        assert!(!mapper.poll().next);

        mapper.apply_key(Keycode::Right, false);
        mapper.apply_key(Keycode::Right, true);
        assert!(mapper.poll().next);
    }

    #[test]
    fn releasing_one_source_keeps_an_action_another_source_still_holds() {
        let mut mapper = FrontendInputMapper::new();

        mapper.apply_key(Keycode::Up, true);
        mapper.apply_button(Button::DPadUp, true);
        assert!(mapper.poll().previous);

        // Releasing the key alone must not re-arm an edge while the D-pad holds.
        mapper.apply_key(Keycode::Up, false);
        mapper.apply_key(Keycode::Up, true);
        assert!(
            !mapper.poll().previous,
            "action was continuously held by the D-pad"
        );

        // Only after every source releases does a fresh press pulse again.
        mapper.apply_key(Keycode::Up, false);
        mapper.apply_button(Button::DPadUp, false);
        mapper.apply_key(Keycode::Up, true);
        assert!(mapper.poll().previous);
    }

    #[test]
    fn an_axis_swinging_across_center_re_arms_the_opposite_direction() {
        let mut mapper = FrontendInputMapper::new();

        mapper.apply_axis(Axis::LeftX, i16::MAX);
        assert!(mapper.poll().next);

        mapper.apply_axis(Axis::LeftX, 0);
        let centered = mapper.poll();
        assert!(!centered.next && !centered.previous);

        mapper.apply_axis(Axis::LeftX, i16::MIN);
        assert!(mapper.poll().previous);
    }

    #[test]
    fn poll_with_no_new_press_is_the_neutral_input() {
        let mut mapper = FrontendInputMapper::new();
        assert_eq!(mapper.poll(), FrontendInput::default());
    }

    #[test]
    fn mapped_input_drives_the_recovered_frontend_selection_contract() {
        // One previous pulse wraps 0 -> last; a confirm then opens that entry.
        let mut mapper = FrontendInputMapper::new();
        let mut state = FrontendState::after_startup_art(0, 0).unwrap();
        // Advance the bootstrap transition into the interactive root screen.
        let _ = state.step(
            FrontendInput::default(),
            1,
            TransitionResult::Complete,
            true,
        );
        let _ = state.step(
            FrontendInput::default(),
            2,
            TransitionResult::Complete,
            true,
        );

        let last_selection = (ROOT_SCREEN.entries.len() - 1) as u8;
        mapper.apply_button(Button::DPadLeft, true);
        let effects = state.step(mapper.poll(), 3, TransitionResult::Pending, true);
        assert_eq!(state.root_selection().index(), last_selection);
        assert_eq!(
            effects,
            vec![FrontendEffect::PersistRootSelection(last_selection)]
        );

        mapper.apply_button(Button::DPadLeft, false);
        mapper.apply_key(Keycode::Return, true);
        let _ = state.step(mapper.poll(), 4, TransitionResult::Pending, true);
        assert!(state.pending_screen().is_some());
    }

    #[test]
    fn gamepad_menu_vks_track_dpad_stick_and_buttons_without_clobbering_keyboard() {
        let mut keys = KeyboardVkState::new();
        // Hold Left on the keyboard AND deflect LeftX left on the gamepad.
        keys.apply_key(Keycode::Left, true);
        keys.apply_gamepad_axis(Axis::LeftX, -AXIS_DEAD_ZONE);
        assert!(keys.is_down(0x25));
        assert!(keys.is_keyboard_down(0x25));

        // Centering the stick must not release the still-held keyboard Left arrow.
        keys.apply_gamepad_axis(Axis::LeftX, 0);
        assert!(keys.is_down(0x25));
        assert!(keys.is_keyboard_down(0x25));

        // Releasing keyboard Left while DPadLeft is held keeps 0x25 active for
        // menus (`is_down`), while `is_keyboard_down` is false.
        keys.apply_gamepad_button(Button::DPadLeft, true);
        keys.apply_key(Keycode::Left, false);
        assert!(keys.is_down(0x25));
        assert!(!keys.is_keyboard_down(0x25));
        keys.apply_gamepad_button(Button::DPadLeft, false);
        assert!(!keys.is_down(0x25));

        // South maps to Return (0x0d); East/Start/Back map to Escape (0x1b),
        // while only Start/Back/Escape trigger `is_pause_down`.
        keys.apply_gamepad_button(Button::South, true);
        assert!(keys.is_down(0x0d));
        keys.apply_gamepad_button(Button::South, false);
        assert!(!keys.is_down(0x0d));

        keys.apply_gamepad_button(Button::East, true);
        assert!(keys.is_down(0x1b));
        assert!(!keys.is_pause_down());
        keys.apply_gamepad_button(Button::East, false);

        keys.apply_gamepad_button(Button::Start, true);
        assert!(keys.is_down(0x1b));
        assert!(keys.is_pause_down());
        keys.apply_gamepad_button(Button::Start, false);
        assert!(!keys.is_pause_down());
    }

    #[test]
    fn gamepad_dpad_provides_digital_steer_and_pedal_fallback() {
        let mut pad = GamepadDrivingState::new();
        pad.apply_button(Button::DPadLeft, true);
        assert_eq!(pad.steer_axis(), 0);
        assert!(pad.has_driving_input());
        pad.apply_button(Button::DPadLeft, false);
        pad.apply_button(Button::DPadRight, true);
        assert_eq!(pad.steer_axis(), 0xffff);
        pad.apply_button(Button::DPadRight, false);
        assert_eq!(pad.steer_axis(), 0x8000);

        pad.apply_button(Button::DPadUp, true);
        assert!(pad.pedal_axis() < 0x8000);
        pad.apply_button(Button::DPadUp, false);
        pad.apply_button(Button::DPadDown, true);
        assert_eq!(pad.pedal_axis(), 0xffff);
    }
}
