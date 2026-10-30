//! Reconstruction of the music and start-of-stage countdown play paths.
//!
//! Two non-engine playback cues recovered together:
//!
//! - **Music** (frontend/replay): each is a single wave loaded *whole* (not
//!   streamed) through the managed-handle loader variant `FUN_004959f0` and
//!   played **looping** (`QSWaveMixPlayEx` dwFlags `0xffffffff`) on its own
//!   slot/channel pair. Frontend start is `FUN_00443b20` (one-shot init from the
//!   frontend init `FUN_00437ac0`); a second instance `FUN_004c9d70` is a
//!   per-frame keep-alive that re-starts the loop when the channel reports done
//!   and ramps a fade-in. Replay music (`replay.wav`) and the setup-screen
//!   `SETUP.WAV` build their paths inline and load through the same
//!   `FUN_004959f0` variant. All are gated by the sound-disabled flag
//!   `DAT_00d3ca88`.
//! - **Start-of-stage countdown** (`5-4-3-2-1-GO`): preloader `FUN_0046a740`
//!   ("PreSnd") loads six `countdwn\` waves into consecutive slots from a base
//!   (`DAT_00592ed8`) with the base loader `FUN_004958e0(path, 0x2b11, 0x10, 0)`
//!   (11025 Hz / 16-bit / mono), under `<speech base>` + `<language dir>` +
//!   `countdwn\`. In rally-school mode (`DAT_005f1e20 >= 2`) it additionally
//!   preloads the `intro2\` set. Play helper `FUN_0046af10(index)` plays slot
//!   `base + index` **one-shot** (`dwFlags 0`) on a shared SFX channel; the
//!   cadence driver is `FUN_00439b10`.
//!
//! The managed-handle loader `FUN_004959f0` differs from the base loader
//! `FUN_004958e0` by taking fixed format arguments and recording the returned
//! wave handle in a persistent table (stride `0x43`, counter `DAT_00a1f848`,
//! mixer handle `DAT_00a1f840`, ready flag `DAT_00a1f838`) - the "long-lived
//! looped wave" path, vs. the base "bulk SFX/speech slot" path.
//!
//! Observations: music is a single looped wave via `FUN_004959f0`; the two
//! music instances and their distinct globals; the countdown file set, its
//! 11025/16/mono format, one-shot play of slot `base+index`, and the `intro2\`
//! tutorial set. Inferences: `DAT_00d3ca88` is the sound-disabled gate; the two
//! music instances are the attract/menu variants. Unknowns (not reproduced):
//! the countdown index->value slot mapping and the `FUN_00439b10` tick cadence,
//! and the replay/SETUP control flow (their enclosing functions were not bound
//! by the decompiler). See evidence `rea.official-eu.audio-music-countdown`.
//!
//! # Music selection and lifecycle (H05)
//!
//! **Recorded decisive negative result: the retail EU build plays no
//! CD / Redbook / MCI music whatsoever.** `WINMM.dll` is imported and its MCI
//! interface (`mciSendCommandA`/`mciGetDeviceIDA`) is used, but every MCI call
//! targets the `avivideo` device for full-motion video (see
//! [`crate::intro_video`]), never a `cdaudio` device; there is no
//! `mciSendStringA`, no `auxGetNumDevs`/`auxSetVolume` CD-volume control, and no
//! `cdaudio`/`mci`/`music` command strings. Game music is exclusively the QSound
//! PCM `.wav` path (the two [`MusicTrack`]s). A faithful reconstruction must not
//! implement any CD/MCI music subsystem.
//!
//! "Which music plays" is therefore not a PCM-vs-CD choice but a pure context
//! choice. The global context flag `DAT_00d3ca88` (frontend-vs-in-race) gates
//! playback: frontend music (`FUN_00443b20`) and the stage countdown
//! (`FUN_0046a740`) start **only when it is `0`** ([`FRONTEND_CONTEXT`] /
//! [`frontend_music_active`]); the frontend context loops `frontend.wav`, the
//! replay context loops `replay.wav`, and in-stage there is no background music.
//! Start (`FUN_00443b20`) loads the whole wave, sets the volume from the config
//! byte `DAT_00d3ca80 + 7` ([`music_volume`]: `(byte & 0x7f) * 400000 / 100`),
//! and plays looping (loop count `0xffffffff`). The per-frame keep-alive
//! `FUN_004c9d70` ([`music_keepalive`]) performs a fade-in ramp toward a target
//! derived from a second config block (`0x00d3ac60 + 0xf`, [`music_fade_target`]:
//! `(byte & 0x7f) * 0x14`) while the voice plays, and reloads + restarts the
//! loop when the voice drains. This resolves the H02 "volume scaling units
//! unknown" note (the scaling is recovered; the absolute QMixer units it feeds
//! remain a mixer-backend policy). The replay-context start site itself was not
//! bound by the decompiler, so [`frontend_music_active`] models only the
//! recovered frontend gate.
//!
//! The config byte offsets are source-root-independent structural metadata; no
//! CD subsystem, no MCI music path, and no payload bytes are reproduced.

use crate::audio_mixer::{PLAY_LOOPING, PLAY_ONE_SHOT};
use crate::audio_pacenote::SpeechLanguage;

/// Music plays on a sustained loop.
pub const MUSIC_PLAY_FLAGS: u32 = PLAY_LOOPING;

/// A looped music track, each loaded whole through the managed-handle loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicTrack {
    /// Frontend/menu music (`FUN_00443b20` / keep-alive `FUN_004c9d70`).
    Frontend,
    /// Replay music.
    Replay,
}

impl MusicTrack {
    /// The recovered wave path (relative to the installation root), reusing the
    /// catalogued constants in [`crate::audio_assets`].
    pub fn path(self) -> &'static str {
        match self {
            MusicTrack::Frontend => crate::audio_assets::FRONTEND_MUSIC,
            MusicTrack::Replay => crate::audio_assets::REPLAY_MUSIC,
        }
    }
}

/// Loop count passed to the play call for music (`0xffffffff` = play forever).
pub const MUSIC_LOOP_COUNT: u32 = 0xffff_ffff;

/// Value of the context flag `DAT_00d3ca88` in which frontend music and the
/// stage countdown start (frontend / menu / replay context).
pub const FRONTEND_CONTEXT: u32 = 0;

/// Config byte offset (into `DAT_00d3ca80`) holding the music volume setting.
pub const MUSIC_VOLUME_CONFIG_OFFSET: usize = 7;

/// Config byte offset (into the second config block at `0x00d3ac60`) holding the
/// fade-in target level.
pub const MUSIC_FADE_CONFIG_OFFSET: usize = 0xf;

/// Mask applied to both the volume and fade config bytes (`0..=127`).
pub const MUSIC_CONFIG_MASK: u8 = 0x7f;

/// Numerator of the recovered music-volume scaling (`FUN_00443b20`).
pub const MUSIC_VOLUME_NUMERATOR: i32 = 400_000;

/// Denominator of the recovered music-volume scaling.
pub const MUSIC_VOLUME_DENOMINATOR: i32 = 100;

/// Per-step scale of the fade-in target level (`FUN_004c9d70`).
pub const MUSIC_FADE_TARGET_SCALE: i32 = 0x14;

/// The mixer-volume per ramp step the keep-alive fade applies (`FUN_004c97f0`:
/// `QSWaveMixSetVolume(channel, counter * 20000 / 100)` = `counter * 200`).
pub const MUSIC_FADE_VOLUME_STEP: i32 = 20_000 / 100;

/// The mixer volume the keep-alive fade sets for a ramp count (`FUN_004c97f0`):
/// `fade_elapsed * 200`. At the fade target ([`music_fade_target`]) this equals
/// the full [`music_volume`] (`cfg*20 * 200 == cfg*400000/100`), so the ramp
/// lands exactly on the configured volume.
#[must_use]
pub fn music_fade_volume(fade_elapsed: i32) -> i32 {
    fade_elapsed * MUSIC_FADE_VOLUME_STEP
}

/// Whether frontend music / the stage countdown is active for a context flag
/// value, reproducing the `DAT_00d3ca88 == 0` gate.
pub fn frontend_music_active(context: u32) -> bool {
    context == FRONTEND_CONTEXT
}

/// Scale a music-volume config byte to the value passed to the mixer, exactly as
/// `FUN_00443b20` does: `(byte & 0x7f) * 400000 / 100`. The absolute QMixer
/// units this feeds are a mixer-backend policy.
pub fn music_volume(config_byte: u8) -> i32 {
    ((config_byte & MUSIC_CONFIG_MASK) as i32 * MUSIC_VOLUME_NUMERATOR) / MUSIC_VOLUME_DENOMINATOR
}

/// The fade-in target level the keep-alive ramps toward, from the second config
/// block's byte (`FUN_004c9d70`): `(byte & 0x7f) * 0x14`.
pub fn music_fade_target(config_byte: u8) -> i32 {
    (config_byte & MUSIC_CONFIG_MASK) as i32 * MUSIC_FADE_TARGET_SCALE
}

/// What the per-frame music keep-alive (`FUN_004c9d70`) does this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicKeepAlive {
    /// The loop is playing and has reached the fade target: hold steady.
    HoldFadedIn,
    /// The loop is playing but below the fade target: advance the fade-in ramp.
    FadeIn,
    /// The loop has drained: reload the wave and restart it looping (and reset
    /// the fade ramp).
    Restart,
}

/// Decide one keep-alive step, reproducing `FUN_004c9d70`'s branches: a drained
/// voice restarts the loop; a playing voice below the fade target advances the
/// ramp; otherwise the volume holds.
pub fn music_keepalive(voice_playing: bool, fade_elapsed: i32, fade_target: i32) -> MusicKeepAlive {
    if !voice_playing {
        MusicKeepAlive::Restart
    } else if fade_elapsed < fade_target {
        MusicKeepAlive::FadeIn
    } else {
        MusicKeepAlive::HoldFadedIn
    }
}

/// The directory (under the selected speech-language dir) holding the
/// start-of-stage countdown waves (`countdwn\`).
pub const COUNTDOWN_DIR: &str = r"countdwn\";

/// Recovered sample rate/bit depth/channel layout shared by the countdown and
/// `intro2\` waves (base loader `FUN_004958e0(path, 0x2b11, 0x10, 0)`).
pub const COUNTDOWN_WAVE_RATE_HZ: u32 = 11_025;
pub const COUNTDOWN_WAVE_BITS: u16 = 16;

/// Countdown waves always play one-shot.
pub const COUNTDOWN_PLAY_FLAGS: u32 = PLAY_ONE_SHOT;

/// The six `countdwn\` waves, as observed in `.rdata` (the spoken count-in
/// `5-4-3-2-1-GO`). The exact slot offset each receives within the bank (the
/// index `FUN_0046af10` plays) was not confidently recovered, so no
/// index->value mapping is asserted here.
pub const COUNTDOWN_WAVES: [&str; 6] = ["go!.wav", "1.wav", "2.wav", "3.wav", "4.wav", "5.wav"];

/// The `intro2\` countdown set additionally preloaded in rally-school/tutorial
/// mode (`DAT_005f1e20 >= 2`).
pub const INTRO2_WAVES: [&str; 4] = [
    r"intro2\three.wav",
    r"intro2\two.wav",
    r"intro2\one.wav",
    r"intro2\go.wav",
];

/// Build the countdown wave path relative to the source root, reproducing the
/// loader's concatenation `<language dir>` + `countdwn\` + `<filename>`
/// (e.g. `sounds\speech\english\countdwn\go!.wav`). The absolute path the game
/// forms additionally prepends the installation root.
pub fn countdown_relative_path(language: SpeechLanguage, filename: &str) -> String {
    format!("{}{COUNTDOWN_DIR}{filename}", language.dir())
}

/// Start-of-stage countdown cue (`FUN_00439b50` -> `FUN_0046af10`, `0x439d08` / `0x439d3b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountdownCue {
    Go,
    One,
    Two,
    Three,
    Four,
    Five,
}

impl CountdownCue {
    /// Filename under `<language dir>countdwn\` (`COUNTDOWN_WAVES`).
    #[must_use]
    pub const fn filename(self) -> &'static str {
        match self {
            Self::Go => COUNTDOWN_WAVES[0],
            Self::One => COUNTDOWN_WAVES[1],
            Self::Two => COUNTDOWN_WAVES[2],
            Self::Three => COUNTDOWN_WAVES[3],
            Self::Four => COUNTDOWN_WAVES[4],
            Self::Five => COUNTDOWN_WAVES[5],
        }
    }

    /// Relative path under `game\` for the selected speech language.
    #[must_use]
    pub fn relative_path(self, language: SpeechLanguage) -> String {
        countdown_relative_path(language, self.filename())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_tracks_resolve_to_catalogued_paths() {
        assert_eq!(
            MusicTrack::Frontend.path(),
            crate::audio_assets::FRONTEND_MUSIC
        );
        assert_eq!(MusicTrack::Replay.path(), crate::audio_assets::REPLAY_MUSIC);
        assert!(MusicTrack::Frontend.path().ends_with(r"frontend.wav"));
        assert!(MusicTrack::Replay.path().ends_with(r"replay.wav"));
        // Music loops; countdown is one-shot.
        assert_eq!(MUSIC_PLAY_FLAGS, PLAY_LOOPING);
        assert_eq!(COUNTDOWN_PLAY_FLAGS, PLAY_ONE_SHOT);
    }

    #[test]
    fn frontend_music_gate_is_context_zero() {
        assert!(frontend_music_active(FRONTEND_CONTEXT));
        assert!(frontend_music_active(0));
        assert!(!frontend_music_active(1));
        assert!(!frontend_music_active(0xffff_ffff));
    }

    #[test]
    fn music_volume_scales_the_config_byte() {
        // 0 -> 0; full 127 -> 127*400000/100 = 508000.
        assert_eq!(music_volume(0), 0);
        assert_eq!(
            music_volume(0x7f),
            127 * MUSIC_VOLUME_NUMERATOR / MUSIC_VOLUME_DENOMINATOR
        );
        assert_eq!(music_volume(0x7f), 508_000);
        // The high bit is masked off (0xff -> 0x7f).
        assert_eq!(music_volume(0xff), music_volume(0x7f));
        // A mid value.
        assert_eq!(music_volume(50), 200_000);
    }

    #[test]
    fn music_fade_target_scales_the_second_config_byte() {
        assert_eq!(music_fade_target(0), 0);
        assert_eq!(music_fade_target(0x7f), 127 * 0x14);
        assert_eq!(music_fade_target(0x80), 0); // only the low 7 bits select
        assert_eq!(music_fade_target(10), 10 * 0x14);
    }

    #[test]
    fn keepalive_restarts_when_drained_and_fades_otherwise() {
        // Drained voice -> reload + restart (regardless of the fade numbers).
        assert_eq!(music_keepalive(false, 0, 100), MusicKeepAlive::Restart);
        assert_eq!(music_keepalive(false, 500, 100), MusicKeepAlive::Restart);
        // Playing but below target -> ramp.
        assert_eq!(music_keepalive(true, 40, 100), MusicKeepAlive::FadeIn);
        // Playing at/above target -> hold.
        assert_eq!(music_keepalive(true, 100, 100), MusicKeepAlive::HoldFadedIn);
        assert_eq!(music_keepalive(true, 200, 100), MusicKeepAlive::HoldFadedIn);
        // A zero target holds immediately.
        assert_eq!(music_keepalive(true, 0, 0), MusicKeepAlive::HoldFadedIn);
        // Music loops forever.
        assert_eq!(MUSIC_LOOP_COUNT, 0xffff_ffff);
    }

    #[test]
    fn fade_ramp_lands_on_the_configured_full_volume() {
        // The per-step volume is counter*200.
        assert_eq!(music_fade_volume(0), 0);
        assert_eq!(music_fade_volume(1), 200);
        assert_eq!(music_fade_volume(50), 10_000);
        // Ramping to the fade target yields exactly the full configured volume
        // (cfg*20 steps * 200 == cfg*400000/100), for every config byte.
        for cfg in [0u8, 1, 10, 50, 0x7f] {
            assert_eq!(music_fade_volume(music_fade_target(cfg)), music_volume(cfg));
        }
    }

    #[test]
    fn countdown_set_matches_recovery() {
        assert_eq!(COUNTDOWN_WAVES.len(), 6);
        assert_eq!(COUNTDOWN_WAVES[0], "go!.wav");
        assert_eq!(
            &COUNTDOWN_WAVES[1..],
            &["1.wav", "2.wav", "3.wav", "4.wav", "5.wav"]
        );
        assert_eq!(COUNTDOWN_WAVE_RATE_HZ, 11_025);
        assert_eq!(COUNTDOWN_WAVE_BITS, 16);
        assert_eq!(INTRO2_WAVES.len(), 4);
        assert_eq!(INTRO2_WAVES[3], r"intro2\go.wav");
    }

    #[test]
    fn countdown_path_matches_the_countdwn_shape() {
        assert_eq!(
            countdown_relative_path(SpeechLanguage::English, "go!.wav"),
            r"sounds\speech\english\countdwn\go!.wav"
        );
        assert_eq!(
            countdown_relative_path(SpeechLanguage::Italian, "3.wav"),
            r"sounds\speech\italian\countdwn\3.wav"
        );
    }
}
