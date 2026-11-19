//! Reconstruction of the wet-tyre and interior-rain sound: the loader
//! `FUN_0048d340` and the tick `FUN_0048d4c0` (player 0, after the surface
//! and landing sounds).
//!
//! The loader opens one channel at volume 0, loads `rain_in.wav` when the
//! weather type (`FUN_0046b220`) is 3 (starting it at once in the camera
//! modes 3 and 4) and `wet_out.wav` when the stage's `.srf` holds a wet
//! surface. The tick plays `wet_out` while a wheel is on a wet surface, by
//! speed plus slip, and in the interior views over a rain stage drives
//! `rain_in`'s volume and frequency from the speed. Both share the one
//! channel, so a camera change mid-stage crosses them over (kept). Every
//! volume goes through `FUN_0048d4a0` (`ftol(v * 0.66)`) and the
//! centre-proximity curve. A description of disassembly, not original
//! source. Evidence: `rea.official-eu.wet-sound`.

use crate::engine_sound::EngineVoices;

const SLIP: i32 = 0x3333;

fn qmul(a: i32, b: i32) -> i32 {
    ((i64::from(a) * i64::from(b)) >> 16) as i32
}

fn ftol(value: f64) -> i32 {
    if value.is_finite() && value > -2_147_483_649.0 && value < 2_147_483_648.0 {
        value as i32
    } else {
        i32::MIN
    }
}

/// The words the wet loader decides from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WetLoad {
    /// `FUN_0046b220() == 3`.
    pub rain_weather: bool,
    /// `FUN_0048b710()`: the surface loader saw a wet surface.
    pub wet_surface: bool,
    /// `DAT_00550494` at load time.
    pub camera_mode: i32,
}

/// The waves the loader registers (relative to the install root), in slot
/// order, and whether it starts `rain_in` looping at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WetBank {
    pub waves: Vec<&'static str>,
    pub start_rain: bool,
}

/// The wet sound's words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WetSoundState {
    /// `DAT_0082af40`: the last level.
    pub level: i32,
    /// `DAT_0082af48`: `rain_in` is loaded (and `wet_out` sits one slot on).
    pub rain_loaded: bool,
}

/// `FUN_0048d340`.
pub fn load_wet_bank(state: &mut WetSoundState, load: WetLoad) -> WetBank {
    let mut waves = Vec::new();
    state.rain_loaded = false;
    let mut start_rain = false;
    if load.rain_weather {
        waves.push("sounds\\weather\\rain_in.wav");
        start_rain = matches!(load.camera_mode, 3 | 4);
        state.rain_loaded = true;
    }
    if load.wet_surface {
        waves.push("sounds\\weather\\wet_out.wav");
    }
    WetBank { waves, start_rain }
}

/// The words one wet tick reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WetTickInput {
    pub surfaces: [i16; 4],
    pub contact: [u8; 4],
    pub longitudinal: [i32; 4],
    pub lateral: [i32; 4],
    pub speed_ec: i32,
    /// `ctl +0x164`.
    pub control_164: i32,
    /// `DAT_00550494`.
    pub camera_mode: i32,
    /// The static wet surface ids `0x4f9930` and `Kw` (`0x5020e4`).
    pub wet_surfaces: [i32; 15],
    pub wet_k: i32,
    pub centre_proximity: i32,
    pub master_percent: i32,
}

struct Out<'a, V: EngineVoices> {
    voices: &'a mut V,
    channel: u32,
    input: &'a WetTickInput,
}

impl<V: EngineVoices> Out<'_, V> {
    fn playing(&self) -> bool {
        self.voices.playing(self.channel)
    }
    fn curve(&self, v: i32) -> i32 {
        crate::engine_listener::centre_curve(self.input.centre_proximity, v)
    }
    /// `FUN_0048d4a0(ch, v)`: the master-scaled volume of `ftol(v * 0.66)`.
    fn volume(&mut self, v: i32) {
        let v = ftol(f64::from(v) * 0.66);
        let scaled = self.input.master_percent.wrapping_mul(v) / 100;
        self.voices.set_volume(self.channel, scaled);
    }
}

impl WetSoundState {
    fn fade<V: EngineVoices>(&mut self, out: &mut Out<'_, V>) {
        if out.playing() {
            self.level -= self.level / 8;
            if self.level > 0 {
                let v = out.curve(self.level);
                out.volume(v);
            } else {
                self.level = 0;
                out.volume(0);
            }
        }
    }

    /// `FUN_0048d4c0`.
    pub fn tick<V: EngineVoices>(
        &mut self,
        voices: &mut V,
        slot_base: u32,
        channel: u32,
        input: &WetTickInput,
    ) {
        let mut out = Out {
            voices,
            channel,
            input,
        };
        let wet = input.wet_surfaces.iter().any(|&id| {
            input
                .surfaces
                .iter()
                .any(|&surface| i32::from(surface) == id)
        });
        let interior = matches!(input.camera_mode, 3 | 4);
        // Off the wet surfaces, in the interior views, or with no wheel on
        // the ground, the level only decays.
        let grounded = input.contact.contains(&0);
        if !wet || interior || !grounded {
            self.fade(&mut out);
        } else {
            let hud = qmul(input.speed_ec, 0x43_1168);
            let mut v = if hud < 0x10000 {
                0
            } else {
                ((hud.min(0x64_0000) >> 16).wrapping_mul(input.wet_k)) / 110
            };
            if v != 0 {
                let lateral = input
                    .lateral
                    .iter()
                    .map(|x| x.wrapping_abs())
                    .fold(i32::MIN, i32::max);
                let longitudinal = input
                    .longitudinal
                    .iter()
                    .map(|x| x.wrapping_abs())
                    .fold(i32::MIN, i32::max);
                let a = lateral - SLIP;
                let b = (longitudinal - SLIP) / 2;
                let scale = ftol(f64::from(input.wet_k.wrapping_mul(2)) * 65536.0);
                if a > b && lateral > SLIP {
                    v = v.wrapping_add(qmul(a, scale) >> 16);
                } else if longitudinal > SLIP {
                    v = v.wrapping_add(qmul(b, scale) >> 16);
                }
            }
            let v = v.min(0x7fff);
            if !out.playing() {
                let wave = slot_base + u32::from(self.rain_loaded);
                out.voices.play(wave, channel, u32::MAX);
            }
            if out.playing() {
                let level = out.curve(v);
                out.volume(level);
            }
            self.level = v;
        }
        if self.rain_loaded && interior {
            let f = qmul(input.speed_ec, input.control_164).min(0x10000);
            let r = 100 - (qmul(f, 0x5a_0000) >> 16);
            if out.playing() {
                let level = ftol(f64::from(input.wet_k.wrapping_mul(r)) * 0.8 * 0.01);
                let level = out.curve(level);
                out.volume(level);
            }
            self.level = r;
            let frequency = (f.wrapping_mul(0x5622) >> 16).wrapping_add(0x5622);
            out.voices.set_frequency(channel, frequency as u32);
        }
    }
}

/// `FUN_0048d8e0(player, camera_mode)` (`0x48d8e0..0x48d952`):
/// Called immediately after `FUN_0048b680` whenever the active camera mode
/// changes (`0x4341e6`). In cockpit views (`3 | 4`), restarts the looping
/// `rain_in.wav` channel when rain is loaded; in exterior views (`1 | 2 | 5 | 6`),
/// stops the channel so `wet_out.wav` can take over cleanly.
pub fn set_wet_camera_mode<V: EngineVoices>(
    state: &WetSoundState,
    voices: &mut V,
    slot_base: u32,
    channel: u32,
    player: u8,
    camera_mode: i32,
) {
    if player == 1 {
        return;
    }
    match camera_mode {
        3 | 4 => {
            if voices.playing(channel) {
                voices.stop(channel);
            }
            if state.rain_loaded {
                voices.play(slot_base, channel, u32::MAX);
            }
        }
        1 | 2 | 5 | 6 => {
            if voices.playing(channel) {
                voices.stop(channel);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_mixer::Position3d;
    use std::collections::BTreeSet;

    #[derive(Default)]
    struct Voices {
        playing: BTreeSet<u32>,
        plays: Vec<(u32, u32)>,
        volumes: Vec<i32>,
        frequencies: Vec<u32>,
    }

    impl EngineVoices for Voices {
        fn playing(&self, channel: u32) -> bool {
            self.playing.contains(&channel)
        }
        fn play(&mut self, wave: u32, channel: u32, _loops: u32) {
            self.playing.insert(channel);
            self.plays.push((wave, channel));
        }
        fn play_with_loop_start(&mut self, wave: u32, channel: u32, _loop_start: u32) {
            self.play(wave, channel, 0);
        }
        fn stop(&mut self, channel: u32) {
            self.playing.remove(&channel);
        }
        fn set_volume(&mut self, _channel: u32, volume: i32) {
            self.volumes.push(volume);
        }
        fn set_frequency(&mut self, _channel: u32, frequency: u32) {
            self.frequencies.push(frequency);
        }
        fn set_position(&mut self, _channel: u32, _position: Position3d) {}
        fn wave_busy(&self, _wave: u32) -> bool {
            false
        }
    }

    fn input() -> WetTickInput {
        WetTickInput {
            surfaces: [7; 4],
            contact: [0; 4],
            longitudinal: [0; 4],
            lateral: [0; 4],
            speed_ec: 0x2_0000,
            control_164: 0x1000,
            camera_mode: 5,
            wet_surfaces: [7; 15],
            wet_k: 0x200,
            centre_proximity: 1000,
            master_percent: 100,
        }
    }

    #[test]
    fn the_loader_orders_rain_before_wet() {
        let mut state = WetSoundState::default();
        let bank = load_wet_bank(
            &mut state,
            WetLoad {
                rain_weather: true,
                wet_surface: true,
                camera_mode: 3,
            },
        );
        assert_eq!(
            bank.waves,
            vec![
                "sounds\\weather\\rain_in.wav",
                "sounds\\weather\\wet_out.wav"
            ]
        );
        assert!(bank.start_rain && state.rain_loaded);
        let bank = load_wet_bank(
            &mut state,
            WetLoad {
                rain_weather: false,
                wet_surface: true,
                camera_mode: 3,
            },
        );
        assert_eq!(bank.waves, vec!["sounds\\weather\\wet_out.wav"]);
        assert!(!bank.start_rain && !state.rain_loaded);
    }

    #[test]
    fn a_wet_surface_plays_wet_out_by_speed() {
        let mut state = WetSoundState::default();
        let mut voices = Voices::default();
        state.tick(&mut voices, 50, 19, &input());
        assert_eq!(voices.plays, vec![(50, 19)]);
        let hud = qmul(0x2_0000, 0x43_1168);
        let v = ((hud.min(0x64_0000) >> 16) * 0x200) / 110;
        assert_eq!(state.level, v);
        assert_eq!(voices.volumes.last(), Some(&ftol(f64::from(v) * 0.66)));
        // Off the wet surface it fades by an eighth.
        let dry = WetTickInput {
            surfaces: [1; 4],
            ..input()
        };
        state.tick(&mut voices, 50, 19, &dry);
        assert_eq!(state.level, v - v / 8);
    }

    #[test]
    fn interior_rain_drives_volume_and_frequency() {
        let mut state = WetSoundState {
            level: 0,
            rain_loaded: true,
        };
        let mut voices = Voices::default();
        voices.playing.insert(19);
        let cockpit = WetTickInput {
            camera_mode: 3,
            ..input()
        };
        state.tick(&mut voices, 50, 19, &cockpit);
        let f = qmul(0x2_0000, 0x1000);
        assert_eq!(state.level, 100 - (qmul(f, 0x5a_0000) >> 16));
        assert_eq!(
            voices.frequencies,
            vec![((f * 0x5622) >> 16) as u32 + 0x5622]
        );
    }

    #[test]
    fn camera_mode_switch_starts_and_stops_interior_rain_like_48d8e0() {
        let state = WetSoundState {
            level: 0,
            rain_loaded: true,
        };
        let mut voices = Voices::default();
        // Cockpit mode 3 starts looping `rain_in` on channel 19.
        set_wet_camera_mode(&state, &mut voices, 50, 19, 0, 3);
        assert!(voices.playing.contains(&19));
        assert_eq!(voices.plays, vec![(50, 19)]);
        // Exterior chase mode 5 stops it.
        set_wet_camera_mode(&state, &mut voices, 50, 19, 0, 5);
        assert!(!voices.playing.contains(&19));
        // Player 1 is ignored (`0x48d8e0`).
        set_wet_camera_mode(&state, &mut voices, 50, 19, 1, 3);
        assert!(!voices.playing.contains(&19));
    }
}
