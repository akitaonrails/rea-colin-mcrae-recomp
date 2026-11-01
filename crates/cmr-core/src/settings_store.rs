//! Portable settings image, save-slot model, and data-path resolution
//! (reconstruction item C05).
//!
//! # Recovered behavior
//!
//! The options/config blob is a single fixed-size structure persisted verbatim.
//! The loader (`FUN_004b9e70`) reads `save\cmRally.cfg` under the writable base
//! path, requires its length to be **exactly [`SETTINGS_BLOB_LEN`]** or it is
//! rejected (the statically-initialized defaults then stand), copies the bytes
//! into the live blob, then **zeroes a runtime region** at
//! [`SETTINGS_RUNTIME_ZERO_OFFSET`] for [`SETTINGS_RUNTIME_ZERO_LEN`] bytes
//! before re-deriving runtime state. The saver (`FUN_004ba5e0`) writes the full
//! blob unless the **do-not-save flag** (byte [`SETTINGS_NO_SAVE_FLAG_OFFSET`]
//! bit `0`) is set. There is no header, magic, version, or checksum: the only
//! validation is the exact-length gate. Save games are
//! `SAVE\SAVESLOT<n>.cms`, [`SAVE_SLOT_COUNT`] slots of exactly
//! [`SAVE_SLOT_LEN`] bytes under the writable base; a wrong-sized slot file is
//! treated as empty.
//!
//! # Portability policy (reconstruction decision)
//!
//! The shipped game resolves its writable (HD) and media (CD) base paths from
//! the Windows registry (`HKLM\SOFTWARE\Codemasters\Rally\1.00\{CDPath,HDPath}`)
//! with hard-coded absolute fallbacks (`c:\codemast\cmrally`, `q:\new_fe`,
//! `Q:\Game`). That strategy is deliberately **not** reproduced: this project
//! takes explicit roots (a `--source` media root / the documented
//! `CMR_ORIGINAL_DIR` example, and a writable user-data root) and builds every
//! path portably with [`std::path`]. [`DataPaths`] preserves the recovered
//! on-disk layout (the `save`/`SAVE` subdirectories, the `cmRally.cfg` name, the
//! `SAVESLOT<n>.cms` template, and the media `new_fe`/`game` subroots) while
//! owning neither registry access nor absolute defaults.
//!
//! The statically-initialized default blob is game data in the shipped `.data`
//! image and is **not** vendored; a caller supplies its own default image (from
//! the user's original data) when no settings file loads.
//!
//! See evidence `rea.official-eu.settings-paths`.

use std::path::{Path, PathBuf};

/// Byte length of the options/config blob (`FUN_004b9e70`/`FUN_004ba5e0` move
/// exactly this many bytes).
pub const SETTINGS_BLOB_LEN: usize = 0x1e20;

/// Offset of the runtime region zeroed after a successful load.
pub const SETTINGS_RUNTIME_ZERO_OFFSET: usize = 0x184;

/// Length of the runtime region zeroed after a successful load.
pub const SETTINGS_RUNTIME_ZERO_LEN: usize = 0x450;

/// Byte offset of the do-not-save flag; bit `0` suppresses persistence.
pub const SETTINGS_NO_SAVE_FLAG_OFFSET: usize = 0x08;

/// Bit within [`SETTINGS_NO_SAVE_FLAG_OFFSET`] that suppresses saving.
pub const SETTINGS_NO_SAVE_FLAG_BIT: u8 = 0x01;

/// Per-player stride within the blob.
pub const PLAYER_STRIDE: usize = 0x9c;

/// Number of players the blob carries.
pub const PLAYER_COUNT: usize = 2;

/// Writable subdirectory holding the config file.
pub const SETTINGS_DIR: &str = "save";

/// The config file name.
pub const SETTINGS_FILE_NAME: &str = "cmRally.cfg";

/// Writable subdirectory holding the save slots.
pub const SAVE_SLOT_DIR: &str = "SAVE";

/// Save-slot file-name prefix (`SAVESLOT<n>.cms`).
pub const SAVE_SLOT_PREFIX: &str = "SAVESLOT";

/// Save-slot file extension (without the dot).
pub const SAVE_SLOT_EXTENSION: &str = "cms";

/// Byte length of one save-slot file.
pub const SAVE_SLOT_LEN: usize = 0x6ac;

/// Number of save slots.
pub const SAVE_SLOT_COUNT: usize = 12;

/// Media subroot for the front-end data (`<media>/new_fe`).
pub const MEDIA_FRONTEND_SUBDIR: &str = "new_fe";

/// Media subroot for the in-game data (`<media>/game`).
pub const MEDIA_GAME_SUBDIR: &str = "game";

/// Why a settings image could not be accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsError {
    /// The file was not exactly [`SETTINGS_BLOB_LEN`] bytes, so the recovered
    /// loader rejects it and the defaults stand.
    WrongLength {
        /// The required length.
        expected: usize,
        /// The length actually read.
        actual: usize,
    },
}

/// A validated options/config blob image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsImage {
    bytes: Vec<u8>,
}

impl SettingsImage {
    /// Accept raw file bytes only when they are exactly [`SETTINGS_BLOB_LEN`]
    /// long, reproducing the loader's exact-size gate. A wrong length is
    /// rejected; the caller then keeps its own default image (the shipped
    /// defaults are game data and are not vendored here).
    pub fn from_file_bytes(raw: &[u8]) -> Result<Self, SettingsError> {
        if raw.len() != SETTINGS_BLOB_LEN {
            return Err(SettingsError::WrongLength {
                expected: SETTINGS_BLOB_LEN,
                actual: raw.len(),
            });
        }
        Ok(SettingsImage {
            bytes: raw.to_vec(),
        })
    }

    /// Whether the blob should be persisted: the do-not-save flag is clear.
    pub fn should_save(&self) -> bool {
        self.bytes[SETTINGS_NO_SAVE_FLAG_OFFSET] & SETTINGS_NO_SAVE_FLAG_BIT == 0
    }

    /// Zero the runtime region re-initialized after a load.
    pub fn clear_runtime_region(&mut self) {
        let end = SETTINGS_RUNTIME_ZERO_OFFSET + SETTINGS_RUNTIME_ZERO_LEN;
        self.bytes[SETTINGS_RUNTIME_ZERO_OFFSET..end].fill(0);
    }

    /// The validated blob bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Whether a save-slot file of the given length is occupied: the recovered
/// loader treats any slot whose file is not exactly [`SAVE_SLOT_LEN`] bytes as
/// empty.
pub fn save_slot_is_occupied(file_len: usize) -> bool {
    file_len == SAVE_SLOT_LEN
}

/// Portable resolver for the writable and media base paths.
///
/// Both roots are supplied explicitly by the caller (never read from the
/// registry or an absolute default): the media root is the `--source` /
/// `CMR_ORIGINAL_DIR` original-data directory, and the user-data root is the
/// writable location for the config and save slots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataPaths {
    user_data: PathBuf,
    media: PathBuf,
}

impl DataPaths {
    /// Build a resolver from an explicit writable user-data root and an explicit
    /// read-only media (original-data) root.
    pub fn new(user_data_root: impl Into<PathBuf>, media_root: impl Into<PathBuf>) -> Self {
        DataPaths {
            user_data: user_data_root.into(),
            media: media_root.into(),
        }
    }

    /// Path to the config file (`<user-data>/save/cmRally.cfg`).
    pub fn settings_file(&self) -> PathBuf {
        self.user_data.join(SETTINGS_DIR).join(SETTINGS_FILE_NAME)
    }

    /// Path to the modern settings file (`<user-data>/save/modern.cfg`).
    pub fn modern_settings_file(&self) -> PathBuf {
        self.user_data
            .join(SETTINGS_DIR)
            .join(crate::modern_settings::MODERN_SETTINGS_FILE_NAME)
    }

    /// Path to save slot `index` (`<user-data>/SAVE/SAVESLOT<index>.cms`).
    pub fn save_slot(&self, index: usize) -> PathBuf {
        self.user_data
            .join(SAVE_SLOT_DIR)
            .join(format!("{SAVE_SLOT_PREFIX}{index}.{SAVE_SLOT_EXTENSION}"))
    }

    /// The front-end media subroot (`<media>/new_fe`).
    pub fn frontend_root(&self) -> PathBuf {
        self.media.join(MEDIA_FRONTEND_SUBDIR)
    }

    /// The in-game media subroot (`<media>/game`).
    pub fn game_root(&self) -> PathBuf {
        self.media.join(MEDIA_GAME_SUBDIR)
    }

    /// The writable user-data root.
    pub fn user_data_root(&self) -> &Path {
        &self.user_data
    }

    /// The read-only media root.
    pub fn media_root(&self) -> &Path {
        &self.media
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob() -> Vec<u8> {
        vec![0u8; SETTINGS_BLOB_LEN]
    }

    #[test]
    fn exact_length_is_accepted_other_lengths_rejected() {
        assert!(SettingsImage::from_file_bytes(&blob()).is_ok());
        assert_eq!(
            SettingsImage::from_file_bytes(&[0u8; 16]),
            Err(SettingsError::WrongLength {
                expected: SETTINGS_BLOB_LEN,
                actual: 16,
            })
        );
        // One byte short of the gate is rejected.
        assert_eq!(
            SettingsImage::from_file_bytes(&vec![0u8; SETTINGS_BLOB_LEN - 1]),
            Err(SettingsError::WrongLength {
                expected: SETTINGS_BLOB_LEN,
                actual: SETTINGS_BLOB_LEN - 1,
            })
        );
    }

    #[test]
    fn do_not_save_flag_gates_persistence() {
        let mut raw = blob();
        let image = SettingsImage::from_file_bytes(&raw).unwrap();
        assert!(image.should_save());
        raw[SETTINGS_NO_SAVE_FLAG_OFFSET] = SETTINGS_NO_SAVE_FLAG_BIT;
        assert!(!SettingsImage::from_file_bytes(&raw).unwrap().should_save());
        // Other bits of the flag byte do not suppress saving.
        raw[SETTINGS_NO_SAVE_FLAG_OFFSET] = 0xfe;
        assert!(SettingsImage::from_file_bytes(&raw).unwrap().should_save());
    }

    #[test]
    fn clear_runtime_region_zeroes_exactly_the_recovered_span() {
        let raw = vec![0xabu8; SETTINGS_BLOB_LEN];
        let mut image = SettingsImage::from_file_bytes(&raw).unwrap();
        image.clear_runtime_region();
        let bytes = image.as_bytes();
        let end = SETTINGS_RUNTIME_ZERO_OFFSET + SETTINGS_RUNTIME_ZERO_LEN;
        // Neighbors untouched.
        assert_eq!(bytes[SETTINGS_RUNTIME_ZERO_OFFSET - 1], 0xab);
        assert_eq!(bytes[end], 0xab);
        // Region zeroed.
        assert!(
            bytes[SETTINGS_RUNTIME_ZERO_OFFSET..end]
                .iter()
                .all(|&b| b == 0)
        );
    }

    #[test]
    fn save_slot_occupancy_is_an_exact_size_gate() {
        assert!(save_slot_is_occupied(SAVE_SLOT_LEN));
        assert!(!save_slot_is_occupied(0));
        assert!(!save_slot_is_occupied(SAVE_SLOT_LEN - 1));
        assert!(!save_slot_is_occupied(SAVE_SLOT_LEN + 1));
    }

    #[test]
    fn data_paths_preserve_the_recovered_layout_portably() {
        let paths = DataPaths::new("/home/u/.local/share/cmr", "/mnt/original");
        assert_eq!(
            paths.settings_file(),
            Path::new("/home/u/.local/share/cmr/save/cmRally.cfg")
        );
        assert_eq!(
            paths.save_slot(3),
            Path::new("/home/u/.local/share/cmr/SAVE/SAVESLOT3.cms")
        );
        assert_eq!(paths.frontend_root(), Path::new("/mnt/original/new_fe"));
        assert_eq!(paths.game_root(), Path::new("/mnt/original/game"));
        assert_eq!(
            paths.user_data_root(),
            Path::new("/home/u/.local/share/cmr")
        );
        assert_eq!(paths.media_root(), Path::new("/mnt/original"));
    }

    #[test]
    fn save_slot_indices_span_the_recovered_slot_count() {
        let paths = DataPaths::new("u", "m");
        for index in 0..SAVE_SLOT_COUNT {
            let slot = paths.save_slot(index);
            assert_eq!(
                slot.file_name().unwrap().to_str().unwrap(),
                format!("SAVESLOT{index}.cms")
            );
        }
    }

    #[test]
    fn recovered_structural_constants() {
        assert_eq!(SETTINGS_BLOB_LEN, 7712);
        assert_eq!(
            SETTINGS_RUNTIME_ZERO_OFFSET + SETTINGS_RUNTIME_ZERO_LEN,
            0x5d4
        );
        assert_eq!(SAVE_SLOT_LEN, 1708);
        assert_eq!(SAVE_SLOT_COUNT, 12);
        assert_eq!(PLAYER_STRIDE * PLAYER_COUNT, 0x138);
    }
}
