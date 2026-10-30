//! Recovered intro / cut-scene full-motion-video selection, playback, and skip
//! (item H06).
//!
//! # Provenance
//!
//! The retail EU `Rally.exe` plays full-motion video through **Windows MCI**
//! using the **`avivideo`** device type (Video for Windows `.avi` via
//! `mciSendCommandA`, winmm.dll). There is no DirectShow, no `AVIStreamOpen`,
//! and no Smacker/Bink. The publisher/developer "logos" are static `.bmp`
//! images, not video; the only true FMV in `Rally.exe` is the generic MCI AVI
//! player (`FUN_004af8a0`), which has exactly two call sites in the entire
//! binary, both in the Rally School menu render (`FUN_0046f670`):
//! - `0x46f897` (`FUN_0046f7d0`): plays the fixed
//!   `\game\rallschl\avi\zoom.avi` transition;
//! - `0x46fc5a` (`FUN_0046f920`): plays the instructor briefing speech WAV
//!   followed by the difficulty- and task-selected clip
//!   `\game\rallschl\AVI\<EASY|MEDIUM|HARD>\<task>.avi` (in window rectangle
//!   `{x: 80, y: 32, w: 320, h: 208}`).
//!
//! (Note: `.rsp` files loaded by `FUN_0046de70` are Rally School Pan replay
//! files, not FMV scripts; any CD intro movie is played by the separate
//! `Game.exe` launcher executable before spawning `Rally.exe`.)
//!
//! The player (`FUN_004af8a0`) opens the `avivideo` device (`FUN_004afbd0`:
//! `MCI_OPEN` with `MCI_OPEN_ELEMENT | MCI_DGV_OPEN_WS | MCI_DGV_OPEN_PARENT`,
//! then `MCI_WINDOW`), positions it (`MCI_PUT`), plays with `MCI_NOTIFY`
//! (`FUN_004afcc0`), and runs a `PeekMessage` pump that ends naturally when MCI
//! posts `MM_MCINOTIFY` (`0x3B9`). The clip is skipped by pressing **ENTER**
//! (`VK_RETURN`) or **ESC** (`VK_ESCAPE`); **SPACE** toggles pause
//! (`FUN_004afd20`). `WM_ACTIVATEAPP` (`0x1C`) is deliberately not dispatched
//! during playback.
//!
//! ## Recorded negative results
//!
//! - **`Rally.exe` does no command-line parsing**: neither `GetCommandLineA` nor
//!   `__getmainargs` is imported, and no `-skip`/`nointro`/`windowed`/autorun
//!   tokens exist. There is no in-binary skip-intro flag; intro playback is
//!   gated only by game mode (`intro.rsp` is loaded only when the game-mode
//!   global is `< 2`) and by the presence of the `.avi` assets / `AVIPath`
//!   config value. Any launcher/setup executable is a separate binary, not part
//!   of this artifact.
//!
//! ## Separation of observation, inference, and design decision
//!
//! - **Observations**: the MCI command/flag constants and composites, the
//!   `MM_MCINOTIFY`/`VK_*`/`WM_ACTIVATEAPP` codes, the difficulty selector
//!   `(cfg[+0x6c0] & 0x18) >> 3`, the language selector `cfg[+0x18] & 7`, the
//!   `AVI\EASY\`/`AVI\MEDIUM\`/`AVI\HARD\` subdirectory table, the uppercase
//!   language directory names, the fixed `zoom.avi` relative path, the
//!   `AVIPath` config key, and the `intro.rsp`-load game-mode gate (`< 2`).
//! - **Inference**: the passive-vs-interactive pump selection (`DAT_0096efcc`
//!   is modelled as [`PumpMode`]); the clip order is data-driven by the `.rsp`
//!   script and is not reproduced here.
//! - **Design decision**: the exact variant filename suffix (the format string
//!   `DAT_004ed6e8` was not expanded) is an unknown, so this module reconstructs
//!   the selection *components* (difficulty subdir + language dir), not the full
//!   path; the window/HWND/MCI device I/O is backend glue (H07).
//!
//! The MCI constants are public Windows SDK values (not game data); the `.avi`
//! path tokens are source-root-relative path structure, not media payload.

use crate::localization::Language;

/// MCI command and flag constants (public Windows `mmsystem.h` values,
/// reproduced for the compatibility layer; not game data).
pub mod mci {
    /// `MCI_OPEN`.
    pub const OPEN: u32 = 0x803;
    /// `MCI_CLOSE`.
    pub const CLOSE: u32 = 0x804;
    /// `MCI_PLAY`.
    pub const PLAY: u32 = 0x806;
    /// `MCI_PAUSE`.
    pub const PAUSE: u32 = 0x809;
    /// `MCI_WINDOW`.
    pub const WINDOW: u32 = 0x841;
    /// `MCI_PUT`.
    pub const PUT: u32 = 0x842;

    /// `MCI_NOTIFY` (request `MM_MCINOTIFY` on completion).
    pub const NOTIFY: u32 = 0x1;
    /// `MCI_OPEN_TYPE` (open by device-type string, used for the warm-up probe).
    pub const OPEN_TYPE: u32 = 0x2000;
    /// `MCI_OPEN_ELEMENT` (open a named file element).
    pub const OPEN_ELEMENT: u32 = 0x200;
    /// `MCI_DGV_OPEN_WS` (window style).
    pub const DGV_OPEN_WS: u32 = 0x10000;
    /// `MCI_DGV_OPEN_PARENT` (parent window).
    pub const DGV_OPEN_PARENT: u32 = 0x20000;
    /// `MCI_DGV_WINDOW_HWND`.
    pub const DGV_WINDOW_HWND: u32 = 0x10000;
    /// `MCI_DGV_WINDOW_STATE`.
    pub const DGV_WINDOW_STATE: u32 = 0x40000;

    /// Composite flags for the element-open call (`FUN_004afbd0`): `0x30200`.
    pub const OPEN_ELEMENT_FLAGS: u32 = OPEN_ELEMENT | DGV_OPEN_WS | DGV_OPEN_PARENT;
    /// Composite flags for the window / put call: `0x50000`.
    pub const WINDOW_FLAGS: u32 = DGV_WINDOW_HWND | DGV_WINDOW_STATE;

    /// `MM_MCINOTIFY`: the window message MCI posts when a notified command
    /// (here `MCI_PLAY | MCI_NOTIFY`) completes.
    pub const MM_MCINOTIFY: u32 = 0x3B9;

    /// `SW_SHOW`, the window state passed with `MCI_WINDOW`.
    pub const SW_SHOW: u32 = 5;

    /// The MCI device type string the game opens.
    pub const DEVICE_TYPE: &str = "avivideo";
}

/// `VK_RETURN` (ENTER): skips the clip.
pub const VK_RETURN: u32 = 0x0D;
/// `VK_ESCAPE` (ESC): skips the clip.
pub const VK_ESCAPE: u32 = 0x1B;
/// `VK_SPACE`: toggles pause.
pub const VK_SPACE: u32 = 0x20;
/// `WM_ACTIVATEAPP`: deliberately not dispatched during playback.
pub const WM_ACTIVATEAPP: u32 = 0x1C;

/// Game-mode ceiling below which the intro `.rsp` script is loaded
/// (`DAT_005f1e20 < 2`).
pub const INTRO_SCRIPT_MODE_MAX: u32 = 2;

/// Config byte offset holding the difficulty bits (`DAT_00d3ca80 + 0x6c0`).
pub const DIFFICULTY_CONFIG_OFFSET: usize = 0x6c0;
/// Config byte offset holding the language bits (`DAT_00d3ca80 + 0x18`).
pub const LANGUAGE_CONFIG_OFFSET: usize = 0x18;

/// Fixed relative path of the `zoom.avi` transition clip (`FUN_0046f7d0`),
/// joined under the `AVIPath` config root.
pub const ZOOM_AVI_RELATIVE_PATH: &str = r"\game\rallschl\avi\zoom.avi";

/// Relative root the variant clip is built under (`FUN_0046f920`), before the
/// difficulty subdir and language directory.
pub const VARIANT_RELATIVE_ROOT: &str = r"\game\rallschl\";

/// Config key naming the FMV asset root.
pub const AVI_PATH_CONFIG_KEY: &str = "AVIPath";

/// The difficulty tier selecting the variant-clip subdirectory
/// (`FUN_0046f920`'s pointer table at `0x4f4ba8`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Difficulty {
    /// Index 0 -> `AVI\EASY\`.
    Easy,
    /// Index 1 -> `AVI\MEDIUM\`.
    Medium,
    /// Index 2 -> `AVI\HARD\`.
    Hard,
}

/// Decode the difficulty from the config byte: `(byte & 0x18) >> 3`. Index 3 is
/// the table's null slot (no variant clip), returned as `None`.
pub fn difficulty_from_config(config_byte: u8) -> Option<Difficulty> {
    match (config_byte & 0x18) >> 3 {
        0 => Some(Difficulty::Easy),
        1 => Some(Difficulty::Medium),
        2 => Some(Difficulty::Hard),
        _ => None,
    }
}

/// The variant-clip subdirectory for a difficulty (`AVI\EASY\` etc.).
pub fn avi_difficulty_subdir(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Easy => r"AVI\EASY\",
        Difficulty::Medium => r"AVI\MEDIUM\",
        Difficulty::Hard => r"AVI\HARD\",
    }
}

/// Decode the FMV language from the config byte: `byte & 7`, reusing the shared
/// [`Language`] enum. Only `0..=4` are mapped (the switch in `FUN_0046f920`);
/// higher values return `None`.
pub fn language_from_config(config_byte: u8) -> Option<Language> {
    Language::from_index((config_byte & 7) as usize)
}

/// The uppercase language directory name the variant clip is built with
/// (`ENGLISH`/`FRENCH`/`GERMAN`/`SPANISH`/`ITALIAN` at `0x4f4c3c`..`0x4f4c64`).
pub fn avi_language_dir(language: Language) -> &'static str {
    match language {
        Language::English => "ENGLISH",
        Language::French => "FRENCH",
        Language::German => "GERMAN",
        Language::Spanish => "SPANISH",
        Language::Italian => "ITALIAN",
    }
}

/// The recovered path *prefix* for a variant clip: the difficulty subdirectory
/// followed by the language directory (used by the briefing WAV half of
/// `FUN_0046f920` at `0x46f97d..0x46fb1c`).
pub fn avi_variant_prefix(difficulty: Difficulty, language: Language) -> String {
    format!(
        "{}{}",
        avi_difficulty_subdir(difficulty),
        avi_language_dir(language)
    )
}

/// Format string `DAT_004ed6e8` (`"%d"`) used by `FUN_0046f920` (`0x46fbe8`) to
/// format the task index `(cfg[+0x6c0] >> 5)`.
pub const BRIEFING_TASK_FORMAT_4ED6E8: &str = "%d";

/// Extension literal `DAT_004f4c1c` (`".avi"`) appended by `FUN_0046f920` (`0x46fc27`).
pub const AVI_EXTENSION_4F4C1C: &str = ".avi";

/// Window placement rectangle `{x: 80, y: 32, w: 320, h: 208}` (`0x46f93b..0x46f950`)
/// passed to `FUN_004af8a0` for the Rally School briefing AVI.
pub const BRIEFING_AVI_RECT_46F93B: [u16; 4] = [80, 32, 320, 208];

/// Build the complete relative path for the Rally School briefing `.avi` clip
/// (`FUN_0046f920`, `0x46fb20..0x46fc5f`):
/// `\game\rallschl\AVI\<EASY|MEDIUM|HARD>\<task>.avi` where
/// `difficulty = (cfg[+0x6c0] >> 3) & 3` and `task = cfg[+0x6c0] >> 5`.
#[must_use]
pub fn briefing_avi_relative_path_46f920(config_byte_6c0: u8) -> Option<String> {
    let difficulty = difficulty_from_config(config_byte_6c0)?;
    let task = config_byte_6c0 >> 5;
    Some(format!(
        "{}{}{}{}",
        VARIANT_RELATIVE_ROOT,
        avi_difficulty_subdir(difficulty),
        task,
        AVI_EXTENSION_4F4C1C
    ))
}

/// Build the relative path prefix + task stem for the Rally School briefing speech
/// `.wav` (`FUN_0046f920`, `0x46f97d..0x46fb1c`):
/// `AVI\<EASY|MEDIUM|HARD>\<LANGUAGE>\<task>.wav`.
#[must_use]
pub fn briefing_wav_relative_path_46f920(
    config_byte_6c0: u8,
    config_byte_18: u8,
) -> Option<String> {
    let difficulty = difficulty_from_config(config_byte_6c0)?;
    let language = language_from_config(config_byte_18)?;
    let task = config_byte_6c0 >> 5;
    Some(format!(
        "{}{}\\{}.wav",
        avi_difficulty_subdir(difficulty),
        avi_language_dir(language),
        task
    ))
}

/// Whether the intro `.rsp` pan-script (and thus the intro FMV) is loaded for a
/// given game-mode value: only when `game_mode < 2` (`FUN_0046de70`).
pub fn loads_intro_script(game_mode: u32) -> bool {
    game_mode < INTRO_SCRIPT_MODE_MAX
}

/// Which message pump the player runs, selected by `DAT_0096efcc`: the passive
/// pump just dispatches messages (no skip), the interactive pump also polls the
/// skip/pause keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PumpMode {
    /// `DAT_0096efcc == 0`: forced / non-skippable playback.
    Passive,
    /// `DAT_0096efcc != 0`: user may skip (ENTER/ESC) or pause (SPACE).
    Interactive,
}

impl PumpMode {
    /// Select the pump from the `DAT_0096efcc` flag value.
    pub fn from_flag(flag: u32) -> Self {
        if flag == 0 {
            PumpMode::Passive
        } else {
            PumpMode::Interactive
        }
    }
}

/// The per-iteration playback signals the pump observes: whether MCI has posted
/// `MM_MCINOTIFY`, and the current state of the skip/pause keys
/// (`GetAsyncKeyState`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PlaybackInput {
    /// `MM_MCINOTIFY` seen in the message queue (clip reached its end).
    pub mci_notify: bool,
    /// `VK_RETURN` currently down.
    pub enter: bool,
    /// `VK_ESCAPE` currently down.
    pub escape: bool,
    /// `VK_SPACE` currently down.
    pub space: bool,
}

/// What the playback loop does this iteration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaybackStep {
    /// Keep playing / keep pumping messages.
    Continue,
    /// Toggle pause/resume (`FUN_004afd20`).
    TogglePause,
    /// Clip ended naturally (`MM_MCINOTIFY`); tear down and return.
    Finished,
    /// User skipped (ENTER/ESC); tear down and return.
    Aborted,
}

/// Decide one iteration of the player loop (`FUN_004af8a0`).
///
/// `MM_MCINOTIFY` ends the clip in either pump mode. Skip/pause keys act only in
/// the interactive pump. ENTER/ESC is terminal (abort) and takes precedence over
/// SPACE: in the original the SPACE pause-toggle runs first and then the
/// ENTER/ESC check tears the clip down in the same iteration, so the net
/// terminal outcome when both are held is still an abort.
pub fn playback_step(mode: PumpMode, input: PlaybackInput) -> PlaybackStep {
    if input.mci_notify {
        return PlaybackStep::Finished;
    }
    if mode == PumpMode::Interactive {
        if input.enter || input.escape {
            return PlaybackStep::Aborted;
        }
        if input.space {
            return PlaybackStep::TogglePause;
        }
    }
    PlaybackStep::Continue
}

/// Whether a virtual-key code is a skip key (ENTER or ESC).
pub fn is_skip_key(vk: u32) -> bool {
    vk == VK_RETURN || vk == VK_ESCAPE
}

/// Whether a virtual-key code is the pause-toggle key (SPACE).
pub fn is_pause_key(vk: u32) -> bool {
    vk == VK_SPACE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mci_composite_flags_match_the_recovered_values() {
        assert_eq!(mci::OPEN_ELEMENT_FLAGS, 0x30200);
        assert_eq!(mci::WINDOW_FLAGS, 0x50000);
        assert_eq!(mci::DEVICE_TYPE, "avivideo");
        assert_eq!(mci::MM_MCINOTIFY, 0x3B9);
    }

    #[test]
    fn difficulty_decodes_from_the_0x18_bits() {
        assert_eq!(difficulty_from_config(0x00), Some(Difficulty::Easy));
        assert_eq!(difficulty_from_config(0x08), Some(Difficulty::Medium));
        assert_eq!(difficulty_from_config(0x10), Some(Difficulty::Hard));
        // Index 3 is the null slot -> no variant clip.
        assert_eq!(difficulty_from_config(0x18), None);
        // Unrelated bits are masked off.
        assert_eq!(difficulty_from_config(0xe7), Some(Difficulty::Easy));
        assert_eq!(difficulty_from_config(0xff), None);
    }

    #[test]
    fn difficulty_subdirs_match() {
        assert_eq!(avi_difficulty_subdir(Difficulty::Easy), r"AVI\EASY\");
        assert_eq!(avi_difficulty_subdir(Difficulty::Medium), r"AVI\MEDIUM\");
        assert_eq!(avi_difficulty_subdir(Difficulty::Hard), r"AVI\HARD\");
    }

    #[test]
    fn language_decodes_from_the_low_three_bits() {
        assert_eq!(language_from_config(0), Some(Language::English));
        assert_eq!(language_from_config(1), Some(Language::French));
        assert_eq!(language_from_config(4), Some(Language::Italian));
        // The switch only handles 0..=4.
        assert_eq!(language_from_config(5), None);
        assert_eq!(language_from_config(7), None);
        // Only the low three bits select.
        assert_eq!(language_from_config(0xf8 | 4), Some(Language::Italian));
    }

    #[test]
    fn language_dirs_are_uppercase() {
        assert_eq!(avi_language_dir(Language::English), "ENGLISH");
        assert_eq!(avi_language_dir(Language::Italian), "ITALIAN");
        assert_eq!(
            avi_variant_prefix(Difficulty::Medium, Language::German),
            r"AVI\MEDIUM\GERMAN"
        );
    }

    #[test]
    fn intro_script_gate_is_mode_below_two() {
        assert!(loads_intro_script(0));
        assert!(loads_intro_script(1));
        assert!(!loads_intro_script(2));
        assert!(!loads_intro_script(3));
    }

    #[test]
    fn pump_mode_from_flag() {
        assert_eq!(PumpMode::from_flag(0), PumpMode::Passive);
        assert_eq!(PumpMode::from_flag(1), PumpMode::Interactive);
        assert_eq!(PumpMode::from_flag(0xffff_ffff), PumpMode::Interactive);
    }

    #[test]
    fn notify_finishes_in_either_pump_mode() {
        let notify = PlaybackInput {
            mci_notify: true,
            ..Default::default()
        };
        assert_eq!(
            playback_step(PumpMode::Passive, notify),
            PlaybackStep::Finished
        );
        assert_eq!(
            playback_step(PumpMode::Interactive, notify),
            PlaybackStep::Finished
        );
    }

    #[test]
    fn interactive_pump_skips_and_pauses() {
        let enter = PlaybackInput {
            enter: true,
            ..Default::default()
        };
        let escape = PlaybackInput {
            escape: true,
            ..Default::default()
        };
        let space = PlaybackInput {
            space: true,
            ..Default::default()
        };
        assert_eq!(
            playback_step(PumpMode::Interactive, enter),
            PlaybackStep::Aborted
        );
        assert_eq!(
            playback_step(PumpMode::Interactive, escape),
            PlaybackStep::Aborted
        );
        assert_eq!(
            playback_step(PumpMode::Interactive, space),
            PlaybackStep::TogglePause
        );
        // ENTER/ESC takes precedence over SPACE (abort is terminal).
        let both = PlaybackInput {
            enter: true,
            space: true,
            ..Default::default()
        };
        assert_eq!(
            playback_step(PumpMode::Interactive, both),
            PlaybackStep::Aborted
        );
        assert_eq!(
            playback_step(PumpMode::Interactive, PlaybackInput::default()),
            PlaybackStep::Continue
        );
    }

    #[test]
    fn passive_pump_ignores_skip_and_pause_keys() {
        let keys = PlaybackInput {
            enter: true,
            escape: true,
            space: true,
            mci_notify: false,
        };
        assert_eq!(
            playback_step(PumpMode::Passive, keys),
            PlaybackStep::Continue
        );
    }

    #[test]
    fn key_classifiers() {
        assert!(is_skip_key(VK_RETURN));
        assert!(is_skip_key(VK_ESCAPE));
        assert!(!is_skip_key(VK_SPACE));
        assert!(is_pause_key(VK_SPACE));
        assert!(!is_pause_key(VK_RETURN));
    }

    #[test]
    fn briefing_avi_and_wav_paths_match_fun_0046f920() {
        assert_eq!(BRIEFING_TASK_FORMAT_4ED6E8, "%d");
        assert_eq!(AVI_EXTENSION_4F4C1C, ".avi");
        assert_eq!(BRIEFING_AVI_RECT_46F93B, [80, 32, 320, 208]);

        // Easy (level 0), task 2 -> cfg_6c0 = (2 << 5) | (0 << 3) = 0x40
        assert_eq!(
            briefing_avi_relative_path_46f920(0x40).as_deref(),
            Some(r"\game\rallschl\AVI\EASY\2.avi")
        );
        // Hard (level 2), task 4 -> cfg_6c0 = (4 << 5) | (2 << 3) = 0x90
        assert_eq!(
            briefing_avi_relative_path_46f920(0x90).as_deref(),
            Some(r"\game\rallschl\AVI\HARD\4.avi")
        );
        // Invalid level 3 returns None
        assert_eq!(briefing_avi_relative_path_46f920(0x18), None);

        // Briefing WAV path includes the uppercase language directory
        assert_eq!(
            briefing_wav_relative_path_46f920(0x48, 2).as_deref(),
            Some(r"AVI\MEDIUM\GERMAN\2.wav")
        );
        assert_eq!(briefing_wav_relative_path_46f920(0x48, 5), None);
    }
}
