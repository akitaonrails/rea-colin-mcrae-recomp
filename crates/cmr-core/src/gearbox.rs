//! The gearbox part of the per-car control processor `FUN_0041b5b0`.
//!
//! The processor runs the gearbox before steering and pedals, and only while
//! the two per-player runtime flags (`FUN_0043bc60`/`FUN_0043bc50`, i.e.
//! `DAT_00550cf0[player]`/`DAT_00551558[player]`) and the residual countdown
//! control `+0x43b` are all zero. A pending shift request `+0x40c` is applied
//! first and ends the gearbox for that tick (`+0x43a = +0x43f`, request
//! cleared). Otherwise the mode `+0x3f0` selects:
//!
//! - `0`, automatic (`FUN_0041b810`): walk from the current gear, then gate the
//!   result; any nonzero `+0x314` toggles first and reverse;
//! - `1`, manual (`FUN_0041bb30`, only when `+0x314 != 0`): step by `+-1`;
//! - `2`, automatic with brake-to-reverse (`FUN_0041bf00`).
//!
//! In manual mode the processor ends with `FUN_0041bc90`, which reruns the walk
//! on unsigned gears and publishes whether it would shift up (`+0x420`).
//!
//! Observations (instruction level for `FUN_0041b810`, `0x41b810..0x41bb2b`;
//! the other routines at pseudocode level with the same walk):
//!
//! - The automatic walk runs only when a pedal (`+0x304`/`+0x308`) is pressed
//!   and at least one of car `+0x73a..+0x73d` is zero. It starts from gear
//!   `g = +0x43a` with engine speed `rpm = +0x2e0` and wheel-side speed
//!   `base = qmul(rpm, +0x1f0[g])`. Each step tests `lo = qmul(+0x1b0[g],
//!   +0x15c)` and `hi = qmul(+0x190[g], +0x15c)`: inside `(lo, hi)` it stops;
//!   at or below `lo` it shifts down when `2 <= g <= 6` and it has not shifted
//!   up; otherwise at or above `hi` it shifts up when `1 <= g <= 5` and it has
//!   not shifted down. Each step sets `rpm = qmul(base, +0x1d0[g])`. When the
//!   per-car-type word `DAT_0050e170[type]` exceeds `0xb333`, a step landing on
//!   gear 2 either skips it (to 1 or 3, reloading the counter `+0x443 = 5`) or
//!   stays and decrements the counter.
//! - The result is published only if it differs from the current gear and the
//!   cooldown `+0x441` is zero or the last direction `+0x442` allows it (no
//!   reversal while cooling down): `+0x40c = 1`, `+0x442` = 2 up / 1 down,
//!   `+0x441 = 10`, `+0x43f` = gear. The cooldown decrements once per call
//!   before the walk.
//!
//! Gear 7 is reverse and gear 0 neutral: neither ever shifts automatically, so
//! table entries 0 and 7 are read but never decide anything. Inference only:
//! `+0x73a..+0x73d` read like per-wheel airborne flags.

use crate::fixed_point::Fixed16;

/// Reverse gear.
pub const REVERSE_GEAR: i8 = 7;
/// Shift cooldown reload (`+0x441`).
const SHIFT_COOLDOWN: i8 = 10;
/// Gear-2 skip counter reload (`+0x443`).
const GEAR_TWO_SKIP_RELOAD: i8 = 5;
/// Mode 2 selects reverse below this engine speed (`+0x2e0`).
const BRAKE_REVERSE_ENGINE_LIMIT: i32 = 0x1999;

/// Per-gear tables from the control record (index = gear `0..=7`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GearboxTables {
    /// `+0x190`: upshift thresholds (scaled by `+0x15c`).
    pub upper_190: [i32; 8],
    /// `+0x1b0`: downshift thresholds (scaled by `+0x15c`).
    pub lower_1b0: [i32; 8],
    /// `+0x1d0`: gear ratios.
    pub ratio_1d0: [i32; 8],
    /// `+0x1f0`: inverse gear ratios.
    pub inverse_1f0: [i32; 8],
    /// `+0x15c`: engine-speed scale (redline).
    pub redline_15c: i32,
}

/// The gearbox words the processor owns across ticks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GearboxState {
    /// `+0x43a`: current gear.
    pub gear_43a: i8,
    /// `+0x43f`: requested gear.
    pub target_43f: i8,
    /// `+0x40c`: a shift is pending (applied on the next gearbox call).
    pub shift_40c: bool,
    /// `+0x441`: cooldown after a published automatic shift.
    pub cooldown_441: i8,
    /// `+0x442`: direction of the last automatic shift (2 up, 1 down).
    pub last_direction_442: u8,
    /// `+0x443`: gear-2 skip counter.
    pub skip_443: i8,
    /// `+0x408`: reversing mode set by mode 2 (swaps the pedals).
    pub reverse_mode_408: bool,
    /// `+0x420`: the manual-mode upshift hint from `FUN_0041bc90`.
    pub upshift_hint_420: bool,
}

impl GearboxState {
    /// Every word at zero: neutral, nothing pending.
    pub const ZERO: Self = Self {
        gear_43a: 0,
        target_43f: 0,
        shift_40c: false,
        cooldown_441: 0,
        last_direction_442: 0,
        skip_443: 0,
        reverse_mode_408: false,
        upshift_hint_420: false,
    };

    /// The car initializer's seed (`0x40465f..0x40467c`, plus `FUN_00407270`
    /// setting `ctl + 0x420 = 1` at `0x4072b4`): first gear when the runtime
    /// global `DAT_00514478` is nonzero, else neutral, with the shift request
    /// cleared and the upshift hint `ctl + 0x420` enabled.
    #[must_use]
    pub const fn at_setup(start_in_gear: bool) -> Self {
        let gear = if start_in_gear { 1 } else { 0 };
        Self {
            gear_43a: gear,
            target_43f: gear,
            upshift_hint_420: true,
            ..Self::ZERO
        }
    }
}

/// The car setup's gearbox mode (`0x406fa1..0x406fe9`): control `+0x3f0`
/// is 1 (manual) when the player's config car byte (`cfg[0xb6 + 0x9c p]`) has
/// the manual bit `0x10`, else 2 (automatic with brake-to-reverse).
#[must_use]
pub const fn setup_mode(car_byte: u8) -> i32 {
    if car_byte & 0x10 != 0 { 1 } else { 2 }
}

/// Per-car gearbox configuration and the runtime gates the caller resolves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GearboxParams {
    pub tables: GearboxTables,
    /// `+0x3f0`: 0 automatic, 1 manual, 2 automatic with brake-to-reverse.
    pub mode_3f0: i32,
    /// `DAT_0050e170[type] > 0xb333`.
    pub gear_two_skip: bool,
    /// Mode 2's early-return gate is clear.
    pub brake_reverse_enabled: bool,
    /// Either per-player runtime flag (`DAT_00550cf0`/`DAT_00551558`) is set.
    pub player_locked: bool,
    /// Car `+0x73a..+0x73d`.
    pub wheel_flags_73a: [u8; 4],
    /// The drivetrain's raw gear ratio for each gear, as the wheel passes read
    /// it after a gear change.
    pub drive_ratios: [i32; 8],
}

/// Per-tick gearbox inputs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GearboxInputs {
    /// `+0x3f0`: 0 automatic, 1 manual, 2 automatic with brake-to-reverse.
    pub mode_3f0: i32,
    /// `+0x314`: the mapper's gear command (`1` up, `-1` down, `0` none).
    pub gear_command_314: i8,
    pub throttle_304: i32,
    pub brake_308: i32,
    /// `+0x2e0`: engine speed.
    pub engine_2e0: i32,
    /// Car `+0x73a..+0x73d`.
    pub wheel_flags_73a: [u8; 4],
    /// `DAT_0050e170[type] > 0xb333`.
    pub gear_two_skip: bool,
    /// Either per-player runtime flag is set.
    pub player_locked: bool,
    /// Control `+0x43b`, the residual countdown.
    pub residual_43b: u8,
    /// Mode 2's early-return gate is clear (`DAT_0057b0ac[type] == 100`, or
    /// `DAT_00d3ca88 != 0`, or `DAT_004eb0cc == 0`).
    pub brake_reverse_enabled: bool,
}

/// A gear outside the eight-entry tables reached a table read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GearOutOfRange {
    pub gear: i32,
}

impl std::fmt::Display for GearOutOfRange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "gear {} is outside the 0..=7 gearbox tables",
            self.gear
        )
    }
}

impl std::error::Error for GearOutOfRange {}

fn qmul(left: i32, right: i32) -> i32 {
    Fixed16::from_raw(left).mul(Fixed16::from_raw(right)).raw()
}

fn entry(table: &[i32; 8], gear: i32) -> Result<i32, GearOutOfRange> {
    usize::try_from(gear)
        .ok()
        .and_then(|index| table.get(index).copied())
        .ok_or(GearOutOfRange { gear })
}

/// The gear-2 skip rule shared by every walk and the manual step: returns the
/// gear after the rule.
fn apply_gear_two_skip(gear: i32, skip_to: i32, enabled: bool, counter: &mut i8) -> i32 {
    if !enabled || gear != 2 {
        return gear;
    }
    if *counter < 1 {
        *counter = GEAR_TWO_SKIP_RELOAD;
        skip_to
    } else {
        *counter -= 1;
        gear
    }
}

/// The shared automatic walk; returns the gear it settles on.
fn walk(
    start: i32,
    engine: i32,
    tables: &GearboxTables,
    gear_two_skip: bool,
    counter: &mut i8,
) -> Result<i32, GearOutOfRange> {
    #[derive(PartialEq)]
    enum Shifted {
        None,
        Down,
        Up,
    }
    let mut gear = start;
    let mut rpm = engine;
    let base = qmul(engine, entry(&tables.inverse_1f0, gear)?);
    let mut shifted = Shifted::None;
    loop {
        let lower = qmul(entry(&tables.lower_1b0, gear)?, tables.redline_15c);
        let upper = qmul(entry(&tables.upper_190, gear)?, tables.redline_15c);
        if rpm > lower && rpm < upper {
            return Ok(gear);
        }
        if rpm <= lower {
            if !(2..=6).contains(&gear) || shifted == Shifted::Up {
                return Ok(gear);
            }
            gear = apply_gear_two_skip(gear - 1, 1, gear_two_skip, counter);
            shifted = Shifted::Down;
        } else {
            if !(1..=5).contains(&gear) || shifted == Shifted::Down {
                return Ok(gear);
            }
            gear = apply_gear_two_skip(gear + 1, 3, gear_two_skip, counter);
            shifted = Shifted::Up;
        }
        rpm = qmul(base, entry(&tables.ratio_1d0, gear)?);
    }
}

/// The automatic walk plus its publication gate (`FUN_0041b810` and the shared
/// head of `FUN_0041bf00`).
fn automatic(
    state: &mut GearboxState,
    inputs: GearboxInputs,
    tables: &GearboxTables,
) -> Result<(), GearOutOfRange> {
    let grounded = inputs.wheel_flags_73a.contains(&0);
    if state.cooldown_441 > 0 {
        state.cooldown_441 -= 1;
    }
    if !(inputs.throttle_304 != 0 || inputs.brake_308 != 0) || !grounded {
        return Ok(());
    }
    let current = i32::from(state.gear_43a);
    let gear = walk(
        current,
        inputs.engine_2e0,
        tables,
        inputs.gear_two_skip,
        &mut state.skip_443,
    )?;
    if gear == current {
        return Ok(());
    }
    if state.cooldown_441 != 0
        && ((state.last_direction_442 == 2 && gear < current)
            || (state.last_direction_442 == 1 && gear > current))
    {
        return Ok(());
    }
    state.shift_40c = true;
    state.last_direction_442 = if gear > current { 2 } else { 1 };
    state.cooldown_441 = SHIFT_COOLDOWN;
    state.target_43f = gear as i8;
    Ok(())
}

/// The manual step `FUN_0041bb30`.
fn manual(state: &mut GearboxState, inputs: GearboxInputs) {
    let gear = state.gear_43a;
    if inputs.gear_command_314 == 1 {
        if gear < 6 {
            let target = apply_gear_two_skip(
                i32::from(gear) + 1,
                3,
                inputs.gear_two_skip,
                &mut state.skip_443,
            );
            state.target_43f = target as i8;
            state.shift_40c = true;
        } else if gear == REVERSE_GEAR {
            state.target_43f = 1;
            state.shift_40c = true;
        }
    } else if inputs.gear_command_314 == -1 && gear < REVERSE_GEAR {
        let target = apply_gear_two_skip(
            i32::from(gear) - 1,
            1,
            inputs.gear_two_skip,
            &mut state.skip_443,
        );
        state.target_43f = if target == 0 {
            REVERSE_GEAR
        } else {
            target as i8
        };
        state.shift_40c = true;
    }
}

/// `FUN_0041bc90`: rerun the walk on the unsigned gear and publish whether it
/// would not shift down. The walk's gear-2 counter writes are kept.
fn manual_hint(
    state: &mut GearboxState,
    inputs: GearboxInputs,
    tables: &GearboxTables,
) -> Result<(), GearOutOfRange> {
    let gear = i32::from(state.gear_43a as u8);
    let settled = walk(
        gear,
        inputs.engine_2e0,
        tables,
        inputs.gear_two_skip,
        &mut state.skip_443,
    )?;
    state.upshift_hint_420 = settled >= i32::from(state.gear_43a);
    Ok(())
}

/// Apply the gearbox part of `FUN_0041b5b0`.
///
/// # Errors
///
/// Returns [`GearOutOfRange`], leaving the state unchanged, when a walk would
/// read a table entry outside `0..=7` (for example gear `-1` after a manual
/// down-shift from neutral); the shipped code would read neighbouring control
/// words instead.
pub fn apply_gearbox(
    state: GearboxState,
    inputs: GearboxInputs,
    tables: &GearboxTables,
) -> Result<GearboxState, GearOutOfRange> {
    let mut next = state;
    if !inputs.player_locked && inputs.residual_43b == 0 {
        if next.shift_40c {
            next.shift_40c = false;
            next.gear_43a = next.target_43f;
        } else {
            match inputs.mode_3f0 {
                0 => {
                    automatic(&mut next, inputs, tables)?;
                    if inputs.gear_command_314 != 0 {
                        if next.gear_43a == REVERSE_GEAR {
                            next.target_43f = 1;
                            next.shift_40c = true;
                        } else if next.gear_43a == 1 {
                            next.target_43f = REVERSE_GEAR;
                            next.shift_40c = true;
                        }
                    }
                }
                1 if inputs.gear_command_314 != 0 => manual(&mut next, inputs),
                2 => {
                    automatic(&mut next, inputs, tables)?;
                    brake_reverse(&mut next, inputs);
                }
                _ => {}
            }
        }
    }
    if inputs.mode_3f0 == 1 {
        manual_hint(&mut next, inputs, tables)?;
    }
    Ok(next)
}

/// Mode 2's brake-to-reverse tail (`FUN_0041bf00` after the walk).
fn brake_reverse(state: &mut GearboxState, inputs: GearboxInputs) {
    if !inputs.brake_reverse_enabled {
        return;
    }
    let throttle = inputs.throttle_304 != 0;
    let brake = inputs.brake_308 != 0;
    if brake && state.gear_43a == 1 && !throttle {
        if inputs.engine_2e0 < BRAKE_REVERSE_ENGINE_LIMIT {
            state.target_43f = REVERSE_GEAR;
            state.shift_40c = true;
            state.reverse_mode_408 = true;
        }
        return;
    }
    if throttle && !brake && state.gear_43a == REVERSE_GEAR {
        state.target_43f = 1;
        state.shift_40c = true;
        state.reverse_mode_408 = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every forward gear holds engine speeds in `(1.0, 3.0)` (redline 1.0);
    /// gear `g` has ratio `8 - g` and inverse `1 / (8 - g)`, so a downshift
    /// raises the engine speed and an upshift lowers it.
    fn tables() -> GearboxTables {
        let mut tables = GearboxTables {
            redline_15c: 0x1_0000,
            ..GearboxTables::default()
        };
        for gear in 1..=6 {
            let ratio = 8 - gear as i32;
            tables.lower_1b0[gear] = 0x1_0000;
            tables.upper_190[gear] = 0x3_0000;
            tables.ratio_1d0[gear] = ratio * 0x1_0000;
            tables.inverse_1f0[gear] = 0x1_0000 / ratio;
        }
        tables
    }

    fn driving(engine: i32) -> GearboxInputs {
        GearboxInputs {
            throttle_304: 1,
            engine_2e0: engine,
            ..GearboxInputs::default()
        }
    }

    fn in_gear(gear: i8) -> GearboxState {
        GearboxState {
            gear_43a: gear,
            target_43f: gear,
            ..GearboxState::default()
        }
    }

    #[test]
    fn automatic_upshift_requests_then_applies_on_the_next_call() {
        // 4.0 in second: base 4/6, third gives 3.33 (still high), fourth 2.67.
        let state = apply_gearbox(in_gear(2), driving(0x4_0000), &tables()).unwrap();
        assert!(state.shift_40c);
        assert_eq!(state.target_43f, 4);
        assert_eq!(state.gear_43a, 2);
        assert_eq!((state.last_direction_442, state.cooldown_441), (2, 10));
        let applied = apply_gearbox(state, driving(0x4_0000), &tables()).unwrap();
        assert!(!applied.shift_40c);
        assert_eq!(applied.gear_43a, state.target_43f);
        // Applying a shift does not touch the cooldown.
        assert_eq!(applied.cooldown_441, 10);
    }

    #[test]
    fn in_band_engine_speed_keeps_the_gear() {
        let state = apply_gearbox(in_gear(3), driving(0x2_0000), &tables()).unwrap();
        assert!(!state.shift_40c);
        assert_eq!(state.gear_43a, 3);
    }

    #[test]
    fn automatic_downshift_and_its_bounds() {
        // 0.875 in fourth: base 0.21875, third gives 1.09, inside the band.
        let state = apply_gearbox(in_gear(4), driving(0xe000), &tables()).unwrap();
        assert!(state.shift_40c);
        assert_eq!(state.target_43f, 3);
        assert_eq!(state.last_direction_442, 1);
        // First, neutral and reverse never shift down automatically.
        for gear in [0, 1, REVERSE_GEAR] {
            let held = apply_gearbox(in_gear(gear), driving(0), &tables()).unwrap();
            assert!(!held.shift_40c, "gear {gear}");
        }
        // Sixth never shifts up.
        let top = apply_gearbox(in_gear(6), driving(0x40_0000), &tables()).unwrap();
        assert!(!top.shift_40c);
    }

    #[test]
    fn the_walk_needs_a_pedal_and_a_grounded_wheel_but_the_cooldown_still_ticks() {
        let cooling = GearboxState {
            cooldown_441: 3,
            ..in_gear(2)
        };
        let coasting = GearboxInputs {
            throttle_304: 0,
            ..driving(0x4_0000)
        };
        let next = apply_gearbox(cooling, coasting, &tables()).unwrap();
        assert!(!next.shift_40c);
        assert_eq!(next.cooldown_441, 2);
        let airborne = GearboxInputs {
            wheel_flags_73a: [1; 4],
            ..driving(0x4_0000)
        };
        assert!(
            !apply_gearbox(in_gear(2), airborne, &tables())
                .unwrap()
                .shift_40c
        );
        let one_down = GearboxInputs {
            wheel_flags_73a: [1, 1, 0, 1],
            ..driving(0x4_0000)
        };
        assert!(
            apply_gearbox(in_gear(2), one_down, &tables())
                .unwrap()
                .shift_40c
        );
    }

    #[test]
    fn cooldown_blocks_only_a_reversal_of_the_last_direction() {
        let just_upshifted = GearboxState {
            cooldown_441: 5,
            last_direction_442: 2,
            ..in_gear(4)
        };
        let blocked = apply_gearbox(just_upshifted, driving(0xe000), &tables()).unwrap();
        assert!(!blocked.shift_40c);
        assert_eq!(blocked.cooldown_441, 4);
        // Another upshift is allowed: 8.0 in fourth climbs to the sixth bound.
        let again = apply_gearbox(just_upshifted, driving(0x8_0000), &tables()).unwrap();
        assert!(again.shift_40c);
        assert_eq!(again.target_43f, 6);
    }

    #[test]
    fn locks_and_the_residual_countdown_freeze_the_gearbox() {
        let pending = GearboxState {
            shift_40c: true,
            target_43f: 3,
            ..in_gear(2)
        };
        for inputs in [
            GearboxInputs {
                player_locked: true,
                ..driving(0)
            },
            GearboxInputs {
                residual_43b: 1,
                ..driving(0)
            },
        ] {
            assert_eq!(apply_gearbox(pending, inputs, &tables()).unwrap(), pending);
        }
    }

    #[test]
    fn automatic_gear_command_toggles_first_and_reverse() {
        let toggle = GearboxInputs {
            gear_command_314: 1,
            throttle_304: 0,
            ..GearboxInputs::default()
        };
        let reverse = apply_gearbox(in_gear(1), toggle, &tables()).unwrap();
        assert_eq!(
            (reverse.target_43f, reverse.shift_40c),
            (REVERSE_GEAR, true)
        );
        let forward = apply_gearbox(in_gear(REVERSE_GEAR), toggle, &tables()).unwrap();
        assert_eq!((forward.target_43f, forward.shift_40c), (1, true));
        assert!(
            !apply_gearbox(in_gear(3), toggle, &tables())
                .unwrap()
                .shift_40c
        );
    }

    #[test]
    fn gear_two_skip_alternates_with_its_counter() {
        let inputs = GearboxInputs {
            gear_two_skip: true,
            ..driving(0x1_0000)
        };
        // Down from 3 at 1.0 lands on 2; the counter at 0 skips to 1.
        let skipped = apply_gearbox(in_gear(3), inputs, &tables()).unwrap();
        assert_eq!(skipped.target_43f, 1);
        assert_eq!(skipped.skip_443, 5);
        // With the counter loaded, the walk stays on 2 and decrements it.
        let counted = apply_gearbox(
            GearboxState {
                skip_443: 5,
                ..in_gear(3)
            },
            inputs,
            &tables(),
        )
        .unwrap();
        assert_eq!(counted.target_43f, 2);
        assert_eq!(counted.skip_443, 4);
    }

    #[test]
    fn manual_steps_wrap_through_reverse() {
        let up = GearboxInputs {
            mode_3f0: 1,
            gear_command_314: 1,
            ..GearboxInputs::default()
        };
        let down = GearboxInputs {
            gear_command_314: -1,
            ..up
        };
        assert_eq!(
            apply_gearbox(in_gear(3), up, &tables()).unwrap().target_43f,
            4
        );
        assert!(!apply_gearbox(in_gear(6), up, &tables()).unwrap().shift_40c);
        assert_eq!(
            apply_gearbox(in_gear(REVERSE_GEAR), up, &tables())
                .unwrap()
                .target_43f,
            1
        );
        assert_eq!(
            apply_gearbox(in_gear(1), down, &tables())
                .unwrap()
                .target_43f,
            REVERSE_GEAR
        );
        assert!(
            !apply_gearbox(in_gear(REVERSE_GEAR), down, &tables())
                .unwrap()
                .shift_40c
        );
        // Without a command the manual mode only refreshes its hint.
        let idle = GearboxInputs {
            gear_command_314: 0,
            engine_2e0: 0x8_0000,
            ..up
        };
        let hinted = apply_gearbox(in_gear(3), idle, &tables()).unwrap();
        assert!(!hinted.shift_40c);
        assert!(hinted.upshift_hint_420);
        let low = GearboxInputs {
            engine_2e0: 0x1_0000,
            ..idle
        };
        assert_eq!(walk(3, 0x1_0000, &tables(), false, &mut 0), Ok(2));
        assert!(
            !apply_gearbox(in_gear(3), low, &tables())
                .unwrap()
                .upshift_hint_420
        );
    }

    #[test]
    fn mode_two_brakes_into_reverse_and_throttles_back_out() {
        let braking = GearboxInputs {
            mode_3f0: 2,
            throttle_304: 0,
            brake_308: 1,
            engine_2e0: 0x1998,
            wheel_flags_73a: [1; 4],
            brake_reverse_enabled: true,
            ..GearboxInputs::default()
        };
        let reversing = apply_gearbox(in_gear(1), braking, &tables()).unwrap();
        assert_eq!(
            (
                reversing.target_43f,
                reversing.shift_40c,
                reversing.reverse_mode_408
            ),
            (REVERSE_GEAR, true, true)
        );
        let fast = GearboxInputs {
            engine_2e0: 0x1999,
            ..braking
        };
        assert!(
            !apply_gearbox(in_gear(1), fast, &tables())
                .unwrap()
                .shift_40c
        );
        let throttle = GearboxInputs {
            throttle_304: 1,
            brake_308: 0,
            ..braking
        };
        let forward = apply_gearbox(
            GearboxState {
                reverse_mode_408: true,
                ..in_gear(REVERSE_GEAR)
            },
            throttle,
            &tables(),
        )
        .unwrap();
        assert_eq!(
            (
                forward.target_43f,
                forward.shift_40c,
                forward.reverse_mode_408
            ),
            (1, true, false)
        );
        let gated = GearboxInputs {
            brake_reverse_enabled: false,
            ..braking
        };
        assert!(
            !apply_gearbox(in_gear(1), gated, &tables())
                .unwrap()
                .shift_40c
        );
    }

    #[test]
    fn a_gear_outside_the_tables_is_reported() {
        assert_eq!(
            apply_gearbox(in_gear(-1), driving(0x1_0000), &tables()),
            Err(GearOutOfRange { gear: -1 })
        );
        let manual = GearboxInputs {
            mode_3f0: 1,
            ..GearboxInputs::default()
        };
        assert_eq!(
            apply_gearbox(in_gear(-1), manual, &tables()),
            Err(GearOutOfRange { gear: 0xff })
        );
    }

    #[test]
    fn the_car_setup_picks_manual_or_automatic_with_reverse() {
        assert_eq!(setup_mode(0x16), 1);
        assert_eq!(setup_mode(0x06), 2);
        assert_eq!(setup_mode(0), 2);
    }
}
