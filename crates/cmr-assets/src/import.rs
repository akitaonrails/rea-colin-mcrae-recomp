use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::verify::{join_portable, require_real_directory, sha256, verify_file};
use crate::{
    AssetProfile, IMPORT_STATE_SCHEMA_VERSION, ReviewedProfile, VerificationError, verify_source,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub files: u64,
    pub stored_bytes: u64,
}

#[derive(Debug)]
pub enum ImportError {
    Verification(VerificationError),
    TooManyFiles,
    InvalidOutput(PathBuf),
    OutputParentNotDirectory(PathBuf),
    OutputExists(PathBuf),
    StageExists(PathBuf),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verification(error) => write!(formatter, "source verification failed: {error}"),
            Self::TooManyFiles => write!(formatter, "asset file count does not fit import state"),
            Self::InvalidOutput(path) => {
                write!(formatter, "invalid output path: {}", path.display())
            }
            Self::OutputParentNotDirectory(path) => {
                write!(
                    formatter,
                    "output parent is not a real directory: {}",
                    path.display()
                )
            }
            Self::OutputExists(path) => {
                write!(formatter, "output already exists: {}", path.display())
            }
            Self::StageExists(path) => {
                write!(formatter, "staging path already exists: {}", path.display())
            }
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Serialize(error) => write!(formatter, "cannot serialize import state: {error}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Verification(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Serialize(error) => Some(error),
            _ => None,
        }
    }
}

impl From<VerificationError> for ImportError {
    fn from(error: VerificationError) -> Self {
        Self::Verification(error)
    }
}

/// Copies a fully verified source into a fresh output directory.
///
/// The source is verified once as a complete tree, then every file is verified
/// again from the bytes copied into staging. Existing output or staging paths
/// are never overwritten.
pub fn import_source(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedProfile,
) -> Result<ImportSummary, ImportError> {
    let profile = reviewed.profile();
    let profile_sha256 = reviewed.sha256();

    let verified = verify_source(source, profile)?;
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_real_directory(parent)
        .map_err(|_| ImportError::OutputParentNotDirectory(parent.to_owned()))?;
    let source_root = fs::canonicalize(source).map_err(|error| ImportError::Io {
        path: source.to_owned(),
        source: error,
    })?;
    let output_parent = fs::canonicalize(parent).map_err(|error| ImportError::Io {
        path: parent.to_owned(),
        source: error,
    })?;
    if output_parent.starts_with(&source_root) {
        return Err(ImportError::InvalidOutput(output.to_owned()));
    }
    let name = output
        .file_name()
        .ok_or_else(|| ImportError::InvalidOutput(output.to_owned()))?;
    let name = name
        .to_str()
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(|| ImportError::InvalidOutput(output.to_owned()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(ImportError::OutputExists(output.to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ImportError::Io {
                path: output.to_owned(),
                source,
            });
        }
    }

    let stage = parent.join(format!(".{}.cmr-stage-{}", name, std::process::id()));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(ImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(ImportError::Io {
                path: stage,
                source,
            });
        }
    }

    fs::create_dir(&stage).map_err(|source| ImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate_stage(source, &stage, profile, profile_sha256, verified);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                cleanup_stage(&stage);
                return Err(ImportError::Io {
                    path: output.to_owned(),
                    source,
                });
            }
            Ok(summary)
        }
        Err(error) => {
            cleanup_stage(&stage);
            Err(error)
        }
    }
}

fn populate_stage(
    source: &Path,
    stage: &Path,
    profile: &AssetProfile,
    profile_sha256: &str,
    verified: crate::VerificationSummary,
) -> Result<ImportSummary, ImportError> {
    for relative in &profile.empty_directories {
        let directory = join_portable(stage, relative);
        fs::create_dir_all(&directory).map_err(|source| ImportError::Io {
            path: directory,
            source,
        })?;
    }

    for expected in &profile.files {
        let bytes = verify_file(source, expected, true)?;
        let destination = join_portable(stage, &expected.path);
        let parent = destination.parent().expect("profile file has a parent");
        fs::create_dir_all(parent).map_err(|source| ImportError::Io {
            path: parent.to_owned(),
            source,
        })?;
        fs::write(&destination, &bytes).map_err(|source| ImportError::Io {
            path: destination.clone(),
            source,
        })?;
        let written = fs::read(&destination).map_err(|source| ImportError::Io {
            path: destination.clone(),
            source,
        })?;
        let actual = sha256(&written);
        if actual != expected.sha256 {
            return Err(VerificationError::HashMismatch {
                path: expected.path.clone(),
                expected: expected.sha256.clone(),
                actual,
            }
            .into());
        }
    }

    let summary = ImportSummary {
        schema_version: IMPORT_STATE_SCHEMA_VERSION,
        profile: profile.id.clone(),
        profile_sha256: profile_sha256.to_owned(),
        files: u64::try_from(verified.files).map_err(|_| ImportError::TooManyFiles)?,
        stored_bytes: verified.stored_bytes,
    };
    let state = serde_json::to_vec_pretty(&summary).map_err(ImportError::Serialize)?;
    let state_path = stage.join("import-state.json");
    fs::write(&state_path, state).map_err(|source| ImportError::Io {
        path: state_path,
        source,
    })?;
    Ok(summary)
}

fn cleanup_stage(stage: &Path) {
    let _ = fs::remove_dir_all(stage);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{AssetFile, StorageEncoding};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("cmr-import-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn asset(path: &str, bytes: &[u8]) -> AssetFile {
        AssetFile {
            path: path.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
            encoding: StorageEncoding::Raw,
        }
    }

    fn fixture(base: &TestDirectory) -> (PathBuf, AssetProfile) {
        let source = base.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(source.join("Game/empty")).unwrap();
        fs::create_dir(source.join("New_Fe")).unwrap();
        fs::write(source.join("Rally.exe"), b"sentinel").unwrap();
        fs::write(source.join("Game/a.bin"), b"asset").unwrap();
        let profile = AssetProfile {
            schema_version: 1,
            id: "synthetic".to_owned(),
            roots: vec!["Game".to_owned(), "New_Fe".to_owned()],
            empty_directories: vec!["Game/empty".to_owned()],
            sentinels: vec![asset("Rally.exe", b"sentinel")],
            files: vec![asset("Game/a.bin", b"asset")],
        };
        (source, profile)
    }

    fn reviewed(profile: &AssetProfile) -> ReviewedProfile {
        ReviewedProfile::from_json(&serde_json::to_vec(profile).unwrap()).unwrap()
    }

    #[test]
    fn imports_exact_bytes_and_state_through_atomic_destination() {
        let base = TestDirectory::new();
        let (source, profile) = fixture(&base);
        let output = base.0.join("runtime-data");
        let reviewed = reviewed(&profile);
        let summary = import_source(&source, &output, &reviewed).unwrap();
        assert_eq!(fs::read(output.join("Game/a.bin")).unwrap(), b"asset");
        assert!(output.join("Game/empty").is_dir());
        let state: ImportSummary =
            serde_json::from_slice(&fs::read(output.join("import-state.json")).unwrap()).unwrap();
        assert_eq!(state, summary);
        assert_eq!(summary.files, 1);
    }

    #[test]
    fn refuses_existing_output_without_changing_it() {
        let base = TestDirectory::new();
        let (source, profile) = fixture(&base);
        let output = base.0.join("runtime-data");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("keep"), b"untouched").unwrap();
        let error = import_source(&source, &output, &reviewed(&profile)).unwrap_err();
        assert!(matches!(error, ImportError::OutputExists(_)));
        assert_eq!(fs::read(output.join("keep")).unwrap(), b"untouched");
    }

    #[test]
    fn failed_verification_creates_no_output_or_stage() {
        let base = TestDirectory::new();
        let (source, profile) = fixture(&base);
        fs::write(source.join("Game/a.bin"), b"changed").unwrap();
        let output = base.0.join("runtime-data");
        assert!(import_source(&source, &output, &reviewed(&profile)).is_err());
        assert!(!output.exists());
        let stage = base
            .0
            .join(format!(".runtime-data.cmr-stage-{}", std::process::id()));
        assert!(!stage.exists());
    }

    #[test]
    fn refuses_output_inside_source_tree() {
        let base = TestDirectory::new();
        let (source, profile) = fixture(&base);
        let output = source.join("Game/imported");
        let error = import_source(&source, &output, &reviewed(&profile)).unwrap_err();
        assert!(matches!(error, ImportError::InvalidOutput(path) if path == output));
    }
}
