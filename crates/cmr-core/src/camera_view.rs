//! The unified in-race **camera view controller** (`rea.official-eu.camera-modes`):
//! the per-frame camera driver / slot evaluator (`FUN_004343a0`/`FUN_00436490`)
//! that holds the active [`CameraMode`], advances it on the player's "next view"
//! input ([`camera_mode::cycle_next_view`]), and dispatches each frame to the
//! matching reconstructed placement to produce the camera world matrix.
//!
//! This ties the camera modules together into one driver the windowed session
//! can hold: chase ([`gameplay_camera`]) with its orientation ease, cockpit/mirror
//! ([`camera_interior`]), and the cinematic replay/TV cams ([`camera_cinematic`]).
//!
//! It reproduces the recovered **double-buffered mode-change crossfade** of
//! `FUN_004343a0`: a player "next view" request copies the live camera into the
//! other of two slots, retargets it to the new mode, and starts a countdown; each
//! frame while the countdown runs, **both** the old (A) and new (B) slot cameras
//! are re-evaluated live from the car pose and blended by the recovered weight
//! ([`camera_crossfade::blend_weight_q16`]/[`camera_crossfade::blend_pose`]), with
//! the displayed mode held at [`CameraMode::Blending`]; when it drains the new
//! slot becomes active. The helmet look overlay and the trackside director are
//! their own (recovered) modules and are not folded in here; modes without a
//! self-contained placement fall back to the chase camera.

use crate::camera_cinematic::{cinematic_camera, cinematic_offset_fixed, cinematic_offset_random};
use crate::camera_crossfade::{blend_pose, blend_weight_q16};
use crate::camera_interior::cockpit_camera;
use crate::camera_mode::{
    CameraMode, TransitionResult, ViewGate, cycle_next_view, resolve_transition,
};
use crate::fixed_point::{TRIG_TABLE_LEN, Vec3, build_sine_table};
use crate::game_rng::GameLcg;
use crate::gameplay_camera::{
    ChasePreset, GameplayCamera, START_ORBIT_TICKS, car_world_q16, start_orbit_camera,
};

/// Q16.16 one.
const ONE: i32 = 0x1_0000;

/// The windowed/in-race driver runs real-time NTSC with the slow-motion flag
/// (`DAT_00551af0`) clear, so the crossfade durations take the single (non-
/// doubled) base. The flag's exact identity is unresolved in the static image.
const SLOW_MOTION: bool = false;

/// One camera slot of the recovered per-player double buffer (the two `0x60`
/// records at `DAT_00550650`): its mode plus the per-mode placement state the
/// slot needs (its own eased chase camera and current cinematic shot offset).
#[derive(Clone)]
struct ViewSlot {
    mode: CameraMode,
    chase: GameplayCamera,
    cinematic_offset: [i32; 3],
    orbit_remaining: i32,
}

impl ViewSlot {
    fn new(mode: CameraMode) -> Self {
        Self {
            mode,
            chase: GameplayCamera::new(),
            cinematic_offset: cinematic_offset_fixed(),
            orbit_remaining: 0,
        }
    }
}

/// The stateful camera driver: the two mode slots, which one is active (the
/// recovered parity flag `DAT_00550644`), and the crossfade countdown
/// (`DAT_00550558`) / duration (`DAT_00550640`).
pub struct ViewController {
    slots: [ViewSlot; 2],
    /// The active (displayed, and during a blend the *old*/A) slot index.
    active: usize,
    /// Blend countdown: `< 1` means no blend is running (`0` is the swap frame).
    counter: i32,
    /// The crossfade duration the countdown started from.
    duration: i32,
    /// The engine BAM cosine table for the blend weight (`DAT_0082d380`, built
    /// at runtime; here rotated from the sine table a quarter turn).
    cos_lut: [i32; TRIG_TABLE_LEN],
    /// The BAM sine table the placements use.
    trig: [i32; TRIG_TABLE_LEN],
}

impl ViewController {
    /// A controller starting in `mode` (the game seeds [`CameraMode::ChaseNear`]).
    #[must_use]
    pub fn new(mode: CameraMode) -> Self {
        let trig = build_sine_table();
        // cos(x) = sin(x + quarter-turn); the BAM quarter-turn is 0x400 entries.
        let mut cos_lut = [0i32; TRIG_TABLE_LEN];
        for (i, slot) in cos_lut.iter_mut().enumerate() {
            *slot = trig[(i + 0x400) & 0xfff];
        }
        let slot = ViewSlot::new(mode);
        Self {
            slots: [slot.clone(), slot],
            active: 0,
            counter: -1,
            duration: 0,
            cos_lut,
            trig,
        }
    }

    /// Whether a mode-change crossfade is currently running.
    #[must_use]
    pub fn is_blending(&self) -> bool {
        self.counter >= 1
    }

    /// The player's selected view. During a crossfade this is the target (the new
    /// slot B); the displayed mode field itself holds [`CameraMode::Blending`] in
    /// the shipped driver, exposed via [`ViewController::is_blending`].
    #[must_use]
    pub fn mode(&self) -> CameraMode {
        if self.is_blending() {
            self.slots[self.active ^ 1].mode
        } else {
            self.slots[self.active].mode
        }
    }

    /// Advance to the next selectable view (the player's "change camera" input,
    /// `FUN_00434210` + `FUN_00434060`): pick the next mode under `gate`, copy the
    /// live slot into the other and retarget it (re-seeding the cinematic shot
    /// offset for a cinematic cam), then start the crossfade - or cut instantly
    /// when the gate suppresses the fade (a showroom side). A request while a
    /// blend is already running is refused (the recovered lock).
    pub fn cycle(&mut self, gate: ViewGate, rng: &mut GameLcg) {
        if self.is_blending() {
            return;
        }
        let current = self.slots[self.active].mode;
        let next = cycle_next_view(current, gate);
        if next == current {
            return;
        }

        // Copy the current slot into the other, then retarget it (FUN_00434060).
        let other = self.active ^ 1;
        self.slots[other] = self.slots[self.active].clone();
        self.slots[other].mode = next;
        match next {
            CameraMode::Bonnet => self.slots[other].cinematic_offset = cinematic_offset_fixed(),
            CameraMode::ReplayAction => {
                self.slots[other].cinematic_offset = cinematic_offset_random(rng)
            }
            _ => {}
        }

        match resolve_transition(current, next, true, SLOW_MOTION) {
            TransitionResult::Crossfade { frames, .. } => {
                // Start the blend; the active (old) slot stays displayed until the
                // countdown drains, then the parity flips to the new slot.
                self.counter = frames;
                self.duration = frames;
            }
            TransitionResult::Immediate { .. } => {
                // Instant cut (a showroom side, or a zero-length fade).
                self.active = other;
                self.counter = -1;
            }
            TransitionResult::Locked => {}
        }
    }

    /// Enter the 75-tick Mode 8 start-line camera orbit (`FUN_00439920` ->
    /// `FUN_00434060(0, 8, 0, 0)`).
    pub fn begin_start_orbit(&mut self) {
        self.slots[self.active].mode = CameraMode::Helmet;
        self.slots[self.active].orbit_remaining = START_ORBIT_TICKS;
        self.counter = -1;
        self.duration = 0;
    }

    /// Crossfade from the completed start-line orbit into `target` over 25
    /// ticks (`FUN_00439a55` -> `FUN_004360a0(0, 1)`).
    pub fn finish_start_orbit_to_mode(&mut self, target: CameraMode) {
        let other = self.active ^ 1;
        self.slots[other] = ViewSlot::new(target);
        self.slots[self.active].orbit_remaining = 0;
        self.counter = 25;
        self.duration = 25;
    }

    /// The camera world matrix for this frame (`FUN_004343a0`). When no blend is
    /// running the active slot is placed directly; during a blend both slots are
    /// re-evaluated live from the car pose and blended by the recovered weight,
    /// and on the frame the countdown reaches zero the new slot becomes active.
    pub fn update(&mut self, position: Vec3, heading: u16, pal: bool) -> [i32; 16] {
        if self.counter < 1 {
            // Not blending - or the swap frame (counter == 0): promote the new
            // slot, then display it.
            if self.counter == 0 {
                self.active ^= 1;
            }
            self.counter = -1;
            return self.place(self.active, position, heading, pal);
        }

        // Blending: evaluate the old (A = active) and new (B = other) cameras from
        // the current car pose, blend, and count down one frame.
        let a = self.place(self.active, position, heading, pal);
        let b = self.place(self.active ^ 1, position, heading, pal);
        let weight = blend_weight_q16(self.counter, self.duration, &self.cos_lut);
        self.counter -= 1;
        blend_pose(&a, &b, weight)
    }

    /// Place one slot's camera from the car pose, dispatched by its mode
    /// (`FUN_00436490`): chase modes ease toward the target; cockpit/mirror and the
    /// cinematic cams are placed directly; any mode without a self-contained
    /// placement here falls back to the near chase.
    fn place(&mut self, slot: usize, position: Vec3, heading: u16, pal: bool) -> [i32; 16] {
        match self.slots[slot].mode {
            CameraMode::ChaseNear | CameraMode::ChaseFar | CameraMode::ChaseThird => {
                let preset = self.slots[slot]
                    .mode
                    .chase_preset()
                    .unwrap_or(ChasePreset::Near);
                *self.slots[slot]
                    .chase
                    .update(position, heading, preset, pal)
            }
            CameraMode::CockpitA => {
                let car = car_world_q16(&self.trig, position, heading);
                cockpit_camera(&car, 0, false, &self.trig)
            }
            CameraMode::CockpitB => {
                let car = car_world_q16(&self.trig, position, heading);
                cockpit_camera(&car, 0, true, &self.trig)
            }
            CameraMode::Bonnet | CameraMode::ReplayAction => {
                let car = car_world_q16(&self.trig, position, heading);
                let car_pos = [car[12], car[13], car[14]];
                // Bonnet looks along the car forward; the replay cam keeps world up.
                let up_ref = if self.slots[slot].mode == CameraMode::Bonnet {
                    [car[8], car[9], car[10]]
                } else {
                    [0, ONE, 0]
                };
                cinematic_camera(car_pos, self.slots[slot].cinematic_offset, up_ref)
            }
            CameraMode::Helmet => {
                let car = car_world_q16(&self.trig, position, heading);
                let remaining = self.slots[slot].orbit_remaining;
                self.slots[slot].orbit_remaining = (remaining - 1).max(0);
                start_orbit_camera(&self.trig, &car, remaining)
            }
            _ => {
                // Trackside / showroom / sentinels: no self-contained
                // placement here - fall back to the near chase.
                *self.slots[slot]
                    .chase
                    .update(position, heading, ChasePreset::Near, pal)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed_point::Fixed16;

    fn v(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3::new(
            Fixed16::from_raw(x),
            Fixed16::from_raw(y),
            Fixed16::from_raw(z),
        )
    }

    fn translation(m: &[i32; 16]) -> [i32; 3] {
        [m[12], m[13], m[14]]
    }

    fn q16_dot(a: [i32; 3], b: [i32; 3]) -> i64 {
        (0..3)
            .map(|i| (i64::from(a[i]) * i64::from(b[i])) >> 16)
            .sum()
    }

    const IN_RACE_GATE: ViewGate = ViewGate {
        cockpit_enabled: true,
        showroom_allowed: false,
    };

    #[test]
    fn starts_in_the_seeded_mode_and_cycles() {
        let mut vc = ViewController::new(CameraMode::ChaseNear);
        assert_eq!(vc.mode(), CameraMode::ChaseNear);
        let mut rng = GameLcg::new(1);
        // Cockpit enabled, showroom blocked: near -> far ...
        vc.cycle(IN_RACE_GATE, &mut rng);
        // The target view is reported immediately; the blend is now running.
        assert_eq!(vc.mode(), CameraMode::ChaseFar);
        assert!(vc.is_blending());
    }

    #[test]
    fn chase_mode_places_the_camera_behind_and_above_a_still_car() {
        let mut vc = ViewController::new(CameraMode::ChaseNear);
        let m = vc.update(v(0, 0, 0), 0, false);
        // Near chase sits above the ground and behind the origin car.
        assert!(m[13] > 0, "camera above ground");
        assert_ne!(translation(&m), [0, 0, 0]);
    }

    #[test]
    fn cockpit_mode_places_the_eye_near_the_car() {
        let mut vc = ViewController::new(CameraMode::CockpitA);
        let m = vc.update(v(0, 0, 0), 0, false);
        // The cockpit eye is roughly at driver height (~1.0), close to the car.
        assert!(
            m[13] > 0 && m[13] < 0x2_0000,
            "eye near ~1.0 height, got {}",
            m[13]
        );
    }

    #[test]
    fn entering_the_replay_cam_seeds_a_random_offset_within_envelope() {
        // Put the active slot directly on the replay action cam with a random
        // framing (a gate that leaves only chase won't reach it).
        let mut vc = ViewController::new(CameraMode::ChaseNear);
        let mut rng = GameLcg::new(7);
        let active = vc.active;
        vc.slots[active].mode = CameraMode::ReplayAction;
        vc.slots[active].cinematic_offset = cinematic_offset_random(&mut rng);
        let m = vc.update(v(0, 0, 0), 0, false);
        // The eye is offset from the car by the cinematic distance (4.5..10).
        let off = translation(&m);
        let len = ((off[0] as f64 / 65536.0).powi(2)
            + (off[1] as f64 / 65536.0).powi(2)
            + (off[2] as f64 / 65536.0).powi(2))
        .sqrt();
        assert!(
            (4.0..10.5).contains(&len),
            "replay cam distance {len} out of envelope"
        );
    }

    #[test]
    fn a_crossfade_runs_for_its_duration_then_promotes_the_new_slot() {
        let mut vc = ViewController::new(CameraMode::ChaseNear);
        let mut rng = GameLcg::new(1);
        vc.cycle(IN_RACE_GATE, &mut rng);
        assert!(vc.is_blending());
        let duration = vc.duration;
        assert_eq!(duration, 25, "NTSC chase-to-chase crossfade is 25 frames");

        // Each blend frame produces a clean orthonormal pose, and a second cycle
        // is refused while the blend runs.
        for _ in 0..duration {
            assert!(vc.is_blending());
            vc.cycle(IN_RACE_GATE, &mut rng); // locked - no effect
            let m = vc.update(v(0, 0, 0), 0, false);
            for row in [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]] {
                let len2 = q16_dot(row, row);
                assert!(
                    (len2 - ONE as i64).abs() <= 8,
                    "blend axis not unit: {len2:#x}"
                );
            }
        }
        // The swap frame drains the countdown and the new mode becomes active.
        let _ = vc.update(v(0, 0, 0), 0, false);
        assert!(!vc.is_blending());
        assert_eq!(vc.mode(), CameraMode::ChaseFar);
    }
}
