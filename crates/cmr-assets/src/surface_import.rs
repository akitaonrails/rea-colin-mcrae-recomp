use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cmr_formats::pe_surface::{
    PeSurfaceError, SURFACE_CAPABILITY_BYTES, extract_official_eu_surface_capabilities,
    extract_official_eu_surface_coefficients,
};
use serde::{Deserialize, Serialize};

use crate::verify::{require_real_directory, sha256};
use crate::{ReviewedSurfaceCapabilityProfile, ReviewedSurfaceProfile, SurfaceProfileError};

pub const SURFACE_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const SURFACE_DATA_FILE: &str = "surface-coefficients-q16le.bin";
pub const SURFACE_STATE_FILE: &str = "surface-import-state.json";
pub const SURFACE_CAPABILITY_IMPORT_SCHEMA_VERSION: u32 = 1;
pub const SURFACE_CAPABILITY_DATA_FILE: &str = "surface-capabilities-u8.bin";
pub const SURFACE_CAPABILITY_STATE_FILE: &str = "surface-capability-import-state.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SurfaceImportFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SurfaceImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: SurfaceImportFile,
    pub output: SurfaceImportFile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SurfaceCapabilityImportSummary {
    pub schema_version: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub source: SurfaceImportFile,
    pub output: SurfaceImportFile,
}

#[derive(Debug)]
pub enum SurfaceImportError {
    Profile(SurfaceProfileError),
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
    Format(PeSurfaceError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    WrittenDataMismatch(PathBuf),
}

impl fmt::Display for SurfaceImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(error) => write!(f, "invalid surface profile: {error}"),
            Self::SourceNotDirectory(path) => {
                write!(f, "source is not a real directory: {}", path.display())
            }
            Self::Symlink(path) => write!(f, "refusing symbolic link: {}", path.display()),
            Self::SpecialFile(path) => write!(f, "refusing special file: {}", path.display()),
            Self::SizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "size mismatch for {path}: expected {expected}, got {actual}"
                )
            }
            Self::HashMismatch {
                path,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "SHA-256 mismatch for {path}: expected {expected}, got {actual}"
                )
            }
            Self::InvalidOutput(path) => write!(f, "invalid output path: {}", path.display()),
            Self::OutputParentNotDirectory(path) => write!(
                f,
                "output parent is not a real directory: {}",
                path.display()
            ),
            Self::OutputExists(path) => write!(f, "output already exists: {}", path.display()),
            Self::StageExists(path) => write!(f, "staging path already exists: {}", path.display()),
            Self::Format(error) => write!(f, "cannot extract surface coefficients: {error}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(error) => write!(f, "cannot serialize surface state: {error}"),
            Self::WrittenDataMismatch(path) => {
                write!(f, "written surface data changed at {}", path.display())
            }
        }
    }
}

impl std::error::Error for SurfaceImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Profile(error) => Some(error),
            Self::Format(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Serialize(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SurfaceProfileError> for SurfaceImportError {
    fn from(error: SurfaceProfileError) -> Self {
        Self::Profile(error)
    }
}

impl From<PeSurfaceError> for SurfaceImportError {
    fn from(error: PeSurfaceError) -> Self {
        Self::Format(error)
    }
}

pub fn extract_surface_tables(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedSurfaceProfile,
) -> Result<SurfaceImportSummary, SurfaceImportError> {
    let profile = reviewed.profile();
    profile.validate()?;
    require_directory(source)?;
    let executable =
        read_profiled_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual_hash = sha256(&executable);
    if actual_hash != profile.executable.sha256 {
        return Err(SurfaceImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual: actual_hash,
        });
    }
    let families = extract_official_eu_surface_coefficients(&executable)?;
    let mut data = Vec::with_capacity(2 * families.first.values.len() * 4);
    for value in families.first.values.iter().chain(&families.second.values) {
        data.extend_from_slice(&value.to_le_bytes());
    }

    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| SurfaceImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate_stage(&stage, profile, reviewed.sha256(), &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                cleanup_stage(&stage);
                return Err(SurfaceImportError::Io {
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

/// Extract the reviewed capability span into a caller-selected new directory.
pub fn extract_surface_capabilities(
    source: &Path,
    output: &Path,
    reviewed: &ReviewedSurfaceCapabilityProfile,
) -> Result<SurfaceCapabilityImportSummary, SurfaceImportError> {
    let profile = reviewed.profile();
    profile.validate()?;
    require_directory(source)?;
    let executable =
        read_profiled_executable(source, &profile.executable.path, profile.executable.size)?;
    let actual_hash = sha256(&executable);
    if actual_hash != profile.executable.sha256 {
        return Err(SurfaceImportError::HashMismatch {
            path: profile.executable.path.clone(),
            expected: profile.executable.sha256.clone(),
            actual: actual_hash,
        });
    }
    let tables = extract_official_eu_surface_capabilities(&executable)?;
    let data: Vec<u8> = tables.values.into_iter().flatten().collect();
    let stage = validate_output(source, output)?;
    fs::create_dir(&stage).map_err(|source| SurfaceImportError::Io {
        path: stage.clone(),
        source,
    })?;
    let result = populate_capability_stage(&stage, profile, reviewed.sha256(), &data);
    match result {
        Ok(summary) => {
            if let Err(source) = fs::rename(&stage, output) {
                cleanup_stage(&stage);
                return Err(SurfaceImportError::Io {
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
) -> Result<Vec<u8>, SurfaceImportError> {
    let mut path = source.to_owned();
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        path.push(component);
        let metadata = fs::symlink_metadata(&path).map_err(|source| SurfaceImportError::Io {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(SurfaceImportError::Symlink(path));
        }
        if components.peek().is_some() && !metadata.is_dir() {
            return Err(SurfaceImportError::SpecialFile(path));
        }
    }

    let file = fs::File::open(&path).map_err(|source| SurfaceImportError::Io {
        path: path.clone(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| SurfaceImportError::Io {
        path: path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(SurfaceImportError::SpecialFile(path));
    }
    if metadata.len() != expected_size {
        return Err(SurfaceImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: metadata.len(),
        });
    }

    let limit = expected_size
        .checked_add(1)
        .expect("validated executable-size bound fits u64");
    let capacity = usize::try_from(limit).expect("validated executable-size bound fits usize");
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| SurfaceImportError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() as u64 != expected_size {
        return Err(SurfaceImportError::SizeMismatch {
            path: relative.to_owned(),
            expected: expected_size,
            actual: bytes.len() as u64,
        });
    }
    Ok(bytes)
}

fn require_directory(path: &Path) -> Result<(), SurfaceImportError> {
    require_real_directory(path)
        .map_err(|_| SurfaceImportError::SourceNotDirectory(path.to_owned()))
}

fn validate_output(source: &Path, output: &Path) -> Result<PathBuf, SurfaceImportError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    require_real_directory(parent)
        .map_err(|_| SurfaceImportError::OutputParentNotDirectory(parent.to_owned()))?;
    let source_root = fs::canonicalize(source).map_err(|error| SurfaceImportError::Io {
        path: source.to_owned(),
        source: error,
    })?;
    let output_parent = fs::canonicalize(parent).map_err(|source| SurfaceImportError::Io {
        path: parent.to_owned(),
        source,
    })?;
    if output_parent.starts_with(&source_root) {
        return Err(SurfaceImportError::InvalidOutput(output.to_owned()));
    }
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != "." && *name != "..")
        .ok_or_else(|| SurfaceImportError::InvalidOutput(output.to_owned()))?;
    match fs::symlink_metadata(output) {
        Ok(_) => return Err(SurfaceImportError::OutputExists(output.to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(SurfaceImportError::Io {
                path: output.to_owned(),
                source,
            });
        }
    }
    let stage = parent.join(format!(".{name}.cmr-surface-stage-{}", std::process::id()));
    match fs::symlink_metadata(&stage) {
        Ok(_) => return Err(SurfaceImportError::StageExists(stage)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(SurfaceImportError::Io {
                path: stage,
                source,
            });
        }
    }
    Ok(stage)
}

fn populate_stage(
    stage: &Path,
    profile: &crate::SurfaceTableProfile,
    profile_sha256: &str,
    data: &[u8],
) -> Result<SurfaceImportSummary, SurfaceImportError> {
    let data_path = stage.join(SURFACE_DATA_FILE);
    fs::write(&data_path, data).map_err(|source| SurfaceImportError::Io {
        path: data_path.clone(),
        source,
    })?;
    let written = fs::read(&data_path).map_err(|source| SurfaceImportError::Io {
        path: data_path.clone(),
        source,
    })?;
    if written != data {
        return Err(SurfaceImportError::WrittenDataMismatch(data_path));
    }
    let output_hash = sha256(&written);
    let summary = SurfaceImportSummary {
        schema_version: SURFACE_IMPORT_SCHEMA_VERSION,
        profile: profile.id.clone(),
        profile_sha256: profile_sha256.to_owned(),
        source: SurfaceImportFile {
            path: profile.executable.path.clone(),
            size: profile.executable.size,
            sha256: profile.executable.sha256.clone(),
        },
        output: SurfaceImportFile {
            path: SURFACE_DATA_FILE.to_owned(),
            size: data.len() as u64,
            sha256: output_hash,
        },
    };
    let mut state = serde_json::to_vec_pretty(&summary).map_err(SurfaceImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(SURFACE_STATE_FILE);
    fs::write(&state_path, &state).map_err(|source| SurfaceImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    let read_state = fs::read(&state_path).map_err(|source| SurfaceImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    if read_state != state {
        return Err(SurfaceImportError::WrittenDataMismatch(state_path));
    }
    Ok(summary)
}

fn populate_capability_stage(
    stage: &Path,
    profile: &crate::SurfaceCapabilityProfile,
    profile_sha256: &str,
    data: &[u8],
) -> Result<SurfaceCapabilityImportSummary, SurfaceImportError> {
    let data_path = stage.join(SURFACE_CAPABILITY_DATA_FILE);
    fs::write(&data_path, data).map_err(|source| SurfaceImportError::Io {
        path: data_path.clone(),
        source,
    })?;
    let written = fs::read(&data_path).map_err(|source| SurfaceImportError::Io {
        path: data_path.clone(),
        source,
    })?;
    if written != data {
        return Err(SurfaceImportError::WrittenDataMismatch(data_path));
    }
    let summary = SurfaceCapabilityImportSummary {
        schema_version: SURFACE_CAPABILITY_IMPORT_SCHEMA_VERSION,
        profile: profile.id.clone(),
        profile_sha256: profile_sha256.to_owned(),
        source: SurfaceImportFile {
            path: profile.executable.path.clone(),
            size: profile.executable.size,
            sha256: profile.executable.sha256.clone(),
        },
        output: SurfaceImportFile {
            path: SURFACE_CAPABILITY_DATA_FILE.to_owned(),
            size: SURFACE_CAPABILITY_BYTES as u64,
            sha256: sha256(&written),
        },
    };
    let mut state = serde_json::to_vec_pretty(&summary).map_err(SurfaceImportError::Serialize)?;
    state.push(b'\n');
    let state_path = stage.join(SURFACE_CAPABILITY_STATE_FILE);
    fs::write(&state_path, &state).map_err(|source| SurfaceImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    let read_state = fs::read(&state_path).map_err(|source| SurfaceImportError::Io {
        path: state_path.clone(),
        source,
    })?;
    if read_state != state {
        return Err(SurfaceImportError::WrittenDataMismatch(state_path));
    }
    Ok(summary)
}

fn cleanup_stage(stage: &Path) {
    let _ = fs::remove_dir_all(stage);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SurfaceExecutableProfile, SurfaceTableEncoding, SurfaceTableProfile, SurfaceTableSpec,
    };
    use cmr_formats::pe_surface::OFFICIAL_EU_SURFACE_FAMILY_VA;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "cmr-surface-{}-{}",
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

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn synthetic_pe() -> Vec<u8> {
        let mut bytes = vec![0; 0xa00];
        let pe = 0x80;
        let optional = pe + 24;
        let sections = optional + 0xe0;
        let raw = 0x200;
        let target = raw + 0x2cc;
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 0x3c, pe as u32);
        bytes[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, pe + 4, 0x14c);
        put_u16(&mut bytes, pe + 6, 1);
        put_u16(&mut bytes, pe + 20, 0xe0);
        put_u16(&mut bytes, optional, 0x10b);
        put_u32(&mut bytes, optional + 28, 0x400000);
        put_u32(&mut bytes, sections + 12, 0xf7000);
        put_u32(&mut bytes, sections + 16, 0x800);
        put_u32(&mut bytes, sections + 20, raw as u32);
        for index in 0..240 {
            put_u32(&mut bytes, target + index * 4, (index as i32 - 120) as u32);
        }
        bytes
    }

    fn synthetic_capability_pe() -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        let pe = 0x80;
        let optional = pe + 24;
        let sections = optional + 0xe0;
        let raw = 0x200;
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 0x3c, pe as u32);
        bytes[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, pe + 4, 0x14c);
        put_u16(&mut bytes, pe + 6, 1);
        put_u16(&mut bytes, pe + 20, 0xe0);
        put_u16(&mut bytes, optional, 0x10b);
        put_u32(&mut bytes, optional + 28, 0x400000);
        put_u32(&mut bytes, sections + 8, 0x900);
        put_u32(&mut bytes, sections + 12, 0xec000);
        put_u32(&mut bytes, sections + 16, 0x900);
        put_u32(&mut bytes, sections + 20, raw as u32);
        for (index, byte) in bytes[raw + 0xf8..raw + 0xf8 + SURFACE_CAPABILITY_BYTES]
            .iter_mut()
            .enumerate()
        {
            *byte = (index % 251) as u8;
        }
        bytes
    }

    fn reviewed_capabilities(bytes: &[u8]) -> ReviewedSurfaceCapabilityProfile {
        let profile = crate::SurfaceCapabilityProfile {
            schema_version: 1,
            id: "synthetic-capabilities".into(),
            executable: SurfaceExecutableProfile {
                path: "Rally.exe".into(),
                size: bytes.len() as u64,
                sha256: sha256(bytes),
            },
            table: crate::SurfaceCapabilitySpec {
                virtual_address: cmr_formats::pe_surface::OFFICIAL_EU_SURFACE_CAPABILITY_VA,
                tables: 16,
                entries_per_table: 120,
                entry_encoding: crate::SurfaceCapabilityEncoding::UnsignedU8,
            },
        };
        ReviewedSurfaceCapabilityProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap()
    }

    fn reviewed_at(bytes: &[u8], path: &str) -> ReviewedSurfaceProfile {
        let profile = SurfaceTableProfile {
            schema_version: 1,
            id: "synthetic".into(),
            executable: SurfaceExecutableProfile {
                path: path.into(),
                size: bytes.len() as u64,
                sha256: sha256(bytes),
            },
            table: SurfaceTableSpec {
                virtual_address: OFFICIAL_EU_SURFACE_FAMILY_VA,
                families: 2,
                entries_per_family: 120,
                entry_encoding: SurfaceTableEncoding::SignedI32LittleEndian,
            },
        };
        ReviewedSurfaceProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap()
    }

    fn reviewed(bytes: &[u8]) -> ReviewedSurfaceProfile {
        reviewed_at(bytes, "Rally.exe")
    }

    #[test]
    fn extracts_synthetic_pe_atomically_with_path_free_state() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = synthetic_pe();
        fs::write(source.join("Rally.exe"), &bytes).unwrap();
        let output = temp.0.join("runtime-data");
        let summary = extract_surface_tables(&source, &output, &reviewed(&bytes)).unwrap();
        assert_eq!(summary.output.size, 960);
        let data = fs::read(output.join(SURFACE_DATA_FILE)).unwrap();
        assert_eq!(data.len(), 960);
        assert_eq!(i32::from_le_bytes(data[..4].try_into().unwrap()), -120);
        let state = fs::read_to_string(output.join(SURFACE_STATE_FILE)).unwrap();
        assert!(!state.contains(temp.0.to_str().unwrap()));
        assert_eq!(
            serde_json::from_str::<SurfaceImportSummary>(&state).unwrap(),
            summary
        );
    }

    #[test]
    fn extracts_capabilities_atomically_with_path_free_state() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = synthetic_capability_pe();
        fs::write(source.join("Rally.exe"), &bytes).unwrap();
        let output = temp.0.join("runtime-capabilities");
        let summary =
            extract_surface_capabilities(&source, &output, &reviewed_capabilities(&bytes)).unwrap();
        assert_eq!(summary.output.size, SURFACE_CAPABILITY_BYTES as u64);
        let data = fs::read(output.join(SURFACE_CAPABILITY_DATA_FILE)).unwrap();
        assert_eq!(data.len(), SURFACE_CAPABILITY_BYTES);
        for (index, byte) in data.iter().enumerate() {
            assert_eq!(*byte, (index % 251) as u8);
        }
        let state = fs::read_to_string(output.join(SURFACE_CAPABILITY_STATE_FILE)).unwrap();
        assert!(!state.contains(temp.0.to_str().unwrap()));
        assert_eq!(
            serde_json::from_str::<SurfaceCapabilityImportSummary>(&state).unwrap(),
            summary
        );
        assert!(
            extract_surface_capabilities(&source, &output, &reviewed_capabilities(&bytes)).is_err()
        );
    }

    #[test]
    fn hash_is_rejected_before_malformed_pe_parsing() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"not a pe";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let mut profile = reviewed(bytes).profile().clone();
        profile.executable.sha256 = "11".repeat(32);
        let reviewed =
            ReviewedSurfaceProfile::from_json(&serde_json::to_vec(&profile).unwrap()).unwrap();
        let error = extract_surface_tables(&source, &temp.0.join("out"), &reviewed).unwrap_err();
        assert!(matches!(error, SurfaceImportError::HashMismatch { .. }));
    }

    #[test]
    fn identified_but_malformed_pe_publishes_nothing() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = b"identified malformed PE";
        fs::write(source.join("Rally.exe"), bytes).unwrap();
        let output = temp.0.join("out");
        let error = extract_surface_tables(&source, &output, &reviewed(bytes)).unwrap_err();
        assert!(matches!(error, SurfaceImportError::Format(_)));
        assert!(!output.exists());
        assert!(
            !temp
                .0
                .join(format!(".out.cmr-surface-stage-{}", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn refuses_output_inside_source_and_existing_targets_or_stages() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = synthetic_pe();
        fs::write(source.join("Rally.exe"), &bytes).unwrap();
        let reviewed = reviewed(&bytes);
        assert!(matches!(
            extract_surface_tables(&source, &source.join("out"), &reviewed),
            Err(SurfaceImportError::InvalidOutput(_))
        ));
        let output = temp.0.join("out");
        fs::create_dir(&output).unwrap();
        assert!(matches!(
            extract_surface_tables(&source, &output, &reviewed),
            Err(SurfaceImportError::OutputExists(_))
        ));
        fs::remove_dir(&output).unwrap();
        let stage = temp
            .0
            .join(format!(".out.cmr-surface-stage-{}", std::process::id()));
        fs::create_dir(&stage).unwrap();
        assert!(matches!(
            extract_surface_tables(&source, &output, &reviewed),
            Err(SurfaceImportError::StageExists(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_executable() {
        use std::os::unix::fs::symlink;
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let bytes = synthetic_pe();
        fs::write(temp.0.join("real.exe"), &bytes).unwrap();
        symlink(temp.0.join("real.exe"), source.join("Rally.exe")).unwrap();
        assert!(matches!(
            extract_surface_tables(&source, &temp.0.join("out"), &reviewed(&bytes)),
            Err(SurfaceImportError::Symlink(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_intermediate_executable_directory() {
        use std::os::unix::fs::symlink;

        let temp = Temp::new();
        let source = temp.0.join("source");
        let actual = temp.0.join("actual");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&actual).unwrap();
        let bytes = synthetic_pe();
        fs::write(actual.join("Rally.exe"), &bytes).unwrap();
        symlink(&actual, source.join("bin")).unwrap();

        assert!(matches!(
            extract_surface_tables(
                &source,
                &temp.0.join("out"),
                &reviewed_at(&bytes, "bin/Rally.exe")
            ),
            Err(SurfaceImportError::Symlink(path)) if path == source.join("bin")
        ));
    }
}
