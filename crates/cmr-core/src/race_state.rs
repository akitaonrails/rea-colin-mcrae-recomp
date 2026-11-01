//! A deterministic offline race-state trace (G09): composes the recovered
//! fixed-step pieces into a reproducible per-tick integration.
//!
//! This is a **composition / determinism harness** over the recovered Q16.16
//! formulas - [`crate::fixed_point`] (the BAM trig and `Vec3`),
//! [`crate::vehicle_dynamics`] (lateral grip, net-accel, drag, velocity
//! integration), and [`crate::stage_timing`] (the fixed-step stage clock). Its
//! purpose is to verify that the subsystems interoperate and that identical
//! inputs yield a bit-for-bit identical trace (all state is integer `Fixed16`,
//! so there is no float non-determinism).
//!
//! It is **not** a claim of trajectory accuracy against the original game. The
//! force that drives the car, the inverse mass, and gravity are explicit harness
//! parameters ([`SimParams`]), because the original force-production wiring
//! (drive torque -> contact force) remains a G02/G03 unknown. The recovered
//! translation half-delta kernel is wired with zero damping after acceleration
//! is computed, so the existing lateral-grip order is preserved while position
//! uses `new_velocity - delta_velocity / 2`. The force model remains a documented
//! harness choice, not a recovered trajectory claim.
//!
//! See evidence `impl.sim-race-state-trace`.

use crate::car_reset::{
    ResetDispatchInput, ResetFadeState, ResetNodeError, ResetPlacement, place_on_target,
    select_reset_target, step_reset_fade,
};
use crate::car_setup::CarSetupBlock;
use crate::car_vs_car_collision::{
    CarVsCarSessionState, CarVsCarState, resolve_car_vs_car_collision_442740,
};
use crate::compiled_car_initializer::{
    CompiledControlClearLoop, CompiledControlFamilies, CompiledFourWheelConstants,
    CompiledHalfSignState, CompiledLaneThresholdEffects, CompiledLaneThresholdInputs,
    CompiledPostLaneBasisEffects, derive_compiled_lane_thresholds,
};
use crate::drivetrain::{
    DrivenSmoothingScalarInput, DrivenSmoothingScalars, DrivenWheelSmoothingInput,
    EngineForceInput, FirstWheelEffects, FirstWheelInputs, ForcePassScales, SecondWheelCoreEffects,
    SecondWheelInputs, SmoothedEngineSpeedInput, SmoothedSpeedGateInput, WheelBrakeInput,
    advance_idle_oscillation, advance_idle_reset, advance_residual_enable, apply_rev_limiter,
    derive_alternate_force_pass_factor, derive_driven_smoothing_scalars, derive_engine_force,
    derive_force_pass_scales, derive_gear_ratio, derive_smoothed_engine_speed,
    distribute_drive_torque, equalize_wheel_speeds, execute_first_wheel_iteration,
    execute_second_wheel_core, integrate_driven_wheel_speed, integrate_engine_from_raw_wheels,
    integrate_idle_engine, smoothed_speed_uses_tracking, spin_free_wheel,
    track_smoothed_wheel_speed, wheel_brake_torques, wheel_slip_flag, wheel_slip_shift,
};
use crate::fixed_point::{Fixed16, TRIG_TABLE_LEN, Vec3, build_sine_table, cos_bam, sin_bam};
use crate::gearbox::{GearOutOfRange, GearboxInputs, GearboxParams, GearboxState, apply_gearbox};
use crate::ground_pass::{GroundContact, GroundPassCar, run_ground_pass};
use crate::integrator_head::{
    ContactSumInput, GripAssistInput, RollSurfaceIndexError, RolloverInput, RolloverState,
    contact_force_and_moment, grip_assist, rollover_trigger,
};
use crate::legacy_math::sqrt_q16;
use crate::live_static_collision_car_frame::{
    ActiveCellRecordBuildError, LiveStaticCollisionCarFrameInput,
    LiveStaticCollisionFrameBridgeError, PreviousCarXz, UpperHullDeformationScalars,
    build_active_cell_record, build_live_static_collision_car_frame,
};
use crate::moving_object_collision::{
    MovingObjectCarState, MovingObjectCollisionError, MovingObjectHitCache,
    resolve_moving_object_collisions,
};
use crate::moving_objects::MovingObjects;
use crate::pedal_response::{
    PedalInputs, PedalResponseParams, PedalResponseState, RawPedals, apply_pedal_responses,
};
use crate::stage_timing::{advance_stage_clock, elapsed_centiseconds, stage_clock_step};
use crate::static_collision_kernel::{
    RecoveredStaticCollisionKernel, StaticKernelContext, StaticKernelError,
};
use crate::static_collision_response::{ResponseBasis, StaticDamageOwner, StaticResponseHistory};
use crate::static_collision_scene_builder::ParsedStaticCollisionScene;
use crate::static_world_collision::{
    StaticCollisionExecutionError, StaticContactHistory, resolve_static_world_collisions,
};
use crate::steering_response::{SteeringInputs, SteeringParams, SteeringState, apply_steering};
use crate::trigger_walk::{
    TriggerCells, TriggerEvent, TriggerWalkConfig, TriggerWalkError, TriggerWalkState,
    walk_triggers,
};
use crate::tumble::{TumbleState, arm_righting};
use crate::tyre_load::{
    TyreGains, TyreLoadInputs, TyreSurfaceIndexError, WheelSurfaceTerms, apply_tyre_load_pass,
    derive_wheel_surface_terms, wheel_candidate_bases,
};
use crate::vehicle_dynamics::{
    AngularOrientationInput, ContactBasisInput, ContactDeadzoneGate, ContactVectorUpdateInput,
    GeometricPostprocessError, GeometricPostprocessLane, LaneDirectionSources,
    LookupDependentCandidateError, OutOfControlState, RawSteer, SOLVER_GLOBAL_CONSTANTS,
    SurfaceCoefficientIndexError, TranslationStepInput, apply_angular_orientation_pipeline,
    apply_contact_vector_updates, apply_geometric_lane_postprocess, apply_lateral_grip,
    apply_low_speed_steer_cut, apply_translation_step, build_contact_basis_vectors,
    construct_lane_directions, contact_count_scale, contact_global_update_gate,
    derive_lane_direction_pair, drag, net_accel, resolve_lane_direction_mode, resting_flag_700,
    stabilize_lane_source_s, sum_translation_lane_forces, transform_contact_force,
    upright_flag_404,
};
use crate::vehicle_dynamics::{classify_contact_distance_lanes, grounded_base_force, slope_grip};
use cmr_formats::pe_surface::SurfaceCoefficientFamilies;
use cmr_formats::pe_tyre_tables::TyreSurfaceTables;

/// Fixed per-tick parameters of the integration harness (not recovered game
/// data - explicit inputs that stand in for the unrecovered force wiring).
#[derive(Clone, Copy, Debug)]
pub struct SimParams {
    /// Reciprocal of the car mass applied in [`net_accel`].
    pub inverse_mass: Fixed16,
    /// Constant acceleration added each tick (e.g. gravity / downforce).
    pub gravity: Vec3,
}

/// A driver's per-tick inputs for the harness.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Controls {
    /// Signed BAM steering rate added to the heading each tick.
    pub steer_rate: i16,
    /// Forward driving force magnitude applied along the heading.
    pub forward_force: Fixed16,
    /// Whether the handbrake is held (halves lateral grip).
    pub handbrake: bool,
    /// The recovered raw steer magnitudes `+0x2fc/+0x300`, which the
    /// four-wheel integrator's deadzones read after the low-speed cut.
    pub raw_steer: RawSteer,
    /// The recovered raw pedal words `+0x304/+0x308` and analog flags
    /// `+0x414/+0x418`, which the four-wheel control processor ramps.
    pub raw_pedals: RawPedals,
    /// The mapper's gear command `+0x314` (`1` up, `-1` down, `0` none).
    pub raw_gear_command_314: i8,
}

impl Controls {
    /// Build per-tick [`Controls`] from a decoded replay control record (`FUN_0046cef0` / `FUN_0046dc20`).
    #[must_use]
    pub fn from_decoded_replay_control(decoded: &crate::replay::ReplayDecodedControl) -> Self {
        Self {
            steer_rate: 0,
            forward_force: Fixed16::ZERO,
            handbrake: decoded.actuators.handbrake,
            raw_steer: RawSteer {
                left_2fc: decoded.actuators.steer,
                right_300: decoded.actuators.steer_alternate,
                analog_410: decoded.control_410,
            },
            raw_pedals: RawPedals {
                throttle_304: decoded.actuators.throttle,
                brake_308: decoded.actuators.brake,
                throttle_analog_414: decoded.control_414,
                brake_analog_418: decoded.control_418,
            },
            raw_gear_command_314: decoded.actuators.gear as i8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FirstWheelState {
    pub force: i32,
    pub wheel_speed: i32,
}

/// Runtime inputs for the recovered first-wheel force pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirstWheelParams {
    pub split: i32,
    pub wheel_constant: i32,
    pub wheel_vector: [i32; 3],
    pub state_vector: [i32; 3],
    pub scale_c600: i32,
    pub limit: i32,
    pub shift: u32,
    pub factor_c5c0: i32,
    pub gear: i32,
    pub config_40c: i32,
    pub flag_43b: i32,
    pub config_158: i32,
    pub gear_ratio: i32,
    pub is_front_wheel: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SecondWheelState {
    pub ratio_at_3b0: i32,
    pub angle: i16,
    pub component: i32,
    pub accumulators: [i32; 3],
}

/// Runtime inputs for the second per-wheel core after the first iteration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecondWheelParams<'a> {
    pub axis_dot: i32,
    pub local_68: [i32; 3],
    pub local_98: [i32; 3],
    pub wheel_e8: i32,
    pub scale_c5c4: i32,
    pub residual_flag_147a: bool,
    pub config_2ec: i32,
    pub gear_ratio: i32,
    pub coefficient_34: i32,
    pub arctan_table: &'a [i16],
    pub cos_table: &'a [i32],
    pub config_scale: i32,
    pub transition_gain: i32,
    pub transition_base: i32,
    pub alternate_gain: i32,
    pub alternate_base: i32,
    pub correction_gate: i32,
    pub accumulation_limit: i32,
    pub correction_alternate: i32,
    pub surface_vector: [i32; 3],
    pub table_factor: Option<i32>,
}

/// Select a compiled control-family word by the first-wheel gear, derive its
/// ratio, and update both wheel parameter sets. Invalid gears change nothing.
#[must_use]
pub fn populate_compiled_gear_ratio(
    first: &mut FirstWheelParams,
    second: &mut SecondWheelParams<'_>,
    families: &CompiledControlFamilies,
    factor_c860: Fixed16,
) -> Option<Fixed16> {
    let raw = *families.raw_copy.get(usize::try_from(first.gear).ok()?)?;
    let ratio = derive_gear_ratio(Fixed16::from_raw(raw), factor_c860);
    first.gear_ratio = ratio.raw();
    second.gear_ratio = ratio.raw();
    Some(ratio)
}

/// The drivetrain's raw gear ratio for every gear, by the same rule as
/// [`populate_compiled_gear_ratio`]: `qmul(raw_copy[gear], factor_c860)`.
#[must_use]
pub fn compiled_drive_ratios(families: &CompiledControlFamilies, factor_c860: Fixed16) -> [i32; 8] {
    families
        .raw_copy
        .map(|raw| derive_gear_ratio(Fixed16::from_raw(raw), factor_c860).raw())
}

/// Derive the main force-pass scales once and apply them to both wheel inputs.
#[must_use]
pub fn populate_force_pass_scales(
    first: &mut FirstWheelParams,
    second: &mut SecondWheelParams<'_>,
    state_52c: Fixed16,
    scale_eac70: Fixed16,
    step_eac6c: Fixed16,
) -> ForcePassScales {
    let scales = derive_force_pass_scales(state_52c, scale_eac70, step_eac6c);
    first.scale_c600 = scales.scale_c600.raw();
    first.factor_c5c0 = scales.factor_c5c0.raw();
    second.scale_c5c4 = scales.scale_c5c4.raw();
    scales
}

fn raw_dot(left: [i32; 3], right: [i32; 3]) -> i32 {
    (0..3).fold(0_i32, |sum, index| {
        sum.wrapping_add(
            Fixed16::from_raw(left[index])
                .mul(Fixed16::from_raw(right[index]))
                .raw(),
        )
    })
}

/// Populate the shared first/second-loop vectors from constructed directions
/// and the current per-wheel contact vector.
pub fn populate_wheel_vector_inputs(
    first: &mut FirstWheelParams,
    second: &SecondWheelParams<'_>,
    direction_a: Vec3,
) {
    first.wheel_vector = [
        direction_a.x.raw(),
        direction_a.y.raw(),
        direction_a.z.raw(),
    ];
    first.state_vector = second.surface_vector;
}

/// Derive second-core wheel projection and `0x147a` inputs from native state.
pub fn populate_derived_second_inputs(
    first: &FirstWheelParams,
    second: &mut SecondWheelParams<'_>,
    direction_b: Vec3,
    lane: usize,
) {
    second.wheel_e8 = raw_dot(first.wheel_vector, first.state_vector);
    second.axis_dot = direction_b
        .dot(Vec3::new(
            Fixed16::from_raw(second.surface_vector[0]),
            Fixed16::from_raw(second.surface_vector[1]),
            Fixed16::from_raw(second.surface_vector[2]),
        ))
        .raw();
    let config_gate = if lane < 2 {
        first.config_158 != 0
    } else {
        first.config_158 != 0x10000
    };
    second.residual_flag_147a = first.flag_43b != 0 && config_gate;
}

/// Populate the second-core surface coefficient from the reviewed first family.
/// Invalid surface indices leave the existing table/override selection unchanged.
///
/// # Errors
///
/// Returns the shared surface-index error for indices outside the 120-entry table.
pub fn populate_surface_table_factor(
    second: &mut SecondWheelParams<'_>,
    families: &SurfaceCoefficientFamilies,
    surface_index: usize,
) -> Result<(), SurfaceCoefficientIndexError> {
    let coefficient =
        *families
            .first
            .values
            .get(surface_index)
            .ok_or(SurfaceCoefficientIndexError {
                index: surface_index,
            })?;
    second.table_factor = Some(coefficient);
    Ok(())
}

/// Build the contact vectors and select one for a second-wheel surface input.
#[must_use]
pub fn populate_contact_basis_vector(
    second: &mut SecondWheelParams<'_>,
    input: ContactBasisInput,
    vector_index: usize,
) -> Option<Vec3> {
    if vector_index >= 8 {
        return None;
    }
    let vector = build_contact_basis_vectors(input)[vector_index];
    second.surface_vector = [vector.x.raw(), vector.y.raw(), vector.z.raw()];
    Some(vector)
}

/// Runtime vector lanes for the recovered four-lane postprocess. The current
/// second-core result replaces `lanes[current_lane].accumulator` before it runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FourWheelPostprocessParams<'a> {
    pub lanes: [GeometricPostprocessLane; 4],
    pub current_lane: usize,
    pub argument_scale: Fixed16,
    pub base_force: Vec3,
    pub non_wheel_forces: &'a [Vec3],
    pub wheel_contacts: [bool; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FourWheelPostprocessState {
    pub accumulators: [Vec3; 4],
    pub qualified: [bool; 4],
    pub angle_measures: [Fixed16; 4],
}

/// Runtime gates and scalar sources for the driven `FUN_004088c0` smoothing path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrivenWheelUpdateParams {
    pub scalars: DrivenSmoothingScalarInput,
    pub mode_408: i32,
    pub throttle_304: i32,
    pub brake_308: i32,
    pub split_170: i32,
    pub time_step: i32,
}

impl Default for DrivenWheelUpdateParams {
    fn default() -> Self {
        Self {
            scalars: DrivenSmoothingScalarInput {
                suspension_298: [0; 4],
                longitudinal: 0,
                gear_index: 0,
                gear_ratio: 0,
                gear_ratio_190: 0,
                redline_15c: 0,
                response_1f0: 0,
                engine_2e0: 0,
                response_160: 0,
            },
            mode_408: 0,
            throttle_304: 0,
            brake_308: 0,
            split_170: 0,
            time_step: 0x10000,
        }
    }
}

/// The static tables and per-car variant behind the tyre gains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TyreSurfaceInputs<'a> {
    pub tables: &'a TyreSurfaceTables,
    pub variant_758: i32,
    pub surface_families: Option<&'a SurfaceCoefficientFamilies>,
}

/// Complete per-wheel runtime inputs for one native-order force tick.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FourWheelIterationParams<'a> {
    pub first: [FirstWheelParams; 4],
    pub second: [SecondWheelParams<'a>; 4],
    pub contact_vector_indices: [usize; 4],
    pub contact_offsets: [Vec3; 4],
    /// Per-tick `FUN_0040d400` scalar/gate inputs; vectors are replaced from the
    /// current contact-basis output before the reverse traversal.
    pub contact_update_inputs: [ContactVectorUpdateInput; 8],
    /// Lane/source-Y values for the native `FUN_004126b0` enable/count scan.
    pub lane_threshold_inputs: CompiledLaneThresholdInputs,
    /// Per-car scalars for the control processor's pedal ramps
    /// (`+0x148/+0x150/+0x16c/+0x174/+0x17c/+0x184`).
    pub pedal_response: PedalResponseParams,
    /// Timestep used by the persistent `+0x5d4` stabilizer.
    pub lane_time_step: Fixed16,
    /// The per-car-type base angle `DAT_0050e4fc[type]`; the step adds this
    /// tick's wheel steer angle `+0x3e0` before rotating the lane source.
    pub lane_direction_angle: i16,
    /// Steering scalars, the per-car-type factor `DAT_0050e188[type]` and the
    /// two front-wheel `DAT_004f74ac` values for the control processor.
    pub steering: SteeringParams,
    pub steering_car_factor_e188: i32,
    pub steering_surface_factors: [i32; 2],
    /// The live gearbox. `None` keeps the fixed gear, shift and ratio words in
    /// `first`/`second` (the pre-gearbox harness); `Some` runs the control
    /// processor's gearbox each tick and feeds its gear, shift request
    /// `+0x40c`, drive ratio and reversing mode `+0x408` to the drivetrain.
    pub gearbox: Option<GearboxParams>,
    /// Setup block `+0x854/+0x858` (`DAT_0050e1a4/e1a8`): the front and rear
    /// brake scales of the wheel-force producer.
    pub brake_balance_e1a4: [i32; 2],
    /// The static tyre/surface tables and the car's tyre variant `+0x758`.
    /// `Some` with live ground contact runs the driver's `DAT_004f777c` loop
    /// and `FUN_004095e0` each tick and feeds their gains and bases to both
    /// wheel loops; `None` keeps the harness words in `first`/`second`.
    pub tyre: Option<TyreSurfaceInputs<'a>>,
    /// The hull probes' moment arms `+0x8c/+0x98/+0xa4/+0xb0` (probes 4..7).
    pub hull_contact_offsets: [Vec3; 4],
    /// `ctl +0x128/+0x12c`, the crest threshold and speed-loss gain of the
    /// grip assist (from the auxiliary control patch `FUN_00407270`).
    pub crest_128_12c: [i32; 2],
    /// `ctl +0x11c/+0x120/+0x124` read by the suspension animation
    /// `FUN_00410350` (from the auxiliary control patch).
    pub suspension: crate::wheel_animation::SuspensionConstants,
    /// Setup block `+0x800..+0x80c`: the wheel wobble and camber words.
    pub wheel_setup_800: [i32; 4],
    /// Resolved global/callback inputs; Rebuild takes precedence over Exchange.
    pub lane_rebuild: bool,
    pub lane_exchange: bool,
    pub postprocess: FourWheelPostprocessParams<'a>,
    /// State `+0x4c`, the common prefix's reciprocal of `+0x48`, which the
    /// integrator multiplies into the damped force (`0x40a9f0`).
    pub inverse_mass_4c: Fixed16,
    /// Moment scales/base contact input and angular gates; stateful vectors come
    /// from `MotionState` at each step.
    pub angular: AngularOrientationInput,
    /// Explicit engine redline/slip/load inputs; speeds and gear/mode fields are
    /// replaced from current native state before each update.
    pub engine_smoothing: SmoothedEngineSpeedInput,
    /// Suspension/engine/gate inputs; wheel forces, speeds, gear, limiter and
    /// handbrake come from the current tick.
    pub driven_smoothing: DrivenWheelUpdateParams,
    /// Explicit engine response/brake inputs for the `state+0x530` producer.
    pub engine_force: EngineForceInput,
}

/// Populate wheel contact offsets from the compiled sign vectors at native state
/// offsets `+0x5c`, `+0x68`, `+0x74`, and `+0x80`.
pub fn populate_compiled_contact_offsets(
    wheel: &mut FourWheelIterationParams<'_>,
    state: &CompiledHalfSignState,
) {
    const INDICES: [[usize; 3]; 4] = [[6, 7, 8], [3, 4, 5], [9, 10, 11], [12, 13, 14]];
    /// `+0x8c/+0x98/+0xa4/+0xb0` in the half-sign store order.
    const HULL_INDICES: [[usize; 3]; 4] = [[18, 19, 20], [15, 16, 17], [21, 22, 23], [24, 25, 26]];
    let offset = |[x, y, z]: [usize; 3]| {
        Vec3::new(
            Fixed16::from_raw(state.words[x]),
            Fixed16::from_raw(state.words[y]),
            Fixed16::from_raw(state.words[z]),
        )
    };
    wheel.contact_offsets = INDICES.map(offset);
    wheel.hull_contact_offsets = HULL_INDICES.map(offset);
}

/// Populate the integrator's mass, moment scale and inertia, and the control
/// processor's pedal scalars and `+0x408` mode, from the recovered initializer
/// constants.
pub fn populate_compiled_four_wheel_constants(
    wheel: &mut FourWheelIterationParams<'_>,
    constants: &CompiledFourWheelConstants,
) {
    wheel.inverse_mass_4c = Fixed16::from_raw(constants.state_word_4c);
    for first in &mut wheel.first {
        first.config_158 = constants.control_word_158;
    }
    wheel.angular.momentum.moment_scale = Fixed16::from_raw(constants.state_word_48);
    let [x, y, z] = constants.inertia_50_54_58.map(Fixed16::from_raw);
    wheel.angular.momentum.inertia_scale = Vec3::new(x, y, z);
    wheel.pedal_response = PedalResponseParams {
        throttle_max_148: constants.control_word_148,
        throttle_rate_150: constants.control_word_150,
        brake_max_16c: constants.control_word_16c,
        brake_rate_174: constants.control_word_174,
        handbrake_rate_17c: constants.control_word_17c,
        handbrake_max_184: constants.control_word_184,
    };
    wheel.driven_smoothing.mode_408 = constants.control_word_408;
    wheel.driven_smoothing.split_170 = constants.control_word_170;
    wheel.driven_smoothing.time_step = SOLVER_GLOBAL_CONSTANTS.time_step.raw();
    wheel.driven_smoothing.scalars.redline_15c = constants.control_word_15c;
    wheel.driven_smoothing.scalars.response_160 = constants.control_word_160;
    // The steering slew multiplies its rate by `DAT_004eac6c`.
    wheel.steering.time_step = SOLVER_GLOBAL_CONSTANTS.time_step.raw();
    wheel.steering.centering_188 = constants.control_word_188;
    wheel.steering.max_angle_3e4 = constants.control_u16_3e4 as i16;
    wheel.steering.flags_440 = constants.control_byte_440;
    wheel.engine_smoothing.redline_15c = constants.control_word_15c;
    wheel.engine_force.redline_15c = constants.control_word_15c;
    wheel.engine_force.engine_brake_144 = constants.control_word_144;
    wheel.postprocess.base_force = gravity_force_51c(
        Fixed16::from_raw(constants.state_word_48),
        Fixed16::from_raw(constants.gravity_c5fc),
    );
    for second in &mut wheel.second {
        second.correction_gate = constants.control_word_420;
    }
}

/// The per-tick base force the mechanics driver writes to `+0x51c..+0x524`
/// (`0x407f13..`): `(0, -qmul(+0x48, qmul(DAT_0050c5fc, 0x8000)), 0)`, the
/// starting value of the integrator's force sum.
#[must_use]
pub fn gravity_force_51c(mass_48: Fixed16, gravity_c5fc: Fixed16) -> Vec3 {
    Vec3::new(
        Fixed16::ZERO,
        mass_48
            .mul(gravity_c5fc.mul(Fixed16::from_raw(0x8000)))
            .neg(),
        Fixed16::ZERO,
    )
}

/// The per-tick first-wheel inputs from one lane's parameters; the caller sets
/// the live drive slot, wheel speed and handbrake.
fn first_wheel_inputs(wheel: &FirstWheelParams) -> FirstWheelInputs {
    FirstWheelInputs {
        force: 0,
        wheel_speed: 0,
        split: wheel.split,
        wheel_constant: wheel.wheel_constant,
        wheel_vector: wheel.wheel_vector,
        state_vector: wheel.state_vector,
        scale_c600: wheel.scale_c600,
        limit: wheel.limit,
        shift: wheel.shift,
        handbrake: 0,
        factor_c5c0: wheel.factor_c5c0,
        gear: wheel.gear,
        config_40c: wheel.config_40c,
        config_158: wheel.config_158,
        gear_ratio: wheel.gear_ratio,
        is_front_wheel: wheel.is_front_wheel,
    }
}

/// Populate the per-slot setup-block scalars the four-wheel passes read: the
/// engine response `DAT_0050e1ac` (`+0x85c`), steering factor `DAT_0050e188`
/// (`+0x838`), per-lane wheel constants `DAT_0050e18c[lane]` (`+0x83c`), base
/// steer angle `DAT_0050e4fc` (`+0xbac`) and, with a live gearbox, the gear-2
/// skip from `DAT_0050e170` (`+0x820`).
pub fn populate_car_setup(wheel: &mut FourWheelIterationParams<'_>, setup: &CarSetupBlock) {
    wheel.engine_force.response_e1ac = setup.engine_response_85c;
    wheel.steering_car_factor_e188 = setup.steering_factor_838;
    wheel.lane_direction_angle = setup.base_angle_bac;
    wheel.brake_balance_e1a4 = [setup.words_84c[2], setup.words_84c[3]];
    for (first, constant) in wheel.first.iter_mut().zip(setup.wheel_constants_83c) {
        first.wheel_constant = constant;
    }
    wheel.wheel_setup_800 = [
        setup.setup_800[0],
        setup.setup_800[1],
        setup.setup_800[2],
        setup.setup_800[3],
    ];
    if let Some(gearbox) = &mut wheel.gearbox {
        gearbox.gear_two_skip = setup.gear_two_skip();
    }
}

/// Populate each update's limit and threshold from the interleaved compiled
/// control-clear words at `+0x31c..+0x358`.
pub fn populate_compiled_contact_update_inputs(
    wheel: &mut FourWheelIterationParams<'_>,
    state: &CompiledControlClearLoop,
) {
    for (index, input) in wheel.contact_update_inputs.iter_mut().enumerate() {
        input.limit = Fixed16::from_raw(state.words[index * 2]);
        input.threshold = Fixed16::from_raw(state.words[index * 2 + 1]);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FourWheelState {
    pub first: [FirstWheelState; 4],
    pub second: [SecondWheelState; 4],
    /// Separate smoothed speeds at `control+0x2b8..+0x2c4`.
    pub smoothed_wheel_speeds: [i32; 4],
    pub postprocess: FourWheelPostprocessState,
    /// Persistent `FUN_0040d400` destinations, including skipped records.
    pub contact_updates: [Vec3; 8],
    /// Prior native velocity magnitude at `state+0xec`.
    pub speed_ec: i32,
    /// Per-frame residual-enable countdown at `control+0x43b`.
    pub residual_enable: u8,
    /// Engine-speed input at `control+0x2e0` and smoothed output at `+0x2e8`.
    pub engine_2e0: i32,
    pub engine_speed_2e8: i32,
    /// Rev-limiter state and cut at `control+0x444/+0x448`.
    pub limiter_444: bool,
    pub limiter_cut_448: i32,
    /// Engine force and neutral/idle state at `state+0x530/+0x6e8/+0x6ec`.
    pub engine_force_530: i32,
    pub idle_6e8: i32,
    pub idle_countdown_6ec: i16,
    /// The control processor's ramps and responses, including the throttle
    /// response `control+0x2d8` read by the engine force and lane stabilizer.
    pub pedals: PedalResponseState,
    /// The control processor's steering words (`+0x318/+0x114/+0x3e0/+0x3e2`).
    pub steering: SteeringState,
    /// The control processor's gearbox words (`+0x43a/+0x43f/+0x40c/...`).
    pub gearbox: GearboxState,
    /// Control `+0x404`, recomputed at the start of each four-wheel tick.
    pub upright_404: bool,
    /// Resting flag at `state+0x700`, written by the integrator each tick.
    pub resting_700: bool,
    /// The basis the mechanics driver copies to `+0x27c` at the start of the
    /// tick; the ground pass restores it for a resting, aligned car.
    pub start_basis_27c: [Vec3; 3],
    /// The driver's per-wheel `DAT_004f777c` words `ctl +0x278/+0x288/+0x298`.
    pub wheel_terms: WheelSurfaceTerms,
    /// The tyre gain words `ctl +0x31c..+0x378` written by `FUN_004095e0`.
    pub tyre_gains: TyreGains,
    /// This tick's support sums `+0x3fc + 12i`, cleared by the driver.
    pub support_3fc: [Vec3; 8],
    /// The out-of-control words of the tumbling pass.
    pub tumble: TumbleState,
    /// This tick's contact vectors `+0x4bc..+0x518` (`FUN_00409f90`).
    pub contact_vectors_4bc: [Vec3; 8],
    /// `ctl +0x3c0 + 4w`: the second loop's lateral dot
    /// `local_98[w] . car(+0x4bc + 12w)`, kept while airborne.
    pub lateral_dot_3c0: [i32; 4],
    /// The rollover words `+0x734/+0x610/+0x614/+0x60c`.
    pub rollover: RolloverState,
    /// `+0x330`: the ground normal saved before the last ground pass.
    pub previous_normal_330: Vec3,
    /// `+0xe0..+0xe8`: prior world velocity saved in `FUN_0040a4a0` (`0x40ab04`)
    /// before `apply_translation_step` and read by `FUN_00416760`.
    pub prior_velocity_e0: Vec3,
    /// `+0x39c..+0x3c8`: the four wheel ground normals saved at the start of
    /// `FUN_00410a60` (`0x410a9b`) and read by `FUN_0040fbe0`.
    pub previous_wheel_normals_39c: [[i32; 3]; 4],
    /// The reset words `ctl +0x41c/+0x424/+0x428/+0x460` (`FUN_00414850`).
    pub reset: ResetFadeState,
    /// `+0x3e4..+0x3ec`: the centre saved before the physics step.
    pub saved_position_3e4: Vec3,
    /// `+0x15c..+0x1b8`: the probes snapshotted after the last trigger walk.
    pub trigger_prior_15c: [[i32; 3]; 8],
    /// `+0x3f0..+0x3f8`: the last barrier impact point (body frame).
    pub barrier_impact_3f0: [i32; 3],
    /// `ctl +0x380/+0x38c/+0x398/+0x3a4`: the force-pass head's published
    /// lane vectors (A lane 0, A lane 2, B lane 0, B lane 2), which the wheel
    /// particles read.
    pub head_vectors_380: [Vec3; 4],
    /// `+0x6dc`: a barrier touched the car in the last walk (read by the
    /// rollover trigger on the next tick).
    pub barrier_contact_6dc: bool,
    /// `DAT_0050c574`, `+0x6e0` and the `+0x75c` timer.
    pub trigger_walk: TriggerWalkState,
    /// The wheel and suspension animation words the renderer draws.
    pub wheels: crate::wheel_animation::WheelAnimationState,
}

/// Apply neutral-gear zeroing or the ordinary post-step tracking update.
pub fn update_smoothed_wheel_speeds(state: &mut FourWheelState, gear_ratio: i32) {
    if gear_ratio == 0 {
        state.smoothed_wheel_speeds = [0; 4];
        return;
    }
    for index in 0..4 {
        state.smoothed_wheel_speeds[index] = track_smoothed_wheel_speed(
            state.first[index].wheel_speed,
            state.smoothed_wheel_speeds[index],
        );
    }
}

fn driven_smoothing_scalars(
    state: &FourWheelState,
    first: &FirstWheelParams,
    params: &DrivenWheelUpdateParams,
) -> DrivenSmoothingScalars {
    derive_driven_smoothing_scalars(DrivenSmoothingScalarInput {
        gear_index: first.gear,
        gear_ratio: first.gear_ratio,
        engine_2e0: state.engine_2e0,
        ..params.scalars
    })
}

fn update_driven_wheel_speeds(
    state: &mut FourWheelState,
    first: &[FirstWheelParams; 4],
    drive_forces_2c8: [i32; 4],
    handbrake: bool,
    params: &DrivenWheelUpdateParams,
    scalars: DrivenSmoothingScalars,
) {
    if first[0].gear_ratio == 0 {
        state.smoothed_wheel_speeds = [0; 4];
        return;
    }
    for (lane, wheel) in first.iter().enumerate() {
        let raw = state.first[lane].wheel_speed;
        let current = state.smoothed_wheel_speeds[lane];
        state.smoothed_wheel_speeds[lane] =
            if smoothed_speed_uses_tracking(SmoothedSpeedGateInput {
                lane,
                mode_408: params.mode_408,
                throttle_304: params.throttle_304,
                brake_308: params.brake_308,
                handbrake_30c: i32::from(handbrake),
                config_158: wheel.config_158,
                split_170: params.split_170,
                limiter_444: i32::from(state.limiter_444),
            }) {
                track_smoothed_wheel_speed(raw, current)
            } else {
                integrate_driven_wheel_speed(DrivenWheelSmoothingInput {
                    raw,
                    current,
                    drive_force_2c8: drive_forces_2c8[lane],
                    slip_gain: scalars.slip_gain,
                    direction_gain: scalars.direction_gain,
                    time_step: params.time_step,
                    signed_speed_limit: scalars.signed_speed_limit,
                    gear: wheel.gear,
                    config_40c: wheel.config_40c,
                    config_158: wheel.config_158,
                    gear_ratio: wheel.gear_ratio,
                    is_rear_wheel: lane >= 2,
                })
            };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WheelPostprocessError {
    LaneIndex(usize),
    ContactVectorIndex {
        lane: usize,
        vector_index: usize,
    },
    Candidate(LookupDependentCandidateError),
    Geometry(GeometricPostprocessError),
    /// The live gearbox reached a gear outside its eight-entry tables.
    Gear(GearOutOfRange),
    /// The ground pass's query failed on this lane.
    GroundQuery {
        failed_lane: usize,
    },
    /// A surface word or tyre variant outside the static tyre tables.
    TyreSurface(TyreSurfaceIndexError),
    /// A wheel surface word outside the roll-limit table.
    RollSurface(RollSurfaceIndexError),
    /// The reset dispatcher read outside the `.cat` nodes.
    ResetNode(ResetNodeError),
    /// The trigger walk could not run.
    TriggerWalk(TriggerWalkError),
}

/// Report a ground-query failure by the lane that failed.
fn ground_query_error<P, C>(
    failure: crate::compiled_car_initializer::CompiledEightLaneExecutionFailure<P, C>,
) -> WheelPostprocessError {
    use crate::compiled_car_initializer::CompiledEightLaneExecutionFailure::{Config, Dispatch};
    let (Dispatch { failed_lane, .. } | Config { failed_lane, .. }) = failure;
    WheelPostprocessError::GroundQuery { failed_lane }
}

impl From<ResetNodeError> for WheelPostprocessError {
    fn from(error: ResetNodeError) -> Self {
        Self::ResetNode(error)
    }
}

impl From<RollSurfaceIndexError> for WheelPostprocessError {
    fn from(error: RollSurfaceIndexError) -> Self {
        Self::RollSurface(error)
    }
}

impl From<TyreSurfaceIndexError> for WheelPostprocessError {
    fn from(error: TyreSurfaceIndexError) -> Self {
        Self::TyreSurface(error)
    }
}

impl From<GearOutOfRange> for WheelPostprocessError {
    fn from(error: GearOutOfRange) -> Self {
        Self::Gear(error)
    }
}

impl From<LookupDependentCandidateError> for WheelPostprocessError {
    fn from(error: LookupDependentCandidateError) -> Self {
        Self::Candidate(error)
    }
}

impl From<GeometricPostprocessError> for WheelPostprocessError {
    fn from(error: GeometricPostprocessError) -> Self {
        Self::Geometry(error)
    }
}

/// One static scenery hit's impact-sound inputs: the severity `FUN_0041f630`
/// returned, the world contact point and the object type `+0x54 & 0x7f`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticMaterialImpact {
    pub severity: i32,
    pub world_contact: [i32; 3],
    pub material: u8,
}

/// What one static scenery collision pass produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticCollisionStep {
    pub contacts: usize,
    pub impacts: Vec<StaticMaterialImpact>,
    /// The breakage cues (`FUN_0046a2a0`): the damage's kind and the world
    /// contact, in hit order.
    pub contact_effects: Vec<(i32, [i32; 3])>,
    /// The last normal hit's surface material byte (`DAT_00558fb0`).
    pub debug_material_558fb0: Option<u8>,
}

/// Orientation and angular inputs shared by the native motion/contact passes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MotionState {
    /// The three axes published through physics `+0x2a0`.
    pub basis: [Vec3; 3],
    /// The distinct AK11 projection axis at `+0x324/+0x328/+0x32c`.
    pub projection_axis: Vec3,
    /// Primary angular velocity at `+0xf0/+0xf4/+0xf8`.
    pub angular_velocity: Vec3,
    /// Persistent prior moment at `+0x3d8/+0x3dc/+0x3e0`.
    pub moment: Vec3,
    /// Arithmetic-half state at `+0x2c/+0x30/+0x34` from the compiled initializer.
    pub compiled_half_words: Vec3,
    /// Persistent stabilized direction source at `+0x5d4/+0x5d8/+0x5dc`.
    pub lane_source_s: Vec3,
    /// Four strict contact-distance flags at `+0x68c..+0x698`.
    pub contact_distance_flags: [bool; 4],
    /// Count of those flags at `+0x674`, consumed by static response mode 4.
    pub contact_count_674: i32,
    /// Setup/live collision deformation scalars at `+0x638/+0x63c/+0x640`.
    pub upper_hull_deformation: UpperHullDeformationScalars,
}

/// Cross-tick static-collision state owned by the live car.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LiveCollisionState {
    /// Linear correction at `+0x3cc..+0x3d4`; angular `+0x3d8` has the single
    /// owner [`MotionState::moment`].
    pub pending_linear_3cc: Vec3,
    pub history: StaticContactHistory,
    /// Moving-object repeat-contact table at `+0x71e..+0x733`.
    pub object_history: StaticResponseHistory,
    pub response_state_680: bool,
    pub recovery_latch_688: bool,
    /// `+0xbc/+0xc0/+0xc4`, written by the angular pass's moment-overflow
    /// trigger when it sets `+0x680/+0x688`.
    pub out_of_control_axis_bc: Vec3,
}

impl LiveCollisionState {
    /// The out-of-control words in the shape the angular pass reads and sets.
    #[must_use]
    pub fn out_of_control(&self) -> OutOfControlState {
        OutOfControlState {
            out_of_control_680: self.response_state_680,
            recovery_latch_688: self.recovery_latch_688,
            axis_snapshot_bc: self.out_of_control_axis_bc,
        }
    }

    /// Publish the angular pass's out-of-control result back into the owners.
    pub fn set_out_of_control(&mut self, state: OutOfControlState) {
        self.response_state_680 = state.out_of_control_680;
        self.recovery_latch_688 = state.recovery_latch_688;
        self.out_of_control_axis_bc = state.axis_snapshot_bc;
    }

    /// Snapshot and clear the three linear words before any collision pass can
    /// accumulate their replacements for the next tick.
    #[must_use]
    pub fn consume_pending_linear(&mut self) -> Vec3 {
        std::mem::take(&mut self.pending_linear_3cc)
    }
}

impl MotionState {
    /// Neutral harness values; no physical orientation or trajectory is claimed.
    pub const fn harness_default() -> Self {
        Self {
            basis: [
                Vec3::new(Fixed16::ONE.neg(), Fixed16::ZERO, Fixed16::ZERO),
                Vec3::new(Fixed16::ZERO, Fixed16::ONE.neg(), Fixed16::ZERO),
                Vec3::new(Fixed16::ZERO, Fixed16::ZERO, Fixed16::ONE.neg()),
            ],
            projection_axis: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            moment: Vec3::ZERO,
            compiled_half_words: Vec3::new(Fixed16::ONE, Fixed16::ONE, Fixed16::ONE),
            lane_source_s: Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
            contact_distance_flags: [false; 4],
            contact_count_674: 0,
            upper_hull_deformation: UpperHullDeformationScalars {
                scale_638: Fixed16::ZERO,
                scale_63c: Fixed16::ZERO,
                scale_640: Fixed16::ZERO,
            },
        }
    }

    /// Copy the first three recovered half-words from the compiled state record.
    pub fn apply_compiled_half_state(&mut self, state: &CompiledHalfSignState) {
        self.compiled_half_words = Vec3::new(
            Fixed16::from_raw(state.words[0]),
            Fixed16::from_raw(state.words[1]),
            Fixed16::from_raw(state.words[2]),
        );
    }

    /// Publish the compiled AK11 normal and rebuilt orientation basis.
    pub fn apply_compiled_post_lane_basis(
        &mut self,
        normal: [i32; 3],
        effects: &CompiledPostLaneBasisEffects,
    ) {
        self.projection_axis = Vec3::new(
            Fixed16::from_raw(normal[0]),
            Fixed16::from_raw(normal[1]),
            Fixed16::from_raw(normal[2]),
        );
        self.basis = [effects.vector_258, effects.vector_264, effects.vector_270].map(|vector| {
            Vec3::new(
                Fixed16::from_raw(vector[0]),
                Fixed16::from_raw(vector[1]),
                Fixed16::from_raw(vector[2]),
            )
        });
        // The initializer later seeds the persistent lane source `+0x5d4` from
        // the published row `+0x258` (`0x407073..0x40708b`).
        self.lane_source_s = self.basis[0];
    }

    /// Form the six angular inputs and add the current native velocity field.
    #[must_use]
    pub fn contact_basis(self, velocity: Vec3) -> ContactBasisInput {
        ContactBasisInput {
            rows: self.basis,
            angular: [
                self.angular_velocity.x,
                self.angular_velocity.y,
                self.angular_velocity.z,
                self.compiled_half_words.x,
                self.compiled_half_words.y,
                self.compiled_half_words.z,
            ],
            velocity,
        }
    }
}

/// The minimal per-car state the harness integrates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CarState {
    /// World position (Q16.16 on the x/z ground plane, y up).
    pub position: Vec3,
    /// World velocity.
    pub velocity: Vec3,
    /// Heading as a 16-bit BAM angle about the y axis.
    pub heading: u16,
    /// Orientation and angular state consumed by contact production.
    pub motion: MotionState,
    /// Persistent collision corrections, history, and recovery flags.
    pub collision: LiveCollisionState,
    /// Recovered first-wheel force and speed state.
    pub first_wheel: FirstWheelState,
    /// Residual, angle, component and accumulators from the second core.
    pub second_wheel: SecondWheelState,
    /// All four lane accumulators after `FUN_0040fbe0`.
    pub postprocess: FourWheelPostprocessState,
    /// Independent state for the complete four-wheel producer path.
    pub four_wheel: FourWheelState,
    /// The live ground-contact words, present once a stage's compiled ground
    /// query has seeded them; drives the per-tick ground pass `FUN_00410a60`.
    pub ground: Option<GroundContact>,
}

impl CarState {
    /// A car at the world origin, at rest, facing +x (heading 0).
    pub const fn at_rest() -> Self {
        CarState {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            heading: 0,
            motion: MotionState::harness_default(),
            collision: LiveCollisionState {
                pending_linear_3cc: Vec3::ZERO,
                history: StaticContactHistory {
                    previous_ids: [0; crate::static_world_collision::STATIC_CONTACT_CAPACITY],
                    previous_len: 0,
                    current_ids: [0; crate::static_world_collision::STATIC_CONTACT_CAPACITY],
                    current_len: 0,
                    current_overflow: false,
                },
                object_history: StaticContactHistory {
                    previous_ids: [0; crate::static_world_collision::STATIC_CONTACT_CAPACITY],
                    previous_len: 0,
                    current_ids: [0; crate::static_world_collision::STATIC_CONTACT_CAPACITY],
                    current_len: 0,
                    current_overflow: false,
                },
                response_state_680: false,
                recovery_latch_688: false,
                out_of_control_axis_bc: Vec3::ZERO,
            },
            first_wheel: FirstWheelState {
                force: 0,
                wheel_speed: 0,
            },
            second_wheel: SecondWheelState {
                ratio_at_3b0: 0,
                angle: 0,
                component: 0,
                accumulators: [0; 3],
            },
            postprocess: FourWheelPostprocessState {
                accumulators: [Vec3::ZERO; 4],
                qualified: [false; 4],
                angle_measures: [Fixed16::ZERO; 4],
            },
            four_wheel: FourWheelState {
                first: [FirstWheelState {
                    force: 0,
                    wheel_speed: 0,
                }; 4],
                second: [SecondWheelState {
                    ratio_at_3b0: 0,
                    angle: 0,
                    component: 0,
                    accumulators: [0; 3],
                }; 4],
                smoothed_wheel_speeds: [0; 4],
                postprocess: FourWheelPostprocessState {
                    accumulators: [Vec3::ZERO; 4],
                    qualified: [false; 4],
                    angle_measures: [Fixed16::ZERO; 4],
                },
                contact_updates: [Vec3::ZERO; 8],
                speed_ec: 0,
                residual_enable: 10,
                engine_2e0: 0,
                engine_speed_2e8: 0,
                limiter_444: false,
                limiter_cut_448: 0,
                engine_force_530: 0,
                idle_6e8: 0,
                idle_countdown_6ec: 0,
                pedals: PedalResponseState::ZERO,
                steering: SteeringState::ZERO,
                gearbox: GearboxState::ZERO,
                upright_404: false,
                resting_700: false,
                start_basis_27c: [Vec3::ZERO; 3],
                wheel_terms: WheelSurfaceTerms {
                    term_278: [0; 4],
                    term_288: [0; 4],
                    term_298: [0; 4],
                },
                tyre_gains: TyreGains {
                    gain_31c: [0; 8],
                    gain_33c: [0; 8],
                    gain_35c: [0; 4],
                    gain_36c: [0; 4],
                },
                support_3fc: [Vec3::ZERO; 8],
                tumble: TumbleState {
                    righting_3ec: false,
                    roof_3f4: false,
                    side_3f8: false,
                    end_3fc: false,
                    roof_ticks_43c: 0,
                    side_ticks_43d: 0,
                    end_ticks_43e: 0,
                    hull_ticks_678: 0,
                    inverted_684: false,
                    lowest_wheel_64c: 0,
                    lowest_wheel_650: 0,
                },
                contact_vectors_4bc: [Vec3::ZERO; 8],
                lateral_dot_3c0: [0; 4],
                rollover: RolloverState {
                    slope_ticks_734: 0,
                    roll_610: 0,
                    roll_ticks_614: 0,
                    roll_angle_60c: 0,
                },
                previous_normal_330: Vec3::ZERO,
                prior_velocity_e0: Vec3::ZERO,
                previous_wheel_normals_39c: [[0; 3]; 4],
                reset: ResetFadeState {
                    active_41c: false,
                    countdown_424: 0,
                    fade_428: 0,
                    penalty_460: false,
                },
                saved_position_3e4: Vec3::ZERO,
                trigger_prior_15c: [[0; 3]; 8],
                barrier_impact_3f0: [0; 3],
                head_vectors_380: [Vec3::ZERO; 4],
                barrier_contact_6dc: false,
                trigger_walk: TriggerWalkState {
                    selected_probe_574: 0,
                    previous_contact_6e0: false,
                    penalty_timer_75c: 0,
                },
                wheels: crate::wheel_animation::WheelAnimationState::REST,
            },
            ground: None,
        }
    }

    /// Seed this [`CarState`] from a replay Region-A initial-state block
    /// (`FUN_0046d460`: `FUN_0046d150` control restore `+0xcd0..+0x1134` and
    /// `FUN_0046d1f0` physics restore `+0x11f4..+0x195c`).
    pub fn seed_from_replay_region_a_46d460(&mut self, view: crate::replay::ReplayRegionAView<'_>) {
        let phys = view.physics_bytes();
        let ctl = view.control_bytes();
        let p_i32 = |off: usize| i32::from_le_bytes(phys[off..off + 4].try_into().unwrap());
        let p_u16 = |off: usize| u16::from_le_bytes(phys[off..off + 2].try_into().unwrap());
        let p_i16 = |off: usize| i16::from_le_bytes(phys[off..off + 2].try_into().unwrap());
        let p_raw3 = |off: usize| [p_i32(off), p_i32(off + 4), p_i32(off + 8)];
        let p_vec3 = |off: usize| {
            Vec3::new(
                Fixed16::from_raw(p_i32(off)),
                Fixed16::from_raw(p_i32(off + 4)),
                Fixed16::from_raw(p_i32(off + 8)),
            )
        };
        let c_i32 = |off: usize| i32::from_le_bytes(ctl[off..off + 4].try_into().unwrap());
        let c_u16 = |off: usize| u16::from_le_bytes(ctl[off..off + 2].try_into().unwrap());
        let c_i16 = |off: usize| i16::from_le_bytes(ctl[off..off + 2].try_into().unwrap());
        let c_vec3 = |off: usize| {
            Vec3::new(
                Fixed16::from_raw(c_i32(off)),
                Fixed16::from_raw(c_i32(off + 4)),
                Fixed16::from_raw(c_i32(off + 8)),
            )
        };

        self.position = p_vec3(0xc8);
        self.velocity = p_vec3(0xd4);
        self.motion = MotionState {
            basis: [p_vec3(0x258), p_vec3(0x264), p_vec3(0x270)],
            projection_axis: p_vec3(0x324),
            angular_velocity: p_vec3(0xf0),
            moment: p_vec3(0x3d8),
            compiled_half_words: p_vec3(0x2c),
            lane_source_s: p_vec3(0x5d4),
            contact_distance_flags: std::array::from_fn(|i| p_i32(0x68c + i * 4) != 0),
            contact_count_674: p_i32(0x674),
            upper_hull_deformation: UpperHullDeformationScalars {
                scale_638: Fixed16::from_raw(p_i32(0x638)),
                scale_63c: Fixed16::from_raw(p_i32(0x63c)),
                scale_640: Fixed16::from_raw(p_i32(0x640)),
            },
        };
        self.collision.pending_linear_3cc = p_vec3(0x3cc);
        self.collision.response_state_680 = p_i32(0x680) != 0;
        self.collision.recovery_latch_688 = p_i32(0x688) != 0;
        self.collision.out_of_control_axis_bc = p_vec3(0xbc);

        let wheel = &mut self.four_wheel;
        wheel.first = std::array::from_fn(|w| FirstWheelState {
            force: p_i32(0x534 + w * 4),
            wheel_speed: c_i32(0x2a8 + w * 4),
        });
        for w in 0..4 {
            wheel.second[w].ratio_at_3b0 = c_i32(0x3b0 + w * 4);
        }
        wheel.smoothed_wheel_speeds = std::array::from_fn(|w| c_i32(0x2b8 + w * 4));
        wheel.contact_updates = std::array::from_fn(|i| p_vec3(0x45c + i * 12));
        wheel.speed_ec = p_i32(0xec);
        wheel.residual_enable = ctl[0x43b];
        wheel.engine_2e0 = c_i32(0x2e0);
        wheel.engine_speed_2e8 = c_i32(0x2e8);
        wheel.limiter_444 = c_i32(0x444) != 0;
        wheel.limiter_cut_448 = c_i32(0x448);
        wheel.engine_force_530 = p_i32(0x530);
        wheel.idle_6e8 = p_i32(0x6e8);
        wheel.idle_countdown_6ec = p_i16(0x6ec);
        wheel.pedals = PedalResponseState {
            response_2d8: c_i32(0x2d8),
            throttle_ramp_2dc: c_i32(0x2dc),
            brake_response_2ec: c_i32(0x2ec),
            brake_ramp_2f0: c_i32(0x2f0),
            handbrake_torque_2f4: c_i32(0x2f4),
            handbrake_ramp_2f8: c_i32(0x2f8),
        };
        wheel.steering = SteeringState {
            position_318: c_i32(0x318),
            direction_114: c_i32(0x114),
            angle_3e0: c_i16(0x3e0),
            target_3e2: c_i16(0x3e2),
        };
        wheel.gearbox = GearboxState {
            gear_43a: ctl[0x43a] as i8,
            target_43f: ctl[0x43f] as i8,
            shift_40c: c_i32(0x40c) != 0,
            cooldown_441: ctl[0x441] as i8,
            last_direction_442: ctl[0x442],
            skip_443: ctl[0x443] as i8,
            reverse_mode_408: c_i32(0x408) != 0,
            upshift_hint_420: c_i32(0x420) != 0,
        };
        wheel.upright_404 = c_i32(0x404) != 0;
        wheel.resting_700 = p_i32(0x700) != 0;
        wheel.start_basis_27c = [p_vec3(0x27c), p_vec3(0x288), p_vec3(0x294)];
        wheel.wheel_terms = WheelSurfaceTerms {
            term_278: std::array::from_fn(|w| c_i32(0x278 + w * 4)),
            term_288: std::array::from_fn(|w| c_i32(0x288 + w * 4)),
            term_298: std::array::from_fn(|w| c_i32(0x298 + w * 4)),
        };
        wheel.tyre_gains = TyreGains {
            gain_31c: std::array::from_fn(|i| c_i32(0x31c + i * 4)),
            gain_33c: std::array::from_fn(|i| c_i32(0x33c + i * 4)),
            gain_35c: std::array::from_fn(|i| c_i32(0x35c + i * 4)),
            gain_36c: std::array::from_fn(|i| c_i32(0x36c + i * 4)),
        };
        wheel.support_3fc = std::array::from_fn(|i| p_vec3(0x3fc + i * 12));
        wheel.tumble = TumbleState {
            righting_3ec: c_i32(0x3ec) != 0,
            roof_3f4: c_i32(0x3f4) != 0,
            side_3f8: c_i32(0x3f8) != 0,
            end_3fc: c_i32(0x3fc) != 0,
            roof_ticks_43c: ctl[0x43c] as i8,
            side_ticks_43d: ctl[0x43d] as i8,
            end_ticks_43e: ctl[0x43e] as i8,
            hull_ticks_678: p_i32(0x678),
            inverted_684: p_i32(0x684) != 0,
            lowest_wheel_64c: p_i32(0x64c),
            lowest_wheel_650: phys[0x650] as i8,
        };
        wheel.contact_vectors_4bc = std::array::from_fn(|i| p_vec3(0x4bc + i * 12));
        wheel.lateral_dot_3c0 = std::array::from_fn(|w| c_i32(0x3c0 + w * 4));
        wheel.rollover = RolloverState {
            slope_ticks_734: p_u16(0x734),
            roll_610: p_i32(0x610),
            roll_ticks_614: p_i16(0x614),
            roll_angle_60c: p_i16(0x60c),
        };
        wheel.previous_normal_330 = p_vec3(0x330);
        wheel.prior_velocity_e0 = p_vec3(0xe0);
        wheel.previous_wheel_normals_39c = std::array::from_fn(|w| p_raw3(0x39c + w * 12));
        wheel.reset = ResetFadeState {
            active_41c: c_i32(0x41c) != 0,
            countdown_424: c_i32(0x424),
            fade_428: ctl[0x428],
            penalty_460: c_i32(0x460) != 0,
        };
        wheel.saved_position_3e4 = p_vec3(0x3e4);
        wheel.trigger_prior_15c = std::array::from_fn(|i| p_raw3(0x15c + i * 12));
        wheel.barrier_impact_3f0 = p_raw3(0x3f0);
        wheel.head_vectors_380 = std::array::from_fn(|i| c_vec3(0x380 + i * 12));
        wheel.barrier_contact_6dc = p_i32(0x6dc) != 0;
        wheel.trigger_walk.previous_contact_6e0 = p_i32(0x6e0) != 0;
        wheel.trigger_walk.penalty_timer_75c = p_i16(0x75c);
        wheel.wheels.compression_55c = std::array::from_fn(|w| p_i32(0x55c + w * 4));
        wheel.wheels.eased_56c = std::array::from_fn(|w| p_i32(0x56c + w * 4));
        wheel.wheels.vibration_57c = std::array::from_fn(|w| p_u16(0x57c + w * 2));
        wheel.wheels.bounce_584 = std::array::from_fn(|w| p_i32(0x584 + w * 4));
        wheel.wheels.travel_594 = std::array::from_fn(|w| p_i32(0x594 + w * 4));
        wheel.wheels.roll_state_88 = [c_i32(0x88), c_i32(0x8c), c_i32(0x90)];
        wheel.wheels.corner_roll_c8 = std::array::from_fn(|w| c_i32(0xc8 + w * 4));
        wheel.wheels.wheel_load_654 = std::array::from_fn(|w| p_i32(0x654 + w * 4));
        wheel.wheels.wobble_phase_604 = std::array::from_fn(|w| p_u16(0x604 + w * 2));
        wheel.wheels.wobble_5e4 =
            std::array::from_fn(|w| [p_i32(0x5e4 + w * 8), p_i32(0x5e8 + w * 8)]);

        let lane_inputs = self.ground.map_or(
            crate::compiled_car_initializer::CompiledSetupPoseLaneInputs {
                gate_words: [0; 8],
                drive_words: [0; 8],
                first_four_subtractions: [0; 4],
            },
            |g| g.lane_inputs,
        );
        self.ground = Some(GroundContact {
            lanes: crate::compiled_car_initializer::CompiledEightLaneExecutionState {
                handles: std::array::from_fn(|i| c_i16(0x3d0 + i * 2)),
                type_words: std::array::from_fn(|i| c_u16(0x42a + i * 2)),
                output_vectors: std::array::from_fn(|i| p_raw3(0x33c + i * 12)),
                lane_words: std::array::from_fn(|i| p_i32(0x2e4 + i * 4)),
                accumulator_words: std::array::from_fn(|i| p_i32(0x618 + i * 4)),
                latch_67c: p_i32(0x67c),
            },
            lane_inputs,
            half_words: p_raw3(0x2c),
            deformation: p_raw3(0x638),
            probes: std::array::from_fn(|i| p_raw3(0xfc + i * 12)),
            thresholds: CompiledLaneThresholdEffects {
                flags_73a: std::array::from_fn(|i| phys[0x73a + i]),
                count_742: phys[0x742],
                flag_743: phys[0x743],
                indices_744: std::array::from_fn(|i| phys[0x744 + i]),
            },
            wheel_reach_648_644: [p_i32(0x648), p_i32(0x644)],
            reach_20: p_i32(0x20),
            radius_44: p_i32(0x44),
        });
    }
}

/// What one [`RaceState::run_reset`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetOutcome {
    /// The dispatcher's write of `DAT_0057ae70[player]`.
    pub cursor: Option<i32>,
    /// The car was placed on the reset target this tick.
    pub placed: bool,
    /// `DAT_00551560[player]`: the mechanics routine snaps the camera
    /// (`FUN_00434870`) before its next update.
    pub camera_reset: bool,
    /// The screen fade level `ctl +0x428`.
    pub fade_428: u8,
    /// The penalty countdown reached zero (sound cue `0x21c0000`).
    pub penalty_cue: bool,
    /// A penalty reset ended (car `+0x760 = 1`).
    pub penalty_done_760: bool,
}

/// A snapshot emitted per tick for the trace.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TraceSample {
    /// The stage clock after the tick (Q16.16 seconds).
    pub clock: Fixed16,
    /// Elapsed time in centiseconds since the trace started.
    pub centiseconds: i32,
    /// The car state after the tick.
    pub car: CarState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveStaticCollisionStepError<E> {
    ActiveCell(ActiveCellRecordBuildError),
    Frame(LiveStaticCollisionFrameBridgeError),
    Execute(StaticCollisionExecutionError<StaticKernelError<E>>),
    MovingObject(MovingObjectCollisionError<E>),
}

/// The deterministic offline race state: a stage clock plus one car, stepped at
/// the recovered fixed 40 ms / 25 Hz tick.
#[derive(Clone)]
pub struct RaceState {
    clock: Fixed16,
    start_clock: Fixed16,
    car: CarState,
    trig: [i32; TRIG_TABLE_LEN],
    /// The tumbling impact of the last ground pass, for its sound.
    last_tumble_impact: Option<TumbleImpact>,
}

/// A tumbling contact's impact-sound inputs (`FUN_00412860`, `0x413990`):
/// the `+0x3cc` vector `FUN_0041f630` measures and the world contact point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TumbleImpact {
    pub severity_vector: [i32; 3],
    pub point: [i32; 3],
    /// The car-local contact `car +0x3f0` the damage dents around.
    pub contact_3f0: [i32; 3],
}

impl RaceState {
    /// Start a race from a given car state with the stage clock at zero.
    pub fn new(car: CarState) -> Self {
        RaceState {
            clock: Fixed16::ZERO,
            start_clock: Fixed16::ZERO,
            car,
            trig: build_sine_table(),
            last_tumble_impact: None,
        }
    }

    /// The tumbling impact of the last ground pass, if any.
    #[must_use]
    pub fn last_tumble_impact(&self) -> Option<TumbleImpact> {
        self.last_tumble_impact
    }

    /// The current stage clock (Q16.16 seconds).
    pub fn clock(&self) -> Fixed16 {
        self.clock
    }

    /// The current car state.
    pub fn car(&self) -> CarState {
        self.car
    }

    /// Store the four wheel surface words `ctl +0x42a`, which the surface
    /// sound `FUN_00489890` rewrites after Mechanics.
    pub fn set_surface_words(&mut self, surfaces: [i16; 4]) {
        if let Some(ground) = self.car.ground.as_mut() {
            for (word, surface) in ground.lanes.type_words.iter_mut().zip(surfaces) {
                *word = surface as u16;
            }
        }
    }

    /// Store `car +0x6ec`, which the engine sound's reset-countdown branch
    /// (`FUN_00488d60`) rewrites after Mechanics.
    pub fn set_idle_countdown_6ec(&mut self, countdown: i16) {
        self.car.four_wheel.idle_countdown_6ec = countdown;
    }

    /// Hold the car in Neutral at the start line during the pre-race orbit and
    /// 5-4-3-2-1 countdown (`FUN_00439950` / `FUN_00439b10`), keeping the stage
    /// clock frozen at `00:00.00`.
    pub fn hold_start_line_neutral(&mut self) {
        self.clock = Fixed16::ZERO;
        self.start_clock = Fixed16::ZERO;
        self.car.four_wheel.gearbox.gear_43a = 0;
        self.car.four_wheel.gearbox.target_43f = 0;
        self.car.four_wheel.gearbox.shift_40c = false;
        self.car.four_wheel.gearbox.reverse_mode_408 = false;
    }

    /// Execute the start-line launch at `"GO!"` (`0x439d4f..0x439d66`,
    /// `FUN_00407590`): shift into 1st gear, transfer the revved neutral engine
    /// speed (`+0x2e0`) into the driven wheels via `inverse_1f0[1]`, arm the
    /// 10-tick launch residual window (`+0x43b = 10`), and start the stage clock
    /// from zero.
    pub fn launch_from_start_line_407590(&mut self, wheel: &FourWheelIterationParams<'_>) {
        self.clock = Fixed16::ZERO;
        self.start_clock = Fixed16::ZERO;
        self.car.four_wheel.gearbox.gear_43a = 1;
        self.car.four_wheel.gearbox.target_43f = 1;
        self.car.four_wheel.gearbox.shift_40c = false;
        self.car.four_wheel.gearbox.reverse_mode_408 = false;
        let response_1f0 = wheel
            .gearbox
            .map_or(0x4000, |gb| gb.tables.inverse_1f0[1]);
        let launched = Fixed16::from_raw(self.car.four_wheel.engine_2e0)
            .mul(Fixed16::from_raw(response_1f0))
            .raw();
        match wheel.first[0].config_158 {
            0 => {
                for w in 2..4 {
                    self.car.four_wheel.first[w].wheel_speed = launched;
                    self.car.four_wheel.smoothed_wheel_speeds[w] = launched;
                }
            }
            0x10000 => {
                for w in 0..2 {
                    self.car.four_wheel.first[w].wheel_speed = launched;
                    self.car.four_wheel.smoothed_wheel_speeds[w] = launched;
                }
            }
            _ => {
                for w in 0..4 {
                    self.car.four_wheel.first[w].wheel_speed = launched;
                    self.car.four_wheel.smoothed_wheel_speeds[w] = launched;
                }
            }
        }
        self.car.four_wheel.residual_enable = 10;
    }

    /// Resolve one post-integration static-scene pass and atomically publish
    /// depenetration, alternate velocity, next-tick corrections, history, and
    /// persistent response flags into the live car.
    pub fn resolve_static_collision_after_step<D: StaticDamageOwner>(
        &mut self,
        previous_center_xz: PreviousCarXz,
        parsed: &ParsedStaticCollisionScene,
        damage_owner: &mut D,
    ) -> Result<StaticCollisionStep, LiveStaticCollisionStepError<D::Error>> {
        let car = self.car;
        let primary_cell =
            build_active_cell_record(&parsed.grid, &parsed.main_cell_centers, car.position)
                .map_err(LiveStaticCollisionStepError::ActiveCell)?;
        let deformation = car
            .motion
            .upper_hull_deformation
            .bind(car.motion.basis[0], car.motion.basis[2]);
        let live = build_live_static_collision_car_frame(LiveStaticCollisionCarFrameInput {
            previous_center_xz,
            current_center: car.position,
            current_basis: car.motion.basis,
            half_extents: car.motion.compiled_half_words,
            projection_axis_258: car.motion.basis[0],
            projection_axis_264: car.motion.basis[1],
            deformation: Some(deformation),
            primary_cell,
        });
        let frame = live
            .into_static_frame(
                car.collision.pending_linear_3cc,
                car.motion.moment,
                Vec3::ZERO,
                car.motion.compiled_half_words,
            )
            .map_err(LiveStaticCollisionStepError::Frame)?;
        let speed_ec =
            Fixed16::from_raw(sqrt_q16(car.velocity.dot(car.velocity).raw() as u32) as i32);
        let projection = live.projection;
        let context = StaticKernelContext {
            projected_corners: projection.projected_corners,
            projection_axis: projection.axis,
            projection_perpendicular: projection.perpendicular,
            projection_axis_radius: projection.axis_radius,
            projection_perpendicular_radius: projection.perpendicular_radius,
            // `FUN_0043f600` keeps the car's top in `DAT_00552f60` and its
            // bottom in `DAT_00552f80` (`0x43f58f..0x43f5ad`).
            y_gate_552f60: projection.max_y,
            y_gate_552f80: projection.min_y,
            velocity_d4: car.velocity,
            alternate_axis_324: car.motion.projection_axis,
            projected_axis_270: car.motion.basis[2],
            response_basis_2a0: ResponseBasis {
                rows: car.motion.basis,
            },
            out_of_control_680: car.collision.response_state_680,
            out_of_control_688: car.collision.recovery_latch_688,
            // The windowed compatibility session currently owns player slot
            // zero and the fresh-process subtype byte remains zero.
            car_type_738: 0,
            car_subtype_739: 0,
            contact_count_674: car.motion.contact_count_674,
            velocity_normalizer_ec: speed_ec,
            fallback_axis_258: car.motion.basis[0],
        };
        let mut kernel =
            RecoveredStaticCollisionKernel::with_damage_owner(context, [], damage_owner);
        let effects = resolve_static_world_collisions(
            &parsed.scene,
            &frame,
            car.collision.history,
            &mut kernel,
        )
        .map_err(LiveStaticCollisionStepError::Execute)?;

        let mut committed = car;
        committed.collision.pending_linear_3cc = effects.linear_impulse_3cc;
        committed.motion.moment = effects.angular_impulse_3d8;
        committed.collision.history = effects.history;
        if let Some(kernel_state) = kernel.state() {
            committed.position = kernel_state.position;
            committed.velocity = kernel_state.velocity_d4;
            committed.collision.response_state_680 = kernel_state.out_of_control_680;
            committed.collision.recovery_latch_688 = kernel_state.out_of_control_688;
        }
        let contacts = effects.contacts.len();
        // FUN_0043d760's impact sound inputs (`FUN_00469e80`), in hit order.
        let impacts = kernel
            .hits()
            .iter()
            .filter_map(|hit| match hit {
                crate::static_collision_kernel::StaticKernelHit::Normal { response, .. } => {
                    Some(response.events.iter().filter_map(|event| match *event {
                        crate::static_collision_response::StaticResponseEvent::MaterialImpact {
                            severity,
                            world_contact,
                            material,
                        } => Some(StaticMaterialImpact {
                            severity: severity.raw(),
                            world_contact: [
                                world_contact.x.raw(),
                                world_contact.y.raw(),
                                world_contact.z.raw(),
                            ],
                            material,
                        }),
                        _ => None,
                    }))
                }
                crate::static_collision_kernel::StaticKernelHit::Alternate { .. } => None,
            })
            .flatten()
            .collect();
        self.car = committed;
        let contact_effects = kernel
            .hits()
            .iter()
            .filter_map(|hit| match hit {
                crate::static_collision_kernel::StaticKernelHit::Normal { response, .. } => {
                    Some(response.events.iter().filter_map(|event| match *event {
                        crate::static_collision_response::StaticResponseEvent::ContactEffect {
                            handle,
                            world_contact,
                        } => Some((
                            handle,
                            [
                                world_contact.x.raw(),
                                world_contact.y.raw(),
                                world_contact.z.raw(),
                            ],
                        )),
                        _ => None,
                    }))
                }
                crate::static_collision_kernel::StaticKernelHit::Alternate { .. } => None,
            })
            .flatten()
            .collect();
        let debug_material_558fb0 = kernel
            .hits()
            .iter()
            .filter_map(|hit| match hit {
                crate::static_collision_kernel::StaticKernelHit::Normal { response, .. } => {
                    response.debug_material_write
                }
                crate::static_collision_kernel::StaticKernelHit::Alternate { .. } => None,
            })
            .next_back();
        Ok(StaticCollisionStep {
            contacts,
            impacts,
            contact_effects,
            debug_material_558fb0,
        })
    }

    /// The 5120-word Q16.16 sine/cosine table (`DAT_00557940`).
    #[must_use]
    pub const fn trig(&self) -> &[i32; TRIG_TABLE_LEN] {
        &self.trig
    }

    /// Resolve the current car state against the stage's dynamic trackside
    /// objects (`FUN_0043f790`, `0x43f790..0x43fa09`).
    ///
    /// Runs right after [`Self::resolve_static_collision_after_step`] and
    /// returns the impact/breakage cues plus `DAT_00552f68` (whether the car
    /// touched any moving object this tick).
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_moving_object_collision_after_step<D: StaticDamageOwner>(
        &mut self,
        previous_center_xz: PreviousCarXz,
        parsed: &ParsedStaticCollisionScene,
        objects: &mut MovingObjects,
        hit_cache: &mut MovingObjectHitCache,
        double_rate_551af0: bool,
        mass_48: Fixed16,
        damage_owner: &mut D,
    ) -> Result<(StaticCollisionStep, bool), LiveStaticCollisionStepError<D::Error>> {
        let car = self.car;
        let primary_cell =
            build_active_cell_record(&parsed.grid, &parsed.main_cell_centers, car.position)
                .map_err(LiveStaticCollisionStepError::ActiveCell)?;
        let deformation = car
            .motion
            .upper_hull_deformation
            .bind(car.motion.basis[0], car.motion.basis[2]);
        let live = build_live_static_collision_car_frame(LiveStaticCollisionCarFrameInput {
            previous_center_xz,
            current_center: car.position,
            current_basis: car.motion.basis,
            half_extents: car.motion.compiled_half_words,
            projection_axis_258: car.motion.basis[0],
            projection_axis_264: car.motion.basis[1],
            deformation: Some(deformation),
            primary_cell,
        });
        let frame = live
            .into_static_frame(
                car.collision.pending_linear_3cc,
                car.motion.moment,
                Vec3::ZERO,
                car.motion.compiled_half_words,
            )
            .map_err(LiveStaticCollisionStepError::Frame)?;
        let speed_ec = sqrt_q16(car.velocity.dot(car.velocity).raw() as u32) as i32;
        let radius_44 = car.ground.map_or_else(
            || {
                sqrt_q16(
                    car.motion
                        .compiled_half_words
                        .dot(car.motion.compiled_half_words)
                        .raw() as u32,
                ) as i32
            },
            |g| g.radius_44,
        );
        let flag_743 = car.ground.map_or(0, |g| g.thresholds.flag_743);
        let raw_v = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        let v3 = |r: [i32; 3]| {
            Vec3::new(
                Fixed16::from_raw(r[0]),
                Fixed16::from_raw(r[1]),
                Fixed16::from_raw(r[2]),
            )
        };
        let mut car_state = MovingObjectCarState {
            position_c8: raw_v(car.position),
            previous_xz_3e4: [previous_center_xz.x.raw(), previous_center_xz.z.raw()],
            velocity_d4: raw_v(car.velocity),
            velocity_normalizer_ec: speed_ec,
            hull_points_fc: live.collision_hull.map(raw_v),
            radius_44,
            mass_48: mass_48.raw(),
            contact_limits_2c: raw_v(car.motion.compiled_half_words),
            axis_258: raw_v(car.motion.basis[0]),
            axis_264: raw_v(car.motion.basis[1]),
            projected_axis_270: raw_v(car.motion.basis[2]),
            basis_2a0: car.motion.basis.map(raw_v),
            linear_correction_3cc: raw_v(car.collision.pending_linear_3cc),
            angular_correction_3d8: raw_v(car.motion.moment),
            contact_3f0: [0; 3],
            contact_count_674: car.motion.contact_count_674,
            out_of_control_680: car.collision.response_state_680,
            out_of_control_688: car.collision.recovery_latch_688,
            flag_743,
            car_type_738: 0,
            car_subtype_739: 0,
            active_nodes_750: frame.active_nodes,
            object_history: car.collision.object_history,
        };
        let effects = resolve_moving_object_collisions(
            objects,
            &mut car_state,
            hit_cache,
            double_rate_551af0,
            damage_owner,
        )
        .map_err(LiveStaticCollisionStepError::MovingObject)?;
        let mut committed = car;
        committed.position = v3(car_state.position_c8);
        committed.velocity = v3(car_state.velocity_d4);
        committed.collision.pending_linear_3cc = v3(car_state.linear_correction_3cc);
        committed.motion.moment = v3(car_state.angular_correction_3d8);
        committed.collision.response_state_680 = car_state.out_of_control_680;
        committed.collision.recovery_latch_688 = car_state.out_of_control_688;
        committed.collision.object_history = car_state.object_history;
        self.car = committed;

        let contacts = effects.contacts.len();
        let impacts = effects
            .contacts
            .iter()
            .flat_map(|c| {
                c.events.iter().filter_map(|event| match *event {
                    crate::static_collision_response::StaticResponseEvent::MaterialImpact {
                        severity,
                        world_contact,
                        material,
                    } => Some(StaticMaterialImpact {
                        severity: severity.raw(),
                        world_contact: [
                            world_contact.x.raw(),
                            world_contact.y.raw(),
                            world_contact.z.raw(),
                        ],
                        material,
                    }),
                    _ => None,
                })
            })
            .collect();
        let contact_effects = effects
            .contacts
            .iter()
            .flat_map(|c| {
                c.events.iter().filter_map(|event| match *event {
                    crate::static_collision_response::StaticResponseEvent::ContactEffect {
                        handle,
                        world_contact,
                    } => Some((
                        handle,
                        [
                            world_contact.x.raw(),
                            world_contact.y.raw(),
                            world_contact.z.raw(),
                        ],
                    )),
                    _ => None,
                })
            })
            .collect();
        Ok((
            StaticCollisionStep {
                contacts,
                impacts,
                contact_effects,
                debug_material_558fb0: effects.debug_material_558fb0,
            },
            effects.object_contact_552f68,
        ))
    }

    /// Run `FUN_00442740` (`FUN_004438a0` -> `FUN_00442860` -> `FUN_00441f30`)
    /// between `self` (`car_a`) and `other` (`car_b`) after both cars' static
    /// and moving-object collision passes.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_car_vs_car_collision_after_step<D: StaticDamageOwner>(
        &mut self,
        other: &mut Self,
        previous_center_xz_a: PreviousCarXz,
        previous_center_xz_b: PreviousCarXz,
        parsed: &ParsedStaticCollisionScene,
        session: &mut CarVsCarSessionState,
        mode_d3ca88: i32,
        replay_car_count_550c08: i32,
        mass_a_48: Fixed16,
        mass_b_48: Fixed16,
        damage_owner: &mut D,
    ) -> Result<StaticCollisionStep, LiveStaticCollisionStepError<D::Error>> {
        let raw_v = |v: Vec3| [v.x.raw(), v.y.raw(), v.z.raw()];
        let v3 = |r: [i32; 3]| {
            Vec3::new(
                Fixed16::from_raw(r[0]),
                Fixed16::from_raw(r[1]),
                Fixed16::from_raw(r[2]),
            )
        };
        let build_car_state =
            |car: &CarState,
             prev_xz: PreviousCarXz,
             mass_48: Fixed16,
             car_type: i8|
             -> Result<CarVsCarState, LiveStaticCollisionStepError<D::Error>> {
                let primary_cell =
                    build_active_cell_record(&parsed.grid, &parsed.main_cell_centers, car.position)
                        .map_err(LiveStaticCollisionStepError::ActiveCell)?;
                let deformation = car
                    .motion
                    .upper_hull_deformation
                    .bind(car.motion.basis[0], car.motion.basis[2]);
                let live =
                    build_live_static_collision_car_frame(LiveStaticCollisionCarFrameInput {
                        previous_center_xz: prev_xz,
                        current_center: car.position,
                        current_basis: car.motion.basis,
                        half_extents: car.motion.compiled_half_words,
                        projection_axis_258: car.motion.basis[0],
                        projection_axis_264: car.motion.basis[1],
                        deformation: Some(deformation),
                        primary_cell,
                    });
                let radius_44 = car.ground.map_or_else(
                    || {
                        sqrt_q16(
                            car.motion
                                .compiled_half_words
                                .dot(car.motion.compiled_half_words)
                                .raw() as u32,
                        ) as i32
                    },
                    |g| g.radius_44,
                );
                let flag_743 = car.ground.map_or(0, |g| g.thresholds.flag_743);
                Ok(CarVsCarState {
                    position_c8: raw_v(car.position),
                    previous_xz_3e4: [prev_xz.x.raw(), prev_xz.z.raw()],
                    velocity_d4: raw_v(car.velocity),
                    hull_points_fc: live.collision_hull.map(raw_v),
                    radius_44,
                    mass_48: mass_48.raw(),
                    contact_limits_2c: raw_v(car.motion.compiled_half_words),
                    axis_258: raw_v(car.motion.basis[0]),
                    axis_264: raw_v(car.motion.basis[1]),
                    basis_2a0: car.motion.basis.map(raw_v),
                    linear_correction_3cc: raw_v(car.collision.pending_linear_3cc),
                    angular_correction_3d8: raw_v(car.motion.moment),
                    contact_3f0: [0; 3],
                    opponent_hit_6e4: false,
                    car_type_738: car_type,
                    car_subtype_739: 0,
                    flag_743,
                    active_cells_750: live.active_cells,
                })
            };

        let mut car_a = build_car_state(&self.car, previous_center_xz_a, mass_a_48, 0)?;
        let mut car_b = build_car_state(&other.car, previous_center_xz_b, mass_b_48, 1)?;
        let resolved = resolve_car_vs_car_collision_442740(
            &mut car_a,
            &mut car_b,
            session,
            mode_d3ca88,
            replay_car_count_550c08,
            damage_owner,
        )
        .map_err(|err| {
            LiveStaticCollisionStepError::MovingObject(
                crate::moving_object_collision::MovingObjectCollisionError::Damage(err),
            )
        })?;

        self.car.position = v3(car_a.position_c8);
        self.car.collision.pending_linear_3cc = v3(car_a.linear_correction_3cc);
        self.car.motion.moment = v3(car_a.angular_correction_3d8);

        other.car.position = v3(car_b.position_c8);
        other.car.collision.pending_linear_3cc = v3(car_b.linear_correction_3cc);
        other.car.motion.moment = v3(car_b.angular_correction_3d8);

        let Some(contact) = resolved else {
            return Ok(StaticCollisionStep::default());
        };
        let impacts = contact
            .events
            .iter()
            .filter_map(|event| match *event {
                crate::static_collision_response::StaticResponseEvent::MaterialImpact {
                    severity,
                    world_contact,
                    material,
                } => Some(StaticMaterialImpact {
                    severity: severity.raw(),
                    world_contact: [
                        world_contact.x.raw(),
                        world_contact.y.raw(),
                        world_contact.z.raw(),
                    ],
                    material,
                }),
                _ => None,
            })
            .collect();
        let contact_effects = contact
            .events
            .iter()
            .filter_map(|event| match *event {
                crate::static_collision_response::StaticResponseEvent::ContactEffect {
                    handle,
                    world_contact,
                } => Some((
                    handle,
                    [
                        world_contact.x.raw(),
                        world_contact.y.raw(),
                        world_contact.z.raw(),
                    ],
                )),
                _ => None,
            })
            .collect();
        Ok(StaticCollisionStep {
            contacts: 1,
            impacts,
            contact_effects,
            debug_material_558fb0: None,
        })
    }

    /// The unit forward vector for the current heading on the x/z plane.
    fn forward_unit(&self) -> Vec3 {
        Vec3::new(
            cos_bam(&self.trig, self.car.heading),
            Fixed16::ZERO,
            sin_bam(&self.trig, self.car.heading),
        )
    }

    /// Advance one fixed tick, composing the recovered formulas in their
    /// observed order: steer, build the forward vector, form the net force
    /// (drive force minus the recovered quadratic drag), integrate acceleration
    /// into velocity, apply the recovered half-delta position update, apply the
    /// lateral-grip constraint, and advance the stage clock.
    pub fn step(&mut self, controls: &Controls, params: &SimParams) -> TraceSample {
        let mut sample = self.step_with_optional_world_force(controls, params, None, false);
        self.finish_mechanics_tick();
        sample.car = self.car;
        sample
    }

    /// The end-of-tick clear the per-car mechanics driver `FUN_00407690` applies
    /// after `FUN_0040a4a0` and the drivetrain passes `FUN_0040d0c0`/`FUN_004088c0`:
    /// it zeroes `+0x3cc..+0x3e0`, so the next static collision pass accumulates
    /// both corrections from zero. The linear words are already consumed and
    /// cleared by the translation step; the angular moment `+0x3d8` is cleared
    /// here. Neither drivetrain pass reads `+0x3d8`, so the decayed value
    /// `FUN_0040a4a0` writes back is dead once the angular pass has used it.
    fn finish_mechanics_tick(&mut self) {
        self.car.motion.moment = Vec3::ZERO;
    }

    fn step_with_optional_world_force(
        &mut self,
        controls: &Controls,
        params: &SimParams,
        world_force: Option<Vec3>,
        contacts_active: bool,
    ) -> TraceSample {
        self.car.heading = self.car.heading.wrapping_add(controls.steer_rate as u16);
        let forward = self.forward_unit();

        let recovered_force_path = world_force.is_some();
        let drive_force = world_force.unwrap_or_else(|| forward.scale(controls.forward_force));
        let speed = self.car.velocity.length();
        let net_force = drive_force.sub(drag(self.car.velocity, speed));

        let accel = net_accel(net_force, params.inverse_mass, params.gravity);
        let pending_collision_acceleration = self.car.collision.consume_pending_linear();
        // `FUN_0040a4a0` (`0x40ab04..0x40ab21`) saves `velocity_d4` into
        // `prior_velocity_e0` right before the translation step integrates the
        // new velocity.
        self.car.four_wheel.prior_velocity_e0 = self.car.velocity;
        let translation = apply_translation_step(TranslationStepInput {
            position: self.car.position,
            velocity: self.car.velocity,
            cached_speed: if recovered_force_path {
                speed
            } else {
                Fixed16::ZERO
            },
            contacts_active,
            accumulated_force: accel,
            inverse_mass: Fixed16::ONE,
            additive_acceleration: pending_collision_acceleration,
        });
        self.car.position = translation.position;
        self.car.velocity = apply_lateral_grip(translation.velocity, forward, controls.handbrake);

        self.clock = advance_stage_clock(self.clock, stage_clock_step(false));

        TraceSample {
            clock: self.clock,
            centiseconds: elapsed_centiseconds(self.clock, self.start_clock),
            car: self.car,
        }
    }

    fn execute_first_wheel_state(
        state: &mut FirstWheelState,
        handbrake: bool,
        wheel: &FirstWheelParams,
    ) -> FirstWheelEffects {
        let effects = execute_first_wheel_iteration(FirstWheelInputs {
            force: state.force,
            wheel_speed: state.wheel_speed,
            handbrake: i32::from(handbrake),
            ..first_wheel_inputs(wheel)
        });
        *state = FirstWheelState {
            force: effects.force_at_540,
            wheel_speed: effects.wheel_speed,
        };
        effects
    }

    fn run_first_wheel(&mut self, handbrake: bool, wheel: &FirstWheelParams) -> FirstWheelEffects {
        Self::execute_first_wheel_state(&mut self.car.first_wheel, handbrake, wheel)
    }

    /// Run the recovered first-wheel force pass, then translate with its force.
    pub fn step_with_first_wheel(
        &mut self,
        controls: &Controls,
        params: &SimParams,
        wheel: &FirstWheelParams,
    ) -> TraceSample {
        let effects = self.run_first_wheel(controls.handbrake, wheel);
        self.step(
            &Controls {
                forward_force: Fixed16::from_raw(effects.force_at_540),
                ..*controls
            },
            params,
        )
    }

    fn execute_second_wheel_state(
        state: &mut SecondWheelState,
        prior_speed: i32,
        current_speed: i32,
        first: &FirstWheelParams,
        second: &SecondWheelParams<'_>,
    ) -> Result<SecondWheelCoreEffects, LookupDependentCandidateError> {
        let dot = (0..3).fold(0_i32, |sum, index| {
            sum.wrapping_add(
                Fixed16::from_raw(first.wheel_vector[index])
                    .mul(Fixed16::from_raw(first.state_vector[index]))
                    .raw(),
            )
        });
        let effects = execute_second_wheel_core(SecondWheelInputs {
            dot,
            speed_at_2a8: prior_speed,
            speed_at_2b8: current_speed,
            axis_dot: second.axis_dot,
            local_68: second.local_68,
            local_98: second.local_98,
            wheel_e8: second.wheel_e8,
            scale_c5c4: second.scale_c5c4,
            residual_flag_147a: second.residual_flag_147a,
            config_2ec: second.config_2ec,
            gear_ratio: second.gear_ratio,
            coefficient_34: second.coefficient_34,
            arctan_table: second.arctan_table,
            cos_table: second.cos_table,
            config_scale: second.config_scale,
            transition_gain: second.transition_gain,
            transition_base: second.transition_base,
            alternate_gain: second.alternate_gain,
            alternate_base: second.alternate_base,
            correction_gate: second.correction_gate,
            accumulation_limit: second.accumulation_limit,
            correction_alternate: second.correction_alternate,
            surface_vector: second.surface_vector,
            table_factor: second.table_factor,
        })?;
        *state = SecondWheelState {
            ratio_at_3b0: effects.ratio_at_3b0,
            angle: effects.angle,
            component: effects.component,
            accumulators: effects.accumulators,
        };
        Ok(effects)
    }

    fn run_second_wheel(
        &mut self,
        prior_speed: i32,
        first: &FirstWheelParams,
        second: &SecondWheelParams<'_>,
    ) -> Result<SecondWheelCoreEffects, LookupDependentCandidateError> {
        Self::execute_second_wheel_state(
            &mut self.car.second_wheel,
            prior_speed,
            self.car.first_wheel.wheel_speed,
            first,
            second,
        )
    }

    /// Compose the second-wheel core after the first iteration, then translate.
    ///
    /// # Errors
    ///
    /// Propagates a checked candidate-lookup failure.
    pub fn step_with_second_wheel(
        &mut self,
        controls: &Controls,
        params: &SimParams,
        first: &FirstWheelParams,
        second: &SecondWheelParams<'_>,
    ) -> Result<TraceSample, LookupDependentCandidateError> {
        let prior_speed = self.car.first_wheel.wheel_speed;
        let first_effects = self.run_first_wheel(controls.handbrake, first);
        self.run_second_wheel(prior_speed, first, second)?;
        Ok(self.step(
            &Controls {
                forward_force: Fixed16::from_raw(first_effects.force_at_540),
                ..*controls
            },
            params,
        ))
    }

    /// Feed the current second-core lane and the other three lanes through the
    /// recovered `FUN_0040fbe0` postprocess, preserving every lane accumulator.
    pub fn step_with_four_wheel_postprocess(
        &mut self,
        controls: &Controls,
        params: &SimParams,
        first: &FirstWheelParams,
        second: &SecondWheelParams<'_>,
        postprocess: &FourWheelPostprocessParams,
    ) -> Result<TraceSample, WheelPostprocessError> {
        if postprocess.current_lane >= postprocess.lanes.len() {
            return Err(WheelPostprocessError::LaneIndex(postprocess.current_lane));
        }
        let prior_speed = self.car.first_wheel.wheel_speed;
        self.run_first_wheel(controls.handbrake, first);
        self.run_second_wheel(prior_speed, first, second)?;
        let current = self.car.second_wheel.accumulators;
        let mut lanes = postprocess.lanes;
        lanes[postprocess.current_lane].accumulator = Vec3::new(
            Fixed16::from_raw(current[0]),
            Fixed16::from_raw(current[1]),
            Fixed16::from_raw(current[2]),
        );
        let outcome = apply_geometric_lane_postprocess(&mut lanes, postprocess.argument_scale)?;
        self.car.postprocess = FourWheelPostprocessState {
            accumulators: lanes.map(|lane| lane.accumulator),
            qualified: outcome.qualified,
            angle_measures: outcome.angle_measures,
        };
        self.car.second_wheel.accumulators = {
            let current = self.car.postprocess.accumulators[postprocess.current_lane];
            [current.x.raw(), current.y.raw(), current.z.raw()]
        };
        let force = sum_translation_lane_forces(
            postprocess.base_force,
            postprocess.non_wheel_forces,
            std::array::from_fn(|index| {
                (postprocess.wheel_contacts[index], lanes[index].accumulator)
            }),
        );
        let mut sample = self.step_with_optional_world_force(
            controls,
            params,
            Some(force),
            postprocess.wheel_contacts.contains(&true),
        );
        self.finish_mechanics_tick();
        sample.car = self.car;
        Ok(sample)
    }

    /// The per-tick words of the wheel-force producer's head that the harness
    /// path keeps fixed: `DAT_0050c5c4 = qmul(+0x52c, DAT_004eac70)` from the
    /// live contact-count share `+0x52c` (`FUN_0040d740`), and each wheel's
    /// slip downshift from the processed pedals and the residual countdown.
    fn apply_live_force_pass_head(
        &self,
        mass_48: Fixed16,
        pedals: PedalInputs,
        first: &mut [FirstWheelParams; 4],
        second: &mut [SecondWheelParams<'_>; 4],
    ) {
        let count_674 = u8::try_from(self.car.motion.contact_count_674).unwrap_or(0);
        let scale_52c = contact_count_scale(mass_48, count_674);
        let scale_c5c4 = scale_52c.mul(SOLVER_GLOBAL_CONSTANTS.force_scale).raw();
        let residual = self.car.four_wheel.residual_enable;
        for (wheel, (first, second)) in first.iter_mut().zip(second.iter_mut()).enumerate() {
            second.scale_c5c4 = scale_c5c4;
            let flag = wheel_slip_flag(residual, first.config_158, wheel >= 2);
            second.residual_flag_147a = flag;
            first.shift = wheel_slip_shift(i32::from(flag), pedals.throttle_304, pedals.brake_308);
        }
    }

    /// The driver's `DAT_004f777c` loop and `FUN_004095e0`, then the wheel
    /// loops' reads of their results: the first loop's slip limit and the
    /// second loop's gains and candidate bases. `ctl +0x258..+0x26c` and
    /// `car +0x654..` have no recovered writer and stay at zero.
    #[expect(clippy::too_many_arguments)]
    fn apply_tyre_loads(
        &mut self,
        tyre: TyreSurfaceInputs<'_>,
        ground: &GroundContact,
        thresholds: &CompiledLaneThresholdEffects,
        mass_48: Fixed16,
        normal_324: Vec3,
        first: &mut [FirstWheelParams; 4],
        second: &mut [SecondWheelParams<'_>; 4],
    ) -> Result<(), WheelPostprocessError> {
        let surface_words = ground.lanes.type_words.map(|word| word as i16);
        let wheel_surfaces: [i16; 4] = std::array::from_fn(|wheel| surface_words[wheel]);
        let four_wheel = &mut self.car.four_wheel;
        if tyre.surface_families.is_some() && four_wheel.wheels.wheel_load_654 == [0; 4] {
            four_wheel.wheels.wheel_load_654 = [mass_48.raw() >> 2; 4];
        }
        four_wheel.wheel_terms =
            derive_wheel_surface_terms(wheel_surfaces, tyre.variant_758, tyre.tables)?;
        let effects = apply_tyre_load_pass(
            &mut four_wheel.tyre_gains,
            &TyreLoadInputs {
                count_742: thresholds.count_742,
                mass_48,
                normal_324,
                upright_404: four_wheel.upright_404,
                flags_73a: thresholds.flags_73a,
                contact_68c: self.car.motion.contact_distance_flags,
                probe_normal_y: std::array::from_fn(|wheel| ground.lanes.output_vectors[wheel][1]),
                wheel_load_654: four_wheel.wheels.wheel_load_654,
                surface_words,
                term_268: [0; 4],
                term_278: four_wheel.wheel_terms.term_278,
                shared_250: 0,
                shared_254: 0,
                tables: tyre.tables,
            },
        )?;
        four_wheel.support_3fc = effects.support_3fc;
        let gains = four_wheel.tyre_gains;
        for wheel in 0..4 {
            let (transition_base, alternate_base) = wheel_candidate_bases(
                wheel,
                wheel_surfaces[wheel],
                0,
                four_wheel.wheel_terms.term_288[wheel],
                tyre.tables,
            )?;
            first[wheel].limit = gains.gain_36c[wheel];
            let input = &mut second[wheel];
            input.transition_gain = gains.gain_36c[wheel];
            input.transition_base = transition_base;
            input.alternate_gain = gains.gain_35c[wheel];
            input.alternate_base = alternate_base;
            input.coefficient_34 = alternate_base;
            input.config_scale = gains.gain_35c[wheel];
            input.accumulation_limit = gains.gain_35c[wheel];
            input.correction_alternate = gains.gain_36c[wheel];
            if let Some(families) = &tyre.surface_families
                && let Ok(index) = usize::try_from(wheel_surfaces[wheel])
                && let Some(&factor) = families.first.values.get(index)
            {
                input.table_factor = Some(factor);
            }
        }
        Ok(())
    }

    /// Rebuild contact vectors from the pre-step velocity, run all four first
    /// iterations in reverse order, equalize each axle, run all second iterations
    /// in reverse order, postprocess, update all eight contacts, then translate.
    ///
    /// # Errors
    ///
    /// Propagates contact-index, candidate lookup, gear, tyre-surface and
    /// geometric postprocess failures. A failed tick leaves the race state as
    /// it was before the call.
    pub fn step_with_all_four_wheels(
        &mut self,
        controls: &Controls,
        wheel: &FourWheelIterationParams<'_>,
    ) -> Result<TraceSample, WheelPostprocessError> {
        let (car, clock) = (self.car, self.clock);
        let result = self.step_with_all_four_wheels_unchecked(controls, wheel);
        if result.is_err() {
            (self.car, self.clock) = (car, clock);
        }
        result
    }

    fn step_with_all_four_wheels_unchecked(
        &mut self,
        controls: &Controls,
        wheel: &FourWheelIterationParams<'_>,
    ) -> Result<TraceSample, WheelPostprocessError> {
        if wheel.postprocess.current_lane >= wheel.postprocess.lanes.len() {
            return Err(WheelPostprocessError::LaneIndex(
                wheel.postprocess.current_lane,
            ));
        }
        // The driver saves the centre before the physics step (`+0x3e4`) and
        // advances the out-of-bounds timer `+0x75c` (`0x40771d..0x40785f`);
        // its expiry sets the latch and the slow penalty reset (`ctl +0x460`).
        self.car.four_wheel.saved_position_3e4 = self.car.position;
        let out_of_bounds = crate::collision::advance_out_of_bounds(
            self.car.four_wheel.trigger_walk.penalty_timer_75c,
            false,
            true,
        );
        self.car.four_wheel.trigger_walk.penalty_timer_75c = out_of_bounds.timer;
        if out_of_bounds.totalled {
            self.car.four_wheel.reset.penalty_460 = true;
            if let Some(ground) = self.car.ground.as_mut() {
                ground.lanes.latch_67c = 1;
            }
        }
        let contact_vectors =
            build_contact_basis_vectors(self.car.motion.contact_basis(self.car.velocity));
        self.car.four_wheel.contact_vectors_4bc = contact_vectors;
        // The driver arms the righting animation while it is idle
        // (`FUN_004102e0`, `0x4079dc`).
        if !self.car.four_wheel.tumble.righting_3ec {
            arm_righting(&mut self.car.four_wheel.tumble);
        }
        for (lane, vector_index) in wheel.contact_vector_indices.into_iter().enumerate() {
            if vector_index >= contact_vectors.len() {
                return Err(WheelPostprocessError::ContactVectorIndex { lane, vector_index });
            }
        }
        let compiled_live_path = wheel.gearbox.is_some() && wheel.tyre.is_some();
        if compiled_live_path {
            // `FUN_00407690` (`0x407df0..0x407efd`) clears `car + 0x45c..+0x4b8`
            // (`contact_updates[0..8]`) at the start of every tick.
            self.car.four_wheel.contact_updates = [Vec3::ZERO; 8];
        }
        // `FUN_00407690` Loop 1 (`0x407a9c..0x407aee`) decays `ctl + 0x43b` at
        // the start of the tick from the previous tick's wheel speeds and
        // `speed_ec`, before `FUN_0041b5b0` and `FUN_0040d810` consume it.
        self.car.four_wheel.residual_enable = advance_residual_enable(
            self.car.four_wheel.residual_enable,
            wheel.first[0].config_158,
            self.car.four_wheel.first[0].wheel_speed,
            self.car.four_wheel.first[2].wheel_speed,
            self.car.four_wheel.speed_ec,
        );
        self.car.four_wheel.start_basis_27c = self.car.motion.basis;
        let start_normal = self.car.motion.projection_axis;
        // With live ground contact, the threshold scan and the contact-distance
        // gate `FUN_0040d740` read the current lane words and probe points.
        let threshold_inputs = match &self.car.ground {
            Some(ground) => {
                let (flags, count) = classify_contact_distance_lanes(
                    std::array::from_fn(|lane| ground.lanes.lane_words[lane]),
                    std::array::from_fn(|lane| ground.probes[lane][1]),
                );
                self.car.motion.contact_distance_flags = flags;
                self.car.motion.contact_count_674 = i32::from(count);
                CompiledLaneThresholdInputs {
                    lane_words: ground.lanes.lane_words,
                    source_y: ground.probes.map(|probe| probe[1]),
                }
            }
            None => wheel.lane_threshold_inputs,
        };
        let thresholds = derive_compiled_lane_thresholds(threshold_inputs);
        let scale_528 =
            contact_count_scale(wheel.angular.momentum.moment_scale, thresholds.count_742);
        // The mechanics driver `FUN_00407690` cuts the steer magnitudes against
        // the previous tick's `+0xec`, then recomputes `+0x404` from the same
        // start-of-tick body axis, before any integrator runs.
        let steer = apply_low_speed_steer_cut(controls.raw_steer, self.car.four_wheel.speed_ec);
        // The control processor `FUN_0041b5b0` follows the cut. Its gearbox
        // runs first (only with a live gearbox) and decides this tick's gear,
        // shift request and reversing mode.
        let live_gear = match wheel.gearbox {
            Some(gearbox) => {
                let wheel_flags_73a = if self.car.ground.is_some() {
                    std::array::from_fn(|w| thresholds.flags_73a[w])
                } else {
                    gearbox.wheel_flags_73a
                };
                let state = apply_gearbox(
                    self.car.four_wheel.gearbox,
                    GearboxInputs {
                        mode_3f0: gearbox.mode_3f0,
                        gear_command_314: controls.raw_gear_command_314,
                        throttle_304: controls.raw_pedals.throttle_304,
                        brake_308: controls.raw_pedals.brake_308,
                        engine_2e0: self.car.four_wheel.engine_2e0,
                        wheel_flags_73a,
                        gear_two_skip: gearbox.gear_two_skip,
                        player_locked: gearbox.player_locked,
                        residual_43b: self.car.four_wheel.residual_enable,
                        brake_reverse_enabled: gearbox.brake_reverse_enabled,
                    },
                    &gearbox.tables,
                )?;
                let gear = i32::from(state.gear_43a);
                let ratio = usize::try_from(gear)
                    .ok()
                    .and_then(|index| gearbox.drive_ratios.get(index).copied())
                    .ok_or(GearOutOfRange { gear })?;
                self.car.four_wheel.gearbox = state;
                Some((gear, i32::from(state.shift_40c), ratio))
            }
            None => None,
        };
        let mode_408 = if wheel.gearbox.is_some() {
            i32::from(self.car.four_wheel.gearbox.reverse_mode_408)
        } else {
            wheel.driven_smoothing.mode_408
        };
        // Its steering
        // half turns the cut magnitudes into the wheel angle `+0x3e0`, then its
        // pedal half ramps the raw pedals into this tick's responses.
        let front_surface_factors = match (
            wheel.tyre.and_then(|t| t.surface_families),
            &self.car.ground,
        ) {
            (Some(families), Some(ground)) => [
                families
                    .second
                    .values
                    .get(usize::from(ground.lanes.type_words[1]))
                    .copied()
                    .unwrap_or(0),
                families
                    .second
                    .values
                    .get(usize::from(ground.lanes.type_words[0]))
                    .copied()
                    .unwrap_or(0),
            ],
            _ => wheel.steering_surface_factors,
        };
        self.car.four_wheel.steering = apply_steering(
            self.car.four_wheel.steering,
            wheel.steering,
            SteeringInputs {
                steer,
                analog_410: steer.analog_410,
                car_factor_e188: wheel.steering_car_factor_e188,
                front_surface_factors,
                forward_axis: self.car.motion.basis[0],
                velocity: self.car.velocity,
            },
        );
        let pedal_inputs = PedalInputs::from_raw(controls.raw_pedals, controls.handbrake, mode_408);
        self.car.four_wheel.pedals = apply_pedal_responses(
            self.car.four_wheel.pedals,
            pedal_inputs,
            wheel.pedal_response,
            &self.trig,
        )
        .expect("the race-state trig table is complete");
        let body_axis =
            transform_contact_force(self.car.motion.basis, self.car.motion.projection_axis);
        self.car.four_wheel.upright_404 = upright_flag_404(body_axis);
        let update_gate = contact_global_update_gate(body_axis);
        let mut contact_update_inputs = wheel.contact_update_inputs;
        for (index, (input, vector)) in contact_update_inputs
            .iter_mut()
            .zip(contact_vectors)
            .enumerate()
        {
            input.vector = vector;
            input.basis = self.car.motion.projection_axis;
            input.scale_528 = scale_528;
            input.extra_vector = Vec3::ZERO;
            input.update_gate = update_gate;
            input.enabled = if compiled_live_path {
                thresholds.flags_73a[index] == 0
            } else {
                thresholds.flags_73a[index] != 0
            };
        }
        let source_s = stabilize_lane_source_s(
            self.car.motion.lane_source_s,
            self.car.motion.basis[0],
            self.car.motion.basis[1],
            Fixed16::from_raw(self.car.four_wheel.pedals.response_2d8),
            wheel.lane_time_step,
        );
        self.car.motion.lane_source_s = source_s;
        let (source_p, source_r) = derive_lane_direction_pair(
            source_s,
            self.car.motion.basis[1],
            wheel
                .lane_direction_angle
                .wrapping_add(self.car.four_wheel.steering.angle_3e0),
            &self.trig,
        )
        .expect("the race-state trig table is complete");
        let directions = construct_lane_directions(
            LaneDirectionSources {
                projection_axis: self.car.motion.projection_axis,
                source_a0: source_p,
                source_b0: source_r,
                source_a2: source_s,
                cross_operand: self.car.motion.basis[1],
            },
            resolve_lane_direction_mode(wheel.lane_rebuild, wheel.lane_exchange),
        );
        self.car.four_wheel.head_vectors_380 = directions.published_unique_vectors();
        let mut first = wheel.first;
        for input in &mut first {
            input.flag_43b = i32::from(self.car.four_wheel.residual_enable);
            if let Some((gear, shift, ratio)) = live_gear {
                input.gear = gear;
                input.config_40c = shift;
                input.gear_ratio = ratio;
            }
        }
        let mut second = wheel.second.clone();
        for input in &mut second {
            // The second loop's gear-ratio gate reads the brake response.
            input.config_2ec = self.car.four_wheel.pedals.brake_response_2ec;
            if let Some((_, _, ratio)) = live_gear {
                input.gear_ratio = ratio;
                input.correction_gate = i32::from(self.car.four_wheel.gearbox.upshift_hint_420);
            }
        }
        if let (Some(tyre), Some(ground)) = (wheel.tyre, self.car.ground) {
            self.apply_live_force_pass_head(
                wheel.angular.momentum.moment_scale,
                pedal_inputs,
                &mut first,
                &mut second,
            );
            if compiled_live_path && wheel.suspension.up_e8 != 0 {
                let mass_48 = wheel.angular.momentum.moment_scale;
                let count_u8 = u8::try_from(self.car.motion.contact_count_674).unwrap_or(0);
                let scale_52c = contact_count_scale(mass_48, count_u8);
                crate::wheel_animation::step_body_roll_416760(
                    &mut self.car.four_wheel.wheels,
                    &crate::wheel_animation::BodyRollInput {
                        velocity_d4: self.car.velocity,
                        prior_velocity_e0: self.car.four_wheel.prior_velocity_e0,
                        start_basis_27c: self.car.four_wheel.start_basis_27c,
                        half_x_2c: ground.half_words[0],
                        half_z_34: ground.half_words[2],
                        mass_48: mass_48.raw(),
                        count_674: self.car.motion.contact_count_674,
                        scale_52c: scale_52c.raw(),
                        upright_404: self.car.four_wheel.upright_404,
                        resting_700: self.car.four_wheel.resting_700,
                        constants: wheel.suspension,
                    },
                );
            }
            self.apply_tyre_loads(
                tyre,
                &ground,
                &thresholds,
                wheel.angular.momentum.moment_scale,
                start_normal,
                &mut first,
                &mut second,
            )?;
        }
        for (lane, vector_index) in wheel.contact_vector_indices.into_iter().enumerate() {
            let Some(vector) = contact_vectors.get(vector_index) else {
                return Err(WheelPostprocessError::ContactVectorIndex { lane, vector_index });
            };
            second[lane].surface_vector = [vector.x.raw(), vector.y.raw(), vector.z.raw()];
            second[lane].local_68 = [
                directions.direction_a[lane].x.raw(),
                directions.direction_a[lane].y.raw(),
                directions.direction_a[lane].z.raw(),
            ];
            second[lane].local_98 = [
                directions.direction_b[lane].x.raw(),
                directions.direction_b[lane].y.raw(),
                directions.direction_b[lane].z.raw(),
            ];
            populate_wheel_vector_inputs(
                &mut first[lane],
                &second[lane],
                directions.direction_a[lane],
            );
        }
        // `FUN_0040ce40`: the engine force `+0x530`, then its distribution to
        // the four drive slots `+0x534..+0x540`, both before the wheel-force
        // producer `FUN_0040d810` reads those slots.
        let engine_force = derive_engine_force(EngineForceInput {
            limiter_444: self.car.four_wheel.limiter_444,
            cut_448: self.car.four_wheel.limiter_cut_448,
            engine_2e0: self.car.four_wheel.engine_2e0,
            redline_15c: wheel.engine_smoothing.redline_15c,
            gear: first[0].gear,
            config_40c: first[0].config_40c,
            response_2d8: self.car.four_wheel.pedals.response_2d8,
            throttle_304: pedal_inputs.throttle_304,
            ..wheel.engine_force
        });
        self.car.four_wheel.engine_force_530 = engine_force.force_530;
        self.car.four_wheel.engine_2e0 = engine_force.engine_2e0;
        let torques = distribute_drive_torque(
            engine_force.force_530,
            first[0].gear,
            first[0].config_40c != 0,
            first[0].gear_ratio,
            first[0].config_158,
        );
        for (state, torque) in self.car.four_wheel.first.iter_mut().zip(torques) {
            state.force = torque;
        }
        // `FUN_0040d810`'s brake torques replace each lane's drag word, then
        // the producer runs its grounded body only with contacts (`+0x674`)
        // and an upright car (`+0x404`); otherwise every wheel spins freely
        // with drag and the airborne factor, and no lane force is produced.
        let (front_brake, rear_brake) = wheel_brake_torques(WheelBrakeInput {
            throttle_304: pedal_inputs.throttle_304,
            brake_308: pedal_inputs.brake_308,
            gear: first[0].gear,
            bias_170: wheel.driven_smoothing.split_170,
            brake_response_2ec: self.car.four_wheel.pedals.brake_response_2ec,
            balance_e1a4: wheel.brake_balance_e1a4,
            handbrake_30c: controls.handbrake,
            handbrake_torque_2f4: self.car.four_wheel.pedals.handbrake_torque_2f4,
        });
        for (index, input) in first.iter_mut().enumerate() {
            input.split = if index < 2 { front_brake } else { rear_brake };
        }
        let grounded = self.car.motion.contact_count_674 != 0 && self.car.four_wheel.upright_404;
        let mut prior_speeds = [0; 4];
        let mut drive_forces_2c8 = [0; 4];
        for index in (0..4).rev() {
            prior_speeds[index] = self.car.four_wheel.first[index].wheel_speed;
            let state = &mut self.car.four_wheel.first[index];
            if grounded && self.car.motion.contact_distance_flags[index] {
                let inputs = FirstWheelInputs {
                    force: state.force,
                    wheel_speed: state.wheel_speed,
                    handbrake: i32::from(controls.handbrake),
                    ..first_wheel_inputs(&first[index])
                };
                let outcome = execute_first_wheel_iteration(inputs);
                drive_forces_2c8[index] = outcome.force_at_2d4;
                *state = FirstWheelState {
                    force: outcome.force_at_540,
                    wheel_speed: outcome.wheel_speed,
                };
                continue;
            }
            let inputs = FirstWheelInputs {
                force: state.force,
                wheel_speed: state.wheel_speed,
                handbrake: i32::from(controls.handbrake),
                ..first_wheel_inputs(&first[index])
            };
            let factor = if grounded {
                Fixed16::from_raw(first[index].factor_c5c0)
            } else {
                derive_alternate_force_pass_factor(wheel.lane_time_step)
            };
            let (force, speed) = spin_free_wheel(inputs, !grounded, factor);
            drive_forces_2c8[index] = force;
            *state = FirstWheelState {
                force,
                wheel_speed: speed,
            };
        }
        for axle in [0, 2] {
            let (left, right) = equalize_wheel_speeds(
                Fixed16::from_raw(self.car.four_wheel.first[axle].wheel_speed),
                Fixed16::from_raw(self.car.four_wheel.first[axle + 1].wheel_speed),
            );
            self.car.four_wheel.first[axle].wheel_speed = left.raw();
            self.car.four_wheel.first[axle + 1].wheel_speed = right.raw();
        }
        let mut lanes = wheel.postprocess.lanes;
        if compiled_live_path
            && let Some(ground) = &self.car.ground
        {
            for (lane, output) in lanes.iter_mut().enumerate() {
                output.active = self.car.motion.contact_distance_flags[lane];
                let n_cur = ground.lanes.output_vectors[lane];
                let n_prev = self.car.four_wheel.previous_wheel_normals_39c[lane];
                output.vector_a = Vec3::new(
                    Fixed16::from_raw(n_cur[0]),
                    Fixed16::from_raw(n_cur[1]),
                    Fixed16::from_raw(n_cur[2]),
                );
                output.vector_b = Vec3::new(
                    Fixed16::from_raw(n_prev[0]),
                    Fixed16::from_raw(n_prev[1]),
                    Fixed16::from_raw(n_prev[2]),
                );
                output.vector_c = self.car.four_wheel.contact_vectors_4bc[lane];
            }
        } else {
            for (lane, output) in lanes.iter_mut().enumerate() {
                output.vector_a = directions.direction_a[lane];
                output.vector_b = directions.direction_b[lane];
            }
        }
        for index in (0..4).rev() {
            if !grounded {
                break;
            }
            if compiled_live_path && !self.car.motion.contact_distance_flags[index] {
                lanes[index].accumulator = Vec3::ZERO;
                continue;
            }
            populate_derived_second_inputs(
                &first[index],
                &mut second[index],
                directions.direction_b[index],
                index,
            );
            self.car.four_wheel.lateral_dot_3c0[index] = second[index].axis_dot;
            let speed_at_2a8 = if compiled_live_path {
                self.car.four_wheel.first[index].wheel_speed
            } else {
                prior_speeds[index]
            };
            Self::execute_second_wheel_state(
                &mut self.car.four_wheel.second[index],
                speed_at_2a8,
                self.car.four_wheel.smoothed_wheel_speeds[index],
                &first[index],
                &second[index],
            )?;
            let current = self.car.four_wheel.second[index].accumulators;
            lanes[index].accumulator = Vec3::new(
                Fixed16::from_raw(current[0]),
                Fixed16::from_raw(current[1]),
                Fixed16::from_raw(current[2]),
            );
        }
        if grounded {
            let postprocess_scale = if compiled_live_path {
                Fixed16::from_raw(second[0].scale_c5c4)
            } else {
                wheel.postprocess.argument_scale
            };
            let outcome = apply_geometric_lane_postprocess(&mut lanes, postprocess_scale)?;
            self.car.four_wheel.postprocess = FourWheelPostprocessState {
                accumulators: lanes.map(|lane| lane.accumulator),
                qualified: outcome.qualified,
                angle_measures: outcome.angle_measures,
            };
        } else {
            // The driver clears the lane accumulators every tick, and the
            // airborne pass never writes them.
            for lane in &mut lanes {
                lane.accumulator = Vec3::ZERO;
            }
            self.car.four_wheel.postprocess = FourWheelPostprocessState {
                accumulators: [Vec3::ZERO; 4],
                qualified: [false; 4],
                angle_measures: [Fixed16::ZERO; 4],
            };
        }
        self.car.four_wheel.contact_updates = apply_contact_vector_updates(
            contact_update_inputs,
            self.car.four_wheel.contact_updates,
        );
        // `FUN_004095e0` replaces the driver's gravity base force with its
        // ground projection while a probe touches, using the slope grip of
        // the start-of-tick normal (`FUN_00408f50`).
        let base_force = grounded_base_force(
            wheel.postprocess.base_force,
            start_normal,
            slope_grip(start_normal.y),
            thresholds.count_742,
        );
        // The integrator head `FUN_0040a4a0`: the grip assist `FUN_0040b240`
        // and the rollover trigger `FUN_0040bfe0` run before the contact sum.
        // `+0x2a0` still holds the start-of-tick basis there.
        let published = self.car.four_wheel.start_basis_27c;
        let mut lane_accumulators = lanes.map(|lane| lane.accumulator);
        let assist = grip_assist(
            &GripAssistInput {
                contact_count_674: self.car.motion.contact_count_674,
                upright_404: self.car.four_wheel.upright_404,
                time_step: wheel.lane_time_step,
                basis: self.car.motion.basis,
                velocity: self.car.velocity,
                angular_velocity: self.car.motion.angular_velocity,
                speed_ec: self.car.four_wheel.speed_ec,
                steer_angle_3e0: self.car.four_wheel.steering.angle_3e0,
                steer_position_318: self.car.four_wheel.steering.position_318,
                handbrake_30c: controls.handbrake,
                steer_force_reversed: wheel.lane_exchange,
                normal: self.car.motion.projection_axis,
                previous_normal_330: self.car.four_wheel.previous_normal_330,
                crest_threshold_128: wheel.crest_128_12c[0],
                crest_gain_12c: wheel.crest_128_12c[1],
                front_accumulators: [lane_accumulators[0], lane_accumulators[1]],
                published,
            },
            &self.trig,
        );
        self.car.motion.basis = assist.basis;
        self.car.velocity = assist.velocity;
        self.car.motion.angular_velocity = assist.angular_velocity;
        [lane_accumulators[0], lane_accumulators[1]] = assist.front_accumulators;
        let wheel_surfaces = self.car.ground.map_or([0; 4], |ground| {
            std::array::from_fn(|w| ground.lanes.type_words[w] as i16)
        });
        let rollover = rollover_trigger(
            &mut self.car.four_wheel.rollover,
            self.car.collision.response_state_680,
            &RolloverInput {
                contact_count_674: self.car.motion.contact_count_674,
                count_742: thresholds.count_742,
                normal: self.car.motion.projection_axis,
                basis: self.car.motion.basis,
                published,
                velocity: self.car.velocity,
                base_force,
                inverse_mass_4c: wheel.inverse_mass_4c,
                // The last trigger walk's barrier contact.
                car_contact_6dc: self.car.four_wheel.barrier_contact_6dc,
                wheel_surfaces,
                roll_limits: wheel.tyre.map(|tyre| &tyre.tables.roll_limits_768c),
            },
        )?;
        if rollover.out_of_control {
            self.car.collision.response_state_680 = true;
        }
        if let Some(snapshot) = rollover.axis_snapshot_bc {
            self.car.collision.out_of_control_axis_bc = snapshot;
        }
        self.car.motion.moment = self.car.motion.moment.add(rollover.moment);
        let offsets: [Vec3; 8] = std::array::from_fn(|probe| {
            if probe < 4 {
                wheel.contact_offsets[probe]
            } else {
                wheel.hull_contact_offsets[probe - 4]
            }
        });
        let hull_accumulators =
            std::array::from_fn(|hull| self.car.four_wheel.contact_updates[hull + 4]);
        let (contact_force, contact_moment) = contact_force_and_moment(&ContactSumInput {
            base_force,
            support_3fc: self.car.four_wheel.support_3fc,
            lane_accumulators,
            hull_accumulators,
            count_742: thresholds.count_742,
            touching_744: thresholds.indices_744,
            contact_68c: self.car.motion.contact_distance_flags,
            offsets,
            published,
        });
        // Harness-only extra forces (empty on the compiled path).
        let force = wheel
            .postprocess
            .non_wheel_forces
            .iter()
            .fold(contact_force, |sum, extra| sum.add(*extra));
        // The integrator's own translation: damping by the previous `+0xec`,
        // inverse mass `+0x4c`, the pending collision correction, then the new
        // `+0xec`. The low-speed suppression is gated on the threshold scan's
        // `+0x742 > 0` (`0x40afcc`), not on the wheel flags. Unlike the planar
        // harness there is no heading-rate steering, no separate drag, and no
        // lateral grip: lateral and drive forces arrive through the lanes.
        // `FUN_0040a4a0` (`0x40ab04..0x40ab21`) saves `velocity_d4` into
        // `prior_velocity_e0` right before the translation step integrates the
        // new velocity.
        self.car.four_wheel.prior_velocity_e0 = self.car.velocity;
        let translation = apply_translation_step(TranslationStepInput {
            position: self.car.position,
            velocity: self.car.velocity,
            cached_speed: Fixed16::from_raw(self.car.four_wheel.speed_ec),
            contacts_active: thresholds.count_742 > 0,
            accumulated_force: force,
            inverse_mass: wheel.inverse_mass_4c,
            additive_acceleration: self.car.collision.consume_pending_linear(),
        });
        self.car.position = translation.position;
        self.car.velocity = translation.velocity;
        self.car.four_wheel.speed_ec = translation.speed.raw();
        let mut angular = wheel.angular;
        angular.basis = self.car.motion.basis;
        angular.momentum.angular_velocity = self.car.motion.angular_velocity;
        angular.momentum.prior_moment = self.car.motion.moment;
        angular.momentum.contact_moment = angular.momentum.contact_moment.add(contact_moment);
        let orientation = apply_angular_orientation_pipeline(
            angular,
            self.car.collision.out_of_control(),
            self.car.motion.projection_axis,
            ContactDeadzoneGate {
                contacts_active_742: thresholds.count_742 > 0,
                steer,
                upright_404: self.car.four_wheel.upright_404,
            },
            &self.trig,
        )
        .expect("the race-state trig table is complete");
        self.car.four_wheel.resting_700 =
            resting_flag_700(orientation.angle_increments, translation.speed);
        self.car.motion.basis = orientation.basis;
        self.car.motion.angular_velocity = orientation.angular_velocity;
        // The angular pass consumed the decayed moment
        // (`orientation.momentum.scaled_prior_moment`); the mechanics driver
        // then clears it.
        self.car
            .collision
            .set_out_of_control(orientation.out_of_control);
        self.clock = advance_stage_clock(self.clock, stage_clock_step(false));
        let mut sample = TraceSample {
            clock: self.clock,
            centiseconds: elapsed_centiseconds(self.clock, self.start_clock),
            car: self.car,
        };
        if first[0].gear_ratio != 0 && first[0].config_40c == 0 {
            self.car.four_wheel.engine_2e0 = integrate_engine_from_raw_wheels(
                self.car.four_wheel.engine_2e0,
                std::array::from_fn(|index| self.car.four_wheel.first[index].wheel_speed),
                first[0].config_158,
                first[0].gear_ratio,
            );
        } else if self.car.four_wheel.idle_6e8 == 0 && self.car.four_wheel.idle_countdown_6ec == 0 {
            self.car.four_wheel.engine_2e0 = integrate_idle_engine(
                self.car.four_wheel.engine_2e0,
                self.car.four_wheel.engine_force_530,
            );
        } else if self.car.four_wheel.idle_6e8 == 0 && self.car.four_wheel.idle_countdown_6ec > 0 {
            let oscillation = advance_idle_oscillation(
                wheel.engine_smoothing.redline_15c,
                self.car.four_wheel.idle_countdown_6ec,
                &self.trig,
            );
            self.car.four_wheel.engine_2e0 = oscillation.engine_2e0;
            self.car.four_wheel.idle_countdown_6ec = oscillation.countdown_6ec;
            if let Some(response) = oscillation.response_2d8 {
                self.car.four_wheel.pedals.response_2d8 = response;
            }
        } else if self.car.four_wheel.idle_6e8 != 0 {
            (
                self.car.four_wheel.idle_6e8,
                self.car.four_wheel.idle_countdown_6ec,
            ) = advance_idle_reset(
                self.car.four_wheel.idle_6e8,
                self.car.four_wheel.idle_countdown_6ec,
            );
        }
        let rev_limit = apply_rev_limiter(
            Fixed16::from_raw(self.car.four_wheel.engine_2e0),
            Fixed16::from_raw(wheel.engine_smoothing.redline_15c),
        );
        self.car.four_wheel.engine_2e0 = rev_limit.rpm.raw();
        self.car.four_wheel.limiter_444 = rev_limit.active;
        self.car.four_wheel.limiter_cut_448 = rev_limit.cut.raw();
        let mut driven_smoothing = DrivenWheelUpdateParams {
            mode_408,
            throttle_304: pedal_inputs.throttle_304,
            brake_308: pedal_inputs.brake_308,
            ..wheel.driven_smoothing
        };
        if let Some(gearbox) = &wheel.gearbox {
            let gear_idx = usize::try_from(first[0].gear).unwrap_or(0).min(7);
            driven_smoothing.scalars.suspension_298 = self.car.four_wheel.wheel_terms.term_298;
            driven_smoothing.scalars.longitudinal = self
                .car
                .velocity
                .dot(self.car.motion.basis[0])
                .raw()
                .wrapping_abs();
            driven_smoothing.scalars.gear_ratio_190 = gearbox.tables.upper_190[gear_idx];
            driven_smoothing.scalars.response_1f0 = gearbox.tables.inverse_1f0[gear_idx];
        }
        let driven_scalars =
            driven_smoothing_scalars(&self.car.four_wheel, &first[0], &driven_smoothing);
        let smoothing_forces_2c8 = if compiled_live_path {
            drive_forces_2c8
        } else {
            std::array::from_fn(|index| self.car.four_wheel.first[index].force)
        };
        update_driven_wheel_speeds(
            &mut self.car.four_wheel,
            &first,
            smoothing_forces_2c8,
            controls.handbrake,
            &driven_smoothing,
            driven_scalars,
        );
        if first[0].gear_ratio == 0 {
            self.car.four_wheel.engine_speed_2e8 = self.car.four_wheel.engine_2e0;
        } else {
            let engine = derive_smoothed_engine_speed(SmoothedEngineSpeedInput {
                wheel_speeds: self.car.four_wheel.smoothed_wheel_speeds,
                config_158: first[0].config_158,
                gear_ratio: first[0].gear_ratio,
                engine_2e0: self.car.four_wheel.engine_2e0,
                slip_gain: driven_scalars.slip_gain,
                engine_load: driven_scalars.engine_load,
                ..wheel.engine_smoothing
            });
            self.car.four_wheel.engine_speed_2e8 = engine.speed_2e8;
        }
        self.finish_mechanics_tick();
        sample.car = self.car;
        Ok(sample)
    }

    /// Run the per-car ground pass `FUN_00410a60` after a four-wheel step, when
    /// the car has live ground contact. Nothing changes on a query failure.
    ///
    /// # Errors
    ///
    /// Reports the lane whose ground query failed.
    pub fn run_ground_pass<P, C>(
        &mut self,
        time_step: Fixed16,
        providers: &mut [P; 8],
        config: &mut C,
    ) -> Result<(), WheelPostprocessError>
    where
        P: crate::compiled_car_initializer::CompiledLaneGeometryProvider,
        C: crate::compiled_car_initializer::CompiledLaneConfigProvider,
    {
        let Some(ground) = self.car.ground else {
            return Ok(());
        };
        // `FUN_00410a60` saves the normal and the four wheel contact normals
        // (`0x410a9b..0x410ac4`) before either path updates them.
        self.car.four_wheel.previous_normal_330 = self.car.motion.projection_axis;
        self.car.four_wheel.previous_wheel_normals_39c =
            std::array::from_fn(|w| ground.lanes.output_vectors[w]);
        let car = GroundPassCar {
            position: self.car.position,
            velocity: self.car.velocity,
            angular_velocity: self.car.motion.angular_velocity,
            basis: self.car.motion.basis,
            normal: self.car.motion.projection_axis,
            speed_ec: self.car.four_wheel.speed_ec,
            resting_700: self.car.four_wheel.resting_700,
            out_of_control_680: self.car.collision.response_state_680,
            recovery_688: self.car.collision.recovery_latch_688,
            axis_snapshot_bc: self.car.collision.out_of_control_axis_bc,
            tumble: self.car.four_wheel.tumble,
            contact_vectors: self.car.four_wheel.contact_vectors_4bc,
            impact_3cc: None,
            impact_contact: None,
            landing_impact: None,
        };
        let (after, ground) = run_ground_pass(
            car,
            ground,
            self.car.four_wheel.start_basis_27c,
            time_step,
            providers,
            config,
        )
        .map_err(ground_query_error)?;
        self.car.collision.recovery_latch_688 = after.recovery_688;
        self.car.four_wheel.tumble = after.tumble;
        if let Some(impact) = after.impact_3cc {
            self.car.collision.pending_linear_3cc = impact;
        }
        self.last_tumble_impact = None;
        // The tumbling contact (`FUN_00412860`) or the landing tick's: both
        // play through `FUN_0041f630`/`FUN_00469e80` and leave `+0x3f0`.
        let impact = match (after.impact_3cc, after.impact_contact) {
            (Some(impact), Some(contact)) => Some((impact, contact)),
            _ => after.landing_impact,
        };
        if let Some((impact, contact)) = impact {
            self.car.four_wheel.barrier_impact_3f0 = contact.contact_3f0;
            self.last_tumble_impact = Some(TumbleImpact {
                severity_vector: [impact.x.raw(), impact.y.raw(), impact.z.raw()],
                point: contact.point,
                contact_3f0: contact.contact_3f0,
            });
        }
        self.car.position = after.position;
        self.car.velocity = after.velocity;
        self.car.motion.angular_velocity = after.angular_velocity;
        self.car.motion.basis = after.basis;
        self.car.motion.projection_axis = after.normal;
        self.car.collision.response_state_680 = after.out_of_control_680;
        self.car.collision.out_of_control_axis_bc = after.axis_snapshot_bc;
        self.car.ground = Some(ground);
        Ok(())
    }

    /// The mechanics routine's render-spin block for this car
    /// (`0x43b3a4`, after the trigger walk).
    pub fn animate_wheel_spin(
        &mut self,
        controls: &Controls,
        wheel: &FourWheelIterationParams<'_>,
    ) {
        let four_wheel = &mut self.car.four_wheel;
        crate::wheel_animation::spin_step(
            &mut four_wheel.wheels,
            &crate::wheel_animation::SpinInput {
                raw_2a8: four_wheel.first.map(|first| first.wheel_speed),
                smoothed_2b8: four_wheel.smoothed_wheel_speeds,
                handbrake_30c: i32::from(controls.handbrake),
                brake_308: controls.raw_pedals.brake_308,
                split_170: wheel.driven_smoothing.split_170,
            },
        );
    }

    /// `FUN_00417e20`, run per car before the ground pass.
    pub fn animate_wheel_wobble(&mut self, wheel: &FourWheelIterationParams<'_>) {
        let four_wheel = &mut self.car.four_wheel;
        crate::wheel_animation::wobble_step(
            &mut four_wheel.wheels,
            four_wheel.smoothed_wheel_speeds,
            wheel.wheel_setup_800,
            &self.trig,
        );
    }

    /// `FUN_00410350` and `FUN_00417cf0`, the last step of the ground pass.
    pub fn animate_suspension(&mut self, wheel: &FourWheelIterationParams<'_>) {
        let Some(ground) = self.car.ground else {
            return;
        };
        let car = &mut self.car;
        crate::wheel_animation::suspension_step(
            &mut car.four_wheel.wheels,
            &crate::wheel_animation::SuspensionInput {
                normal: car.motion.projection_axis,
                row1: car.motion.basis[1],
                probe_y: std::array::from_fn(|i| ground.probes[i][1]),
                lane_words: std::array::from_fn(|i| ground.lanes.lane_words[i]),
                constants: wheel.suspension,
                out_of_control_680: car.collision.response_state_680,
                surface_types: std::array::from_fn(|i| ground.lanes.type_words[i]),
                probe_clear: std::array::from_fn(|i| ground.thresholds.flags_73a[i] != 0),
                upright_404: car.four_wheel.upright_404,
                speed_ec: car.four_wheel.speed_ec,
                contact_vectors: std::array::from_fn(|i| car.four_wheel.contact_vectors_4bc[i]),
            },
            &self.trig,
        );
        let mass_48 = wheel.angular.momentum.moment_scale;
        let count_u8 = u8::try_from(car.motion.contact_count_674).unwrap_or(0);
        let scale_52c = contact_count_scale(mass_48, count_u8);
        car.four_wheel.wheels.wheel_load_654 = crate::wheel_animation::compute_wheel_loads_417cf0(
            mass_48.raw(),
            car.motion.contact_count_674,
            scale_52c.raw(),
            car.four_wheel.wheels.travel_594,
            car.four_wheel.wheels.corner_roll_c8,
            wheel.suspension,
        );
    }

    /// The trigger walk `FUN_00403a00` for this car over the `.dat` records of
    /// its cell (`FUN_0048fd20` of the current centre), followed by the
    /// mechanics routine's probe snapshot into `+0x15c`. Barrier pushes,
    /// bounces and moments are applied; a reset latch is set on the ground
    /// words. Returns the records' effects for the caller (cursor writes,
    /// co-driver calls). On error nothing is written back.
    ///
    /// # Errors
    ///
    /// The car's cell is outside the trigger cell table.
    pub fn run_trigger_walk(
        &mut self,
        triggers: &mut [crate::stage_triggers::RuntimeTrigger],
        cells: &TriggerCells,
        cell_centers: &[Vec3],
        config: &TriggerWalkConfig,
    ) -> Result<Vec<TriggerEvent>, TriggerWalkError> {
        let Some(mut ground) = self.car.ground else {
            return Ok(Vec::new());
        };
        let cell = crate::live_static_collision_car_frame::select_primary_cell(
            cell_centers,
            self.car.position,
        );
        let undeformed = crate::ground_pass::GroundContact {
            deformation: [0; 3],
            ..ground
        };
        let probes = crate::ground_pass::rebuild_ground_probes(
            &undeformed,
            self.car.motion.basis,
            self.car.position,
        );
        use crate::tumble::{raw, vec};
        let mut car = crate::barrier_collision::BarrierCar {
            bounds: crate::stage_triggers::TriggerCarBounds {
                current_position: raw(self.car.position),
                saved_position: raw(self.car.four_wheel.saved_position_3e4),
                radius_q16: ground.radius_44,
                current_points: probes,
                prior_points: self.car.four_wheel.trigger_prior_15c,
            },
            reach_20: ground.reach_20,
            velocity: raw(self.car.velocity),
            basis: self.car.motion.basis,
            half_words: ground.half_words,
            impact_3f0: self.car.four_wheel.barrier_impact_3f0,
            moment_3d8: self.car.motion.moment,
            out_of_control_680: self.car.collision.response_state_680,
            recovery_688: self.car.collision.recovery_latch_688,
            contact_6dc: self.car.four_wheel.barrier_contact_6dc,
            previous_contact_6e0: self.car.four_wheel.trigger_walk.previous_contact_6e0,
            impact: None,
        };
        let mut walk_state = self.car.four_wheel.trigger_walk;
        let events = walk_triggers(triggers, cells, cell, config, &mut car, &mut walk_state)?;
        if events
            .iter()
            .any(|event| matches!(event, TriggerEvent::ResetLatch { .. }))
        {
            ground.lanes.latch_67c = 1;
        }
        self.car.ground = Some(ground);
        self.car.position = vec(car.bounds.current_position);
        self.car.velocity = vec(car.velocity);
        self.car.motion.moment = car.moment_3d8;
        self.car.collision.response_state_680 = car.out_of_control_680;
        self.car.collision.recovery_latch_688 = car.recovery_688;
        let wheel = &mut self.car.four_wheel;
        wheel.barrier_impact_3f0 = car.impact_3f0;
        wheel.barrier_contact_6dc = car.contact_6dc;
        wheel.trigger_walk = walk_state;
        wheel.trigger_prior_15c = car.bounds.current_points;
        Ok(events)
    }

    /// The reset dispatcher `FUN_00467210` and its fade-and-place routine
    /// `FUN_00414850` for this car, run after the ground pass. Returns the
    /// dispatcher's cursor write and the presentation effects. On error
    /// nothing is written back.
    ///
    /// # Errors
    ///
    /// A node index outside the `.cat` array or a failed placement query.
    pub fn run_reset<P, C>(
        &mut self,
        dispatch: &ResetDispatchInput,
        providers: &mut [P; 8],
        config: &mut C,
    ) -> Result<ResetOutcome, WheelPostprocessError>
    where
        P: crate::compiled_car_initializer::CompiledLaneGeometryProvider,
        C: crate::compiled_car_initializer::CompiledLaneConfigProvider,
    {
        let Some(ground) = self.car.ground else {
            return Ok(ResetOutcome::default());
        };
        let mut dispatch = *dispatch;
        dispatch.position = self.car.position;
        dispatch.row0 = self.car.motion.basis[0];
        let Some(target) = select_reset_target(ground.lanes.latch_67c != 0, &dispatch)? else {
            return Ok(ResetOutcome::default());
        };
        let mut reset = self.car.four_wheel.reset;
        let step = step_reset_fade(&mut reset, false);
        let placement = if step.place {
            Some(place_on_target(&target, ground, providers, config).map_err(ground_query_error)?)
        } else {
            None
        };
        self.car.four_wheel.reset = reset;
        if step.finished
            && let Some(ground) = self.car.ground.as_mut()
        {
            ground.lanes.latch_67c = 0;
        }
        if let Some(placement) = placement {
            self.apply_reset_placement(placement);
        }
        Ok(ResetOutcome {
            cursor: target.cursor,
            placed: step.place,
            camera_reset: step.place,
            fade_428: self.car.four_wheel.reset.fade_428,
            penalty_cue: step.penalty_cue,
            penalty_done_760: step.penalty_done_760,
        })
    }

    /// `FUN_00414850`'s state writes after the placement geometry: the motion,
    /// collision, tumbling, rollover, drivetrain, pedal, steering and lane
    /// words it clears, and gear 1. Words it clears that this reconstruction
    /// does not model (`ctl +0x88..+0x90`, `+0x130..+0x138`, `+0x2e4`,
    /// `+0x3e8`, car `+0x39c..+0x3c8`, `+0x3e4`, `+0x544`, `+0x550`,
    /// `+0x75c`) are not represented.
    pub fn apply_reset_placement(&mut self, placement: ResetPlacement) {
        let car = &mut self.car;
        let mut ground = placement.ground;
        // `ctl +0x3d0..+0x3de` handles and car `+0x618..+0x634` accumulators.
        ground.lanes.handles = [0; 8];
        ground.lanes.accumulator_words = [0; 8];
        car.ground = Some(ground);
        car.position = placement.position;
        car.velocity = Vec3::ZERO;
        car.motion.basis = placement.basis;
        car.motion.projection_axis = placement.normal;
        car.motion.contact_distance_flags = placement.contact_flags;
        car.motion.contact_count_674 = i32::from(placement.contact_count);
        car.motion.angular_velocity = Vec3::ZERO;
        car.motion.moment = Vec3::ZERO;
        car.motion.lane_source_s = placement.basis[0];
        car.collision.pending_linear_3cc = Vec3::ZERO;
        car.collision.response_state_680 = false;
        car.collision.recovery_latch_688 = false;
        car.collision.out_of_control_axis_bc =
            Vec3::new(Fixed16::ZERO, Fixed16::ONE, Fixed16::ZERO);
        let wheel = &mut car.four_wheel;
        wheel.speed_ec = 0;
        wheel.prior_velocity_e0 = Vec3::ZERO;
        wheel.start_basis_27c = placement.basis;
        wheel.previous_normal_330 = placement.normal;
        wheel.previous_wheel_normals_39c =
            std::array::from_fn(|w| ground.lanes.output_vectors[w]);
        wheel.wheels.roll_state_88 = [0; 3];
        wheel.wheels.corner_roll_c8 = [0; 4];
        wheel.rollover = RolloverState::default();
        wheel.tumble.hull_ticks_678 = 0;
        wheel.tumble.inverted_684 = false;
        wheel.tumble.righting_3ec = false;
        wheel.tumble.roof_3f4 = false;
        wheel.tumble.side_3f8 = false;
        wheel.tumble.end_3fc = false;
        for first in &mut wheel.first {
            first.wheel_speed = 0;
            first.force = 0;
        }
        wheel.smoothed_wheel_speeds = [0; 4];
        wheel.pedals = crate::pedal_response::PedalResponseState::default();
        wheel.engine_2e0 = 0;
        wheel.engine_speed_2e8 = 0;
        wheel.steering.position_318 = 0;
        wheel.steering.angle_3e0 = 0;
        wheel.steering.target_3e2 = 0;
        wheel.gearbox.reverse_mode_408 = false;
        wheel.gearbox.gear_43a = 1;
        wheel.limiter_444 = false;
        wheel.saved_position_3e4 = placement.position;
        wheel.trigger_walk.penalty_timer_75c = 0;
    }

    /// Seed the live car state from a replay Region-A initial-state block (`FUN_0046d460`).
    pub fn seed_from_replay_region_a_46d460(&mut self, view: crate::replay::ReplayRegionAView<'_>) {
        self.car.seed_from_replay_region_a_46d460(view);
        self.last_tumble_impact = None;
    }

    /// Replace the live car state (used by `FUN_0046edd0` to restore car 0 from
    /// its pre-pan backup `FUN_0046eda0`).
    pub fn set_car(&mut self, car: CarState) {
        self.car = car;
        self.last_tumble_impact = None;
    }

    /// Set the car's current gear (`ctl +0x43a` and `+0x43f`, `FUN_0046edd0` /
    /// `0x438087` / `0x4385be`).
    pub fn set_gear_43a(&mut self, gear: i8) {
        self.car.four_wheel.gearbox.gear_43a = gear;
        self.car.four_wheel.gearbox.target_43f = gear;
    }

    /// Run a scripted control sequence, returning the per-tick trace.
    pub fn run_trace(&mut self, controls: &[Controls], params: &SimParams) -> Vec<TraceSample> {
        controls.iter().map(|c| self.step(c, params)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SimParams {
        SimParams {
            inverse_mass: Fixed16::from_raw(0x8000), // 0.5
            gravity: Vec3::ZERO,
        }
    }

    fn first_wheel_params() -> FirstWheelParams {
        FirstWheelParams {
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
        }
    }

    fn straight_throttle(ticks: usize) -> Vec<Controls> {
        vec![
            Controls {
                steer_rate: 0,
                forward_force: Fixed16::ONE,
                handbrake: false,
                raw_steer: Default::default(),
                raw_pedals: Default::default(),
                raw_gear_command_314: 0,
            };
            ticks
        ]
    }

    #[test]
    fn the_trace_is_bit_for_bit_reproducible() {
        let script = straight_throttle(64);
        let mut a = RaceState::new(CarState::at_rest());
        let mut b = RaceState::new(CarState::at_rest());
        let trace_a = a.run_trace(&script, &params());
        let trace_b = b.run_trace(&script, &params());
        assert_eq!(trace_a, trace_b);
        assert_eq!(trace_a.len(), 64);
    }

    #[test]
    fn the_clock_advances_one_fixed_step_per_tick() {
        let mut state = RaceState::new(CarState::at_rest());
        let trace = state.run_trace(&straight_throttle(3), &params());
        let step = stage_clock_step(false);
        assert_eq!(trace[0].clock, step);
        assert_eq!(trace[1].clock, step.add(step));
        assert_eq!(trace[2].clock, step.add(step).add(step));
        // Centiseconds are the elapsed readout of the same clock.
        assert_eq!(
            trace[2].centiseconds,
            elapsed_centiseconds(trace[2].clock, Fixed16::ZERO)
        );
    }

    #[test]
    fn forward_throttle_from_rest_builds_forward_velocity_and_moves() {
        // Heading 0 faces +x, so a forward force accelerates along +x only.
        let mut state = RaceState::new(CarState::at_rest());
        let trace = state.run_trace(&straight_throttle(8), &params());
        let last = trace.last().unwrap().car;
        assert!(last.velocity.x > Fixed16::ZERO);
        assert_eq!(last.velocity.y, Fixed16::ZERO);
        assert_eq!(last.velocity.z, Fixed16::ZERO);
        // The car has moved forward (position integrated from velocity).
        assert!(last.position.x > Fixed16::ZERO);
        assert_eq!(last.position.z, Fixed16::ZERO);
    }

    #[test]
    fn position_uses_the_recovered_half_delta_translation() {
        let mut state = RaceState::new(CarState::at_rest());
        let sample = state.step(&straight_throttle(1)[0], &params());
        let acceleration = net_accel(
            Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
            params().inverse_mass,
            Vec3::ZERO,
        );
        let velocity = crate::vehicle_dynamics::integrate_velocity(Vec3::ZERO, acceleration);
        let delta = velocity.sub(Vec3::ZERO);
        let expected_position = velocity.sub(delta.scale(Fixed16::from_raw(0x8000)));
        assert_eq!(sample.car.position, expected_position);
        assert_eq!(sample.car.velocity, velocity);
    }

    #[test]
    fn first_wheel_force_and_speed_drive_the_translation() {
        let mut car = CarState::at_rest();
        car.first_wheel.wheel_speed = -1;
        let mut state = RaceState::new(car);
        let wheel = first_wheel_params();
        let first = state.step_with_first_wheel(&straight_throttle(1)[0], &params(), &wheel);
        assert_eq!(first.car.first_wheel.force, 0x10004);
        assert_eq!(first.car.first_wheel.wheel_speed, 0x10003);
        assert!(first.car.velocity.x > Fixed16::ZERO);
        let second = state.step_with_first_wheel(&straight_throttle(1)[0], &params(), &wheel);
        assert_eq!(second.car.first_wheel.force, -0xfffc);
        assert_eq!(second.car.first_wheel.wheel_speed, 7);
        assert!(second.car.position.x > first.car.position.x);
    }

    #[test]
    fn planar_step_clears_a_collision_moment_at_the_end_of_the_tick() {
        // The live planar session runs mechanics then collision. A moment the
        // collision pass published must not survive into the next pass, or
        // impulses compound across ticks; `FUN_00407690` zeroes it each tick.
        let mut car = CarState::at_rest();
        car.motion.moment = Vec3::new(
            Fixed16::from_raw(0x4_0000),
            Fixed16::from_raw(-0x2_0000),
            Fixed16::from_raw(0x1_0000),
        );
        let mut state = RaceState::new(car);
        let sample = state.step(&straight_throttle(1)[0], &params());
        assert_eq!(sample.car.motion.moment, Vec3::ZERO);
        assert_eq!(state.car().motion.moment, Vec3::ZERO);
    }

    #[test]
    fn live_collision_out_of_control_words_round_trip_through_their_owners() {
        let mut collision = CarState::at_rest().collision;
        assert_eq!(collision.out_of_control(), OutOfControlState::default());
        let tripped = OutOfControlState {
            out_of_control_680: true,
            recovery_latch_688: true,
            axis_snapshot_bc: Vec3::new(
                Fixed16::from_raw(1),
                Fixed16::from_raw(-2),
                Fixed16::from_raw(3),
            ),
        };
        collision.set_out_of_control(tripped);
        assert!(collision.response_state_680);
        assert!(collision.recovery_latch_688);
        assert_eq!(collision.out_of_control_axis_bc, tripped.axis_snapshot_bc);
        assert_eq!(collision.out_of_control(), tripped);
    }

    #[test]
    fn live_collision_pending_linear_is_consumed_once_without_clearing_persistent_state() {
        let pending = Vec3::new(
            Fixed16::from_raw(1),
            Fixed16::from_raw(-2),
            Fixed16::from_raw(3),
        );
        let mut collision = LiveCollisionState {
            pending_linear_3cc: pending,
            history: StaticContactHistory {
                previous_len: 1,
                ..StaticContactHistory::default()
            },
            object_history: StaticResponseHistory::default(),
            response_state_680: true,
            recovery_latch_688: true,
            out_of_control_axis_bc: Vec3::ZERO,
        };
        assert_eq!(collision.consume_pending_linear(), pending);
        assert_eq!(collision.pending_linear_3cc, Vec3::ZERO);
        assert_eq!(collision.history.previous_len, 1);
        assert!(collision.response_state_680);
        assert!(collision.recovery_latch_688);
        assert_eq!(collision.consume_pending_linear(), Vec3::ZERO);
    }

    #[test]
    fn race_step_consumes_pending_collision_acceleration_before_clearing_it() {
        let mut car = CarState::at_rest();
        car.collision.pending_linear_3cc =
            Vec3::new(Fixed16::from_raw(0x10000), Fixed16::ZERO, Fixed16::ZERO);
        let mut race = RaceState::new(car);
        let sample = race.step(
            &Controls {
                steer_rate: 0,
                forward_force: Fixed16::ZERO,
                handbrake: false,
                raw_steer: Default::default(),
                raw_pedals: Default::default(),
                raw_gear_command_314: 0,
            },
            &SimParams {
                inverse_mass: Fixed16::ZERO,
                gravity: Vec3::ZERO,
            },
        );
        assert_eq!(sample.car.velocity.x, Fixed16::from_raw(0x3333));
        assert_eq!(sample.car.position.x, Fixed16::from_raw(0x199a));
        assert_eq!(sample.car.collision.pending_linear_3cc, Vec3::ZERO);

        let next = race.step(
            &Controls {
                steer_rate: 0,
                forward_force: Fixed16::ZERO,
                handbrake: false,
                raw_steer: Default::default(),
                raw_pedals: Default::default(),
                raw_gear_command_314: 0,
            },
            &SimParams {
                inverse_mass: Fixed16::ZERO,
                gravity: Vec3::ZERO,
            },
        );
        assert_eq!(next.car.velocity.x, sample.car.velocity.x);
    }

    #[test]
    fn live_static_pass_publishes_depenetration_and_next_tick_state() {
        use crate::c3d_terrain_cull::CellGrid;
        use crate::static_collision_kernel::NoStaticDamageOwner;
        use crate::static_world_collision::{
            StaticCollisionNodeId, StaticCollisionRecord, StaticCollisionScene, StaticNarrowObb,
        };

        let one = Fixed16::ONE;
        let identity = [
            Vec3::new(one, Fixed16::ZERO, Fixed16::ZERO),
            Vec3::new(Fixed16::ZERO, one, Fixed16::ZERO),
            Vec3::new(Fixed16::ZERO, Fixed16::ZERO, one),
        ];
        let object = StaticNarrowObb {
            center: Vec3::ZERO,
            projected_corners: [[-128, -128], [128, -128], [128, 128], [-128, 128]],
            radius: Fixed16::from_raw(0x20000),
            max_y_shift9: 128,
            min_y_shift9: -128,
            axis_radius: one,
            perpendicular_radius: one,
            axis: identity[0],
            perpendicular: identity[2],
        };
        let node = StaticCollisionNodeId::new(0).unwrap();
        let parsed = ParsedStaticCollisionScene {
            grid: CellGrid {
                width: 1,
                height: 1,
                cell_count: 1,
                cell_size: 0x20_0000,
                min_x: 0,
                half_cell: 0x10_0000,
                max_z: 0,
            },
            main_cell_centers: vec![Vec3::ZERO],
            scene: StaticCollisionScene::new(
                vec![StaticCollisionRecord {
                    collision_ordinal: 7,
                    raw_material_flags: 0,
                    geometry: object,
                }],
                [(node, 0)],
            )
            .unwrap(),
        };
        let mut car = CarState::at_rest();
        car.motion.basis = identity;
        car.motion.projection_axis = identity[0];
        car.motion.compiled_half_words = Vec3::new(one, one, one);
        car.motion.contact_count_674 = 3;
        car.velocity = Vec3::new(one.neg(), Fixed16::ZERO, one);
        let mut race = RaceState::new(car);
        let previous = PreviousCarXz {
            x: Fixed16::from_raw(-0xa_0000),
            z: Fixed16::ZERO,
        };
        let step = race
            .resolve_static_collision_after_step(previous, &parsed, &mut NoStaticDamageOwner)
            .unwrap();
        let committed = race.car();
        assert_eq!(step.contacts, 1);
        assert_ne!(committed.position, car.position);
        assert_ne!(committed.collision.pending_linear_3cc, Vec3::ZERO);
        assert_eq!(committed.collision.history.current_len, 1);
    }

    fn second_wheel_params(trig: &[i32]) -> SecondWheelParams<'_> {
        SecondWheelParams {
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
            surface_vector: [0x10000, 0, 0],
            table_factor: None,
        }
    }

    #[test]
    fn contact_basis_vectors_populate_second_wheel_with_checked_indices() {
        let trig = build_sine_table();
        let mut second = second_wheel_params(&trig);
        let input = ContactBasisInput {
            rows: [
                Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
                Vec3::new(Fixed16::ZERO, Fixed16::ONE, Fixed16::ZERO),
                Vec3::new(Fixed16::ZERO, Fixed16::ZERO, Fixed16::ONE),
            ],
            angular: [Fixed16::ONE; 6],
            velocity: Vec3::ZERO,
        };
        assert_eq!(
            populate_contact_basis_vector(&mut second, input, 5),
            Some(Vec3::new(
                Fixed16::from_raw(-0x20000),
                Fixed16::from_raw(0x20000),
                Fixed16::ZERO,
            ))
        );
        assert_eq!(second.surface_vector, [-0x20000, 0x20000, 0]);
        second.surface_vector = [7, 8, 9];
        assert_eq!(populate_contact_basis_vector(&mut second, input, 8), None);
        assert_eq!(second.surface_vector, [7, 8, 9]);
    }

    #[test]
    fn wheel_vector_inputs_share_direction_and_contact_vector() {
        let trig = build_sine_table();
        let mut first = first_wheel_params();
        let mut second = second_wheel_params(&trig);
        second.surface_vector = [3, 4, 5];
        populate_wheel_vector_inputs(
            &mut first,
            &second,
            Vec3::new(
                Fixed16::from_raw(0x10000),
                Fixed16::from_raw(-0x20000),
                Fixed16::from_raw(0x30000),
            ),
        );
        assert_eq!(first.wheel_vector, [0x10000, -0x20000, 0x30000]);
        assert_eq!(first.state_vector, [3, 4, 5]);
    }

    #[test]
    fn compiled_drive_ratios_match_the_per_gear_population_rule() {
        let families = CompiledControlFamilies {
            post_processed_raw: [0; 8],
            reciprocal: [0; 8],
            raw_copy: std::array::from_fn(|gear| 0x1_0000 + gear as i32 * 0x3000),
            reciprocal_copy: [0; 8],
        };
        let factor = Fixed16::from_raw(0x1_8000);
        let ratios = compiled_drive_ratios(&families, factor);
        let trig = build_sine_table();
        for (gear, expected) in ratios.into_iter().enumerate() {
            let mut first = FirstWheelParams {
                gear: gear as i32,
                ..first_wheel_params()
            };
            let mut second = second_wheel_params(&trig);
            let ratio = populate_compiled_gear_ratio(&mut first, &mut second, &families, factor);
            assert_eq!(ratio.map(Fixed16::raw), Some(expected));
        }
    }

    #[test]
    fn neutral_gear_zeroes_smoothed_speeds_before_any_tracking() {
        let mut state = FourWheelState {
            first: [FirstWheelState {
                force: 0,
                wheel_speed: 0x20000,
            }; 4],
            smoothed_wheel_speeds: [0x10000; 4],
            ..FourWheelState::default()
        };
        update_smoothed_wheel_speeds(&mut state, 0);
        assert_eq!(state.smoothed_wheel_speeds, [0; 4]);
        update_smoothed_wheel_speeds(&mut state, 1);
        assert_eq!(state.smoothed_wheel_speeds, [0x10000; 4]);
    }

    #[test]
    fn driven_smoothing_consumes_each_first_loop_force_and_derived_scalars() {
        let first = [FirstWheelParams {
            gear: 4,
            gear_ratio: 0x10000,
            config_158: 1,
            ..first_wheel_params()
        }; 4];
        let mut state = FourWheelState {
            first: [FirstWheelState {
                force: 0x20000,
                wheel_speed: 0x10000,
            }; 4],
            smoothed_wheel_speeds: [0x10000; 4],
            ..FourWheelState::default()
        };
        let params = DrivenWheelUpdateParams {
            scalars: DrivenSmoothingScalarInput {
                redline_15c: 0x10000,
                response_1f0: 0x20000,
                gear_ratio_190: 0x10000,
                suspension_298: [0x40000, 0, 0, 0],
                ..DrivenWheelUpdateParams::default().scalars
            },
            throttle_304: 1,
            split_170: 0x10000,
            ..DrivenWheelUpdateParams::default()
        };
        let scalars = driven_smoothing_scalars(&state, &first[0], &params);
        assert_eq!(scalars.slip_gain, 0x9999);
        assert_eq!(scalars.direction_gain, 0x10000);
        assert_eq!(scalars.signed_speed_limit, 0x20000);
        let forces = std::array::from_fn(|index| state.first[index].force);
        update_driven_wheel_speeds(&mut state, &first, forces, false, &params, scalars);
        assert!(
            state
                .smoothed_wheel_speeds
                .iter()
                .all(|speed| *speed > 0x10000)
        );

        state.smoothed_wheel_speeds = [0; 4];
        update_driven_wheel_speeds(
            &mut state,
            &first,
            forces,
            false,
            &DrivenWheelUpdateParams {
                throttle_304: 0,
                ..params
            },
            scalars,
        );
        assert_eq!(state.smoothed_wheel_speeds, [0x10000; 4]);

        let neutral = [FirstWheelParams {
            gear_ratio: 0,
            ..first[0]
        }; 4];
        update_driven_wheel_speeds(&mut state, &neutral, forces, false, &params, scalars);
        assert_eq!(state.smoothed_wheel_speeds, [0; 4]);
    }

    #[test]
    fn derived_second_inputs_use_native_dot_products_and_residual_gates() {
        let trig = build_sine_table();
        let mut first = first_wheel_params();
        first.wheel_vector = [0x10000, 0, 0];
        first.state_vector = [0x20000, 0, 0];
        first.flag_43b = 1;
        let mut second = second_wheel_params(&trig);
        second.surface_vector = [3, 0, 0];
        populate_derived_second_inputs(
            &first,
            &mut second,
            Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
            0,
        );
        assert_eq!(second.wheel_e8, 0x20000);
        assert_eq!(second.axis_dot, 3);
        assert!(second.residual_flag_147a);
        first.config_158 = 0x10000;
        populate_derived_second_inputs(
            &first,
            &mut second,
            Vec3::new(Fixed16::ONE, Fixed16::ZERO, Fixed16::ZERO),
            2,
        );
        assert!(!second.residual_flag_147a);
    }

    #[test]
    fn motion_state_forms_contact_inputs_from_native_fields() {
        let mut motion = MotionState::harness_default();
        motion.angular_velocity = Vec3::new(
            Fixed16::from_raw(3),
            Fixed16::from_raw(4),
            Fixed16::from_raw(5),
        );
        motion.apply_compiled_half_state(
            &crate::compiled_car_initializer::derive_compiled_half_sign_state([4, -6, 8]),
        );
        motion.apply_compiled_post_lane_basis(
            [7, 8, 9],
            &CompiledPostLaneBasisEffects {
                vector_258: [1, 2, 3],
                vector_264: [4, 5, 6],
                vector_270: [7, 8, 9],
            },
        );
        assert_eq!(
            motion.projection_axis,
            Vec3::new(
                Fixed16::from_raw(7),
                Fixed16::from_raw(8),
                Fixed16::from_raw(9),
            )
        );
        assert_eq!(motion.basis[0].x, Fixed16::from_raw(1));
        assert_eq!(motion.basis[1].y, Fixed16::from_raw(5));
        assert_eq!(motion.basis[2].z, Fixed16::from_raw(9));
        let velocity = Vec3::new(
            Fixed16::from_raw(9),
            Fixed16::from_raw(10),
            Fixed16::from_raw(11),
        );
        let input = motion.contact_basis(velocity);
        assert_eq!(input.rows, motion.basis);
        assert_eq!(input.angular, [3, 4, 5, 2, -3, 4].map(Fixed16::from_raw));
        assert_eq!(input.velocity, velocity);
    }

    #[test]
    fn reviewed_surface_family_populates_table_factor_with_checked_indices() {
        let families = SurfaceCoefficientFamilies {
            first: cmr_formats::pe_surface::SurfaceCoefficientFamily {
                values: [0x20000; 120],
            },
            second: cmr_formats::pe_surface::SurfaceCoefficientFamily { values: [0; 120] },
        };
        let trig = build_sine_table();
        let mut second = second_wheel_params(&trig);
        populate_surface_table_factor(&mut second, &families, 3).unwrap();
        assert_eq!(second.table_factor, Some(0x20000));
        second.table_factor = None;
        assert_eq!(
            populate_surface_table_factor(&mut second, &families, 120),
            Err(SurfaceCoefficientIndexError { index: 120 })
        );
        assert_eq!(second.table_factor, None);
    }

    #[test]
    #[ignore = "requires an authenticated surface store under CMR_RUNTIME_DATA"]
    fn official_surface_store_feeds_second_core_without_exposing_values() {
        let root = std::env::var_os("CMR_RUNTIME_DATA").expect("CMR_RUNTIME_DATA");
        let reviewed = cmr_assets::ReviewedSurfaceProfile::from_json(
            &std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../profiles/official-eu-surface-tables-v1.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let store = cmr_assets::SurfaceTableStore::open(
            &std::path::Path::new(&root).join("official-eu-surface-coefficients"),
            &reviewed,
        )
        .unwrap();
        let trig = build_sine_table();
        let mut second = second_wheel_params(&trig);
        populate_surface_table_factor(&mut second, store.families(), 0).unwrap();
        assert_eq!(second.table_factor, Some(store.families().first.values[0]));
    }

    #[test]
    fn force_pass_scales_populate_both_wheel_parameter_sets() {
        let trig = build_sine_table();
        let mut first = first_wheel_params();
        let mut second = second_wheel_params(&trig);
        let scales = populate_force_pass_scales(
            &mut first,
            &mut second,
            Fixed16::from_raw(0x30000),
            Fixed16::from_raw(0x20000),
            Fixed16::from_raw(0x40000),
        );
        assert_eq!(first.scale_c600, 0x1e000);
        assert_eq!(first.factor_c5c0, 0x4444c);
        assert_eq!(second.scale_c5c4, 0x60000);
        assert_eq!(scales.scale_c5c4, Fixed16::from_raw(0x60000));
    }

    #[test]
    fn compiled_families_populate_the_selected_gear_ratio_atomically() {
        let families = CompiledControlFamilies {
            post_processed_raw: [11, 22, 33, 44, 55, 66, 77, 88],
            reciprocal: [0; 8],
            raw_copy: [11, 22, 33, 44, 55, 66, 77, 88],
            reciprocal_copy: [0; 8],
        };
        let trig = build_sine_table();
        let mut first = first_wheel_params();
        first.gear = 2;
        let mut second = second_wheel_params(&trig);
        assert_eq!(
            populate_compiled_gear_ratio(&mut first, &mut second, &families, Fixed16::ONE),
            Some(Fixed16::from_raw(33))
        );
        assert_eq!(first.gear_ratio, 33);
        assert_eq!(second.gear_ratio, 33);

        first.gear = 8;
        first.gear_ratio = 99;
        second.gear_ratio = 98;
        assert_eq!(
            populate_compiled_gear_ratio(&mut first, &mut second, &families, Fixed16::ONE),
            None
        );
        assert_eq!(first.gear_ratio, 99);
        assert_eq!(second.gear_ratio, 98);
    }

    #[test]
    fn second_wheel_core_composes_after_the_first_iteration() {
        let mut car = CarState::at_rest();
        car.first_wheel.wheel_speed = -1;
        let mut state = RaceState::new(car);
        let trig = build_sine_table();
        let second = second_wheel_params(&trig);
        let sample = state
            .step_with_second_wheel(
                &straight_throttle(1)[0],
                &params(),
                &first_wheel_params(),
                &second,
            )
            .unwrap();
        assert_eq!(sample.car.second_wheel.ratio_at_3b0, -0x10003);
        assert_eq!(sample.car.second_wheel.angle, 0);
        assert_eq!(sample.car.second_wheel.component, 1);
        assert_eq!(sample.car.second_wheel.accumulators, [-6554, 0, 0]);
        assert!(sample.car.velocity.x > Fixed16::ZERO);
    }

    #[test]
    fn four_lane_postprocess_consumes_the_current_second_core_lane() {
        let vector = |x, y, z| {
            Vec3::new(
                Fixed16::from_raw(x),
                Fixed16::from_raw(y),
                Fixed16::from_raw(z),
            )
        };
        let inactive = GeometricPostprocessLane {
            active: false,
            vector_a: Vec3::ZERO,
            vector_b: Vec3::ZERO,
            vector_c: Vec3::ZERO,
            accumulator: Vec3::ZERO,
        };
        let mut lanes = [inactive; 4];
        lanes[3] = GeometricPostprocessLane {
            active: true,
            vector_a: vector(0x8000, -1, 0),
            vector_b: vector(0x10000, 0, 0),
            vector_c: vector(0, 0x10000, 0x10000),
            accumulator: vector(-6554, 0, 0),
        };
        let postprocess = FourWheelPostprocessParams {
            lanes,
            current_lane: 3,
            argument_scale: Fixed16::ONE,
            base_force: Vec3::ZERO,
            non_wheel_forces: &[],
            wheel_contacts: [false, false, false, true],
        };
        let trig = build_sine_table();
        let second = second_wheel_params(&trig);
        let mut car = CarState::at_rest();
        car.first_wheel.wheel_speed = -1;
        let mut state = RaceState::new(car);
        let sample = state
            .step_with_four_wheel_postprocess(
                &straight_throttle(1)[0],
                &params(),
                &first_wheel_params(),
                &second,
                &postprocess,
            )
            .unwrap();
        let outcome = apply_geometric_lane_postprocess(&mut lanes, Fixed16::ONE).unwrap();
        assert!(outcome.qualified[3]);
        assert_eq!(sample.car.postprocess.qualified, outcome.qualified);
        assert_eq!(
            sample.car.postprocess.angle_measures,
            outcome.angle_measures
        );
        assert_eq!(
            sample.car.postprocess.accumulators,
            lanes.map(|lane| lane.accumulator)
        );
        let current = lanes[3].accumulator;
        assert_eq!(
            sample.car.second_wheel.accumulators,
            [current.x.raw(), current.y.raw(), current.z.raw()]
        );

        assert!(sample.car.velocity.x < Fixed16::ZERO);
        assert_eq!(sample.car.position, Vec3::ZERO);

        let before = state.car;
        let invalid = FourWheelPostprocessParams {
            current_lane: 4,
            ..postprocess
        };
        assert_eq!(
            state.step_with_four_wheel_postprocess(
                &straight_throttle(1)[0],
                &params(),
                &first_wheel_params(),
                &second,
                &invalid,
            ),
            Err(WheelPostprocessError::LaneIndex(4))
        );
        assert_eq!(state.car, before);
    }

    #[test]
    fn all_four_wheels_run_both_iterations_before_postprocess() {
        let vector = |components: [i32; 3]| {
            Vec3::new(
                Fixed16::from_raw(components[0]),
                Fixed16::from_raw(components[1]),
                Fixed16::from_raw(components[2]),
            )
        };
        let inactive = GeometricPostprocessLane {
            active: false,
            vector_a: Vec3::ZERO,
            vector_b: Vec3::ZERO,
            vector_c: Vec3::ZERO,
            accumulator: Vec3::ZERO,
        };
        let mut lanes = [inactive; 4];
        lanes[3] = GeometricPostprocessLane {
            active: true,
            vector_a: vector([0x8000, -1, 0]),
            vector_b: vector([0x10000, 0, 0]),
            vector_c: vector([0, 0x10000, 0x10000]),
            accumulator: Vec3::ZERO,
        };
        let trig = build_sine_table();
        let mut wheel = FourWheelIterationParams {
            first: [first_wheel_params(); 4],
            second: std::array::from_fn(|_| second_wheel_params(&trig)),
            contact_vector_indices: [2, 3, 6, 7],
            contact_offsets: [Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, vector([0, 0x10000, 0])],
            contact_update_inputs: std::array::from_fn(|index| ContactVectorUpdateInput {
                update_gate: 0,
                enabled: index == 0,
                vector: Vec3::ZERO,
                basis: Vec3::ZERO,
                scale_528: Fixed16::ONE,
                extra_vector: Vec3::ZERO,
                limit: Fixed16::ONE,
                threshold: Fixed16::from_raw(0x7fff0000),
            }),
            lane_threshold_inputs: CompiledLaneThresholdInputs {
                lane_words: [0; 8],
                source_y: [0x199a; 8],
            },
            pedal_response: PedalResponseParams {
                throttle_max_148: 0x8000,
                ..PedalResponseParams::default()
            },
            lane_time_step: crate::vehicle_dynamics::SOLVER_GLOBAL_CONSTANTS.time_step,
            lane_direction_angle: 0,
            steering: SteeringParams::default(),
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
            postprocess: FourWheelPostprocessParams {
                lanes,
                current_lane: 3,
                argument_scale: Fixed16::ONE,
                // A caller-supplied non-wheel force: the four-wheel step has no
                // harness throttle push, so it drives the fixture.
                base_force: vector([0x1_0000, 0, 0]),
                non_wheel_forces: &[],
                wheel_contacts: [true; 4],
            },
            inverse_mass_4c: Fixed16::ONE,
            angular: AngularOrientationInput {
                momentum: crate::vehicle_dynamics::AngularMomentumStepInput {
                    prior_moment: Vec3::ZERO,
                    contact_moment: vector([1, 2, 3]),
                    moment_scale: Fixed16::ONE,
                    inertia_scale: Vec3::new(Fixed16::ONE, Fixed16::ONE, Fixed16::ONE),
                    angular_velocity: Vec3::ZERO,
                    time_step: crate::vehicle_dynamics::SOLVER_GLOBAL_CONSTANTS.time_step,
                },
                clamp_enabled: false,
                control: crate::vehicle_dynamics::ControlAxisFlags {
                    mode_3ec: 0,
                    axis_3f4: 0,
                    axis_3f8: 0,
                },
                basis: [Vec3::ZERO; 3],
            },
            engine_smoothing: SmoothedEngineSpeedInput {
                wheel_speeds: [0; 4],
                config_158: 1,
                gear_ratio: 0x10000,
                engine_2e0: 0,
                redline_15c: 0x10000,
                slip_gain: 0,
                engine_load: 0x10000,
            },
            driven_smoothing: DrivenWheelUpdateParams::default(),
            engine_force: EngineForceInput {
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
        };
        populate_compiled_contact_offsets(
            &mut wheel,
            &crate::compiled_car_initializer::derive_compiled_half_sign_state([
                0x40000, -0x60000, 0x80000,
            ]),
        );
        populate_compiled_contact_update_inputs(
            &mut wheel,
            &CompiledControlClearLoop {
                words: [
                    0x10000, 0x7fff0000, 0x10000, 0x7fff0000, 0x10000, 0x7fff0000, 0x10000,
                    0x7fff0000, 0x10000, 0x7fff0000, 0x10000, 0x7fff0000, 0x10000, 0x7fff0000,
                    0x10000, 0x7fff0000,
                ],
            },
        );
        assert_eq!(wheel.contact_update_inputs[0].limit, Fixed16::ONE);
        assert_eq!(
            wheel.contact_update_inputs[0].threshold,
            Fixed16::from_raw(0x7fff0000)
        );
        assert_eq!(
            wheel.contact_offsets,
            [
                vector([0x20000, 0x30000, 0x40000]),
                vector([0x20000, 0x30000, -0x40000]),
                vector([-0x20000, 0x30000, 0x40000]),
                vector([-0x20000, 0x30000, -0x40000]),
            ]
        );
        let mut car = CarState::at_rest();
        car.motion.apply_compiled_post_lane_basis(
            [0, 0, 0],
            &CompiledPostLaneBasisEffects {
                vector_258: [0x10000, 0, 0],
                vector_264: [0, 0x10000, 0],
                vector_270: [0, 0, 0x10000],
            },
        );
        car.motion.angular_velocity = vector([0x10000, 0x10000, 0x10000]);
        // Grounded and upright, so the wheel-force producer runs its full body.
        car.motion.contact_distance_flags = [true; 4];
        car.motion.contact_count_674 = 4;
        car.motion.projection_axis = vector([0, 0x10000, 0]);
        // A nonzero collision moment from the previous tick: the angular pass
        // must consume it (decayed) and the mechanics driver must clear it.
        car.motion.moment = vector([0x2_0000, -0x1_0000, 0x3_0000]);
        for (index, speed) in [-1, 0x10000, -1, -1].into_iter().enumerate() {
            car.four_wheel.first[index].wheel_speed = speed;
        }
        let mut state = RaceState::new(car);
        let sample = state
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        assert_eq!(
            sample.car.four_wheel.first[0].wheel_speed,
            sample.car.four_wheel.first[1].wheel_speed
        );
        assert_eq!(
            sample.car.four_wheel.first[2].wheel_speed,
            sample.car.four_wheel.first[3].wheel_speed
        );
        assert_eq!(car.four_wheel.residual_enable, 10);
        assert_eq!(sample.car.four_wheel.residual_enable, 0);
        assert_eq!(
            sample.car.four_wheel.speed_ec,
            sqrt_q16(sample.car.velocity.dot(sample.car.velocity).raw() as u32) as i32
        );
        for index in 0..4 {
            assert_eq!(
                sample.car.four_wheel.smoothed_wheel_speeds[index],
                track_smoothed_wheel_speed(sample.car.four_wheel.first[index].wheel_speed, 0)
            );
        }
        let expected_engine = apply_rev_limiter(
            Fixed16::from_raw(integrate_engine_from_raw_wheels(
                0,
                std::array::from_fn(|index| sample.car.four_wheel.first[index].wheel_speed),
                wheel.first[0].config_158,
                wheel.first[0].gear_ratio,
            )),
            Fixed16::from_raw(wheel.engine_smoothing.redline_15c),
        )
        .rpm
        .raw();
        assert_eq!(sample.car.four_wheel.engine_2e0, expected_engine);
        assert_eq!(
            sample.car.four_wheel.engine_speed_2e8,
            derive_smoothed_engine_speed(SmoothedEngineSpeedInput {
                wheel_speeds: sample.car.four_wheel.smoothed_wheel_speeds,
                config_158: wheel.first[0].config_158,
                gear_ratio: wheel.first[0].gear_ratio,
                engine_2e0: expected_engine,
                ..wheel.engine_smoothing
            })
            .speed_2e8
        );
        let contact_vectors = build_contact_basis_vectors(car.motion.contact_basis(car.velocity));
        let thresholds = derive_compiled_lane_thresholds(wheel.lane_threshold_inputs);
        let update_gate = contact_global_update_gate(transform_contact_force(
            car.motion.basis,
            car.motion.projection_axis,
        ));
        let mut contact_inputs = wheel.contact_update_inputs;
        for (index, (input, vector)) in contact_inputs.iter_mut().zip(contact_vectors).enumerate() {
            input.vector = vector;
            input.basis = car.motion.projection_axis;
            input.scale_528 =
                contact_count_scale(wheel.angular.momentum.moment_scale, thresholds.count_742);
            input.extra_vector = Vec3::ZERO;
            input.update_gate = update_gate;
            input.enabled = thresholds.flags_73a[index] != 0;
        }
        assert_eq!(
            sample.car.four_wheel.contact_updates,
            apply_contact_vector_updates(contact_inputs, [Vec3::ZERO; 8])
        );
        let mut expected_lanes = lanes;
        let expected_s = stabilize_lane_source_s(
            car.motion.lane_source_s,
            car.motion.basis[0],
            car.motion.basis[1],
            // No pedal is pressed, so the processor's `+0x2d8` is zero.
            Fixed16::ZERO,
            wheel.lane_time_step,
        );
        let (source_p, source_r) = derive_lane_direction_pair(
            expected_s,
            car.motion.basis[1],
            wheel.lane_direction_angle,
            &trig,
        )
        .unwrap();
        let expected_directions = construct_lane_directions(
            LaneDirectionSources {
                projection_axis: car.motion.projection_axis,
                source_a0: source_p,
                source_b0: source_r,
                source_a2: expected_s,
                cross_operand: car.motion.basis[1],
            },
            resolve_lane_direction_mode(wheel.lane_rebuild, wheel.lane_exchange),
        );
        for (index, lane) in expected_lanes.iter_mut().enumerate() {
            lane.vector_a = expected_directions.direction_a[index];
            lane.vector_b = expected_directions.direction_b[index];
            lane.accumulator = vector(sample.car.four_wheel.second[index].accumulators);
        }
        let outcome = apply_geometric_lane_postprocess(&mut expected_lanes, Fixed16::ONE).unwrap();
        assert_eq!(
            sample.car.four_wheel.postprocess.accumulators,
            expected_lanes.map(|lane| lane.accumulator)
        );
        assert_eq!(
            sample.car.four_wheel.postprocess.qualified,
            outcome.qualified
        );
        assert!(sample.car.velocity.x > Fixed16::ZERO);
        // The integrator has no harness throttle push: the planar
        // `forward_force` leaves the four-wheel tick unchanged.
        let coasting = RaceState::new(car)
            .step_with_all_four_wheels(&Controls::default(), &wheel)
            .unwrap();
        assert_eq!(coasting.car, sample.car);

        // An analog full throttle goes through the control processor: `+0x2d8`
        // becomes `qmul(+0x148, 0x10000)`, the lane stabilizer reads it this
        // tick, and the raw `+0x304` reaches the driven-wheel gate, which now
        // takes the integrating branch on every lane (`config_158 = 1`).
        let pressed_controls = Controls {
            raw_pedals: RawPedals {
                throttle_304: 0x1_0000,
                throttle_analog_414: true,
                ..RawPedals::default()
            },
            ..straight_throttle(1)[0]
        };
        let pressed = RaceState::new(car)
            .step_with_all_four_wheels(&pressed_controls, &wheel)
            .unwrap();
        assert_eq!(pressed.car.four_wheel.pedals.response_2d8, 0x8000);

        // A held digital left above the low-speed cut runs the steering half:
        // the wheel angle slews to -0x22, the lane source is rotated by base +
        // angle, and the lane outputs change. At rest the cut removes the steer
        // and the tick matches the unsteered one.
        let mut steerable = wheel.clone();
        steerable.steering = SteeringParams {
            max_angle_3e4: 0x2aa,
            rate_44c: 0x4000,
            time_step: 0x1_0000,
            ..SteeringParams::default()
        };
        let left = Controls {
            raw_steer: RawSteer {
                left_2fc: 0x1_0000,
                right_300: 0,
                analog_410: false,
            },
            ..straight_throttle(1)[0]
        };
        let mut rolling = car;
        rolling.four_wheel.speed_ec = 0x1000;
        let steered = RaceState::new(rolling)
            .step_with_all_four_wheels(&left, &steerable)
            .unwrap();
        let straight = RaceState::new(rolling)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &steerable)
            .unwrap();
        assert_eq!(steered.car.four_wheel.steering.position_318, 0x4000);
        assert_eq!(steered.car.four_wheel.steering.angle_3e0, -0x22);
        assert_eq!(straight.car.four_wheel.steering, SteeringState::ZERO);
        assert_ne!(
            steered.car.four_wheel.second[0].accumulators,
            straight.car.four_wheel.second[0].accumulators
        );
        let parked = RaceState::new(car)
            .step_with_all_four_wheels(&left, &steerable)
            .unwrap();
        assert_eq!(parked.car.four_wheel.steering, SteeringState::ZERO);

        // A live gearbox: a high engine speed in first requests an upshift on
        // this tick and applies it on the next, after the residual countdown.
        let mut tables = crate::gearbox::GearboxTables {
            redline_15c: 0x1_0000,
            ..Default::default()
        };
        for gear in 1..=6 {
            tables.lower_1b0[gear] = 0x1_0000;
            tables.upper_190[gear] = 0x3_0000;
            tables.ratio_1d0[gear] = (8 - gear as i32) * 0x1_0000;
            tables.inverse_1f0[gear] = 0x1_0000 / (8 - gear as i32);
        }
        let mut geared = wheel.clone();
        geared.gearbox = Some(GearboxParams {
            tables,
            drive_ratios: std::array::from_fn(|gear| gear as i32 * 0x1000),
            ..GearboxParams::default()
        });
        let mut revving = car;
        revving.four_wheel.residual_enable = 0;
        revving.four_wheel.gearbox = GearboxState::at_setup(true);
        revving.four_wheel.engine_2e0 = 0x5_0000;
        let mut live = RaceState::new(revving);
        let requested = live
            .step_with_all_four_wheels(&pressed_controls, &geared)
            .unwrap();
        let gearbox = requested.car.four_wheel.gearbox;
        assert_eq!(gearbox.gear_43a, 1);
        assert!(gearbox.shift_40c);
        assert!(gearbox.target_43f > 1);
        let applied = live
            .step_with_all_four_wheels(&pressed_controls, &geared)
            .unwrap();
        assert_eq!(applied.car.four_wheel.gearbox.gear_43a, gearbox.target_43f);
        assert!(!applied.car.four_wheel.gearbox.shift_40c);

        // Mode 2 with the brake held at rest selects reverse and sets `+0x408`
        // in the same tick, so the pedal half lets the brake drive `+0x2d8`.
        let mut reversing = geared.clone();
        reversing.gearbox = Some(GearboxParams {
            mode_3f0: 2,
            brake_reverse_enabled: true,
            wheel_flags_73a: [1; 4],
            ..geared.gearbox.unwrap()
        });
        let braking = Controls {
            raw_pedals: RawPedals {
                brake_308: 0x1_0000,
                brake_analog_418: true,
                ..RawPedals::default()
            },
            ..straight_throttle(1)[0]
        };
        let mut idle_car = revving;
        idle_car.four_wheel.engine_2e0 = 0;
        let reversed = RaceState::new(idle_car)
            .step_with_all_four_wheels(&braking, &reversing)
            .unwrap();
        assert!(reversed.car.four_wheel.gearbox.reverse_mode_408);
        assert_eq!(reversed.car.four_wheel.gearbox.target_43f, 7);
        assert_eq!(reversed.car.four_wheel.pedals.response_2d8, 0x8000);
        assert_eq!(reversed.car.four_wheel.pedals.brake_response_2ec, 0);

        // Without contacts the producer takes its airborne pass: every wheel
        // spins with drag and the alternate factor, and no lane force or
        // postprocess output is produced.
        let mut airborne_car = car;
        airborne_car.motion.contact_count_674 = 0;
        let airborne = RaceState::new(airborne_car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        assert_eq!(
            airborne.car.four_wheel.postprocess.accumulators,
            [Vec3::ZERO; 4]
        );
        assert_eq!(airborne.car.four_wheel.postprocess.qualified, [false; 4]);
        let alternate = crate::drivetrain::derive_alternate_force_pass_factor(wheel.lane_time_step);
        // No pedal in gear: the coasting brake torques.
        let (front_brake, rear_brake) =
            crate::drivetrain::wheel_brake_torques(crate::drivetrain::WheelBrakeInput {
                gear: wheel.first[0].gear,
                bias_170: wheel.driven_smoothing.split_170,
                balance_e1a4: wheel.brake_balance_e1a4,
                ..Default::default()
            });
        for index in 0..4 {
            let lane = FirstWheelParams {
                split: if index < 2 { front_brake } else { rear_brake },
                ..wheel.first[index]
            };
            let (force, _) = crate::drivetrain::spin_free_wheel(
                FirstWheelInputs {
                    force: 0,
                    wheel_speed: airborne_car.four_wheel.first[index].wheel_speed,
                    ..first_wheel_inputs(&lane)
                },
                true,
                alternate,
            );
            // Zero drive torque (no engine force yet) leaves only the drag.
            assert_eq!(
                airborne.car.four_wheel.first[index].force, force,
                "lane {index}"
            );
        }
        // The same car upright but tipped (`+0x404` clear) is airborne too.
        let mut tipped_car = car;
        tipped_car.motion.projection_axis = vector([0x10000, 0, 0]);
        let tipped = RaceState::new(tipped_car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        assert!(!tipped.car.four_wheel.upright_404);
        assert_eq!(tipped.car.four_wheel.postprocess.qualified, [false; 4]);

        // A grounded car with one non-contacting lane skips that lane's drag
        // and slip: its drive slot is only integrated.
        let mut three_down = car;
        three_down.motion.contact_distance_flags = [true, true, true, false];
        let partial = RaceState::new(three_down)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        let (force, _) = crate::drivetrain::spin_free_wheel(
            FirstWheelInputs {
                force: 0,
                wheel_speed: three_down.four_wheel.first[3].wheel_speed,
                ..first_wheel_inputs(&wheel.first[3])
            },
            false,
            Fixed16::from_raw(wheel.first[3].factor_c5c0),
        );
        assert_eq!(partial.car.four_wheel.first[3].force, force);
        assert_eq!(partial.car.four_wheel.first[3].force, 0);

        // A gear outside the tables is reported, not guessed.
        let mut lost = revving;
        lost.four_wheel.gearbox = GearboxState {
            gear_43a: -1,
            ..GearboxState::ZERO
        };
        assert_eq!(
            RaceState::new(lost).step_with_all_four_wheels(&pressed_controls, &geared),
            Err(WheelPostprocessError::Gear(GearOutOfRange { gear: -1 }))
        );
        assert_eq!(
            pressed.car.motion.lane_source_s,
            stabilize_lane_source_s(
                car.motion.lane_source_s,
                car.motion.basis[0],
                car.motion.basis[1],
                Fixed16::from_raw(0x8000),
                wheel.lane_time_step,
            )
        );
        for lane in 0..4 {
            assert!(!smoothed_speed_uses_tracking(SmoothedSpeedGateInput {
                lane,
                mode_408: wheel.driven_smoothing.mode_408,
                throttle_304: 0x1_0000,
                brake_308: 0,
                handbrake_30c: 0,
                config_158: wheel.first[lane].config_158,
                split_170: wheel.driven_smoothing.split_170,
                limiter_444: 0,
            }));
        }
        assert_ne!(
            pressed.car.four_wheel.smoothed_wheel_speeds,
            sample.car.four_wheel.smoothed_wheel_speeds
        );
        // Every lane passes `0 + 0x1999 < 0x199a`, so `+0x742 == 0` and the
        // native low-speed suppression is off even though the wheel flags are
        // set: the position advances.
        assert_eq!(
            derive_compiled_lane_thresholds(wheel.lane_threshold_inputs).count_742,
            0
        );
        assert_ne!(sample.car.position, Vec3::ZERO);

        // With contacting lanes (`+0x742 > 0`) and a new speed below `0xccc`, the
        // same tick keeps velocity but suppresses the position increment.
        let mut contacting = wheel.clone();
        contacting.lane_threshold_inputs.source_y = [0; 8];
        assert!(derive_compiled_lane_thresholds(contacting.lane_threshold_inputs).count_742 > 0);
        // Only the grounded base force acts here: `grounded_base_force` keeps
        // `1 - grip` of the tangential +x base force on the flat start normal.
        let mut base_only = contacting.clone();
        base_only.postprocess.wheel_contacts = [false; 4];
        let suppressed = RaceState::new(car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &base_only)
            .unwrap();
        assert!(suppressed.car.velocity.x > Fixed16::ZERO);
        assert!(
            suppressed.car.velocity.length()
                < crate::vehicle_dynamics::LOW_SPEED_POSITION_SUPPRESSION
        );
        assert_eq!(suppressed.car.position, Vec3::ZERO);
        // The integrator head's grip assist `FUN_0040b240` runs first and
        // hands its renormalized basis and damped yaw rate to the pipeline.
        let assist = crate::integrator_head::grip_assist(
            &GripAssistInput {
                contact_count_674: car.motion.contact_count_674,
                upright_404: upright_flag_404(transform_contact_force(
                    car.motion.basis,
                    car.motion.projection_axis,
                )),
                time_step: wheel.lane_time_step,
                basis: car.motion.basis,
                velocity: car.velocity,
                angular_velocity: car.motion.angular_velocity,
                speed_ec: car.four_wheel.speed_ec,
                steer_angle_3e0: sample.car.four_wheel.steering.angle_3e0,
                steer_position_318: sample.car.four_wheel.steering.position_318,
                handbrake_30c: straight_throttle(1)[0].handbrake,
                steer_force_reversed: wheel.lane_exchange,
                normal: car.motion.projection_axis,
                previous_normal_330: car.four_wheel.previous_normal_330,
                crest_threshold_128: wheel.crest_128_12c[0],
                crest_gain_12c: wheel.crest_128_12c[1],
                front_accumulators: [Vec3::ZERO; 2],
                published: car.four_wheel.start_basis_27c,
            },
            &trig,
        );
        let mut expected_angular = wheel.angular;
        expected_angular.basis = assist.basis;
        expected_angular.momentum.angular_velocity = assist.angular_velocity;
        expected_angular.momentum.prior_moment = car.motion.moment;
        // `wheel` has no touching probe: the integrator's contact sum adds no
        // lane moment, and the deadzones are skipped.
        expected_angular.momentum.contact_moment = wheel.angular.momentum.contact_moment;
        assert_eq!(
            derive_compiled_lane_thresholds(wheel.lane_threshold_inputs).count_742,
            0
        );
        let expected_orientation = apply_angular_orientation_pipeline(
            expected_angular,
            car.collision.out_of_control(),
            car.motion.projection_axis,
            ContactDeadzoneGate::default(),
            &trig,
        )
        .unwrap();
        let start_axis = transform_contact_force(car.motion.basis, car.motion.projection_axis);
        assert_eq!(
            sample.car.four_wheel.upright_404,
            upright_flag_404(start_axis)
        );
        // An upright car with no rotation or throttle: on contact the
        // deadzones zero the one-LSB residual angles of the lane moments, the
        // basis holds, and the slow new speed marks it at rest. Without
        // contact the integrator adds no lane moment at all, so the basis
        // holds too.
        let mut calm_car = car;
        calm_car.motion.projection_axis = vector([0, 0x10000, 0]);
        calm_car.motion.angular_velocity = Vec3::ZERO;
        calm_car.motion.moment = Vec3::ZERO;
        let calm_step = |wheel: &FourWheelIterationParams<'_>| {
            let mut unforced = wheel.clone();
            unforced.postprocess.base_force = Vec3::ZERO;
            RaceState::new(calm_car)
                .step_with_all_four_wheels(&Controls::default(), &unforced)
                .unwrap()
        };
        // The grip assist `FUN_0040b240` legacy-renormalizes the rows of a
        // grounded upright car every tick, so "holds" means no rotation.
        let axis_aligned = |basis: [Vec3; 3]| {
            basis.iter().enumerate().all(|(row, axis)| {
                let words = [axis.x.raw(), axis.y.raw(), axis.z.raw()];
                words
                    .iter()
                    .enumerate()
                    .all(|(column, &word)| (column == row) == (word != 0))
            })
        };
        let calm = calm_step(&contacting);
        assert!(calm.car.four_wheel.upright_404);
        assert!(
            axis_aligned(calm.car.motion.basis),
            "{:?}",
            calm.car.motion.basis
        );
        assert!(calm.car.four_wheel.resting_700);
        // The loose fixture's lanes still push through the native contact
        // moment sum, so it turns a little but stays upright.
        let loose = calm_step(&wheel);
        assert!(loose.car.four_wheel.upright_404);
        assert!(
            loose.car.motion.basis[1].y.raw() > 0xf000,
            "{:?}",
            loose.car.motion.basis
        );

        // With contacting lanes the deadzones run on the cut steer and the
        // start-of-tick `+0x404`; the fixture starts at rest, so any steer is cut
        // and a steered tick matches an unsteered one.
        let steered = Controls {
            raw_steer: RawSteer {
                left_2fc: 0xfc00,
                right_300: 0,
                analog_410: false,
            },
            ..straight_throttle(1)[0]
        };
        assert_eq!(car.four_wheel.speed_ec, 0);
        let steered_contacting = RaceState::new(car)
            .step_with_all_four_wheels(&steered, &base_only)
            .unwrap();
        assert_eq!(steered_contacting.car.motion, suppressed.car.motion);
        assert_eq!(steered_contacting.car.four_wheel, suppressed.car.four_wheel);
        assert_eq!(sample.car.motion.basis, expected_orientation.basis);
        assert_eq!(
            sample.car.motion.angular_velocity,
            expected_orientation.angular_velocity
        );
        assert_ne!(
            expected_orientation.momentum.scaled_prior_moment,
            Vec3::ZERO,
            "the fixture must exercise a nonzero decayed moment"
        );
        assert_eq!(sample.car.motion.moment, Vec3::ZERO);
        assert_eq!(state.car().motion.moment, Vec3::ZERO);
        assert_eq!(
            sample.car.collision.out_of_control(),
            expected_orientation.out_of_control
        );
        assert_ne!(sample.car.motion.basis, car.motion.basis);
        assert_ne!(
            sample.car.motion.angular_velocity,
            car.motion.angular_velocity
        );
        assert_ne!(
            sample.car.motion.contact_basis(sample.car.velocity).rows,
            car.motion.contact_basis(car.velocity).rows
        );

        let mut moving_car = car;
        moving_car.velocity = vector([0x20000, 0, 0]);
        let moving = RaceState::new(moving_car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        assert_ne!(
            moving.car.four_wheel.second[0].accumulators,
            sample.car.four_wheel.second[0].accumulators
        );

        // With the static tyre tables and live ground contact the step runs
        // the driver's `DAT_004f777c` loop and `FUN_004095e0`, and the wheel
        // loops read their gains, bases and limits instead of harness words.
        let tables = TyreSurfaceTables {
            candidate_base_6b50: std::array::from_fn(|i| 0x1_0000 + i as i32),
            candidate_base_6f10: std::array::from_fn(|i| 0x2_0000 + i as i32),
            load_factor_6b4c: std::array::from_fn(|i| 0x8000 + i as i32),
            load_factor_6f0c: std::array::from_fn(|i| 0x4000 + i as i32),
            wheel_terms_777c: std::array::from_fn(|v| {
                std::array::from_fn(|s| [(v * 4 + s) as u16, 0x100, 0x200])
            }),
            roll_limits_768c: [[0; 2]; 120],
        };
        let mut tyred = wheel.clone();
        tyred.tyre = Some(TyreSurfaceInputs {
            tables: &tables,
            variant_758: 1,
            surface_families: None,
        });
        let mut ground_car = car;
        ground_car.ground = Some(GroundContact {
            lanes: crate::compiled_car_initializer::CompiledEightLaneExecutionState {
                handles: [0; 8],
                type_words: [3; 8],
                output_vectors: [[0, 0x1_0000, 0]; 8],
                lane_words: [0; 8],
                accumulator_words: [0; 8],
                latch_67c: 0,
            },
            lane_inputs: crate::compiled_car_initializer::CompiledSetupPoseLaneInputs {
                gate_words: [0; 8],
                drive_words: [0; 8],
                first_four_subtractions: [0; 4],
            },
            half_words: [0; 3],
            deformation: [0; 3],
            probes: [[0; 3]; 8],
            thresholds: derive_compiled_lane_thresholds(CompiledLaneThresholdInputs {
                lane_words: [0; 8],
                source_y: [0; 8],
            }),
            wheel_reach_648_644: [0; 2],
            reach_20: 0,
            radius_44: 0,
        });
        let tyre_free = RaceState::new(ground_car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &wheel)
            .unwrap();
        let live = RaceState::new(ground_car)
            .step_with_all_four_wheels(&straight_throttle(1)[0], &tyred)
            .unwrap();
        let terms = live.car.four_wheel.wheel_terms;
        assert_eq!(terms.term_278, [-7; 4]);
        assert_eq!(terms.term_288, [-0x100; 4]);
        assert_eq!(terms.term_298, [0x1_0200; 4]);
        assert!(live.car.four_wheel.upright_404);
        // Upright wheels on a level normal: load 30.0 * 0.5 = 15.0.
        let gains = live.car.four_wheel.tyre_gains;
        let load = Fixed16::from_raw(0xf_0000);
        assert_eq!(
            gains.gain_36c,
            [load.mul(Fixed16::from_raw(0x4003 - 7)).raw(); 4]
        );
        assert_eq!(
            gains.gain_35c,
            [load.mul(Fixed16::from_raw(0x8003 - 7)).raw(); 4]
        );
        assert_eq!(tyre_free.car.four_wheel.tyre_gains, TyreGains::default());
        assert_ne!(
            live.car.four_wheel.second[0].accumulators,
            tyre_free.car.four_wheel.second[0].accumulators
        );
        // An out-of-table surface word is rejected before any state changes.
        let mut bad_ground = ground_car;
        if let Some(ground) = bad_ground.ground.as_mut() {
            ground.lanes.type_words[2] = 120;
        }
        let mut bad_state = RaceState::new(bad_ground);
        assert_eq!(
            bad_state.step_with_all_four_wheels(&straight_throttle(1)[0], &tyred),
            Err(WheelPostprocessError::TyreSurface(
                TyreSurfaceIndexError::Surface {
                    probe: 2,
                    surface: 120
                }
            ))
        );
        assert_eq!(bad_state.car(), bad_ground);

        let mut invalid = wheel.clone();
        invalid.contact_vector_indices[1] = 8;
        let before = state.car();
        assert_eq!(
            state.step_with_all_four_wheels(&straight_throttle(1)[0], &invalid),
            Err(WheelPostprocessError::ContactVectorIndex {
                lane: 1,
                vector_index: 8,
            })
        );
        assert_eq!(state.car(), before);
    }

    #[test]
    fn drag_bounds_the_speed_under_constant_force() {
        // With quadratic drag opposing a constant force, speed stops growing
        // without bound: a later tick's acceleration is smaller than an early one.
        let mut state = RaceState::new(CarState::at_rest());
        let trace = state.run_trace(&straight_throttle(200), &params());
        let early_gain = trace[1].car.velocity.x.sub(trace[0].car.velocity.x);
        let late_gain = trace[199].car.velocity.x.sub(trace[198].car.velocity.x);
        assert!(late_gain < early_gain);
    }

    #[test]
    fn steering_rotates_the_forward_axis_into_z() {
        // Steering a quarter turn (0x400 BAM) over the run turns the car so
        // velocity gains a z component.
        let script = vec![
            Controls {
                steer_rate: 0x40,
                forward_force: Fixed16::ONE,
                handbrake: false,
                raw_steer: Default::default(),
                raw_pedals: Default::default(),
                raw_gear_command_314: 0,
            };
            16
        ];
        let mut state = RaceState::new(CarState::at_rest());
        let trace = state.run_trace(&script, &params());
        let last = trace.last().unwrap().car;
        assert_ne!(last.velocity.z, Fixed16::ZERO);
    }

    #[test]
    fn seed_from_replay_region_a_and_replay_controls_populate_car_state() {
        use crate::replay::{
            REPLAY_CONTROL_OFFSET, REPLAY_INITIAL_STATE_BYTES, REPLAY_PHYSICS_OFFSET,
            ReplayControlRecord, ReplayRegionAView, decode_replay_control,
        };

        let mut region_a = Box::new([0u8; REPLAY_INITIAL_STATE_BYTES]);
        let phys_off = REPLAY_PHYSICS_OFFSET;
        let ctl_off = REPLAY_CONTROL_OFFSET;

        // Position (+0xc8), velocity (+0xd4), speed_ec (+0xec), contact_count (+0x674)
        region_a[phys_off + 0xc8..phys_off + 0xcc].copy_from_slice(&0x12_3456i32.to_le_bytes());
        region_a[phys_off + 0xcc..phys_off + 0xd0].copy_from_slice(&0x23_4567i32.to_le_bytes());
        region_a[phys_off + 0xd0..phys_off + 0xd4].copy_from_slice(&0x34_5678i32.to_le_bytes());
        region_a[phys_off + 0xd4..phys_off + 0xd8].copy_from_slice(&0x1_0000i32.to_le_bytes());
        region_a[phys_off + 0xec..phys_off + 0xf0].copy_from_slice(&0x1_0000i32.to_le_bytes());
        region_a[phys_off + 0x674..phys_off + 0x678].copy_from_slice(&4i32.to_le_bytes());
        region_a[phys_off + 0x700..phys_off + 0x704].copy_from_slice(&1i32.to_le_bytes());

        // Control: gear_43a = 2, target_43f = 2, residual_enable = 5, upright_404 = 1
        region_a[ctl_off + 0x404..ctl_off + 0x408].copy_from_slice(&1i32.to_le_bytes());
        region_a[ctl_off + 0x43a] = 2;
        region_a[ctl_off + 0x43b] = 5;
        region_a[ctl_off + 0x43f] = 2;

        let mut state = RaceState::new(CarState::at_rest());
        state.seed_from_replay_region_a_46d460(ReplayRegionAView::new(&region_a));
        let car = state.car();
        assert_eq!(
            car.position,
            Vec3::new(
                Fixed16::from_raw(0x12_3456),
                Fixed16::from_raw(0x23_4567),
                Fixed16::from_raw(0x34_5678),
            )
        );
        assert_eq!(car.velocity.x, Fixed16::ONE);
        assert_eq!(car.four_wheel.speed_ec, 0x1_0000);
        assert_eq!(car.motion.contact_count_674, 4);
        assert!(car.four_wheel.resting_700);
        assert!(car.four_wheel.upright_404);
        assert_eq!(car.four_wheel.gearbox.gear_43a, 2);
        assert_eq!(car.four_wheel.gearbox.target_43f, 2);
        assert_eq!(car.four_wheel.residual_enable, 5);
        assert!(car.ground.is_some());

        // Decode a replay control record and map to Controls
        let decoded = decode_replay_control(ReplayControlRecord([0x04, 0x02, 0x81, 0xe0]), 0);
        let controls = Controls::from_decoded_replay_control(&decoded);
        assert_eq!(controls.raw_steer.left_2fc, 0x20 << 10);
        assert!(controls.raw_steer.analog_410);
        assert_eq!(controls.raw_pedals.throttle_304, -66);
        assert!(!controls.raw_pedals.throttle_analog_414);
        assert!(controls.handbrake);
        assert_eq!(controls.raw_gear_command_314, 1);
    }
}
