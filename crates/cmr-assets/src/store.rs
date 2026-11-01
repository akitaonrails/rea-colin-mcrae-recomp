use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cmr_formats::stored_asset::{DecodeError, DecodeLimits, StoredAsset, decode_stored_asset};

use crate::verify::{join_portable, require_real_directory, sha256};
use crate::{
    AssetFile, AssetIndex, AssetProfile, HdPack, ImportSummary, ResolveError, ReviewedProfile,
    SchemaStatus, StorageEncoding, VerificationError, import_state_schema_status,
};

const MAX_IMPORT_STATE_BYTES: u64 = 64 * 1024;

/// Read-only access to a completed, profiled asset import.
#[derive(Debug)]
pub struct AssetStore {
    root: PathBuf,
    index: AssetIndex,
    files: BTreeMap<String, AssetFile>,
    limits: DecodeLimits,
    hd_pack: Option<HdPack>,
}

#[derive(Debug)]
pub enum AssetStoreError {
    Resolve(ResolveError),
    Verification(VerificationError),
    InvalidImportState {
        field: &'static str,
    },
    StateTooLarge(u64),
    StateJson(serde_json::Error),
    Decode {
        path: String,
        source: DecodeError,
    },
    StoredTooLarge {
        path: String,
        actual: u64,
        limit: usize,
    },
    EncodingMismatch {
        path: String,
        expected: StorageEncoding,
        actual: StorageEncoding,
    },
}

impl fmt::Display for AssetStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolve(error) => write!(formatter, "cannot resolve asset: {error}"),
            Self::Verification(error) => write!(formatter, "asset verification failed: {error}"),
            Self::InvalidImportState { field } => {
                write!(
                    formatter,
                    "import state does not match profile field {field}"
                )
            }
            Self::StateTooLarge(size) => {
                write!(
                    formatter,
                    "import state is {size} bytes; limit is {MAX_IMPORT_STATE_BYTES}"
                )
            }
            Self::StateJson(error) => write!(formatter, "invalid import state: {error}"),
            Self::Decode { path, source } => write!(formatter, "cannot decode {path}: {source}"),
            Self::StoredTooLarge {
                path,
                actual,
                limit,
            } => {
                write!(
                    formatter,
                    "stored asset {path} is {actual} bytes; limit is {limit}"
                )
            }
            Self::EncodingMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "storage encoding mismatch for {path}: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for AssetStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Resolve(error) => Some(error),
            Self::Verification(error) => Some(error),
            Self::StateJson(error) => Some(error),
            Self::Decode { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ResolveError> for AssetStoreError {
    fn from(error: ResolveError) -> Self {
        Self::Resolve(error)
    }
}

impl From<VerificationError> for AssetStoreError {
    fn from(error: VerificationError) -> Self {
        Self::Verification(error)
    }
}

impl AssetStore {
    pub fn open(
        root: &Path,
        reviewed: &ReviewedProfile,
        limits: DecodeLimits,
    ) -> Result<Self, AssetStoreError> {
        let profile = reviewed.profile();
        let profile_sha256 = reviewed.sha256();
        require_real_directory(root)?;
        let index = AssetIndex::from_profile(profile)?;
        let state_path = root.join("import-state.json");
        let metadata =
            fs::symlink_metadata(&state_path).map_err(|source| VerificationError::Io {
                path: state_path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(VerificationError::Symlink(state_path).into());
        }
        if !metadata.is_file() {
            return Err(VerificationError::SpecialFile(state_path).into());
        }
        if metadata.len() > MAX_IMPORT_STATE_BYTES {
            return Err(AssetStoreError::StateTooLarge(metadata.len()));
        }
        let mut state_bytes = Vec::with_capacity(metadata.len() as usize);
        fs::File::open(&state_path)
            .and_then(|file| {
                file.take(MAX_IMPORT_STATE_BYTES + 1)
                    .read_to_end(&mut state_bytes)
            })
            .map_err(|source| VerificationError::Io {
                path: state_path,
                source,
            })?;
        if state_bytes.len() as u64 > MAX_IMPORT_STATE_BYTES {
            return Err(AssetStoreError::StateTooLarge(state_bytes.len() as u64));
        }
        let state: ImportSummary =
            serde_json::from_slice(&state_bytes).map_err(AssetStoreError::StateJson)?;
        validate_state(&state, profile, profile_sha256)?;

        let files = profile
            .files
            .iter()
            .cloned()
            .map(|file| (file.path.clone(), file))
            .collect();
        Ok(Self {
            root: root.to_owned(),
            index,
            files,
            limits,
            hd_pack: None,
        })
    }

    /// Opt this store into content-addressed HD replacements. The original
    /// import remains the authenticated fallback for every missing/bad entry.
    #[must_use]
    pub fn with_hd_pack(mut self, hd_pack: Option<HdPack>) -> Self {
        self.hd_pack = hd_pack;
        self
    }

    /// Resolves, re-verifies, and decodes one imported asset. The returned
    /// bytes contain the inner CMPR payload for wrapped files.
    pub fn load(&self, requested: &str) -> Result<Vec<u8>, AssetStoreError> {
        require_real_directory(&self.root)?;
        let relative = self.index.resolve(requested)?;
        let expected = self.files.get(relative).expect("index and file map agree");
        if expected.size > self.limits.max_stored_bytes as u64 {
            return Err(AssetStoreError::StoredTooLarge {
                path: relative.to_owned(),
                actual: expected.size,
                limit: self.limits.max_stored_bytes,
            });
        }
        reject_symlinked_components(&self.root, relative)?;
        let stored = read_verified_file(&self.root, expected)?;
        let decoded = decode_stored_asset(&stored, self.limits).map_err(|source| {
            AssetStoreError::Decode {
                path: relative.to_owned(),
                source,
            }
        })?;
        let (actual, payload) = match decoded {
            StoredAsset::Raw(_) => (StorageEncoding::Raw, None),
            StoredAsset::Cmpr(payload) => (StorageEncoding::GzipCmpr, Some(payload)),
        };
        if actual != expected.encoding {
            return Err(AssetStoreError::EncodingMismatch {
                path: relative.to_owned(),
                expected: expected.encoding,
                actual,
            });
        }
        let original = payload.unwrap_or(stored);
        Ok(self
            .hd_pack
            .as_ref()
            .and_then(|pack| pack.replacement(&original))
            .unwrap_or(original))
    }
}

fn validate_state(
    state: &ImportSummary,
    profile: &AssetProfile,
    profile_sha256: &str,
) -> Result<(), AssetStoreError> {
    let expected_stored_bytes = profile
        .files
        .iter()
        .try_fold(0u64, |total, file| total.checked_add(file.size))
        .ok_or(AssetStoreError::InvalidImportState {
            field: "stored_bytes",
        })?;
    let expected_files = u64::try_from(profile.files.len())
        .map_err(|_| AssetStoreError::InvalidImportState { field: "files" })?;
    let checks = [
        (
            import_state_schema_status(state.schema_version) == SchemaStatus::Supported,
            "schema_version",
        ),
        (state.profile == profile.id, "profile"),
        (state.profile_sha256 == profile_sha256, "profile_sha256"),
        (state.files == expected_files, "files"),
        (state.stored_bytes == expected_stored_bytes, "stored_bytes"),
    ];
    for (matches, field) in checks {
        if !matches {
            return Err(AssetStoreError::InvalidImportState { field });
        }
    }
    Ok(())
}

fn reject_symlinked_components(root: &Path, relative: &str) -> Result<(), VerificationError> {
    let mut current = root.to_owned();
    let components = relative.split('/').collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|source| VerificationError::Io {
            path: current.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(VerificationError::Symlink(current));
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(VerificationError::SpecialFile(current));
        }
    }
    Ok(())
}

fn read_verified_file(root: &Path, expected: &AssetFile) -> Result<Vec<u8>, VerificationError> {
    let path = join_portable(root, &expected.path);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(VerificationError::MissingFile(expected.path.clone()));
        }
        Err(source) => return Err(VerificationError::Io { path, source }),
    };
    let metadata = file.metadata().map_err(|source| VerificationError::Io {
        path: path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(VerificationError::SpecialFile(path));
    }
    if metadata.len() != expected.size {
        return Err(VerificationError::SizeMismatch {
            path: expected.path.clone(),
            expected: expected.size,
            actual: metadata.len(),
        });
    }

    let mut bytes = Vec::with_capacity(expected.size as usize);
    file.take(expected.size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| VerificationError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() as u64 != expected.size {
        return Err(VerificationError::SizeMismatch {
            path: expected.path.clone(),
            expected: expected.size,
            actual: bytes.len() as u64,
        });
    }
    let actual = sha256(&bytes);
    if actual != expected.sha256 {
        return Err(VerificationError::HashMismatch {
            path: expected.path.clone(),
            expected: expected.sha256.clone(),
            actual,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use crate::verify::sha256;
    use crate::{AssetFile, import_source};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("cmr-store-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn cmpr(payload: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"CMPR").unwrap();
        encoder
            .write_all(&(payload.len() as u32).to_le_bytes())
            .unwrap();
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    }

    fn file(path: &str, bytes: &[u8], encoding: StorageEncoding) -> AssetFile {
        AssetFile {
            path: path.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
            encoding,
        }
    }

    fn fixture(base: &TestDirectory) -> (PathBuf, AssetProfile, Vec<u8>) {
        let source = base.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("Game")).unwrap();
        let wrapped = cmpr(b"decoded");
        fs::write(source.join("Game/raw.bin"), b"raw").unwrap();
        fs::write(source.join("Game/wrapped.bin"), &wrapped).unwrap();
        let profile = AssetProfile {
            schema_version: 1,
            id: "synthetic".to_owned(),
            roots: vec!["Game".to_owned()],
            empty_directories: Vec::new(),
            sentinels: Vec::new(),
            files: vec![
                file("Game/raw.bin", b"raw", StorageEncoding::Raw),
                file("Game/wrapped.bin", &wrapped, StorageEncoding::GzipCmpr),
            ],
        };
        (source, profile, wrapped)
    }

    fn reviewed_profile(profile: &AssetProfile) -> ReviewedProfile {
        ReviewedProfile::from_json(&serde_json::to_vec(profile).unwrap()).unwrap()
    }

    fn imported(base: &TestDirectory) -> (PathBuf, ReviewedProfile) {
        let (source, profile, _) = fixture(base);
        let reviewed = reviewed_profile(&profile);
        let output = base.0.join("runtime-data");
        import_source(&source, &output, &reviewed).unwrap();
        (output, reviewed)
    }

    fn limits() -> DecodeLimits {
        DecodeLimits {
            max_stored_bytes: 1024,
            max_payload_bytes: 1024,
        }
    }

    #[test]
    fn opens_matching_import_and_loads_raw_and_cmpr_assets() {
        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let store = AssetStore::open(&output, &reviewed, limits()).unwrap();
        assert_eq!(store.load("GAME\\RAW.BIN").unwrap(), b"raw");
        assert_eq!(store.load("game/wrapped.bin").unwrap(), b"decoded");
    }

    #[test]
    fn rejects_mismatched_state_and_substituted_profile() {
        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let mut substitute = reviewed.profile().clone();
        substitute.files[0].sha256 = "00".repeat(32);
        let substitute = reviewed_profile(&substitute);
        assert!(matches!(
            AssetStore::open(&output, &substitute, limits()),
            Err(AssetStoreError::InvalidImportState {
                field: "profile_sha256"
            })
        ));

        let state_path = output.join("import-state.json");
        let mut state: ImportSummary =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        state.profile = "wrong".to_owned();
        fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(matches!(
            AssetStore::open(&output, &reviewed, limits()),
            Err(AssetStoreError::InvalidImportState { field: "profile" })
        ));
    }

    #[test]
    fn rejects_an_import_state_from_another_schema_generation() {
        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let state_path = output.join("import-state.json");
        let mut state: ImportSummary =
            serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
        state.schema_version = 2;
        fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        // A newer/older generation is derived data to regenerate, never to
        // migrate in place.
        assert!(matches!(
            AssetStore::open(&output, &reviewed, limits()),
            Err(AssetStoreError::InvalidImportState {
                field: "schema_version"
            })
        ));
    }

    #[test]
    fn rechecks_hash_on_every_load() {
        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let store = AssetStore::open(&output, &reviewed, limits()).unwrap();
        fs::write(output.join("Game/raw.bin"), b"bad").unwrap();
        assert!(matches!(
            store.load("Game/raw.bin"),
            Err(AssetStoreError::Verification(
                VerificationError::HashMismatch { .. }
            ))
        ));
    }

    #[test]
    fn rejects_profile_encoding_that_disagrees_with_verified_bytes() {
        let base = TestDirectory::new();
        let output = base.0.join("runtime-data");
        fs::create_dir(&output).unwrap();
        fs::create_dir(output.join("Game")).unwrap();
        let wrapped = cmpr(b"decoded");
        fs::write(output.join("Game/wrapped.bin"), &wrapped).unwrap();
        let profile = AssetProfile {
            schema_version: 1,
            id: "synthetic".to_owned(),
            roots: vec!["Game".to_owned()],
            empty_directories: Vec::new(),
            sentinels: Vec::new(),
            files: vec![file("Game/wrapped.bin", &wrapped, StorageEncoding::Raw)],
        };
        let reviewed = reviewed_profile(&profile);
        let state = ImportSummary {
            schema_version: 1,
            profile: profile.id.clone(),
            profile_sha256: reviewed.sha256().to_owned(),
            files: 1,
            stored_bytes: wrapped.len() as u64,
        };
        fs::write(
            output.join("import-state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();

        let store = AssetStore::open(&output, &reviewed, limits()).unwrap();
        assert!(matches!(
            store.load("Game/wrapped.bin"),
            Err(AssetStoreError::EncodingMismatch {
                expected: StorageEncoding::Raw,
                actual: StorageEncoding::GzipCmpr,
                ..
            })
        ));
    }

    #[test]
    fn enforces_stored_size_before_reading() {
        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let store = AssetStore::open(
            &output,
            &reviewed,
            DecodeLimits {
                max_stored_bytes: 2,
                max_payload_bytes: 1024,
            },
        )
        .unwrap();
        assert!(matches!(
            store.load("Game/raw.bin"),
            Err(AssetStoreError::StoredTooLarge { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_path_components_after_import() {
        use std::os::unix::fs::symlink;

        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let store = AssetStore::open(&output, &reviewed, limits()).unwrap();
        let moved = output.join("real-game");
        fs::rename(output.join("Game"), &moved).unwrap();
        symlink(&moved, output.join("Game")).unwrap();
        assert!(matches!(
            store.load("Game/raw.bin"),
            Err(AssetStoreError::Verification(VerificationError::Symlink(_)))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_store_root_replaced_by_symlink_after_open() {
        use std::os::unix::fs::symlink;

        let base = TestDirectory::new();
        let (output, reviewed) = imported(&base);
        let store = AssetStore::open(&output, &reviewed, limits()).unwrap();
        let moved = base.0.join("moved-runtime-data");
        fs::rename(&output, &moved).unwrap();
        symlink(&moved, &output).unwrap();
        assert!(matches!(
            store.load("Game/raw.bin"),
            Err(AssetStoreError::Verification(
                VerificationError::SourceNotDirectory(_)
            ))
        ));
    }

    #[test]
    fn import_state_file_count_is_architecture_neutral_u64() {
        let state: ImportSummary = serde_json::from_str(
            r#"{
                "schema_version":1,
                "profile":"synthetic",
                "profile_sha256":"00",
                "files":4294967296,
                "stored_bytes":0
            }"#,
        )
        .unwrap();
        assert_eq!(state.files, 4_294_967_296);
    }
}
