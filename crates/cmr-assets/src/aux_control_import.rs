use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cmr_formats::pe_aux_control_initializer::{
    AUX_CONTROL_COMMON_WORD_COUNT, AUX_CONTROL_ENCODED_LEN, AUX_CONTROL_FORMAT_VERSION,
    AUX_CONTROL_MULTIPLIER_COUNT, AUX_CONTROL_POLICY_COUNT, AUX_CONTROL_POLICY_DIVISOR_COUNT,
    AUX_CONTROL_POLICY_WORD_COUNT, AuxControlCodecError, AuxControlConstantWords,
    AuxControlInitializerError, encode_aux_control_constants,
    extract_aux_control_constants as extract_format_aux_control_constants,
};
use serde::{Deserialize, Serialize};

use crate::aux_control_profile::{
    AuxControlEncoding, AuxControlProfile, AuxControlProfileError, ReviewedAuxControlProfile,
};
use crate::verify::{require_real_directory, sha256};

pub const AUX_CONTROL_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const AUX_CONTROL_DATA_FILE: &str = "aux-control-constants-v1.bin";
pub const AUX_CONTROL_STATE_FILE: &str = "aux-control-constants-import-state.json";

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxControlImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxControlImportShape {
    pub format_version: u32,
    pub common_word_count: u32,
    pub multiplier_count: u32,
    pub policy_count: u32,
    pub policy_word_count: u32,
    pub policy_divisor_count: u32,
    pub encoding: AuxControlEncoding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuxControlImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: AuxControlImportFile,
    pub output: AuxControlImportFile,
    pub shape: AuxControlImportShape,
}

#[derive(Debug)]
pub enum AuxControlImportError {
    Profile(AuxControlProfileError),
    SourceNotDirectory(PathBuf),
    Symlink(PathBuf),
    SpecialFile(PathBuf),
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    InvalidOutput(PathBuf),
    OutputParentNotDirectory(PathBuf),
    OutputExists(PathBuf),
    StageExists(PathBuf),
    Format(AuxControlInitializerError),
    Codec(AuxControlCodecError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}

impl fmt::Display for AuxControlImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => write!(formatter, "invalid aux-control profile: {error}"),
            Self::SourceNotDirectory(path) => {
                write!(
                    formatter,
                    "source is not a real directory: {}",
                    path.display()
                )
            }
            Self::Symlink(path) => write!(formatter, "refusing symbolic link: {}", path.display()),
            Self::SpecialFile(path) => {
                write!(formatter, "refusing special file: {}", path.display())
            }
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "size mismatch for {path}: expected {expected}, got {actual}"
                )
            }
            Self::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "SHA-256 mismatch for {path}: expected {expected}, got {actual}"
            ),
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
            Self::Format(error) => {
                write!(formatter, "cannot extract aux-control constants: {error}")
            }
            Self::Codec(error) => write!(formatter, "cannot validate aux-control output: {error}"),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Serialize(error) => {
                write!(formatter, "cannot serialize aux-control state: {error}")
            }
            Self::WrittenDataMismatch(path) => {
                write!(
                    formatter,
                    "written aux-control data changed at {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for AuxControlImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::Format(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Serialize(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AuxControlProfileError> for AuxControlImportError {
    fn from(error: AuxControlProfileError) -> Self {
        Self::Profile(error)
    }
}

pub fn extract_aux_control_constants(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedAuxControlProfile,
) -> Result<AuxControlImportSummary, AuxControlImportError> {
    extract_with(source, output, reviewed, |bytes, profile| {
        extract_format_aux_control_constants(bytes, &profile.initializer.layout())
            .map_err(AuxControlImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedAuxControlProfile,
    extractor: F,
) -> Result<AuxControlImportSummary, AuxControlImportError>
where
    F: FnOnce(&[u8], &AuxControlProfile) -> Result<AuxControlConstantWords, AuxControlImportError>,
{
    let nonce = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
    extract_with_hooks(source, output, reviewed, extractor, nonce, populate_stage)
}

fn extract_with_hooks<F, P>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedAuxControlProfile,
    extractor: F,
    stage_nonce: u64,
    populate: P,
) -> Result<AuxControlImportSummary, AuxControlImportError>
where
    F: FnOnce(&[u8], &AuxControlProfile) -> Result<AuxControlConstantWords, AuxControlImportError>,
    P: FnOnce(
        &Path,
        &AuxControlProfile,
        &ReviewedAuxControlProfile,
        &AuxControlConstantWords,
        &[u8; AUX_CONTROL_ENCODED_LEN],
    ) -> Result<AuxControlImportSummary, AuxControlImportError>,
{
    let profile = reviewed.profile();
    profile.validate()?;
    require_source_directory(source)?;
    let executable =
        read_profiled_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual_hash = sha256(&executable);
    if actual_hash != profile.executable.sha256 {
        return Err(AuxControlImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual: actual_hash,
        });
    }
    let constants = extractor(&executable, profile)?;
    let data = encode_aux_control_constants(&constants);

    let stage = validate_output(source, output, stage_nonce)?;
    fs::create_dir(&stage).map_err(|source| AuxControlImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate(&stage, profile, reviewed, &constants, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                cleanup_stage(&stage);
                return Err(AuxControlImportError::Io {
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

fn read_profiled_executable(
    source: &Path,
    relative: &str,
    expected_size: u64,
) -> Result<Vec<u8>, AuxControlImportError> {
    let mut path = source.to_owned();
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path).map_err(|source| AuxControlImportError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AuxControlImportError::Symlink(path));
        }
        if components.peek().is_some() && !metadata.is_dir() {
            return Err(AuxControlImportError::SpecialFile(path));
        }
        if components.peek().is_none() && !metadata.is_file() {
            return Err(AuxControlImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| AuxControlImportError::Io {
        path: path.clone(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| AuxControlImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(AuxControlImportError::SpecialFile(path));
    }
    if metadata.len() != expected_size {
        return Err(AuxControlImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: metadata.len(),
        });
    }
    let limit = expected_size
        .checked_add(1)
        .expect("validated executable size fits u64");
    let mut bytes = Vec::with_capacity(
        usize::try_from(expected_size).expect("validated executable size fits usize"),
    );
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| AuxControlImportError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() as u64 != expected_size {
        return Err(AuxControlImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

fn require_source_directory(path: &Path) -> Result<(), AuxControlImportError> {
    require_real_directory(path)
        .map_err(|_| AuxControlImportError::SourceNotDirectory(path.to_owned()))
}

fn validate_output(
    source: &Path,
    output: &Path,
    stage_nonce: u64,
) -> Result<PathBuf, AuxControlImportError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_real_directory(parent)
        .map_err(|_| AuxControlImportError::OutputParentNotDirectory(parent.to_owned()))?;
    let source_root =
        fs::canonicalize(source).map_err(|source_error| AuxControlImportError::Io {
            path: source.to_owned(),
            source: source_error,
        })?;
    let output_parent = fs::canonicalize(parent).map_err(|source| AuxControlImportError::Io {
        path: parent.to_owned(),
        source,
    })?;
    if output_parent.starts_with(&source_root) {
        return Err(AuxControlImportError::InvalidOutput(output.to_owned()));
    }
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(|| AuxControlImportError::InvalidOutput(output.to_owned()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(AuxControlImportError::OutputExists(output.to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AuxControlImportError::Io {
                path: output.to_owned(),
                source,
            });
        }
    }
    let stage = parent.join(format!(
        ".{name}.cmr-aux-stage-{}-{stage_nonce}",
        std::process::id()
    ));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(AuxControlImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(AuxControlImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}

fn populate_stage(
    stage: &Path,
    profile: &AuxControlProfile,
    reviewed: &ReviewedAuxControlProfile,
    constants: &AuxControlConstantWords,
    data: &[u8; AUX_CONTROL_ENCODED_LEN],
) -> Result<AuxControlImportSummary, AuxControlImportError> {
    let data_path = stage.join(AUX_CONTROL_DATA_FILE);
    write_new(&data_path, data)?;
    let summary = AuxControlImportSummary {
        schema_version: AUX_CONTROL_IMPORT_SCHEMA_VERSION,
        profile: profile.id.clone(),
        profile_sha256: reviewed.sha256().to_owned(),
        source: AuxControlImportFile {
            path: profile.executable.path.clone(),
            size: profile.executable.size,
            sha256: profile.executable.sha256.clone(),
        },
        output: AuxControlImportFile {
            path: AUX_CONTROL_DATA_FILE.to_owned(),
            size: AUX_CONTROL_ENCODED_LEN as u64,
            sha256: sha256(data),
        },
        shape: AuxControlImportShape {
            format_version: AUX_CONTROL_FORMAT_VERSION,
            common_word_count: AUX_CONTROL_COMMON_WORD_COUNT as u32,
            multiplier_count: AUX_CONTROL_MULTIPLIER_COUNT as u32,
            policy_count: AUX_CONTROL_POLICY_COUNT as u32,
            policy_word_count: AUX_CONTROL_POLICY_WORD_COUNT as u32,
            policy_divisor_count: AUX_CONTROL_POLICY_DIVISOR_COUNT as u32,
            encoding: AuxControlEncoding::CmrAuxcV1,
        },
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(AuxControlImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(AUX_CONTROL_STATE_FILE);
    write_new(&state_path, &state)?;

    let reopened = crate::aux_control_store::AuxControlConstantsStore::open(stage, reviewed)
        .map_err(|_| AuxControlImportError::WrittenDataMismatch(stage.to_owned()))?;
    if reopened.constants() != constants {
        return Err(AuxControlImportError::WrittenDataMismatch(data_path));
    }
    Ok(summary)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AuxControlImportError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| AuxControlImportError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(bytes)
        .map_err(|source| AuxControlImportError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.sync_all().map_err(|source| AuxControlImportError::Io {
        path: path.to_owned(),
        source,
    })
}

fn cleanup_stage(stage: &Path) {
    let _ = fs::remove_dir_all(stage);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::aux_control_profile::tests::profile;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cmr-aux-import-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    pub(crate) fn constants() -> AuxControlConstantWords {
        let mut next = -19_i32;
        let mut word = || {
            next = next.wrapping_mul(17).wrapping_add(31);
            next
        };
        AuxControlConstantWords {
            common_words: std::array::from_fn(|_| word()),
            q16_multipliers: std::array::from_fn(|_| word()),
            policy_one: policy(&mut word),
            policy_two: policy(&mut word),
            default_policy: policy(&mut word),
        }
    }

    fn policy(
        word: &mut impl FnMut() -> i32,
    ) -> cmr_formats::pe_aux_control_initializer::AuxControlPolicyWords {
        cmr_formats::pe_aux_control_initializer::AuxControlPolicyWords {
            words: std::array::from_fn(|_| word()),
            reciprocal_divisor: word(),
            scaled_reciprocal_divisor: word(),
        }
    }

    fn reviewed(bytes: &[u8]) -> ReviewedAuxControlProfile {
        ReviewedAuxControlProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, sha256(bytes))).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn synthetic_extraction_is_atomic_path_free_and_reopens_owned_data() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"synthetic identified executable";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let output = temp.0.join("runtime-data");
        let expected = constants();
        let reviewed = reviewed(bytes);
        let summary = extract_with(&source, &output, &reviewed, |_, _| Ok(expected)).unwrap();
        assert_eq!(summary.output.size, AUX_CONTROL_ENCODED_LEN as u64);
        assert_eq!(summary.output.path, AUX_CONTROL_DATA_FILE);
        let state = fs::read_to_string(output.join(AUX_CONTROL_STATE_FILE)).unwrap();
        assert!(!state.contains(temp.0.to_str().unwrap()));
        assert_eq!(
            serde_json::from_str::<AuxControlImportSummary>(&state).unwrap(),
            summary
        );
        let store =
            crate::aux_control_store::AuxControlConstantsStore::open(&output, &reviewed).unwrap();
        assert_eq!(store.constants(), &expected);
    }

    #[test]
    fn hash_and_size_are_rejected_before_the_parser() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"not a pe";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let mut candidate = profile(bytes.len() as u64, "11".repeat(32));
        candidate.id = "wrong-hash".into();
        let reviewed =
            ReviewedAuxControlProfile::from_json(&serde_json::to_vec(&candidate).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&source, &temp.0.join("out"), &reviewed, |_, _| panic!(
                "parser must not run"
            )),
            Err(AuxControlImportError::HashMismatch { .. })
        ));

        let mut candidate = profile(bytes.len() as u64 + 1, sha256(bytes));
        candidate.id = "wrong-size".into();
        let reviewed =
            ReviewedAuxControlProfile::from_json(&serde_json::to_vec(&candidate).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&source, &temp.0.join("size-out"), &reviewed, |_, _| panic!(
                "parser must not run"
            )),
            Err(AuxControlImportError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn rejects_existing_inside_missing_parent_and_special_sources() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"synthetic";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let reviewed = reviewed(bytes);
        assert!(matches!(
            extract_with(&source, &source.join("out"), &reviewed, |_, _| Ok(
                constants()
            )),
            Err(AuxControlImportError::InvalidOutput(_))
        ));
        let output = temp.0.join("out");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&source, &output, &reviewed, |_, _| Ok(constants())),
            Err(AuxControlImportError::OutputExists(_))
        ));
        assert!(matches!(
            extract_with(&source, &temp.0.join("missing/out"), &reviewed, |_, _| Ok(
                constants()
            )),
            Err(AuxControlImportError::OutputParentNotDirectory(_))
        ));

        let special_source = temp.0.join("special-source");
        fs::create_dir(&special_source).unwrap();
        fs::create_dir(special_source.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(
                &special_source,
                &temp.0.join("special-out"),
                &reviewed,
                |_, _| Ok(constants())
            ),
            Err(AuxControlImportError::SpecialFile(_))
        ));
    }

    #[test]
    fn staged_failure_collision_and_success_have_deterministic_publication_boundaries() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"synthetic";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let reviewed = reviewed(bytes);
        let output = temp.0.join("out");
        let nonce = 41_u64;
        let stage = temp
            .0
            .join(format!(".out.cmr-aux-stage-{}-{nonce}", std::process::id()));
        let unrelated = temp.0.join("unrelated-stage");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("keep"), b"keep").unwrap();

        let error = extract_with_hooks(
            &source,
            &output,
            &reviewed,
            |_, _| Ok(constants()),
            nonce,
            |stage, _, _, _, _| {
                assert!(stage.is_dir());
                fs::write(stage.join("partial"), b"partial").unwrap();
                Err(AuxControlImportError::WrittenDataMismatch(
                    stage.join("partial"),
                ))
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            AuxControlImportError::WrittenDataMismatch(_)
        ));
        assert!(!stage.exists());
        assert!(!output.exists());
        assert_eq!(fs::read(unrelated.join("keep")).unwrap(), b"keep");

        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("stale"), b"stale").unwrap();
        assert!(matches!(
            extract_with_hooks(
                &source,
                &output,
                &reviewed,
                |_, _| Ok(constants()),
                nonce,
                populate_stage,
            ),
            Err(AuxControlImportError::StageExists(actual)) if actual == stage
        ));
        assert_eq!(fs::read(stage.join("stale")).unwrap(), b"stale");
        fs::remove_dir_all(&stage).unwrap();

        extract_with_hooks(
            &source,
            &output,
            &reviewed,
            |_, _| Ok(constants()),
            nonce,
            populate_stage,
        )
        .unwrap();
        assert!(output.is_dir());
        assert!(!stage.exists());
        assert_eq!(fs::read(unrelated.join("keep")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_linked_and_special_source_components_and_output_parent() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;
        let temp = Temp::new();
        let source = temp.0.join("source");
        let actual = temp.0.join("actual");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&actual).unwrap();
        let bytes = b"synthetic";
        fs::write(actual.join("Rally.exe"), bytes).unwrap();
        symlink(&actual, source.join("bin")).unwrap();
        let mut candidate = profile(bytes.len() as u64, sha256(bytes));
        candidate.executable.path = "bin/Rally.exe".into();
        let linked_reviewed =
            ReviewedAuxControlProfile::from_json(&serde_json::to_vec(&candidate).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&source, &temp.0.join("out"), &linked_reviewed, |_, _| Ok(
                constants()
            )),
            Err(AuxControlImportError::Symlink(_))
        ));

        let linked_parent = temp.0.join("linked-parent");
        symlink(&actual, &linked_parent).unwrap();
        let direct = reviewed(bytes);
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        assert!(matches!(
            extract_with(&source, &linked_parent.join("out"), &direct, |_, _| Ok(
                constants()
            )),
            Err(AuxControlImportError::OutputParentNotDirectory(_))
        ));

        let direct_link_source = temp.0.join("direct-link-source");
        fs::create_dir(&direct_link_source).unwrap();
        symlink(
            actual.join("Rally.exe"),
            direct_link_source.join("Rally.exe"),
        )
        .unwrap();
        assert!(matches!(
            extract_with(
                &direct_link_source,
                &temp.0.join("direct-link-out"),
                &direct,
                |_, _| Ok(constants())
            ),
            Err(AuxControlImportError::Symlink(_))
        ));

        let linked_source_root = temp.0.join("linked-source-root");
        symlink(&actual, &linked_source_root).unwrap();
        assert!(matches!(
            extract_with(
                &linked_source_root,
                &temp.0.join("linked-root-out"),
                &direct,
                |_, _| Ok(constants())
            ),
            Err(AuxControlImportError::SourceNotDirectory(_))
        ));

        let socket_source = temp.0.join("socket-source");
        fs::create_dir(&socket_source).unwrap();
        let _listener = UnixListener::bind(socket_source.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(
                &socket_source,
                &temp.0.join("socket-out"),
                &direct,
                |_, _| Ok(constants())
            ),
            Err(AuxControlImportError::SpecialFile(_))
        ));

        let socket_parent = temp.0.join("socket-parent");
        let _parent_listener = UnixListener::bind(&socket_parent).unwrap();
        assert!(matches!(
            extract_with(&source, &socket_parent.join("out"), &direct, |_, _| Ok(
                constants()
            )),
            Err(AuxControlImportError::OutputParentNotDirectory(_))
        ));
    }
}
