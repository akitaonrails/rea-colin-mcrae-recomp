use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cmr_formats::pe_engine_audio_tables::{
    ENGINE_AUDIO_TABLES_ENCODED_LEN, ENGINE_AUDIO_TABLES_FORMAT_VERSION,
    ENGINE_UPSHIFT_DIP_ENTRIES, EngineAudioTablesError, EngineSoundBandId, EngineSoundTables,
    decode_engine_audio_tables, encode_engine_audio_tables,
    extract_engine_audio_tables as extract_format,
};
use serde::{Deserialize, Serialize};

use crate::engine_audio_tables_profile::{
    EngineAudioTablesEncoding, EngineAudioTablesProfile, EngineAudioTablesProfileError,
    ReviewedEngineAudioTablesProfile,
};
use crate::verify::{require_real_directory, sha256};

pub const ENGINE_AUDIO_TABLES_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const ENGINE_AUDIO_TABLES_DATA_FILE: &str = "engine-audio-tables-v1.bin";
pub const ENGINE_AUDIO_TABLES_STATE_FILE: &str = "engine-audio-tables-import-state.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineAudioTablesImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineAudioTablesImportShape {
    pub format_version: u32,
    /// Array length `N` of the slow, mid, top and whine bands, in that order.
    pub band_entries: [u32; 4],
    pub upshift_dip_entries: u32,
    pub encoding: EngineAudioTablesEncoding,
}

impl EngineAudioTablesImportShape {
    /// The only shape the reviewed layout produces.
    pub fn expected() -> Self {
        Self {
            format_version: ENGINE_AUDIO_TABLES_FORMAT_VERSION,
            band_entries: EngineSoundBandId::ALL.map(|id| id.entries() as u32),
            upshift_dip_entries: ENGINE_UPSHIFT_DIP_ENTRIES as u32,
            encoding: EngineAudioTablesEncoding::CmrEngineAudioTablesV1,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineAudioTablesImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: EngineAudioTablesImportFile,
    pub output: EngineAudioTablesImportFile,
    pub shape: EngineAudioTablesImportShape,
}

#[derive(Debug)]
pub enum EngineAudioTablesImportError {
    Profile(EngineAudioTablesProfileError),
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
    Format(EngineAudioTablesError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}
impl fmt::Display for EngineAudioTablesImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(e) => write!(f, "invalid engine-audio-tables profile: {e}"),
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
            Self::InvalidOutput(p) => write!(f, "invalid output path: {}", p.display()),
            Self::OutputExists(p) => write!(f, "output already exists: {}", p.display()),
            Self::StageExists(p) => write!(f, "staging path already exists: {}", p.display()),
            Self::Format(e) => write!(f, "cannot extract engine audio tables: {e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(e) => write!(f, "cannot serialize engine-audio-tables state: {e}"),
            Self::WrittenDataMismatch(p) => {
                write!(
                    f,
                    "written engine-audio-tables data changed at {}",
                    p.display()
                )
            }
        }
    }
}
impl std::error::Error for EngineAudioTablesImportError {}
impl From<EngineAudioTablesProfileError> for EngineAudioTablesImportError {
    fn from(e: EngineAudioTablesProfileError) -> Self {
        Self::Profile(e)
    }
}

pub fn extract_engine_audio_tables(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedEngineAudioTablesProfile,
) -> Result<EngineAudioTablesImportSummary, EngineAudioTablesImportError> {
    extract_with(source, output, reviewed, |bytes, p| {
        extract_format(bytes, &p.tables.layout()).map_err(EngineAudioTablesImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedEngineAudioTablesProfile,
    extractor: F,
) -> Result<EngineAudioTablesImportSummary, EngineAudioTablesImportError>
where
    F: FnOnce(
        &[u8],
        &EngineAudioTablesProfile,
    ) -> Result<EngineSoundTables, EngineAudioTablesImportError>,
{
    let profile = reviewed.profile();
    profile.validate()?;
    require_dir(source)?;
    let bytes = read_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual = sha256(&bytes);
    if actual != profile.executable.sha256 {
        return Err(EngineAudioTablesImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual,
        });
    }
    let map = extractor(&bytes, profile)?;
    let data = encode_engine_audio_tables(&map).map_err(EngineAudioTablesImportError::Format)?;
    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| EngineAudioTablesImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate(&stage, profile, reviewed.sha256(), &map, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                let _ = fs::remove_dir_all(&stage);
                return Err(EngineAudioTablesImportError::Io {
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
fn require_dir(path: &Path) -> Result<(), EngineAudioTablesImportError> {
    require_real_directory(path)
        .map_err(|_| EngineAudioTablesImportError::SourceNotDirectory(path.into()))
}
fn read_executable(
    root: &Path,
    relative: &str,
    expected: u64,
) -> Result<Vec<u8>, EngineAudioTablesImportError> {
    let mut path = root.to_owned();
    let mut cs = relative.split('/').peekable();
    while let Some(c) = cs.next() {
        path.push(c);
        let m = fs::symlink_metadata(&path).map_err(|source| EngineAudioTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
        if m.file_type().is_symlink() {
            return Err(EngineAudioTablesImportError::Symlink(path));
        }
        if cs.peek().is_some() {
            if !m.is_dir() {
                return Err(EngineAudioTablesImportError::SpecialFile(path));
            }
        } else if !m.is_file() {
            return Err(EngineAudioTablesImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| EngineAudioTablesImportError::Io {
        path: path.clone(),
        source,
    })?;
    let m = file
        .metadata()
        .map_err(|source| EngineAudioTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !m.is_file() {
        return Err(EngineAudioTablesImportError::SpecialFile(path));
    }
    if m.len() != expected {
        return Err(EngineAudioTablesImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: m.len(),
        });
    }
    let mut b = Vec::new();
    file.take(expected + 1)
        .read_to_end(&mut b)
        .map_err(|source| EngineAudioTablesImportError::Io {
            path: path.clone(),
            source,
        })?;
    if b.len() as u64 != expected {
        return Err(EngineAudioTablesImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: b.len() as u64,
        });
    }
    Ok(b)
}
fn validate_output(source: &Path, output: &Path) -> Result<PathBuf, EngineAudioTablesImportError> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_dir(parent)?;
    let source_root = source.to_owned();
    let source = fs::canonicalize(source).map_err(|error| EngineAudioTablesImportError::Io {
        path: source_root,
        source: error,
    })?;
    let parent_real =
        fs::canonicalize(parent).map_err(|source| EngineAudioTablesImportError::Io {
            path: parent.into(),
            source,
        })?;
    if parent_real.starts_with(source) {
        return Err(EngineAudioTablesImportError::InvalidOutput(output.into()));
    }
    let name = output
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| *v != "." && *v != "..")
        .ok_or_else(|| EngineAudioTablesImportError::InvalidOutput(output.into()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(EngineAudioTablesImportError::OutputExists(output.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(EngineAudioTablesImportError::Io {
                path: output.into(),
                source,
            });
        }
    }
    let stage = parent.join(format!(
        ".{name}.cmr-engine-audio-stage-{}",
        std::process::id()
    ));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(EngineAudioTablesImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(EngineAudioTablesImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}
fn populate(
    stage: &Path,
    p: &EngineAudioTablesProfile,
    p_hash: &str,
    map: &EngineSoundTables,
    data: &[u8],
) -> Result<EngineAudioTablesImportSummary, EngineAudioTablesImportError> {
    let dp = stage.join(ENGINE_AUDIO_TABLES_DATA_FILE);
    write_new(&dp, data)?;
    let written = fs::read(&dp).map_err(|source| EngineAudioTablesImportError::Io {
        path: dp.clone(),
        source,
    })?;
    if decode_engine_audio_tables(&written).map_err(EngineAudioTablesImportError::Format)? != *map {
        return Err(EngineAudioTablesImportError::WrittenDataMismatch(dp));
    }
    let summary = EngineAudioTablesImportSummary {
        schema_version: 1,
        profile: p.id.clone(),
        profile_sha256: p_hash.into(),
        source: EngineAudioTablesImportFile {
            path: p.executable.path.clone(),
            size: p.executable.size,
            sha256: p.executable.sha256.clone(),
        },
        output: EngineAudioTablesImportFile {
            path: ENGINE_AUDIO_TABLES_DATA_FILE.into(),
            size: ENGINE_AUDIO_TABLES_ENCODED_LEN as u64,
            sha256: sha256(&written),
        },
        shape: EngineAudioTablesImportShape::expected(),
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(EngineAudioTablesImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(ENGINE_AUDIO_TABLES_STATE_FILE);
    write_new(&state_path, &state)?;
    let written_state =
        fs::read(&state_path).map_err(|source| EngineAudioTablesImportError::Io {
            path: state_path.clone(),
            source,
        })?;
    let reparsed: EngineAudioTablesImportSummary =
        serde_json::from_slice(&written_state).map_err(EngineAudioTablesImportError::Serialize)?;
    if written_state != state || reparsed != summary {
        return Err(EngineAudioTablesImportError::WrittenDataMismatch(
            state_path,
        ));
    }
    Ok(summary)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), EngineAudioTablesImportError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| EngineAudioTablesImportError::Io {
            path: path.into(),
            source,
        })?;
    f.write_all(bytes)
        .map_err(|source| EngineAudioTablesImportError::Io {
            path: path.into(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_audio_tables_profile::tests::{profile, synthetic_tables};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "cmr-engine-audio-{}-{}",
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
    fn hash(b: &[u8]) -> String {
        Sha256::digest(b)
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect()
    }
    fn map() -> EngineSoundTables {
        synthetic_tables()
    }
    fn reviewed(bytes: &[u8]) -> ReviewedEngineAudioTablesProfile {
        ReviewedEngineAudioTablesProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, hash(bytes))).unwrap(),
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
        extract_with(&src, &out, &r, |_, _| Ok(map())).unwrap();
        let state = fs::read_to_string(out.join(ENGINE_AUDIO_TABLES_STATE_FILE)).unwrap();
        assert!(!state.contains(t.0.to_str().unwrap()));
        let store =
            crate::engine_audio_tables_store::EngineAudioTablesStore::open(&out, &r).unwrap();
        assert_eq!(store.tables(), &map());
        let data_path = out.join(ENGINE_AUDIO_TABLES_DATA_FILE);
        let mut data = fs::read(&data_path).unwrap();
        data[20] ^= 1;
        fs::write(&data_path, data).unwrap();
        assert!(matches!(
            crate::engine_audio_tables_store::EngineAudioTablesStore::open(&out, &r),
            Err(
                crate::engine_audio_tables_store::EngineAudioTablesStoreError::InvalidState {
                    field: "output.sha256"
                }
            )
        ));
    }

    #[test]
    fn rejects_identity_before_parser_and_cleans_failed_extraction() {
        let t = Temp::new();
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();

        let wrong_size = ReviewedEngineAudioTablesProfile::from_json(
            &serde_json::to_vec(&profile((bytes.len() + 1) as u64, hash(bytes))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("size-out"), &wrong_size, |_, _| panic!(
                "parser ran"
            )),
            Err(EngineAudioTablesImportError::SizeMismatch { .. })
        ));

        let wrong_hash = ReviewedEngineAudioTablesProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, "11".repeat(32))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("hash-out"), &wrong_hash, |_, _| panic!(
                "parser ran"
            )),
            Err(EngineAudioTablesImportError::HashMismatch { .. })
        ));

        let output = t.0.join("failed-out");
        assert!(matches!(
            extract_with(&src, &output, &reviewed(bytes), |_, _| Err(
                EngineAudioTablesImportError::Format(EngineAudioTablesError::UnsupportedLayout)
            )),
            Err(EngineAudioTablesImportError::Format(_))
        ));
        assert!(!output.exists());
        assert!(
            !t.0.join(format!(
                ".failed-out.cmr-engine-audio-stage-{}",
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
            Err(EngineAudioTablesImportError::SourceNotDirectory(_))
        ));
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let r = reviewed(bytes);
        assert!(matches!(
            extract_with(&src, &src.join("inside"), &r, |_, _| Ok(map())),
            Err(EngineAudioTablesImportError::InvalidOutput(_))
        ));
        let output = t.0.join("existing");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(EngineAudioTablesImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        let stage = t.0.join(format!(
            ".existing.cmr-engine-audio-stage-{}",
            std::process::id()
        ));
        fs::create_dir(&stage).unwrap();
        assert!(
            matches!(extract_with(&src,&output,&r,|_,_|Ok(map())),Err(EngineAudioTablesImportError::StageExists(p)) if p==stage)
        );
        fs::remove_dir(&stage).unwrap();
        fs::remove_file(src.join("Rally.exe")).unwrap();
        fs::create_dir(src.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("special-out"), &r, |_, _| Ok(map())),
            Err(EngineAudioTablesImportError::SpecialFile(_))
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
            Err(EngineAudioTablesImportError::Symlink(_))
        ));
        fs::remove_file(src.join("Rally.exe")).unwrap();
        let real_dir = t.0.join("real-dir");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("Rally.exe"), bytes).unwrap();
        symlink(&real_dir, src.join("linked")).unwrap();
        let mut p = profile(bytes.len() as u64, hash(bytes));
        p.executable.path = "linked/Rally.exe".into();
        // Profile review rejects non-canonical layouts only, not safe nested paths.
        let r =
            ReviewedEngineAudioTablesProfile::from_json(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("intermediate-out"), &r, |_, _| Ok(map())),
            Err(EngineAudioTablesImportError::Symlink(_))
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
            Err(EngineAudioTablesImportError::SourceNotDirectory(_))
        ));
    }
}
