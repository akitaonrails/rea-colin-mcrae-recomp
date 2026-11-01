use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use cmr_formats::pe_car_selector::{
    CAR_SELECTOR_COUNT, CAR_SELECTOR_ENCODED_LEN, CAR_SELECTOR_FORMAT_VERSION, CarSelectorError,
    CarSelectorMap, decode_car_selectors, encode_car_selectors,
    extract_car_selectors as extract_format,
};
use serde::{Deserialize, Serialize};

use crate::car_selector_profile::{
    CarSelectorEncoding, CarSelectorProfile, CarSelectorProfileError, ReviewedCarSelectorProfile,
};
use crate::verify::{require_real_directory, sha256};

pub const CAR_SELECTOR_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const CAR_SELECTOR_DATA_FILE: &str = "car-selector-map-v1.bin";
pub const CAR_SELECTOR_STATE_FILE: &str = "car-selector-import-state.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarSelectorImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarSelectorImportShape {
    pub format_version: u32,
    pub selectors: u32,
    pub code_bytes: u32,
    pub encoding: CarSelectorEncoding,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CarSelectorImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: CarSelectorImportFile,
    pub output: CarSelectorImportFile,
    pub shape: CarSelectorImportShape,
}

#[derive(Debug)]
pub enum CarSelectorImportError {
    Profile(CarSelectorProfileError),
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
    Format(CarSelectorError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}
impl fmt::Display for CarSelectorImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(e) => write!(f, "invalid car-selector profile: {e}"),
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
            Self::Format(e) => write!(f, "cannot extract car selectors: {e}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(e) => write!(f, "cannot serialize car-selector state: {e}"),
            Self::WrittenDataMismatch(p) => {
                write!(f, "written car-selector data changed at {}", p.display())
            }
        }
    }
}
impl std::error::Error for CarSelectorImportError {}
impl From<CarSelectorProfileError> for CarSelectorImportError {
    fn from(e: CarSelectorProfileError) -> Self {
        Self::Profile(e)
    }
}

pub fn extract_car_selectors(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedCarSelectorProfile,
) -> Result<CarSelectorImportSummary, CarSelectorImportError> {
    extract_with(source, output, reviewed, |bytes, p| {
        extract_format(bytes, &p.selector_map.layout()).map_err(CarSelectorImportError::Format)
    })
}

fn extract_with<F>(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedCarSelectorProfile,
    extractor: F,
) -> Result<CarSelectorImportSummary, CarSelectorImportError>
where
    F: FnOnce(&[u8], &CarSelectorProfile) -> Result<CarSelectorMap, CarSelectorImportError>,
{
    let profile = reviewed.profile();
    profile.validate()?;
    require_dir(source)?;
    let bytes = read_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual = sha256(&bytes);
    if actual != profile.executable.sha256 {
        return Err(CarSelectorImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual,
        });
    }
    let map = extractor(&bytes, profile)?;
    let data = encode_car_selectors(&map);
    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| CarSelectorImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate(&stage, profile, reviewed.sha256(), &map, &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                let _ = fs::remove_dir_all(&stage);
                return Err(CarSelectorImportError::Io {
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
fn require_dir(path: &Path) -> Result<(), CarSelectorImportError> {
    require_real_directory(path)
        .map_err(|_| CarSelectorImportError::SourceNotDirectory(path.into()))
}
fn read_executable(
    root: &Path,
    relative: &str,
    expected: u64,
) -> Result<Vec<u8>, CarSelectorImportError> {
    let mut path = root.to_owned();
    let mut cs = relative.split('/').peekable();
    while let Some(c) = cs.next() {
        path.push(c);
        let m = fs::symlink_metadata(&path).map_err(|source| CarSelectorImportError::Io {
            path: path.clone(),
            source,
        })?;
        if m.file_type().is_symlink() {
            return Err(CarSelectorImportError::Symlink(path));
        }
        if cs.peek().is_some() {
            if !m.is_dir() {
                return Err(CarSelectorImportError::SpecialFile(path));
            }
        } else if !m.is_file() {
            return Err(CarSelectorImportError::SpecialFile(path));
        }
    }
    let file = fs::File::open(&path).map_err(|source| CarSelectorImportError::Io {
        path: path.clone(),
        source,
    })?;
    let m = file
        .metadata()
        .map_err(|source| CarSelectorImportError::Io {
            path: path.clone(),
            source,
        })?;
    if !m.is_file() {
        return Err(CarSelectorImportError::SpecialFile(path));
    }
    if m.len() != expected {
        return Err(CarSelectorImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: m.len(),
        });
    }
    let mut b = Vec::new();
    file.take(expected + 1)
        .read_to_end(&mut b)
        .map_err(|source| CarSelectorImportError::Io {
            path: path.clone(),
            source,
        })?;
    if b.len() as u64 != expected {
        return Err(CarSelectorImportError::SizeMismatch {
            path: relative.into(),
            expected,
            actual: b.len() as u64,
        });
    }
    Ok(b)
}
fn validate_output(source: &Path, output: &Path) -> Result<PathBuf, CarSelectorImportError> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_dir(parent)?;
    let source_root = source.to_owned();
    let source = fs::canonicalize(source).map_err(|error| CarSelectorImportError::Io {
        path: source_root,
        source: error,
    })?;
    let parent_real = fs::canonicalize(parent).map_err(|source| CarSelectorImportError::Io {
        path: parent.into(),
        source,
    })?;
    if parent_real.starts_with(source) {
        return Err(CarSelectorImportError::InvalidOutput(output.into()));
    }
    let name = output
        .file_name()
        .and_then(|v| v.to_str())
        .filter(|v| *v != "." && *v != "..")
        .ok_or_else(|| CarSelectorImportError::InvalidOutput(output.into()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(CarSelectorImportError::OutputExists(output.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CarSelectorImportError::Io {
                path: output.into(),
                source,
            });
        }
    }
    let stage = parent.join(format!(".{name}.cmr-selector-stage-{}", std::process::id()));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(CarSelectorImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(CarSelectorImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}
fn populate(
    stage: &Path,
    p: &CarSelectorProfile,
    p_hash: &str,
    map: &CarSelectorMap,
    data: &[u8; CAR_SELECTOR_ENCODED_LEN],
) -> Result<CarSelectorImportSummary, CarSelectorImportError> {
    let dp = stage.join(CAR_SELECTOR_DATA_FILE);
    write_new(&dp, data)?;
    let written = fs::read(&dp).map_err(|source| CarSelectorImportError::Io {
        path: dp.clone(),
        source,
    })?;
    if decode_car_selectors(&written).map_err(CarSelectorImportError::Format)? != *map {
        return Err(CarSelectorImportError::WrittenDataMismatch(dp));
    }
    let summary = CarSelectorImportSummary {
        schema_version: 1,
        profile: p.id.clone(),
        profile_sha256: p_hash.into(),
        source: CarSelectorImportFile {
            path: p.executable.path.clone(),
            size: p.executable.size,
            sha256: p.executable.sha256.clone(),
        },
        output: CarSelectorImportFile {
            path: CAR_SELECTOR_DATA_FILE.into(),
            size: CAR_SELECTOR_ENCODED_LEN as u64,
            sha256: sha256(&written),
        },
        shape: CarSelectorImportShape {
            format_version: CAR_SELECTOR_FORMAT_VERSION,
            selectors: CAR_SELECTOR_COUNT as u32,
            code_bytes: 32,
            encoding: CarSelectorEncoding::CmrCselectorV1,
        },
    };
    let mut state =
        serde_json::to_vec_pretty(&summary).map_err(CarSelectorImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(CAR_SELECTOR_STATE_FILE);
    write_new(&state_path, &state)?;
    let written_state = fs::read(&state_path).map_err(|source| CarSelectorImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    let reparsed: CarSelectorImportSummary =
        serde_json::from_slice(&written_state).map_err(CarSelectorImportError::Serialize)?;
    if written_state != state || reparsed != summary {
        return Err(CarSelectorImportError::WrittenDataMismatch(state_path));
    }
    Ok(summary)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CarSelectorImportError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| CarSelectorImportError::Io {
            path: path.into(),
            source,
        })?;
    f.write_all(bytes)
        .map_err(|source| CarSelectorImportError::Io {
            path: path.into(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car_selector_profile::tests::profile;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "cmr-selector-{}-{}",
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
    fn map() -> CarSelectorMap {
        CarSelectorMap::new(std::array::from_fn(|i| format!("CAR{i:02}"))).unwrap()
    }
    fn reviewed(bytes: &[u8]) -> ReviewedCarSelectorProfile {
        ReviewedCarSelectorProfile::from_json(
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
        let state = fs::read_to_string(out.join(CAR_SELECTOR_STATE_FILE)).unwrap();
        assert!(!state.contains(t.0.to_str().unwrap()));
        let store = crate::car_selector_store::CarSelectorStore::open(&out, &r).unwrap();
        assert_eq!(store.selector_for_code("car03"), Some(3));
        let data_path = out.join(CAR_SELECTOR_DATA_FILE);
        let mut data = fs::read(&data_path).unwrap();
        data[20] ^= 1;
        fs::write(&data_path, data).unwrap();
        assert!(matches!(
            crate::car_selector_store::CarSelectorStore::open(&out, &r),
            Err(
                crate::car_selector_store::CarSelectorStoreError::InvalidState {
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

        let wrong_size = ReviewedCarSelectorProfile::from_json(
            &serde_json::to_vec(&profile((bytes.len() + 1) as u64, hash(bytes))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("size-out"), &wrong_size, |_, _| panic!(
                "parser ran"
            )),
            Err(CarSelectorImportError::SizeMismatch { .. })
        ));

        let wrong_hash = ReviewedCarSelectorProfile::from_json(
            &serde_json::to_vec(&profile(bytes.len() as u64, "11".repeat(32))).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("hash-out"), &wrong_hash, |_, _| panic!(
                "parser ran"
            )),
            Err(CarSelectorImportError::HashMismatch { .. })
        ));

        let output = t.0.join("failed-out");
        assert!(matches!(
            extract_with(&src, &output, &reviewed(bytes), |_, _| Err(
                CarSelectorImportError::Format(CarSelectorError::ConsumerInstruction)
            )),
            Err(CarSelectorImportError::Format(_))
        ));
        assert!(!output.exists());
        assert!(
            !t.0.join(format!(
                ".failed-out.cmr-selector-stage-{}",
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
            Err(CarSelectorImportError::SourceNotDirectory(_))
        ));
        let src = t.0.join("source");
        fs::create_dir(&src).unwrap();
        let bytes = b"synthetic";
        fs::write(src.join("Rally.exe"), bytes).unwrap();
        let r = reviewed(bytes);
        assert!(matches!(
            extract_with(&src, &src.join("inside"), &r, |_, _| Ok(map())),
            Err(CarSelectorImportError::InvalidOutput(_))
        ));
        let output = t.0.join("existing");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_with(&src, &output, &r, |_, _| Ok(map())),
            Err(CarSelectorImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        let stage = t.0.join(format!(
            ".existing.cmr-selector-stage-{}",
            std::process::id()
        ));
        fs::create_dir(&stage).unwrap();
        assert!(
            matches!(extract_with(&src,&output,&r,|_,_|Ok(map())),Err(CarSelectorImportError::StageExists(p)) if p==stage)
        );
        fs::remove_dir(&stage).unwrap();
        fs::remove_file(src.join("Rally.exe")).unwrap();
        fs::create_dir(src.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("special-out"), &r, |_, _| Ok(map())),
            Err(CarSelectorImportError::SpecialFile(_))
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
            Err(CarSelectorImportError::Symlink(_))
        ));
        fs::remove_file(src.join("Rally.exe")).unwrap();
        let real_dir = t.0.join("real-dir");
        fs::create_dir(&real_dir).unwrap();
        fs::write(real_dir.join("Rally.exe"), bytes).unwrap();
        symlink(&real_dir, src.join("linked")).unwrap();
        let mut p = profile(bytes.len() as u64, hash(bytes));
        p.executable.path = "linked/Rally.exe".into();
        // Profile review rejects non-canonical layouts only, not safe nested paths.
        let r = ReviewedCarSelectorProfile::from_json(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert!(matches!(
            extract_with(&src, &t.0.join("intermediate-out"), &r, |_, _| Ok(map())),
            Err(CarSelectorImportError::Symlink(_))
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
            Err(CarSelectorImportError::SourceNotDirectory(_))
        ));
    }
}
