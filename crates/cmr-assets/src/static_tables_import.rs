use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cmr_formats::pe_static_tables::{
    STATIC_TABLES_FORMAT_VERSION, StaticTables, StaticTablesError, StaticTablesLayout,
    decode_static_tables, encode_static_tables, extract_static_tables as extract_format,
};
use serde::{Deserialize, Serialize};

use crate::static_tables_profile::{
    ReviewedStaticTablesProfile, StaticTablesEncoding, StaticTablesProfile,
    StaticTablesProfileError,
};
use crate::verify::{require_real_directory, sha256};

pub const STATIC_TABLES_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const STATIC_TABLES_DATA_FILE: &str = "static-tables-v1.bin";
pub const STATIC_TABLES_STATE_FILE: &str = "static-tables-import-state.json";
/// Canonical store directory name under a runtime-data root.
pub const STATIC_TABLES_STORE_DIRECTORY: &str = "official-eu-static-tables";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StaticTablesImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StaticTablesImportShape {
    pub format_version: u32,
    pub entry_count: u32,
    pub encoding: StaticTablesEncoding,
}

impl StaticTablesImportShape {
    /// The shape a reviewed layout produces.
    pub fn expected(layout: &StaticTablesLayout) -> Self {
        Self {
            format_version: STATIC_TABLES_FORMAT_VERSION,
            entry_count: layout.entries.len() as u32,
            encoding: StaticTablesEncoding::CmrStaticTablesV1,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StaticTablesImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: StaticTablesImportFile,
    pub output: StaticTablesImportFile,
    pub shape: StaticTablesImportShape,
}

#[derive(Debug)]
pub enum StaticTablesImportError {
    Profile(StaticTablesProfileError),
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
    /// The extracted tables do not have the profile's names and shapes.
    LayoutMismatch,
    InvalidOutput(PathBuf),
    OutputExists(PathBuf),
    StageExists(PathBuf),
    Format(StaticTablesError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}
impl fmt::Display for StaticTablesImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(e) => write!(f, "invalid static-tables profile: {e}"),
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
                "extracted static-tables payload SHA-256 mismatch: expected {expected}, got {actual}"
            ),
            Self::LayoutMismatch => {
                write!(f, "extracted static tables do not match the profile layout")
            }
            Self::InvalidOutput(p) => write!(f, "invalid output path: {}", p.display()),
            Self::OutputExists(p) => write!(f, "output already exists: {}", p.display()),
            Self::StageExists(p) => write!(f, "staging path already exists: {}", p.display()),
            Self::Format(e) => write!(f, "cannot extract static tables: {e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(e) => write!(f, "cannot serialize static-tables state: {e}"),
            Self::WrittenDataMismatch(p) => {
                write!(f, "written static-tables data changed at {}", p.display())
            }
        }
    }
}
impl std::error::Error for StaticTablesImportError {}
impl From<StaticTablesProfileError> for StaticTablesImportError {
    fn from(e: StaticTablesProfileError) -> Self {
        Self::Profile(e)
    }
}

/// Authenticate `Rally.exe` under `source` against the reviewed profile,
/// extract its named tables, require the payload digest the profile reviewed
/// and atomically write a store at `output`.
pub fn extract_static_tables(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedStaticTablesProfile,
) -> Result<StaticTablesImportSummary, StaticTablesImportError> {
    extract_with(source, output, reviewed, |bytes, layout| {
        extract_format(bytes, layout).map_err(StaticTablesImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedStaticTablesProfile,
    extractor: F,
) -> Result<StaticTablesImportSummary, StaticTablesImportError>
where
    F: FnOnce(&[u8], &StaticTablesLayout) -> Result<StaticTables, StaticTablesImportError>,
{
    let profile = reviewed.profile();
    let layout = profile.validate()?;
    require_dir(source)?;
    let bytes = read_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual = sha256(&bytes);
    if actual != profile.executable.sha256 {
        return Err(StaticTablesImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual,
        });
    }
    let tables = extractor(&bytes, &layout)?;
    if !tables.matches_layout(&layout) {
        return Err(StaticTablesImportError::LayoutMismatch);
    }
    let data = encode_static_tables(&tables);
    let payload = sha256(&data);
    if payload != profile.tables.payload_sha256 {
        return Err(StaticTablesImportError::PayloadMismatch {
            expected: profile.tables.payload_sha256.clone(),
            actual: payload,
        });
    }
    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| StaticTablesImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate(&stage, profile, &layout, reviewed.sha256(), &tables, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                let _ = fs::remove_dir_all(&stage);
                return Err(StaticTablesImportError::Io {
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
fn require_dir(path: &Path) -> Result<(), StaticTablesImportError> {
    require_real_directory(path)
        .map_err(|_| StaticTablesImportError::SourceNotDirectory(path.into()))
}
fn read_executable(
    root: &Path,
    relative: &str,
    expected: u64,
) -> Result<Vec<u8>, StaticTablesImportError> {
    let mut path = root.to_owned();
    let mut cs = relative.split('/').peekable();
    while let Some(c) = cs.next() {
        path.push(c);
        let m = fs::symlink_metadata(&path).map_err(|source| StaticTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
        if m.file_type().is_symlink() {
            return Err(StaticTablesImportError::Symlink(path));
        }
        if cs.peek().is_some() {
            if !m.is_dir() {
                return Err(StaticTablesImportError::SpecialFile(path));
            }
        } else if !m.is_file() {
            return Err(StaticTablesImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| StaticTablesImportError::Io {
        path: path.clone(),
        source,
    })?;
    let m = file
        .metadata()
        .map_err(|source| StaticTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !m.is_file() {
        return Err(StaticTablesImportError::SpecialFile(path));
    }
    if m.len() != expected {
        return Err(StaticTablesImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: m.len(),
        });
    }
    let mut b = Vec::new();
    file.take(expected + 1)
        .read_to_end(&mut b)
        .map_err(|source| StaticTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
    if b.len() as u64 != expected {
        return Err(StaticTablesImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: b.len() as u64,
        });
    }
    Ok(b)
}
fn validate_output(source: &Path, output: &Path) -> Result<PathBuf, StaticTablesImportError> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_dir(parent)?;
    let source_root = source.to_owned();
    let source = fs::canonicalize(source).map_err(|error| StaticTablesImportError::Io {
        path: source_root,
        source: error,
    })?;
    let parent_real = fs::canonicalize(parent).map_err(|source| StaticTablesImportError::Io {
        path: parent.into(),
        source,
    })?;
    if parent_real.starts_with(source) {
        return Err(StaticTablesImportError::InvalidOutput(output.into()));
    }
    let name = output
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| *v != "." && *v != "..")
        .ok_or_else(|| StaticTablesImportError::InvalidOutput(output.into()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(StaticTablesImportError::OutputExists(output.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StaticTablesImportError::Io {
                path: output.into(),
                source,
            });
        }
    }
    let stage = parent.join(format!(
        ".{name}.cmr-static-tables-stage-{}",
        std::process::id()
    ));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(StaticTablesImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StaticTablesImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}
fn populate(
    stage: &Path,
    p: &StaticTablesProfile,
    layout: &StaticTablesLayout,
    p_hash: &str,
    tables: &StaticTables,
    data: &[u8],
) -> Result<StaticTablesImportSummary, StaticTablesImportError> {
    let dp = stage.join(STATIC_TABLES_DATA_FILE);
    write_new(&dp, data)?;
    let written = fs::read(&dp).map_err(|source| StaticTablesImportError::Io {
        path: dp.clone(),
        source,
    })?;
    if decode_static_tables(&written).map_err(StaticTablesImportError::Format)? != *tables {
        return Err(StaticTablesImportError::WrittenDataMismatch(dp));
    }
    let summary = StaticTablesImportSummary {
        schema_version: STATIC_TABLES_IMPORT_SCHEMA_VERSION,
        profile: p.id.clone(),
        profile_sha256: p_hash.into(),
        source: StaticTablesImportFile {
            path: p.executable.path.clone(),
            size: p.executable.size,
            sha256: p.executable.sha256.clone(),
        },
        output: StaticTablesImportFile {
            path: STATIC_TABLES_DATA_FILE.into(),
            size: written.len() as u64,
            sha256: sha256(&written),
        },
        shape: StaticTablesImportShape::expected(layout),
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(StaticTablesImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(STATIC_TABLES_STATE_FILE);
    write_new(&state_path, &state)?;
    let written_state = fs::read(&state_path).map_err(|source| StaticTablesImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    let reparsed: StaticTablesImportSummary =
        serde_json::from_slice(&written_state).map_err(StaticTablesImportError::Serialize)?;
    if written_state != state || reparsed != summary {
        return Err(StaticTablesImportError::WrittenDataMismatch(state_path));
    }
    Ok(summary)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), StaticTablesImportError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| StaticTablesImportError::Io {
            path: path.into(),
            source,
        })?;
    f.write_all(bytes)
        .map_err(|source| StaticTablesImportError::Io {
            path: path.into(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::static_tables_profile::tests::{profile, synthetic_tables};
    use crate::static_tables_store::{StaticTablesStore, StaticTablesStoreError};
    use cmr_formats::pe_static_tables::{StaticTable, StaticTableData};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "cmr-static-tables-{}-{}",
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
    fn map() -> StaticTables {
        synthetic_tables()
    }
    fn reviewed(bytes: &[u8]) -> ReviewedStaticTablesProfile {
        ReviewedStaticTablesProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, sha256(bytes))).unwrap(),
        )
        .unwrap()
    }
    fn stage_path(t: &Temp, name: &str) -> PathBuf {
        t.0.join(format!(
            ".{name}.cmr-static-tables-stage-{}",
            std::process::id()
        ))
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
        assert_eq!(summary.output.sha256, r.profile().tables.payload_sha256);
        assert_eq!(summary.shape.entry_count, 3);
        let state = fs::read_to_string(out.join(STATIC_TABLES_STATE_FILE)).unwrap();
        assert!(!state.contains(t.0.to_str().unwrap()));
        let store = StaticTablesStore::open(&out, &r).unwrap();
        assert_eq!(store.tables(), &map());
        assert_eq!(store.tables().string("synthetic_name"), Some(&b"Gamma"[..]));
        let data_path = out.join(STATIC_TABLES_DATA_FILE);
        let mut data = fs::read(&data_path).unwrap();
        data[20] ^= 1;
        fs::write(&data_path, data).unwrap();
        assert!(matches!(
            StaticTablesStore::open(&out, &r),
            Err(StaticTablesStoreError::InvalidState {
                field: "output.sha256"
            })
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
        let mut entries = map().entries().to_vec();
        entries[0].data = StaticTableData::Block(vec![1, 2, 3, 5]);
        let other = StaticTables::new(entries).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &reviewed(b), |_, _| Ok(other)),
            Err(StaticTablesImportError::PayloadMismatch { .. })
        ));
        assert!(!output.exists());
        assert!(!stage_path(&t, "out").exists());
    }

    #[test]
    fn rejects_tables_that_do_not_match_the_profile_layout() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let b = b"synthetic";
        fs::write(src.join("Rally.exe"), b).unwrap();
        let output = t.0.join("out");
        let other = StaticTables::new(vec![StaticTable {
            name: "synthetic_block".into(),
            va: 0x0041_0010,
            data: StaticTableData::Block(vec![1, 2, 3, 4]),
        }])
        .unwrap();
        assert!(matches!(
            extract_with(&src, &output, &reviewed(b), |_, _| Ok(other)),
            Err(StaticTablesImportError::LayoutMismatch)
        ));
        assert!(!output.exists());
    }

    #[test]
    fn rejects_identity_before_parser_and_cleans_failed_extraction() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();

        let wrong_size = ReviewedStaticTablesProfile::from_json(
            &serde_json::to_vec(&profile((bytes.len() + 1) as u64, sha256(bytes))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("size-out"), &wrong_size, |_, _| panic!(
                "parser ran"
            )),
            Err(StaticTablesImportError::SizeMismatch { .. })
        ));

        let wrong_hash = ReviewedStaticTablesProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, "11".repeat(32))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("hash-out"), &wrong_hash, |_, _| panic!(
                "parser ran"
            )),
            Err(StaticTablesImportError::HashMismatch { .. })
        ));

        let output = t.0.join("failed-out");
        assert!(matches!(
            extract_with(&src, &output, &reviewed(bytes), |_, _| Err(
                StaticTablesImportError::Format(StaticTablesError::EntryCount(0))
            )),
            Err(StaticTablesImportError::Format(_))
        ));
        assert!(!output.exists());
        assert!(!stage_path(&t, "failed-out").exists());
    }

    #[test]
    fn real_extractor_rejects_a_non_pe_without_output() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic non-PE";
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let output = t.0.join("out");
        assert!(matches!(
            extract_static_tables(&src, &output, &reviewed(bytes)),
            Err(StaticTablesImportError::Format(StaticTablesError::Pe(_)))
        ));
        assert!(!output.exists());
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
            Err(StaticTablesImportError::SourceNotDirectory(_))
        ));
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let r = reviewed(bytes);
        assert!(matches!(
            extract_with(&src, &src.join("inside"), &r, |_, _| Ok(map())),
            Err(StaticTablesImportError::InvalidOutput(_))
        ));
        let output = t.0.join("existing");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(StaticTablesImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        let stage = stage_path(&t, "existing");
        fs::create_dir(&stage).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(StaticTablesImportError::StageExists(p)) if p == stage
        ));
        fs::remove_dir(&stage).unwrap();
        fs::remove_file(src.join("Rally.exe")).unwrap();
        fs::create_dir(src.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("special-out"), &r, |_, _| Ok(map())),
            Err(StaticTablesImportError::SpecialFile(_))
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
            Err(StaticTablesImportError::Symlink(_))
        ));
        fs::remove_file(src.join("Rally.exe")).unwrap();
        let real_dir = t.0.join("real-dir");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("Rally.exe"), bytes).unwrap();
        symlink(&real_dir, src.join("linked")).unwrap();
        let mut p = profile(bytes.len() as u64, sha256(bytes));
        p.executable.path = "linked/Rally.exe".into();
        let r = ReviewedStaticTablesProfile::from_json(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("intermediate-out"), &r, |_, _| Ok(map())),
            Err(StaticTablesImportError::Symlink(_))
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
            Err(StaticTablesImportError::SourceNotDirectory(_))
        ));
    }
}
