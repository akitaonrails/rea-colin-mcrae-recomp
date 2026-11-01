use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cmr_formats::pe_compiled_initializer::{
    COMPILED_CASE_COUNT, COMPILED_CASE_ENCODED_LEN, COMPILED_CASE_FORMAT_VERSION,
    COMPILED_CONTROL_WORD_COUNT, COMPILED_STATE_WORD_COUNT, CompiledCaseCodecError,
    CompiledCaseWords, CompiledInitializerError, decode_compiled_cases, encode_compiled_cases,
    extract_compiled_initializer_cases,
};
use serde::{Deserialize, Serialize};

use crate::compiled_initializer_profile::{
    CompiledInitializerEncoding, CompiledInitializerProfile, CompiledInitializerProfileError,
    ReviewedCompiledInitializerProfile,
};
use crate::verify::{require_real_directory, sha256};

pub const COMPILED_INITIALIZER_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const COMPILED_INITIALIZER_DATA_FILE: &str = "compiled-car-cases-v1.bin";
pub const COMPILED_INITIALIZER_STATE_FILE: &str = "compiled-car-import-state.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledInitializerImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledInitializerImportShape {
    pub format_version: u32,
    pub selectors: u32,
    pub state_words_per_selector: u32,
    pub control_words_per_selector: u32,
    pub encoding: CompiledInitializerEncoding,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledInitializerImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: CompiledInitializerImportFile,
    pub output: CompiledInitializerImportFile,
    pub shape: CompiledInitializerImportShape,
}

#[derive(Debug)]
pub enum CompiledInitializerImportError {
    Profile(CompiledInitializerProfileError),
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
    OutputExists(PathBuf),
    StageExists(PathBuf),
    Format(CompiledInitializerError),
    Codec(CompiledCaseCodecError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}

impl fmt::Display for CompiledInitializerImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => {
                write!(formatter, "invalid compiled initializer profile: {error}")
            }
            Self::SourceNotDirectory(path) => write!(
                formatter,
                "source is not a real directory: {}",
                path.display()
            ),
            Self::Symlink(path) => write!(formatter, "refusing symbolic link: {}", path.display()),
            Self::SpecialFile(path) => {
                write!(formatter, "refusing special file: {}", path.display())
            }
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "size mismatch for {path}: expected {expected}, got {actual}"
            ),
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
            Self::OutputExists(path) => {
                write!(formatter, "output already exists: {}", path.display())
            }
            Self::StageExists(path) => {
                write!(formatter, "staging path already exists: {}", path.display())
            }
            Self::Format(error) => write!(
                formatter,
                "cannot extract compiled initializer cases: {error}"
            ),
            Self::Codec(error) => write!(
                formatter,
                "cannot validate compiled initializer output: {error}"
            ),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Serialize(error) => write!(
                formatter,
                "cannot serialize compiled initializer state: {error}"
            ),
            Self::WrittenDataMismatch(path) => write!(
                formatter,
                "written compiled initializer data changed at {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CompiledInitializerImportError {
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

impl From<CompiledInitializerProfileError> for CompiledInitializerImportError {
    fn from(error: CompiledInitializerProfileError) -> Self {
        Self::Profile(error)
    }
}

pub fn extract_compiled_car_initializer(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedCompiledInitializerProfile,
) -> Result<CompiledInitializerImportSummary, CompiledInitializerImportError> {
    extract_with(source, output, reviewed, |bytes, profile| {
        extract_compiled_initializer_cases(bytes, &profile.initializer.layout())
            .map_err(CompiledInitializerImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedCompiledInitializerProfile,
    extractor: F,
) -> Result<CompiledInitializerImportSummary, CompiledInitializerImportError>
where
    F: FnOnce(
        &[u8],
        &CompiledInitializerProfile,
    )
        -> Result<[CompiledCaseWords; COMPILED_CASE_COUNT], CompiledInitializerImportError>,
{
    let profile = reviewed.profile();
    profile.validate()?;
    require_directory(source)?;
    let executable =
        read_profiled_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual_hash = sha256(&executable);
    if actual_hash != profile.executable.sha256 {
        return Err(CompiledInitializerImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual: actual_hash,
        });
    }
    let cases = extractor(&executable, profile)?;
    let data = encode_compiled_cases(&cases);

    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| CompiledInitializerImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate_stage(&stage, profile, reviewed.sha256(), &cases, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                cleanup_stage(&stage);
                return Err(CompiledInitializerImportError::Io {
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
) -> Result<Vec<u8>, CompiledInitializerImportError> {
    let mut path = source.to_owned();
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| CompiledInitializerImportError::Io {
                path: path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(CompiledInitializerImportError::Symlink(path));
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err(CompiledInitializerImportError::SpecialFile(path));
            }
        } else if !metadata.is_file() {
            return Err(CompiledInitializerImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| CompiledInitializerImportError::Io {
        path: path.clone(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| CompiledInitializerImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(CompiledInitializerImportError::SpecialFile(path));
    }
    if metadata.len() != expected_size {
        return Err(CompiledInitializerImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: metadata.len(),
        });
    }
    let limit = expected_size
        .checked_add(1)
        .expect("validated executable size fits u64");
    let mut bytes =
        Vec::with_capacity(usize::try_from(limit).expect("validated executable size fits usize"));
    file.take(limit).read_to_end(&mut bytes).map_err(|source| {
        CompiledInitializerImportError::Io {
            path: path.clone(),
            source,
        }
    })?;
    if bytes.len() as u64 != expected_size {
        return Err(CompiledInitializerImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

fn require_directory(path: &Path) -> Result<(), CompiledInitializerImportError> {
    require_real_directory(path)
        .map_err(|_| CompiledInitializerImportError::SourceNotDirectory(path.to_owned()))
}

fn validate_output(
    source: &Path,
    output: &Path,
) -> Result<PathBuf, CompiledInitializerImportError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_directory(parent)?;
    let source_root =
        fs::canonicalize(source).map_err(|error| CompiledInitializerImportError::Io {
            path: source.to_owned(),
            source: error,
        })?;
    let output_parent =
        fs::canonicalize(parent).map_err(|source| CompiledInitializerImportError::Io {
            path: parent.to_owned(),
            source,
        })?;
    if output_parent.starts_with(&source_root) {
        return Err(CompiledInitializerImportError::InvalidOutput(
            output.to_owned(),
        ));
    }
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(|| CompiledInitializerImportError::InvalidOutput(output.to_owned()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => {
            return Err(CompiledInitializerImportError::OutputExists(
                output.to_owned(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CompiledInitializerImportError::Io {
                path: output.to_owned(),
                source,
            });
        }
    }
    let stage = parent.join(format!(".{name}.cmr-compiled-stage-{}", std::process::id()));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(CompiledInitializerImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CompiledInitializerImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}

fn populate_stage(
    stage: &Path,
    profile: &CompiledInitializerProfile,
    profile_sha256: &str,
    cases: &[CompiledCaseWords; COMPILED_CASE_COUNT],
    data: &[u8; COMPILED_CASE_ENCODED_LEN],
) -> Result<CompiledInitializerImportSummary, CompiledInitializerImportError> {
    let data_path = stage.join(COMPILED_INITIALIZER_DATA_FILE);
    write_new(&data_path, data)?;
    let written = fs::read(&data_path).map_err(|source| CompiledInitializerImportError::Io {
        path: data_path.clone(),
        source,
    })?;
    if written != data.as_slice() {
        return Err(CompiledInitializerImportError::WrittenDataMismatch(
            data_path,
        ));
    }
    let decoded = decode_compiled_cases(&written).map_err(CompiledInitializerImportError::Codec)?;
    if &decoded != cases {
        return Err(CompiledInitializerImportError::WrittenDataMismatch(
            data_path,
        ));
    }
    let summary = CompiledInitializerImportSummary {
        schema_version: COMPILED_INITIALIZER_IMPORT_SCHEMA_VERSION,
        profile: profile.id.clone(),
        profile_sha256: profile_sha256.to_owned(),
        source: CompiledInitializerImportFile {
            path: profile.executable.path.clone(),
            size: profile.executable.size,
            sha256: profile.executable.sha256.clone(),
        },
        output: CompiledInitializerImportFile {
            path: COMPILED_INITIALIZER_DATA_FILE.to_owned(),
            size: COMPILED_CASE_ENCODED_LEN as u64,
            sha256: sha256(&written),
        },
        shape: CompiledInitializerImportShape {
            format_version: COMPILED_CASE_FORMAT_VERSION,
            selectors: COMPILED_CASE_COUNT as u32,
            state_words_per_selector: COMPILED_STATE_WORD_COUNT as u32,
            control_words_per_selector: COMPILED_CONTROL_WORD_COUNT as u32,
            encoding: CompiledInitializerEncoding::CmrCcaseV1,
        },
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(CompiledInitializerImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(COMPILED_INITIALIZER_STATE_FILE);
    write_new(&state_path, &state)?;
    let read_state =
        fs::read(&state_path).map_err(|source| CompiledInitializerImportError::Io {
            path: state_path.clone(),
            source,
        })?;
    let reparsed: CompiledInitializerImportSummary =
        serde_json::from_slice(&read_state).map_err(CompiledInitializerImportError::Serialize)?;
    if read_state != state || reparsed != summary {
        return Err(CompiledInitializerImportError::WrittenDataMismatch(
            state_path,
        ));
    }
    Ok(summary)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CompiledInitializerImportError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| CompiledInitializerImportError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(bytes)
        .map_err(|source| CompiledInitializerImportError::Io {
            path: path.to_owned(),
            source,
        })
}

fn cleanup_stage(stage: &Path) {
    let _ = fs::remove_dir_all(stage);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled_initializer_profile::tests::profile;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cmr-compiled-import-{}-{}",
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

    fn cases() -> [CompiledCaseWords; COMPILED_CASE_COUNT] {
        std::array::from_fn(|selector| CompiledCaseWords {
            state_words: std::array::from_fn(|index| selector as i32 * 100 + index as i32 - 50),
            control_words: std::array::from_fn(|index| selector as i32 * -100 - index as i32 + 25),
        })
    }

    fn reviewed(bytes: &[u8]) -> ReviewedCompiledInitializerProfile {
        ReviewedCompiledInitializerProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, sha256(bytes))).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn synthetic_extraction_is_atomic_path_free_and_revalidated() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"synthetic identified executable";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let output = temp.0.join("runtime-data");
        let expected = cases();
        let summary =
            extract_with(&source, &output, &reviewed(bytes), |_, _| Ok(expected)).unwrap();
        assert_eq!(summary.output.size, COMPILED_CASE_ENCODED_LEN as u64);
        assert_eq!(
            decode_compiled_cases(&fs::read(output.join(COMPILED_INITIALIZER_DATA_FILE)).unwrap())
                .unwrap(),
            expected
        );
        let state = fs::read_to_string(output.join(COMPILED_INITIALIZER_STATE_FILE)).unwrap();
        assert!(!state.contains(temp.0.to_str().unwrap()));
        assert_eq!(
            serde_json::from_str::<CompiledInitializerImportSummary>(&state).unwrap(),
            summary
        );

        let store = crate::compiled_initializer_store::CompiledInitializerStore::open(
            &output,
            &reviewed(bytes),
        )
        .unwrap();
        assert_eq!(store.cases(), &expected);
    }

    #[test]
    fn hash_is_rejected_before_the_parser() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"not a pe";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let mut candidate = profile(bytes.len() as u64, "11".repeat(32));
        candidate.id = "wrong-hash".into();
        let reviewed =
            ReviewedCompiledInitializerProfile::from_json(&serde_json::to_vec(&candidate).unwrap())
                .unwrap();
        assert!(matches!(
            extract_with(&source, &temp.0.join("out"), &reviewed, |_, _| panic!(
                "parser must not run"
            )),
            Err(CompiledInitializerImportError::HashMismatch { .. })
        ));
    }

    #[test]
    fn existing_inside_and_failed_extractions_publish_nothing() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"synthetic";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let reviewed = reviewed(bytes);
        assert!(matches!(
            extract_with(&source, &source.join("out"), &reviewed, |_, _| Ok(cases())),
            Err(CompiledInitializerImportError::InvalidOutput(_))
        ));
        let output = temp.0.join("out");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&source, &output, &reviewed, |_, _| Ok(cases())),
            Err(CompiledInitializerImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&source, &output, &reviewed, |_, _| Err(
                CompiledInitializerImportError::Format(CompiledInitializerError::Layout(
                    "synthetic failure"
                ))
            )),
            Err(CompiledInitializerImportError::Format(_))
        ));
        assert!(!output.exists());

        let stage = temp
            .0
            .join(format!(".out.cmr-compiled-stage-{}", std::process::id()));
        fs::create_dir(&stage).unwrap();
        assert!(matches!(
            extract_with(&source, &output, &reviewed, |_, _| Ok(cases())),
            Err(CompiledInitializerImportError::StageExists(path)) if path == stage
        ));

        let missing_parent_output = temp.0.join("missing-parent/out");
        assert!(matches!(
            extract_with(&source, &missing_parent_output, &reviewed, |_, _| Ok(
                cases()
            )),
            Err(CompiledInitializerImportError::SourceNotDirectory(_))
        ));
    }

    #[test]
    fn rejects_short_long_and_special_profiled_sources() {
        for (bytes, expected_size) in [(&b"short"[..], 6_u64), (&b"too-long"[..], 7_u64)] {
            let temp = Temp::new();
            let source = temp.0.join("source");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("Rally.exe"), bytes).unwrap();
            let reviewed = ReviewedCompiledInitializerProfile::from_json(
                &serde_json::to_vec(&profile(expected_size, sha256(bytes))).unwrap(),
            )
            .unwrap();
            assert!(matches!(
                extract_with(&source, &temp.0.join("out"), &reviewed, |_, _| Ok(cases())),
                Err(CompiledInitializerImportError::SizeMismatch { .. })
            ));
        }

        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("Rally.exe")).unwrap();
        let reviewed = ReviewedCompiledInitializerProfile::from_json(
            &serde_json::to_vec(&profile(1, "00".repeat(32))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&source, &temp.0.join("out"), &reviewed, |_, _| Ok(cases())),
            Err(CompiledInitializerImportError::SpecialFile(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_linked_source_components() {
        use std::os::unix::fs::symlink;
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
        let linked_component_profile =
            ReviewedCompiledInitializerProfile::from_json(&serde_json::to_vec(&candidate).unwrap())
                .unwrap();
        assert!(matches!(
            extract_with(
                &source,
                &temp.0.join("out"),
                &linked_component_profile,
                |_, _| Ok(cases())
            ),
            Err(CompiledInitializerImportError::Symlink(_))
        ));

        let direct_source = temp.0.join("direct-source");
        fs::create_dir(&direct_source).unwrap();
        symlink(actual.join("Rally.exe"), direct_source.join("Rally.exe")).unwrap();
        let direct_reviewed = reviewed(bytes);
        assert!(matches!(
            extract_with(
                &direct_source,
                &temp.0.join("direct-out"),
                &direct_reviewed,
                |_, _| Ok(cases())
            ),
            Err(CompiledInitializerImportError::Symlink(_))
        ));
    }
}
