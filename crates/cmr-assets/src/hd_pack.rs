//! Read-only access to the optional compact HD art pack.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const MAX_LIST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HdScale {
    X2,
    X4,
}

impl HdScale {
    #[must_use]
    pub const fn for_output(width: u32, height: u32) -> Self {
        if width > 1920 || height > 1080 {
            Self::X4
        } else {
            Self::X2
        }
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::X2 => "x2",
            Self::X4 => "x4",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HdPack {
    root: PathBuf,
    scale: HdScale,
    entries: BTreeMap<String, String>,
    replacements: Arc<AtomicUsize>,
}

#[derive(Debug)]
pub enum HdPackError {
    Io(std::io::Error),
    ListTooLarge(u64),
    InvalidHeader,
    InvalidEntry(String),
    DuplicateEntry(String),
}

impl fmt::Display for HdPackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::ListTooLarge(size) => write!(formatter, "pack list is too large ({size} bytes)"),
            Self::InvalidHeader => write!(formatter, "not a cmr-hd pack 1 manifest"),
            Self::InvalidEntry(entry) => write!(formatter, "invalid HD pack entry {entry:?}"),
            Self::DuplicateEntry(entry) => write!(formatter, "duplicate HD pack entry {entry:?}"),
        }
    }
}

impl std::error::Error for HdPackError {}

impl From<std::io::Error> for HdPackError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl HdPack {
    pub fn open(root: &Path, scale: HdScale) -> Result<Self, HdPackError> {
        let list_path = root.join(cmr_hdpack::LIST);
        let metadata = fs::symlink_metadata(&list_path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(HdPackError::InvalidEntry(cmr_hdpack::LIST.to_owned()));
        }
        if metadata.len() > MAX_LIST_BYTES {
            return Err(HdPackError::ListTooLarge(metadata.len()));
        }
        let list = fs::read_to_string(list_path)?;
        let mut lines = list.lines();
        if lines.next() != Some(cmr_hdpack::HEADER) {
            return Err(HdPackError::InvalidHeader);
        }
        let mut entries = BTreeMap::new();
        for entry in lines.filter(|line| !line.is_empty()) {
            let Some((set, file)) = entry.split_once('/') else {
                return Err(HdPackError::InvalidEntry(entry.to_owned()));
            };
            let Some((stem, extension)) = file.rsplit_once('.') else {
                return Err(HdPackError::InvalidEntry(entry.to_owned()));
            };
            let valid_stem = stem.len() == 16
                && stem
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if !cmr_hdpack::SETS.contains(&set)
                || !valid_stem
                || !matches!(extension, "png" | "webp")
                || file.contains(['/', '\\'])
            {
                return Err(HdPackError::InvalidEntry(entry.to_owned()));
            }
            if set == scale.directory()
                && entries.insert(stem.to_owned(), file.to_owned()).is_some()
            {
                return Err(HdPackError::DuplicateEntry(entry.to_owned()));
            }
        }
        Ok(Self {
            root: root.to_owned(),
            scale,
            entries,
            replacements: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Return a decoded BMP replacement. Missing, unreadable, oversized, or
    /// malformed entries deliberately fail open to the authenticated original.
    #[must_use]
    pub fn replacement(&self, original: &[u8]) -> Option<Vec<u8>> {
        if !original.starts_with(b"BM") {
            return None;
        }
        let name = String::from_utf8(cmr_hdpack::name(original).to_vec()).ok()?;
        let file = self.entries.get(&name)?;
        let path = self.root.join(self.scale.directory()).join(file);
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_ENTRY_BYTES
        {
            return None;
        }
        let compact = fs::read(path).ok()?;
        let bmp = cmr_hdpack::to_bmp(file, &compact).ok()?.1;
        self.replacements.fetch_add(1, Ordering::Relaxed);
        Some(bmp)
    }

    /// Number of replacement images decoded through this pack instance.
    #[must_use]
    pub fn replacement_count(&self) -> usize {
        self.replacements.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_selects_scale_and_missing_entries_fail_open() {
        let root = std::env::temp_dir().join(format!(
            "cmr-hd-pack-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("x2")).unwrap();
        fs::create_dir_all(root.join("x4")).unwrap();

        let original = cmr_hdpack::bmp24(1, 1, &[[1, 2, 3]]);
        let hash = String::from_utf8(cmr_hdpack::name(&original).to_vec()).unwrap();
        let file = format!("{hash}.png");
        let mut png_bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut png_bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[9, 8, 7, 6, 5, 4])
            .unwrap();
        fs::write(root.join("x2").join(&file), png_bytes).unwrap();
        fs::write(
            root.join(cmr_hdpack::LIST),
            format!("{}\nx2/{file}\n", cmr_hdpack::HEADER),
        )
        .unwrap();

        let x2 = HdPack::open(&root, HdScale::X2).unwrap();
        let replacement = x2.replacement(&original).unwrap();
        assert_eq!(&replacement[..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(replacement[18..22].try_into().unwrap()),
            2
        );
        assert_eq!(x2.replacement_count(), 1);
        assert_eq!(
            u16::from_le_bytes(replacement[28..30].try_into().unwrap()),
            8
        );
        assert!(x2.replacement(b"not a bitmap").is_none());
        assert!(
            HdPack::open(&root, HdScale::X4)
                .unwrap()
                .replacement(&original)
                .is_none()
        );
        let _ = fs::remove_dir_all(root);
    }
}
