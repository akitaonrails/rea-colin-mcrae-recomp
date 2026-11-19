//! Recovered force-feedback (FFB) effect semantics (item H08).
//!
//! # Provenance
//!
//! The Windows release drives force feedback through **DirectInput**, and that
//! is the sole reason `DINPUT.dll` / `DirectInputCreateA` are imported: in-race
//! *input* is WinMM (`joyGetPosEx`) + `GetAsyncKeyState` (see
//! [`crate::input_device`]), while DirectInput is instantiated only as the
//! force-feedback *output* path. The device is brought up by `FUN_004aba70`
//! (`DirectInputCreateA` -> `EnumDevices(DIDEVTYPE_JOYSTICK)` -> `CreateDevice`
//! -> `SetDataFormat(c_dfDIJoystick)` -> `SetCooperativeLevel(EXCLUSIVE|
//! BACKGROUND)` -> `Acquire`), stored by the bring-up `FUN_004ab920` behind the
//! device-present flag `DAT_00bb95a8`.
//!
//! Six effects are created once (`FUN_00432ce0`) into the effect-object array
//! `DAT_00bb9578[]` and thereafter only updated per frame via
//! `IDirectInputEffect::SetParameters`. The per-car effect-parameter block is
//! `DAT_00550040` (stride `0x88`), double-buffered (`FUN_004335d0` snapshots
//! current -> previous) so the per-frame commit `FUN_00433620` issues a
//! `SetParameters` call only for the fields that changed.
//!
//! The `DIEFFECT` size observed on the stack (`0x38`) is the **DirectX 7** form,
//! `DIENVELOPE` is `0x14`, `DICONDITION` is `0x18`.
//!
//! ## Separation of observation, inference, and design decision
//!
//! - **Observations** (literal pseudocode): the `0x2710` magnitude rescale, the
//!   `0x10000` game-magnitude clamp, the digital-wheel direction snaps
//!   `9000`/`27000`, the collision peak+decay-step accumulator and its clamp at
//!   `0`, the rumble sample-period floor `1000`, the self-centring `/2` clamp,
//!   the cornering pre-shift clamp `0x27100000`, the per-player FFB-enable bit
//!   `0x08` at config stride `0x9c`, the effect slot/kind table, the condition
//!   selector (`0xb`..`0xe`), and the DirectInput `DIEP_*` / `DIEFF_*` /
//!   cooperative-level ABI constants (these are public DirectInput SDK values,
//!   not game data).
//! - **Inference**: the continuous (analog) collision direction includes a
//!   polar-angle conversion to DirectInput centidegrees. The recovered
//!   signed-ratio phase uses `0x1000` per full period (`0x400` per quarter),
//!   but its force-feedback consumer chain is not yet pinned; therefore
//!   [`polar_centidegrees`] remains a generic conversion with only boundary
//!   behavior asserted.
//! - **Design decision**: the collision decay *step* source and the per-source
//!   raw-sum derivations are left as caller-supplied inputs; this module
//!   reconstructs the confidently-recovered *transforms*, not the unrecovered
//!   force-production wiring (the G02/G03 unknowns feed the raw sums here).
//!
//! No game data is vendored: the predefined-effect GUIDs are DirectInput SDK
//! constants and are represented only as effect *kinds*; the per-surface rumble
//! table values (`DAT_004edb70`) are runtime-loaded game data and are not copied.

/// DirectInput ABI constants (public DirectInput SDK values, reproduced for the
/// compatibility layer; not game data).
pub mod di {
    /// `DISCL_EXCLUSIVE`.
    pub const COOP_EXCLUSIVE: u32 = 0x1;
    /// `DISCL_BACKGROUND`.
    pub const COOP_BACKGROUND: u32 = 0x8;
    /// `SetCooperativeLevel` argument observed at `FUN_004aba70`
    /// (`EXCLUSIVE | BACKGROUND`).
    pub const COOP_LEVEL: u32 = COOP_EXCLUSIVE | COOP_BACKGROUND;

    /// `DIDEVTYPE_JOYSTICK`, the `EnumDevices` filter.
    pub const DEVTYPE_JOYSTICK: u32 = 4;
    /// `DISFFC_STOPALL`, the `SendForceFeedbackCommand` argument at bring-up.
    pub const DISFFC_STOPALL: u32 = 2;

    /// `DIEFF_OBJECTOFFSETS`.
    pub const DIEFF_OBJECTOFFSETS: u32 = 0x2;
    /// `DIEFF_POLAR`.
    pub const DIEFF_POLAR: u32 = 0x20;
    /// `DIEFF_CARTESIAN`.
    pub const DIEFF_CARTESIAN: u32 = 0x10;

    /// `DIEP_SAMPLEPERIOD`.
    pub const DIEP_SAMPLEPERIOD: u32 = 0x2;
    /// `DIEP_GAIN`.
    pub const DIEP_GAIN: u32 = 0x4;
    /// `DIEP_DIRECTION`.
    pub const DIEP_DIRECTION: u32 = 0x40;
    /// `DIEP_TYPESPECIFICPARAMS`.
    pub const DIEP_TYPESPECIFICPARAMS: u32 = 0x100;

    /// `DIEFFECT` size for the DirectX 7 form seen on the stack.
    pub const DIEFFECT_DX7_SIZE: u32 = 0x38;
    /// `DIENVELOPE` size.
    pub const DIENVELOPE_SIZE: u32 = 0x14;
    /// `DICONDITION` size.
    pub const DICONDITION_SIZE: u32 = 0x18;

    /// `DI_FFNOMINALMAX`: the nominal maximum for gain, magnitude, and
    /// saturation in DirectInput units.
    pub const FF_NOMINAL_MAX: i32 = 10000;

    /// `E_INVALIDARG`, returned by every effect wrapper when no FFB device is
    /// present (the whole chain becomes a silent no-op).
    pub const E_INVALIDARG: u32 = 0x8007_0057;
}

/// Exact parameter block passed by `FUN_004abe70` to
/// `IDirectInputEffect::SetParameters`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplePeriodUpdate {
    pub effect_size: u32,
    pub sample_period: u32,
    pub set_flags: u32,
}

#[must_use]
pub fn sample_period_update(sample_period: u32) -> SamplePeriodUpdate {
    SamplePeriodUpdate {
        effect_size: di::DIEFFECT_DX7_SIZE,
        sample_period,
        set_flags: di::DIEP_SAMPLEPERIOD,
    }
}

/// Exact effect/setup block passed by `FUN_004abed0` for a periodic gain update.
/// The two-byte type-specific payload is preserved as observed, not expanded
/// into an assumed SDK `DIPERIODIC` structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeriodicGainUpdate {
    pub effect_size: u32,
    pub effect_flags: u32,
    pub gain: u32,
    pub type_specific_size: u32,
    pub type_specific_first: u32,
    pub set_flags: u32,
}

#[must_use]
pub fn periodic_gain_update(gain: u32, type_specific_first: u32) -> PeriodicGainUpdate {
    PeriodicGainUpdate {
        effect_size: di::DIEFFECT_DX7_SIZE,
        effect_flags: di::DIEFF_POLAR | di::DIEFF_OBJECTOFFSETS,
        gain,
        type_specific_size: 2,
        type_specific_first,
        set_flags: di::DIEP_GAIN | di::DIEP_DIRECTION,
    }
}

/// Game-internal magnitude ceiling: `1.0` in Q16.16 (impacts, grip, and rumble
/// magnitudes are clamped to this before being rescaled for DirectInput).
pub const GAME_MAGNITUDE_MAX: i32 = 0x1_0000;

/// Multiplier the game folds into the magnitude normalisation to map a
/// `0..=0x10000` Q16.16 magnitude onto DirectInput's `0..=10000` range:
/// `value * 0x2710 >> 16` (`0x2710 == 10000 == DI_FFNOMINALMAX`).
pub const DI_MAGNITUDE_SCALE: i32 = 0x2710;

/// A full circle expressed in DirectInput polar centidegrees (`360 * 100`).
pub const DI_FULL_CIRCLE_CENTIDEGREES: i32 = 36000;

/// Digital-wheel collision direction "push right" (90 degrees).
pub const DI_DIRECTION_RIGHT_CENTIDEGREES: i32 = 9000;

/// Digital-wheel collision direction "push left" (270 degrees).
pub const DI_DIRECTION_LEFT_CENTIDEGREES: i32 = 27000;

/// Full-period unit count of the recovered signed-ratio phase.
///
/// `FUN_0048e100` is a three-component Q16 transform, not the phase lookup.
/// How this phase reaches the force-feedback centidegree consumer remains
/// unresolved, so this constant states only the recovered phase scale.
pub const SIGNED_RATIO_PHASE_FULL_PERIOD: i32 = 0x1000;

/// Minimum sample period (microseconds) the surface-rumble computer
/// (`FUN_00432e00`) allows before flooring; a rougher surface shortens the
/// period down to this floor.
pub const RUMBLE_PERIOD_FLOOR_US: i32 = 1000;

/// Pre-shift ceiling the cornering/lateral computer (`FUN_00433b30`) clamps its
/// scaled magnitude to; `>> 16` of this is exactly [`di::FF_NOMINAL_MAX`].
pub const CORNERING_SCALED_MAX: i32 = 0x2710_0000;

/// Per-player config stride inside the option struct `DAT_00d3ca80`.
pub const PER_PLAYER_CONFIG_STRIDE: usize = 0x9c;

/// Offset of the per-player FFB option byte within one player's config record.
pub const PER_PLAYER_CONFIG_OPTION_OFFSET: usize = 0x3c;

/// Bit of the per-player config option byte that enables force feedback
/// (`*(DAT_00d3ca80 + 0x3c + player*0x9c) & 8`).
pub const FFB_ENABLE_BIT: u8 = 0x08;

/// The predefined DirectInput effect kinds the game creates. Represented as
/// kinds rather than GUIDs; the GUIDs are DirectInput SDK constants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EffectKind {
    /// Periodic sine (buzz / rumble).
    PeriodicSine,
    /// Constant force (directional kick / lateral load).
    ConstantForce,
    /// Condition: spring (centring).
    ConditionSpring,
    /// Condition: damper (road feel / self-centring resistance).
    ConditionDamper,
}

/// Number of effect object slots the game creates once at reset
/// (`FUN_00432ce0`).
pub const EFFECT_SLOT_COUNT: usize = 6;

/// The effect kind created into each slot, in creation order
/// (`DAT_00bb9578[slot]`). Returns `None` for an out-of-range slot.
pub fn effect_slot_kind(slot: usize) -> Option<EffectKind> {
    Some(match slot {
        0 => EffectKind::PeriodicSine,
        1 => EffectKind::ConstantForce,
        2 => EffectKind::ConditionSpring,
        3 => EffectKind::ConditionDamper,
        4 => EffectKind::ConstantForce,
        5 => EffectKind::PeriodicSine,
        _ => return None,
    })
}

/// DirectInput `DIEFFECT.dwFlags` the game passes to `CreateEffect` for a given
/// effect kind: polar/object-offsets for periodic and constant force,
/// cartesian/object-offsets for conditions.
pub fn effect_create_flags(kind: EffectKind) -> u32 {
    match kind {
        EffectKind::PeriodicSine | EffectKind::ConstantForce => {
            di::DIEFF_POLAR | di::DIEFF_OBJECTOFFSETS
        }
        EffectKind::ConditionSpring | EffectKind::ConditionDamper => {
            di::DIEFF_CARTESIAN | di::DIEFF_OBJECTOFFSETS
        }
    }
}

/// `DIEFFECT.cAxes` for a given effect kind: 2 for polar periodic/constant
/// effects, 1 for a single-axis condition.
pub fn effect_axis_count(kind: EffectKind) -> u32 {
    match kind {
        EffectKind::PeriodicSine | EffectKind::ConstantForce => 2,
        EffectKind::ConditionSpring | EffectKind::ConditionDamper => 1,
    }
}

/// The condition type selected by the condition-creation helper
/// (`FUN_004ac8e0`): `0xb`=spring, `0xc`=inertia, `0xd`=damper, `0xe`=friction.
/// Returns `None` for an unrecognised selector.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConditionType {
    /// `0xb` -> `GUID_Spring`.
    Spring,
    /// `0xc` -> `GUID_Inertia`.
    Inertia,
    /// `0xd` -> `GUID_Damper`.
    Damper,
    /// `0xe` -> `GUID_Friction`.
    Friction,
}

/// Decode the condition-type selector argument of `FUN_004ac8e0`.
pub fn condition_type(selector: u32) -> Option<ConditionType> {
    Some(match selector {
        0xb => ConditionType::Spring,
        0xc => ConditionType::Inertia,
        0xd => ConditionType::Damper,
        0xe => ConditionType::Friction,
        _ => return None,
    })
}

/// Rescale a game-internal Q16.16 magnitude (`0..=0x10000`) into a DirectInput
/// gain/magnitude in `0..=10000` via the recovered `value * 0x2710 >> 16`,
/// clamping the input to `[0, 0x10000]` first (so the collision impact
/// `0..=0x10000` from [`crate::collision`] maps to a constant-force gain).
pub fn scale_magnitude(game_magnitude: i32) -> i32 {
    let clamped = game_magnitude.clamp(0, GAME_MAGNITUDE_MAX);
    ((clamped as i64 * DI_MAGNITUDE_SCALE as i64) >> 16) as i32
}

/// Clamp and shift a cornering/lateral pre-shift magnitude (`FUN_00433b30`):
/// the scaled value is clamped to `0x27100000` then `>> 16`, yielding a gain in
/// `0..=10000`.
pub fn clamp_cornering_gain(scaled_magnitude: i32) -> i32 {
    scaled_magnitude.clamp(0, CORNERING_SCALED_MAX) >> 16
}

/// Self-centring / road-feel condition gain (`FUN_00433800`): the accumulated
/// raw sum is halved then clamped to `0..=10000`.
pub fn self_centering_gain(raw_sum: i32) -> i32 {
    (raw_sum / 2).clamp(0, di::FF_NOMINAL_MAX)
}

/// Surface-rumble sample period (`FUN_00432e00`): a rougher surface shortens the
/// period, floored at [`RUMBLE_PERIOD_FLOOR_US`].
pub fn rumble_period_us(raw_period_us: i32) -> i32 {
    raw_period_us.max(RUMBLE_PERIOD_FLOOR_US)
}

/// Digital/2-button-wheel collision direction snap (`FUN_004331b0` when the
/// per-player device marker reports a digital device): left -> 270 degrees,
/// right -> 90 degrees, in DirectInput polar centidegrees.
pub fn digital_collision_direction(push_left: bool) -> i32 {
    if push_left {
        DI_DIRECTION_LEFT_CENTIDEGREES
    } else {
        DI_DIRECTION_RIGHT_CENTIDEGREES
    }
}

/// Convert an angle in caller-supplied full-period units into DirectInput polar
/// centidegrees, wrapped into `[0, 36000)`.
///
/// Inference: this models the analog collision direction conversion; only the
/// boundary mapping (0 -> 0, quarter turn -> 9000, full turn -> wrap to 0) is
/// asserted because the exact force-feedback consumer chain is not yet pinned.
pub fn polar_centidegrees(angle: i32, full_circle_units: i32) -> i32 {
    debug_assert!(full_circle_units > 0);
    let wrapped = angle.rem_euclid(full_circle_units);
    ((wrapped as i64 * DI_FULL_CIRCLE_CENTIDEGREES as i64) / full_circle_units as i64) as i32
}

/// A decaying collision force accumulator (`FUN_004330c0`/`FUN_004331b0` seed it,
/// `FUN_00433ac0` plays then decays it each frame): the current `magnitude` is
/// emitted as the live effect gain, then reduced by `step`, clamped at `0`, so a
/// collision produces a kick that ramps down over several frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecayAccumulator {
    /// Current DirectInput gain (`0..=10000`), emitted this frame.
    pub magnitude: i32,
    /// Per-frame decay step subtracted after emitting.
    pub step: i32,
}

impl DecayAccumulator {
    /// Seed from a game-internal Q16.16 impact magnitude (rescaled to DI units)
    /// and an explicit per-frame decay step, for callers that override the
    /// recovered step (for example a non-collision source).
    pub fn seed(game_impact: i32, step: i32) -> Self {
        Self {
            magnitude: scale_magnitude(game_impact),
            step: step.max(0),
        }
    }

    /// Seed a collision accumulator with the recovered `FUN_004330c0` step
    /// formula ([`collision_decay_step`]).
    pub fn seed_collision(game_impact: i32, decay_scale_q16: i32, long_effect: bool) -> Self {
        Self {
            magnitude: collision_peak(game_impact),
            step: collision_decay_step(game_impact, decay_scale_q16, long_effect),
        }
    }

    /// Advance one frame: return the gain emitted this frame, then decay toward
    /// zero (clamped at `0`). Matches `FUN_00433ac0`'s copy-then-decay order.
    pub fn tick(&mut self) -> i32 {
        let emitted = self.magnitude;
        self.magnitude = (self.magnitude - self.step).max(0);
        emitted
    }

    /// Whether the accumulator has fully decayed (no further force to emit).
    pub fn is_idle(&self) -> bool {
        self.magnitude <= 0
    }
}

/// A `DICONDITION` as the game fills it for the centring/damper effects
/// (`FUN_004ac840`): symmetric positive/negative coefficients, full saturation,
/// no offset, no dead band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition {
    /// `lOffset`.
    pub offset: i32,
    /// `lPositiveCoefficient`.
    pub positive_coefficient: i32,
    /// `lNegativeCoefficient`.
    pub negative_coefficient: i32,
    /// `dwPositiveSaturation`.
    pub positive_saturation: u32,
    /// `dwNegativeSaturation`.
    pub negative_saturation: u32,
    /// `lDeadBand`.
    pub dead_band: i32,
}

/// Build a `DICONDITION` for a centring/damper coefficient exactly as
/// `FUN_004ac840` does: offset 0, symmetric coefficients, both saturations at
/// `DI_FFNOMINALMAX`, dead band 0.
pub fn centering_condition(coefficient: i32) -> Condition {
    Condition {
        offset: 0,
        positive_coefficient: coefficient,
        negative_coefficient: coefficient,
        positive_saturation: di::FF_NOMINAL_MAX as u32,
        negative_saturation: di::FF_NOMINAL_MAX as u32,
        dead_band: 0,
    }
}

/// `DIEP_*` flag set for a periodic (sine/rumble) effect update, pushing only
/// the fields that changed (`FUN_00433620` -> `FUN_004abf50`/`004abd30`):
/// sample period and/or gain.
pub fn periodic_update_flags(gain_changed: bool, period_changed: bool) -> u32 {
    let mut flags = 0;
    if gain_changed {
        flags |= di::DIEP_GAIN;
    }
    if period_changed {
        flags |= di::DIEP_SAMPLEPERIOD;
    }
    flags
}

/// `DIEP_*` flag set for a constant-force effect update, pushing only the
/// fields that changed (`FUN_00433620` -> `FUN_004abed0`/`004abd90`/`004abd30`):
/// gain and/or direction (both -> `0x44`).
pub fn constant_force_update_flags(gain_changed: bool, direction_changed: bool) -> u32 {
    let mut flags = 0;
    if gain_changed {
        flags |= di::DIEP_GAIN;
    }
    if direction_changed {
        flags |= di::DIEP_DIRECTION;
    }
    flags
}

/// `DIEP_*` flag set for a condition effect update (`FUN_004ac840`): the whole
/// condition block is type-specific params.
pub fn condition_update_flags(coefficient_changed: bool) -> u32 {
    if coefficient_changed {
        di::DIEP_TYPESPECIFICPARAMS
    } else {
        0
    }
}

/// Speed (`0xa3e` ~ 0.04 Q16.16) at or below which self-centring is at full
/// strength (`FUN_00433800`).
pub const SELF_CENTERING_REST_SPEED_Q16: i32 = 0xa3e;

/// Speed (`0x3333` ~ 0.2 Q16.16) at or above which self-centring is zero.
pub const SELF_CENTERING_ZERO_SPEED_Q16: i32 = 0x3333;

/// Ramp gain of the self-centring speed curve (`0x63ff6` ~ 6.25 Q16.16).
pub const SELF_CENTERING_RAMP_Q16: i64 = 0x63_ff6;

/// The self-centring speed curve of `FUN_00433800`, in DirectInput units:
/// full `10000` at or below [`SELF_CENTERING_REST_SPEED_Q16`], a linear-ish
/// ramp down to `0` at [`SELF_CENTERING_ZERO_SPEED_Q16`], and `0` above it.
///
/// The shipped dispatcher only evaluates this when a front wheel is in contact;
/// the per-wheel lateral term that is added afterwards comes from the runtime
/// telemetry and surface tables.
#[must_use]
pub fn self_centering_speed_gain(speed_q16: i32) -> i32 {
    if speed_q16 < SELF_CENTERING_REST_SPEED_Q16 {
        return di::FF_NOMINAL_MAX;
    }
    if speed_q16 >= SELF_CENTERING_ZERO_SPEED_Q16 {
        return 0;
    }
    let ramp = ((i64::from(speed_q16 - (SELF_CENTERING_REST_SPEED_Q16 - 1))
        * SELF_CENTERING_RAMP_Q16)
        >> 16)
        .min(i64::from(GAME_MAGNITUDE_MAX));
    i32::try_from(((i64::from(GAME_MAGNITUDE_MAX) - ramp) * 0x2710_0000) >> 32).unwrap_or(0)
}

/// Whether force feedback is enabled for a player, given that player's config
/// option byte (`*(DAT_00d3ca80 + 0x3c + player*0x9c) & 8`).
pub fn ffb_enabled(player_config_option: u8) -> bool {
    player_config_option & FFB_ENABLE_BIT != 0
}

/// Collision effect duration constant, `25.0` in Q16.16 (`0x190000`). The
/// shipped decay-step computation multiplies it by [`COLLISION_DURATION_GAIN`]
/// and doubles it when the long-effect flag (`DAT_00551af0`) is set.
pub const COLLISION_DURATION_Q16: i64 = 0x19_0000;

/// Duration gain folded into the collision step (`0x3333` ~ 0.2 Q16.16), so the
/// recovered duration is `25.0 * 0.2 = 5.0` (10.0 in long-effect mode) in Q16.16.
pub const COLLISION_DURATION_GAIN: i64 = 0x3333;

/// Numeric scale of the collision period effect for the peak magnitude
/// (`DAT_00551af0` doubles the duration, halving the per-frame step).
#[must_use]
pub fn collision_peak(game_magnitude: i32) -> i32 {
    scale_magnitude(game_magnitude)
}

/// The decay step `FUN_004330c0` seeds into `DecayAccumulator::step`, recovered
/// as a formula (it is no longer a caller-supplied unknown):
///
/// ```text
/// duration_q16 = (long_effect ? 2 : 1) * 0x190000 * 0x3333 >> 16   // 5.0 / 10.0
/// step = (0x2710 << 32) / duration_q16 * ((magnitude * scale) >> 16) >> 32
/// ```
///
/// with the result floored at `1`. `decay_scale_q16` is the per-source scale the
/// call sites pass (`0x8000`, `0x4ccc`, `0x1999`, ...). The caller only applies
/// the step when the new peak exceeds the accumulator's current magnitude and the
/// magnitude is above the `0x1999` (0.1) threshold.
#[must_use]
pub fn collision_decay_step(game_magnitude: i32, decay_scale_q16: i32, long_effect: bool) -> i32 {
    let magnitude = i64::from(game_magnitude.clamp(0, GAME_MAGNITUDE_MAX));
    let duration_q16 =
        ((if long_effect { 2 } else { 1 }) * COLLISION_DURATION_Q16 * COLLISION_DURATION_GAIN)
            >> 16;
    let step_factor = (i64::from(DI_MAGNITUDE_SCALE) << 32) / duration_q16;
    let scaled = (magnitude * i64::from(decay_scale_q16.max(0))) >> 16;
    let step = (step_factor * scaled) >> 32;
    i32::try_from(step).unwrap_or(i32::MAX).max(1)
}

/// The flagged second collision accumulator's peak (`FUN_004331b0`, when the
/// per-car flag `DAT_005500c4[player * 0x22]` is set): `min(2 * peak, 10000)`.
#[must_use]
pub fn collision_second_peak(game_magnitude: i32) -> i32 {
    (2 * collision_peak(game_magnitude)).min(di::FF_NOMINAL_MAX)
}

/// Threshold (`0x1999` ~ 0.1 Q16.16) above which the collision and engine
/// vibration sources produce any force at all.
pub const FORCE_THRESHOLD_Q16: i32 = 0x1999;

/// The continuous engine/gear vibration magnitude (`FUN_00433ec0`), recovered as
/// a speed ramp: `0` below [`FORCE_THRESHOLD_Q16`], otherwise
/// `min(((speed - 0x1999) * 0x11c71) >> 16, 0x10000) * 6000 >> 16` (0..6000, the
/// DirectInput magnitude scale). The shipped caller gates this on front-wheel
/// contact flags and engine-state guards.
#[must_use]
pub fn engine_vibration_magnitude(speed_q16: i32) -> i32 {
    if speed_q16 < FORCE_THRESHOLD_Q16 {
        return 0;
    }
    let factor = ((i64::from(speed_q16 - FORCE_THRESHOLD_Q16) * 0x1_1c71) >> 16)
        .min(i64::from(GAME_MAGNITUDE_MAX));
    i32::try_from((factor * 6000) >> 16).unwrap_or(6000)
}

/// The collision impact magnitude mapping of `FUN_0041f630`: a square-root
/// magnitude below `0x2624` yields no impact, otherwise
/// `min((sqrt - 0x2624) * 0xf536 >> 16, 0x10000)` (a ~0.958 gain), which the
/// caller rescales for DirectInput.
#[must_use]
pub fn collision_impact_from_sqrt(sqrt_q16: i32) -> Option<i32> {
    const THRESHOLD: i32 = 0x2624;
    const GAIN: i64 = 0xf536;
    if sqrt_q16 < THRESHOLD {
        return None;
    }
    let scaled =
        ((i64::from(sqrt_q16 - THRESHOLD) * GAIN) >> 16).min(i64::from(GAME_MAGNITUDE_MAX));
    Some(i32::try_from(scaled).unwrap_or(GAME_MAGNITUDE_MAX))
}

/// Byte offset of a player's FFB option within the option struct `DAT_00d3ca80`.
pub fn player_config_option_offset(player: usize) -> usize {
    PER_PLAYER_CONFIG_OPTION_OFFSET + player * PER_PLAYER_CONFIG_STRIDE
}

/// Whether the per-car FFB dispatch (`FUN_00433750`) runs this frame: the
/// DirectInput device must be present (`DAT_00bb95a8`), the session's FFB must be
/// active (`DAT_00550484`), and the player's FFB option bit must be set. When
/// false, every effect wrapper short-circuits to [`di::E_INVALIDARG`] and the
/// game runs with no forces.
pub fn ffb_active(device_present: bool, session_active: bool, player_enabled: bool) -> bool {
    device_present && session_active && player_enabled
}

/// Stateful per-car force-feedback mixer driving dual-motor gamepad rumble from
/// the recovered `FUN_00433750` / `FUN_004330c0` / `FUN_00433ac0` /
/// `FUN_00433ec0` effect accumulators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfbRumbleMixer {
    collision: DecayAccumulator,
}

impl FfbRumbleMixer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            collision: DecayAccumulator {
                magnitude: 0,
                step: 0,
            },
        }
    }

    pub fn reset(&mut self) {
        self.collision = DecayAccumulator::default();
    }

    /// Feed a collision impact magnitude in Q16.16 (`0..=0x10000`, `FUN_004330c0`).
    /// Replaces the decaying accumulator only when the new peak exceeds the
    /// current magnitude and passes [`FORCE_THRESHOLD_Q16`].
    pub fn push_collision_impact(&mut self, game_magnitude_q16: i32, long_effect: bool) {
        if game_magnitude_q16 <= FORCE_THRESHOLD_Q16 {
            return;
        }
        let peak = collision_peak(game_magnitude_q16);
        if peak > self.collision.magnitude {
            self.collision =
                DecayAccumulator::seed_collision(game_magnitude_q16, 0x8000, long_effect);
        }
    }

    /// Advance one 25 Hz tick (`FUN_00433750` / `FUN_00433ac0` / `FUN_00433ec0`)
    /// and map the DirectInput effect gains (`0..=10000`) to SDL dual-motor
    /// rumble `(low_freq_u16, high_freq_u16)`.
    pub fn tick(
        &mut self,
        active: bool,
        speed_q16: i32,
        front_contact: bool,
        wheel_slip_q16: u32,
        rough_surface: bool,
    ) -> (u16, u16) {
        if !active {
            self.reset();
            return (0, 0);
        }
        let impact_di = self.collision.tick().clamp(0, di::FF_NOMINAL_MAX) as u32;
        let impact_u16 = (impact_di * 0xFFFF) / (di::FF_NOMINAL_MAX as u32);
        let engine_di = if front_contact {
            engine_vibration_magnitude(speed_q16).clamp(0, di::FF_NOMINAL_MAX) as u32
        } else {
            0
        };
        let engine_low = ((engine_di * 0xFFFF) / (di::FF_NOMINAL_MAX as u32)) / 5;
        let rough_low = if rough_surface && speed_q16 > 0x1000 {
            (speed_q16.unsigned_abs() >> 4).min(0x3800)
        } else {
            0
        };
        let low = impact_u16
            .max(rough_low)
            .saturating_add(engine_low)
            .min(0xFFFF) as u16;
        let slip_high = (wheel_slip_q16 >> 3).min(0xA000);
        let high = slip_high.max(impact_u16 / 2).min(0xFFFF) as u16;
        (low, high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnitude_scales_full_range_to_di_units() {
        // 0 -> 0, full Q16.16 (0x10000) -> DI_FFNOMINALMAX (10000).
        assert_eq!(scale_magnitude(0), 0);
        assert_eq!(scale_magnitude(GAME_MAGNITUDE_MAX), di::FF_NOMINAL_MAX);
        // Half magnitude -> half nominal (floored).
        assert_eq!(scale_magnitude(0x8000), 5000);
        // Out-of-range clamps rather than overflowing.
        assert_eq!(scale_magnitude(-1), 0);
        assert_eq!(scale_magnitude(0x20_0000), di::FF_NOMINAL_MAX);
    }

    #[test]
    fn collision_step_matches_the_recovered_duration_formula() {
        // duration 5.0 s (Q16), full magnitude and the 0.5 call-site scale:
        // (10000<<32)/5.0 * 0.5 >> 32 = 1000 DI units per frame.
        assert_eq!(collision_decay_step(0x10000, 0x8000, false), 1000);
        // The long-effect flag doubles the duration, halving the step.
        assert_eq!(collision_decay_step(0x10000, 0x8000, true), 500);
        // The 0x4ccc (~0.3) call-site scale decays faster than 0x1999 (~0.1).
        let fast = collision_decay_step(0x10000, 0x4ccc, false);
        let slow = collision_decay_step(0x10000, 0x1999, false);
        assert!(fast > slow, "{fast} should exceed {slow}");
        // A magnitude just above the 0.1 threshold decays in a large step,
        // and the smallest magnitude still yields at least one unit.
        assert_eq!(collision_decay_step(0x199a, 0x8000, false), 100);
        assert_eq!(collision_decay_step(1, 0x8000, false), 1);
        assert_eq!(collision_decay_step(0, 0x8000, false), 1);
        // A full 10000-unit peak decays in about ten frames at step 1000.
        let mut accumulator = DecayAccumulator::seed_collision(0x10000, 0x8000, false);
        assert_eq!(accumulator.magnitude, 10000);
        assert_eq!(accumulator.step, 1000);
        assert_eq!(accumulator.tick(), 10000);
        assert_eq!(accumulator.magnitude, 9000);
    }

    #[test]
    fn collision_second_peak_doubles_and_caps() {
        assert_eq!(collision_second_peak(0), 0);
        assert_eq!(collision_second_peak(0x4000), 5000);
        assert_eq!(collision_second_peak(0x8000), di::FF_NOMINAL_MAX);
        assert_eq!(collision_second_peak(0x10000), di::FF_NOMINAL_MAX);
    }

    #[test]
    fn ffb_rumble_mixer_decays_impacts_smoothly_and_clears_when_inactive() {
        let mut mixer = FfbRumbleMixer::new();
        mixer.push_collision_impact(0x1_0000, false);
        let (low0, high0) = mixer.tick(true, 0, false, 0, false);
        let (low1, _) = mixer.tick(true, 0, false, 0, false);
        assert_eq!(low0, 0xFFFF);
        assert!(high0 > 0);
        assert!(low1 > 0 && low1 < low0, "impact must decay across frames");
        assert_eq!(mixer.tick(false, 0x1_0000, true, 0x1_0000, true), (0, 0));
    }

    #[test]
    fn engine_vibration_ramps_from_the_recovered_threshold() {
        assert_eq!(engine_vibration_magnitude(0), 0);
        assert_eq!(engine_vibration_magnitude(FORCE_THRESHOLD_Q16 - 1), 0);
        assert_eq!(engine_vibration_magnitude(FORCE_THRESHOLD_Q16), 0);
        // The integer ramp lands one unit below full scale at exactly 1.0.
        assert_eq!(engine_vibration_magnitude(0x10000), 5999);
        // Monotonic ramp between the threshold and full scale.
        let low = engine_vibration_magnitude(0x4000);
        let high = engine_vibration_magnitude(0xc000);
        assert!(low > 0 && low < high && high < 6000, "{low} {high}");
        // Above full scale the factor clamps at 0x10000 (6000).
        assert_eq!(engine_vibration_magnitude(0x40000), 6000);
    }

    #[test]
    fn collision_impact_threshold_and_gain() {
        assert_eq!(collision_impact_from_sqrt(0), None);
        assert_eq!(collision_impact_from_sqrt(0x2623), None);
        // Right at the threshold the mapping still rounds to zero.
        assert_eq!(collision_impact_from_sqrt(0x2624), Some(0));
        assert_eq!(collision_impact_from_sqrt(0x2625), Some(0));
        // A little above it the integer gain produces a small positive impact.
        assert_eq!(collision_impact_from_sqrt(0x2700), Some(210));
        // The ~0.958 gain over (sqrt - 0x2624) tops out at 53,421, not at the
        // 0x10000 game magnitude (the remaining headroom is never reached).
        assert_eq!(collision_impact_from_sqrt(0x1_0000), Some(53_421));
    }

    #[test]
    fn self_centering_follows_the_recovered_speed_curve() {
        assert_eq!(self_centering_speed_gain(0), di::FF_NOMINAL_MAX);
        assert_eq!(
            self_centering_speed_gain(SELF_CENTERING_REST_SPEED_Q16 - 1),
            di::FF_NOMINAL_MAX
        );
        assert_eq!(
            self_centering_speed_gain(SELF_CENTERING_REST_SPEED_Q16),
            9999
        );
        assert_eq!(self_centering_speed_gain(0x1999), 6250);
        assert_eq!(self_centering_speed_gain(0x2000), 4687);
        assert_eq!(
            self_centering_speed_gain(SELF_CENTERING_ZERO_SPEED_Q16 - 1),
            1
        );
        assert_eq!(self_centering_speed_gain(SELF_CENTERING_ZERO_SPEED_Q16), 0);
        assert_eq!(self_centering_speed_gain(0x10000), 0);
    }

    #[test]
    fn cornering_gain_clamps_before_shift() {
        assert_eq!(clamp_cornering_gain(0), 0);
        assert_eq!(
            clamp_cornering_gain(CORNERING_SCALED_MAX),
            di::FF_NOMINAL_MAX
        );
        // Beyond the pre-shift ceiling still lands on the nominal max.
        assert_eq!(clamp_cornering_gain(i32::MAX), di::FF_NOMINAL_MAX);
        assert_eq!(clamp_cornering_gain(-5), 0);
        // A mid value is a plain >>16 of the clamped input.
        assert_eq!(clamp_cornering_gain(0x1388_0000), 0x1388);
    }

    #[test]
    fn self_centering_halves_and_clamps() {
        assert_eq!(self_centering_gain(0), 0);
        assert_eq!(self_centering_gain(10000), 5000);
        assert_eq!(self_centering_gain(40000), di::FF_NOMINAL_MAX);
        assert_eq!(self_centering_gain(-10), 0);
    }

    #[test]
    fn rumble_period_is_floored() {
        assert_eq!(rumble_period_us(0), RUMBLE_PERIOD_FLOOR_US);
        assert_eq!(rumble_period_us(500), RUMBLE_PERIOD_FLOOR_US);
        assert_eq!(rumble_period_us(36006), 36006);
    }

    #[test]
    fn digital_direction_snaps_left_and_right() {
        assert_eq!(
            digital_collision_direction(false),
            DI_DIRECTION_RIGHT_CENTIDEGREES
        );
        assert_eq!(
            digital_collision_direction(true),
            DI_DIRECTION_LEFT_CENTIDEGREES
        );
    }

    #[test]
    fn polar_conversion_maps_lut_boundaries() {
        // Boundary mapping only (the analog direction is an inference).
        assert_eq!(polar_centidegrees(0, SIGNED_RATIO_PHASE_FULL_PERIOD), 0);
        assert_eq!(
            polar_centidegrees(
                SIGNED_RATIO_PHASE_FULL_PERIOD / 4,
                SIGNED_RATIO_PHASE_FULL_PERIOD
            ),
            9000
        );
        assert_eq!(
            polar_centidegrees(
                SIGNED_RATIO_PHASE_FULL_PERIOD / 2,
                SIGNED_RATIO_PHASE_FULL_PERIOD
            ),
            18000
        );
        // A full turn (and multiples) wraps back to zero.
        assert_eq!(
            polar_centidegrees(
                SIGNED_RATIO_PHASE_FULL_PERIOD,
                SIGNED_RATIO_PHASE_FULL_PERIOD
            ),
            0
        );
        assert_eq!(
            polar_centidegrees(
                SIGNED_RATIO_PHASE_FULL_PERIOD + SIGNED_RATIO_PHASE_FULL_PERIOD / 4,
                SIGNED_RATIO_PHASE_FULL_PERIOD
            ),
            9000
        );
        // Negative angles wrap into range (three-quarter turn).
        assert_eq!(
            polar_centidegrees(
                -SIGNED_RATIO_PHASE_FULL_PERIOD / 4,
                SIGNED_RATIO_PHASE_FULL_PERIOD
            ),
            27000
        );
    }

    #[test]
    fn decay_accumulator_emits_then_ramps_to_zero() {
        // Seed from a half-magnitude impact (5000 DI units) with a 2000/frame step.
        let mut acc = DecayAccumulator::seed(0x8000, 2000);
        assert_eq!(acc.magnitude, 5000);
        assert!(!acc.is_idle());
        // Copy-then-decay: first tick emits the peak, then subtracts the step.
        assert_eq!(acc.tick(), 5000);
        assert_eq!(acc.tick(), 3000);
        assert_eq!(acc.tick(), 1000);
        // Clamps at zero rather than going negative.
        assert_eq!(acc.tick(), 0);
        assert_eq!(acc.tick(), 0);
        assert!(acc.is_idle());
    }

    #[test]
    fn decay_seed_clamps_impact_and_step() {
        // Over-range impact clamps to the nominal max; negative step floors at 0.
        let acc = DecayAccumulator::seed(0x40_0000, -5);
        assert_eq!(acc.magnitude, di::FF_NOMINAL_MAX);
        assert_eq!(acc.step, 0);
    }

    #[test]
    fn effect_update_wrappers_preserve_exact_parameters_and_flags() {
        assert_eq!(
            sample_period_update(1000),
            SamplePeriodUpdate {
                effect_size: 0x38,
                sample_period: 1000,
                set_flags: 2,
            }
        );
        assert_eq!(
            periodic_gain_update(6000, 500),
            PeriodicGainUpdate {
                effect_size: 0x38,
                effect_flags: 0x22,
                gain: 6000,
                type_specific_size: 2,
                type_specific_first: 500,
                set_flags: 0x44,
            }
        );
    }

    #[test]
    fn effect_slot_table_matches_creation_order() {
        assert_eq!(effect_slot_kind(0), Some(EffectKind::PeriodicSine));
        assert_eq!(effect_slot_kind(1), Some(EffectKind::ConstantForce));
        assert_eq!(effect_slot_kind(2), Some(EffectKind::ConditionSpring));
        assert_eq!(effect_slot_kind(3), Some(EffectKind::ConditionDamper));
        assert_eq!(effect_slot_kind(4), Some(EffectKind::ConstantForce));
        assert_eq!(effect_slot_kind(5), Some(EffectKind::PeriodicSine));
        // Out-of-range slot is rejected, not wrapped.
        assert_eq!(effect_slot_kind(EFFECT_SLOT_COUNT), None);
        assert_eq!(effect_slot_kind(99), None);
    }

    #[test]
    fn create_flags_and_axes_split_polar_from_cartesian() {
        assert_eq!(
            effect_create_flags(EffectKind::PeriodicSine),
            di::DIEFF_POLAR | di::DIEFF_OBJECTOFFSETS
        );
        assert_eq!(effect_create_flags(EffectKind::PeriodicSine), 0x22);
        assert_eq!(
            effect_create_flags(EffectKind::ConstantForce),
            di::DIEFF_POLAR | di::DIEFF_OBJECTOFFSETS
        );
        assert_eq!(
            effect_create_flags(EffectKind::ConditionSpring),
            di::DIEFF_CARTESIAN | di::DIEFF_OBJECTOFFSETS
        );
        assert_eq!(effect_create_flags(EffectKind::ConditionDamper), 0x12);
        assert_eq!(effect_axis_count(EffectKind::ConstantForce), 2);
        assert_eq!(effect_axis_count(EffectKind::ConditionDamper), 1);
    }

    #[test]
    fn condition_selector_decodes_known_types() {
        assert_eq!(condition_type(0xb), Some(ConditionType::Spring));
        assert_eq!(condition_type(0xc), Some(ConditionType::Inertia));
        assert_eq!(condition_type(0xd), Some(ConditionType::Damper));
        assert_eq!(condition_type(0xe), Some(ConditionType::Friction));
        assert_eq!(condition_type(0), None);
        assert_eq!(condition_type(0xf), None);
    }

    #[test]
    fn centering_condition_is_symmetric_and_saturated() {
        let c = centering_condition(1234);
        assert_eq!(c.offset, 0);
        assert_eq!(c.positive_coefficient, 1234);
        assert_eq!(c.negative_coefficient, 1234);
        assert_eq!(c.positive_saturation, di::FF_NOMINAL_MAX as u32);
        assert_eq!(c.negative_saturation, di::FF_NOMINAL_MAX as u32);
        assert_eq!(c.dead_band, 0);
    }

    #[test]
    fn update_flags_push_only_changed_fields() {
        // Periodic: gain-only (4), period-only (2), both (6), neither (0).
        assert_eq!(periodic_update_flags(true, false), di::DIEP_GAIN);
        assert_eq!(periodic_update_flags(false, true), di::DIEP_SAMPLEPERIOD);
        assert_eq!(periodic_update_flags(true, true), 0x6);
        assert_eq!(periodic_update_flags(false, false), 0);
        // Constant force: gain-only (4), dir-only (0x40), both (0x44), neither (0).
        assert_eq!(constant_force_update_flags(true, false), di::DIEP_GAIN);
        assert_eq!(constant_force_update_flags(false, true), di::DIEP_DIRECTION);
        assert_eq!(constant_force_update_flags(true, true), 0x44);
        assert_eq!(constant_force_update_flags(false, false), 0);
        // Condition: whole block is type-specific params.
        assert_eq!(condition_update_flags(true), di::DIEP_TYPESPECIFICPARAMS);
        assert_eq!(condition_update_flags(false), 0);
    }

    #[test]
    fn enable_gate_requires_device_session_and_player_bit() {
        assert!(ffb_enabled(FFB_ENABLE_BIT));
        assert!(ffb_enabled(0xff));
        assert!(!ffb_enabled(0));
        assert!(!ffb_enabled(0x04));

        assert!(ffb_active(true, true, true));
        assert!(!ffb_active(false, true, true));
        assert!(!ffb_active(true, false, true));
        assert!(!ffb_active(true, true, false));
    }

    #[test]
    fn player_config_offsets_stride_correctly() {
        assert_eq!(player_config_option_offset(0), 0x3c);
        assert_eq!(player_config_option_offset(1), 0x3c + 0x9c);
        assert_eq!(player_config_option_offset(2), 0x3c + 0x9c * 2);
    }
}
