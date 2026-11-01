use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cmr_formats::pe_rival_records::{
    RIVAL_RECORD_COUNT, RIVAL_RECORD_STRIDE, RIVAL_RECORDS_FORMAT_VERSION, RivalRecords,
    RivalRecordsError, check_rival_record_kinds, decode_rival_records, encode_rival_records,
    extract_rival_records as extract_format,
};
use serde::{Deserialize, Serialize};

use crate::rival_records_profile::{
    ReviewedRivalRecordsProfile, RivalRecordsEncoding, RivalRecordsProfile,
    RivalRecordsProfileError,
};
use crate::verify::{require_real_directory, sha256};

pub const RIVAL_RECORDS_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const RIVAL_RECORDS_DATA_FILE: &str = "rival-records-v1.bin";
pub const RIVAL_RECORDS_STATE_FILE: &str = "rival-records-import-state.json";
/// Canonical store directory name under a runtime-data root.
pub const RIVAL_RECORDS_STORE_DIRECTORY: &str = "official-eu-rival-records";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RivalRecordsImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RivalRecordsImportShape {
    pub format_version: u32,
    pub record_count: u32,
    pub stride: u32,
    pub encoding: RivalRecordsEncoding,
}

impl RivalRecordsImportShape {
    /// The only shape the reviewed layout produces.
    pub fn expected() -> Self {
        Self {
            format_version: RIVAL_RECORDS_FORMAT_VERSION,
            record_count: RIVAL_RECORD_COUNT as u32,
            stride: RIVAL_RECORD_STRIDE as u32,
            encoding: RivalRecordsEncoding::CmrRivalRecordsV1,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RivalRecordsImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: RivalRecordsImportFile,
    pub output: RivalRecordsImportFile,
    pub shape: RivalRecordsImportShape,
}

#[derive(Debug)]
pub enum RivalRecordsImportError {
    Profile(RivalRecordsProfileError),
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
    /// The extracted payload is not the one the profile reviewed.
    PayloadMismatch {
        expected: String,
        actual: String,
    },
    InvalidOutput(PathBuf),
    OutputExists(PathBuf),
    StageExists(PathBuf),
    Format(RivalRecordsError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}
impl fmt::Display for RivalRecordsImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(e) => write!(f, "invalid rival-records profile: {e}"),
            Self::SourceNotDirectory(p) => {
                write!(f, "source is not a real directory: {}", p.display())
            }
            Self::Symlink(p) => write!(f, "refusing symbolic link: {}", p.display()),
            Self::SpecialFile(p) => write!(f, "refusing special file: {}", p.display()),
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "size mismatch for {path}: expected {expected}, got {actual}"
            ),
            Self::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "SHA-256 mismatch for {path}: expected {expected}, got {actual}"
            ),
            Self::PayloadMismatch { expected, actual } => write!(
                f,
                "extracted rival-records payload SHA-256 mismatch: expected {expected}, got {actual}"
            ),
            Self::InvalidOutput(p) => write!(f, "invalid output path: {}", p.display()),
            Self::OutputExists(p) => write!(f, "output already exists: {}", p.display()),
            Self::StageExists(p) => write!(f, "staging path already exists: {}", p.display()),
            Self::Format(e) => write!(f, "cannot extract rival records: {e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(e) => write!(f, "cannot serialize rival-records state: {e}"),
            Self::WrittenDataMismatch(p) => {
                write!(f, "written rival-records data changed at {}", p.display())
            }
        }
    }
}
impl std::error::Error for RivalRecordsImportError {}
impl From<RivalRecordsProfileError> for RivalRecordsImportError {
    fn from(e: RivalRecordsProfileError) -> Self {
        Self::Profile(e)
    }
}

pub fn extract_rival_records(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedRivalRecordsProfile,
) -> Result<RivalRecordsImportSummary, RivalRecordsImportError> {
    extract_with(source, output, reviewed, |bytes, p| {
        extract_format(bytes, &p.records.layout()).map_err(RivalRecordsImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedRivalRecordsProfile,
    extractor: F,
) -> Result<RivalRecordsImportSummary, RivalRecordsImportError>
where
    F: FnOnce(&[u8], &RivalRecordsProfile) -> Result<RivalRecords, RivalRecordsImportError>,
{
    let profile = reviewed.profile();
    profile.validate()?;
    require_dir(source)?;
    let bytes = read_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual = sha256(&bytes);
    if actual != profile.executable.sha256 {
        return Err(RivalRecordsImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual,
        });
    }
    let map = extractor(&bytes, profile)?;
    check_rival_record_kinds(&map).map_err(RivalRecordsImportError::Format)?;
    let data = encode_rival_records(&map);
    let payload = sha256(&data);
    if payload != profile.records.payload_sha256 {
        return Err(RivalRecordsImportError::PayloadMismatch {
            expected: profile.records.payload_sha256.clone(),
            actual: payload,
        });
    }
    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| RivalRecordsImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate(&stage, profile, reviewed.sha256(), &map, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                let _ = fs::remove_dir_all(&stage);
                return Err(RivalRecordsImportError::Io {
                    path: output.into(),
                    source,
                });
            }
            Ok(summary)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&stage);
            Err(e)
        }
    }
}
fn require_dir(path: &Path) -> Result<(), RivalRecordsImportError> {
    require_real_directory(path)
        .map_err(|_| RivalRecordsImportError::SourceNotDirectory(path.into()))
}
fn read_executable(
    root: &Path,
    relative: &str,
    expected: u64,
) -> Result<Vec<u8>, RivalRecordsImportError> {
    let mut path = root.to_owned();
    let mut cs = relative.split('/').peekable();
    while let Some(c) = cs.next() {
        path.push(c);
        let m = fs::symlink_metadata(&path).map_err(|source| RivalRecordsImportError::Io {
            path: path.clone(),
            source,
        })?;
        if m.file_type().is_symlink() {
            return Err(RivalRecordsImportError::Symlink(path));
        }
        if cs.peek().is_some() {
            if !m.is_dir() {
                return Err(RivalRecordsImportError::SpecialFile(path));
            }
        } else if !m.is_file() {
            return Err(RivalRecordsImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| RivalRecordsImportError::Io {
        path: path.clone(),
        source,
    })?;
    let m = file
        .metadata()
        .map_err(|source| RivalRecordsImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !m.is_file() {
        return Err(RivalRecordsImportError::SpecialFile(path));
    }
    if m.len() != expected {
        return Err(RivalRecordsImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: m.len(),
        });
    }
    let mut b = Vec::new();
    file.take(expected + 1)
        .read_to_end(&mut b)
        .map_err(|source| RivalRecordsImportError::Io {
            path: path.clone(),
            source,
        })?;
    if b.len() as u64 != expected {
        return Err(RivalRecordsImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: b.len() as u64,
        });
    }
    Ok(b)
}
fn validate_output(source: &Path, output: &Path) -> Result<PathBuf, RivalRecordsImportError> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_dir(parent)?;
    let source_root = source.to_owned();
    let source = fs::canonicalize(source).map_err(|error| RivalRecordsImportError::Io {
        path: source_root,
        source: error,
    })?;
    let parent_real = fs::canonicalize(parent).map_err(|source| RivalRecordsImportError::Io {
        path: parent.into(),
        source,
    })?;
    if parent_real.starts_with(source) {
        return Err(RivalRecordsImportError::InvalidOutput(output.into()));
    }
    let name = output
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| *v != "." && *v != "..")
        .ok_or_else(|| RivalRecordsImportError::InvalidOutput(output.into()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(RivalRecordsImportError::OutputExists(output.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(RivalRecordsImportError::Io {
                path: output.into(),
                source,
            });
        }
    }
    let stage = parent.join(format!(
        ".{name}.cmr-rival-records-stage-{}",
        std::process::id()
    ));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(RivalRecordsImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(RivalRecordsImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}
fn populate(
    stage: &Path,
    p: &RivalRecordsProfile,
    p_hash: &str,
    map: &RivalRecords,
    data: &[u8],
) -> Result<RivalRecordsImportSummary, RivalRecordsImportError> {
    let dp = stage.join(RIVAL_RECORDS_DATA_FILE);
    write_new(&dp, data)?;
    let written = fs::read(&dp).map_err(|source| RivalRecordsImportError::Io {
        path: dp.clone(),
        source,
    })?;
    if decode_rival_records(&written).map_err(RivalRecordsImportError::Format)? != *map {
        return Err(RivalRecordsImportError::WrittenDataMismatch(dp));
    }
    let summary = RivalRecordsImportSummary {
        schema_version: RIVAL_RECORDS_IMPORT_SCHEMA_VERSION,
        profile: p.id.clone(),
        profile_sha256: p_hash.into(),
        source: RivalRecordsImportFile {
            path: p.executable.path.clone(),
            size: p.executable.size,
            sha256: p.executable.sha256.clone(),
        },
        output: RivalRecordsImportFile {
            path: RIVAL_RECORDS_DATA_FILE.into(),
            size: written.len() as u64,
            sha256: sha256(&written),
        },
        shape: RivalRecordsImportShape::expected(),
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(RivalRecordsImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(RIVAL_RECORDS_STATE_FILE);
    write_new(&state_path, &state)?;
    let written_state = fs::read(&state_path).map_err(|source| RivalRecordsImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    let reparsed: RivalRecordsImportSummary =
        serde_json::from_slice(&written_state).map_err(RivalRecordsImportError::Serialize)?;
    if written_state != state || reparsed != summary {
        return Err(RivalRecordsImportError::WrittenDataMismatch(state_path));
    }
    Ok(summary)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), RivalRecordsImportError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| RivalRecordsImportError::Io {
            path: path.into(),
            source,
        })?;
    f.write_all(bytes)
        .map_err(|source| RivalRecordsImportError::Io {
            path: path.into(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rival_records_profile::tests::{profile, synthetic_records};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "cmr-rival-records-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn map() -> RivalRecords {
        synthetic_records()
    }
    fn reviewed(bytes: &[u8]) -> ReviewedRivalRecordsProfile {
        ReviewedRivalRecordsProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, sha256(bytes))).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn extraction_is_atomic_path_free_and_openable() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let b = b"synthetic";
        fs::write(src.join("Rally.exe"), b).unwrap();
        let r = reviewed(b);
        let out = t.0.join("out");
        let summary = extract_with(&src, &out, &r, |_, _| Ok(map())).unwrap();
        assert_eq!(summary.output.sha256, r.profile().records.payload_sha256);
        let state = fs::read_to_string(out.join(RIVAL_RECORDS_STATE_FILE)).unwrap();
        assert!(!state.contains(t.0.to_str().unwrap()));
        let store = crate::rival_records_store::RivalRecordsStore::open(&out, &r).unwrap();
        assert_eq!(store.records(), &map());
        let data_path = out.join(RIVAL_RECORDS_DATA_FILE);
        let mut data = fs::read(&data_path).unwrap();
        data[20] ^= 1;
        fs::write(&data_path, data).unwrap();
        assert!(matches!(
            crate::rival_records_store::RivalRecordsStore::open(&out, &r),
            Err(
                crate::rival_records_store::RivalRecordsStoreError::InvalidState {
                    field: "output.sha256"
                }
            )
        ));
    }

    #[test]
    fn rejects_an_unreviewed_payload_before_writing() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let b = b"synthetic";
        fs::write(src.join("Rally.exe"), b).unwrap();
        let output = t.0.join("out");
        let mut other = map();
        other.records[15].raw[19] ^= 1;
        assert!(matches!(
            extract_with(&src, &output, &reviewed(b), |_, _| Ok(other)),
            Err(RivalRecordsImportError::PayloadMismatch { .. })
        ));
        assert!(!output.exists());
        assert!(
            !t.0.join(format!(
                ".out.cmr-rival-records-stage-{}",
                std::process::id()
            ))
            .exists()
        );
    }

    #[test]
    fn rejects_identity_before_parser_and_cleans_failed_extraction() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();

        let wrong_size = ReviewedRivalRecordsProfile::from_json(
            &serde_json::to_vec(&profile((bytes.len() + 1) as u64, sha256(bytes))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("size-out"), &wrong_size, |_, _| panic!(
                "parser ran"
            )),
            Err(RivalRecordsImportError::SizeMismatch { .. })
        ));

        let wrong_hash = ReviewedRivalRecordsProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, "11".repeat(32))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("hash-out"), &wrong_hash, |_, _| panic!(
                "parser ran"
            )),
            Err(RivalRecordsImportError::HashMismatch { .. })
        ));

        let output = t.0.join("failed-out");
        assert!(matches!(
            extract_with(&src, &output, &reviewed(bytes), |_, _| Err(
                RivalRecordsImportError::Format(RivalRecordsError::UnsupportedLayout)
            )),
            Err(RivalRecordsImportError::Format(_))
        ));
        assert!(!output.exists());
        assert!(
            !t.0.join(format!(
                ".failed-out.cmr-rival-records-stage-{}",
                std::process::id()
            ))
            .exists()
        );
    }

    #[test]
    fn rejects_source_and_output_path_boundaries() {
        let t = Temp::new();
        let not_dir = t.0.join("file");
        fs::write(&not_dir, b"x").unwrap();
        assert!(matches!(
            extract_with(
                &not_dir,
                &t.0.join("out"),
                &reviewed(b"x"),
                |_, _| Ok(map())
            ),
            Err(RivalRecordsImportError::SourceNotDirectory(_))
        ));
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let r = reviewed(bytes);
        assert!(matches!(
            extract_with(&src, &src.join("inside"), &r, |_, _| Ok(map())),
            Err(RivalRecordsImportError::InvalidOutput(_))
        ));
        let output = t.0.join("existing");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(RivalRecordsImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        let stage = t.0.join(format!(
            ".existing.cmr-rival-records-stage-{}",
            std::process::id()
        ));
        fs::create_dir(&stage).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(RivalRecordsImportError::StageExists(p)) if p == stage
        ));
        fs::remove_dir(&stage).unwrap();
        fs::remove_file(src.join("Rally.exe")).unwrap();
        fs::create_dir(src.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("special-out"), &r, |_, _| Ok(map())),
            Err(RivalRecordsImportError::SpecialFile(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_source_leaf_and_intermediate_component() {
        use std::os::unix::fs::symlink;
        let t = Temp::new();
        let bytes = b"synthetic";
        let real = t.0.join("real.exe");
        fs::write(&real, bytes).unwrap();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        symlink(&real, src.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("leaf-out"), &reviewed(bytes), |_, _| Ok(
                map()
            )),
            Err(RivalRecordsImportError::Symlink(_))
        ));
        fs::remove_file(src.join("Rally.exe")).unwrap();
        let real_dir = t.0.join("real-dir");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("Rally.exe"), bytes).unwrap();
        symlink(&real_dir, src.join("linked")).unwrap();
        let mut p = profile(bytes.len() as u64, sha256(bytes));
        p.executable.path = "linked/Rally.exe".into();
        let r = ReviewedRivalRecordsProfile::from_json(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("intermediate-out"), &r, |_, _| Ok(map())),
            Err(RivalRecordsImportError::Symlink(_))
        ));
        fs::remove_file(src.join("linked")).unwrap();
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let output_parent = t.0.join("output-real");
        fs::create_dir(&output_parent).unwrap();
        let output_link = t.0.join("output-link");
        symlink(&output_parent, &output_link).unwrap();
        assert!(matches!(
            extract_with(&src, &output_link.join("out"), &reviewed(bytes), |_, _| Ok(
                map()
            )),
            Err(RivalRecordsImportError::SourceNotDirectory(_))
        ));
    }
}
