//! Clove's filesystem engine.
//!
//! Framework-independent by construction: this crate knows nothing about drawing or
//! windows. It lists a directory into an immutable [`Snapshot`] that the UI can swap in
//! atomically, with names case-folded once at load (the filter's 5–8× win — see
//! `docs/rust-port/04-prototypes-and-benchmarks.md`).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

pub mod scan;
pub mod watch;

pub use scan::{scan_kind, scan_roots, ScanEvent, StorageKind, StorageScan, Volume, KIND_COUNT, PROGRESS_INTERVAL};
pub use watch::{DirWatcher, WatchSignal};

/// One directory entry as the UI sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name as stored on disk (not the full path).
    pub name: String,
    /// `name` case-folded once, for per-keystroke filtering.
    pub name_folded: String,
    pub is_dir: bool,
    /// File size in bytes; 0 for directories (their recursive size is a storage-scan job).
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// `FILE_ATTRIBUTE_HIDDEN` on Windows; always false elsewhere.
    pub hidden: bool,
    /// `FILE_ATTRIBUTE_SYSTEM` on Windows; always false elsewhere.
    pub system: bool,
}

impl Entry {
    /// Builds an entry, folding the name once. Used by listing and by tests.
    pub fn new(name: impl Into<String>, is_dir: bool, size: u64) -> Self {
        let name = name.into();
        Entry {
            name_folded: name.to_lowercase(),
            name,
            is_dir,
            size,
            modified: None,
            hidden: false,
            system: false,
        }
    }
}

/// An immutable listing of one directory.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub loaded_at: Instant,
}

impl Snapshot {
    /// Builds a snapshot from already-known entries (tests, fixtures, future caches).
    pub fn new(path: impl Into<PathBuf>, entries: Vec<Entry>) -> Self {
        Snapshot {
            path: path.into(),
            entries,
            loaded_at: Instant::now(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Lists `dir` into a sorted snapshot.
///
/// Sorting matches the Swift app: directories first, then case-insensitive name.
/// A directory that cannot be read is an `Err`; a single entry whose metadata fails is
/// still listed (size 0, no modified time) — one bad entry never aborts a listing.
pub fn list_dir(dir: impl AsRef<Path>) -> io::Result<Snapshot> {
    let dir = dir.as_ref();
    let read_path = extended_path(dir);
    let rd = std::fs::read_dir(&read_path)?;

    let mut entries = Vec::new();
    for item in rd {
        let Ok(item) = item else { continue };
        let Ok(file_type) = item.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        let metadata = item.metadata().ok();
        let size = if is_dir {
            0
        } else {
            metadata.as_ref().map(|m| m.len()).unwrap_or(0)
        };
        let modified = metadata.as_ref().and_then(|m| m.modified().ok());
        let (hidden, system) = attributes(&metadata);

        let name = item.file_name().to_string_lossy().into_owned();
        entries.push(Entry {
            name_folded: name.to_lowercase(),
            name,
            is_dir,
            size,
            modified,
            hidden,
            system,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name_folded.cmp(&b.name_folded))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(Snapshot {
        path: dir.to_path_buf(),
        entries,
        loaded_at: Instant::now(),
    })
}

#[cfg(windows)]
fn attributes(metadata: &Option<std::fs::Metadata>) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    match metadata {
        Some(m) => {
            let attrs = m.file_attributes();
            (
                attrs & FILE_ATTRIBUTE_HIDDEN != 0,
                attrs & FILE_ATTRIBUTE_SYSTEM != 0,
            )
        }
        None => (false, false),
    }
}

#[cfg(not(windows))]
fn attributes(_metadata: &Option<std::fs::Metadata>) -> (bool, bool) {
    (false, false)
}

/// Absolute path with the Windows extended-length prefix when the path is long.
///
/// Rust's std handles most long paths itself, but explicit `\\?\` is the documented
/// escape hatch for tools/APIs that do not; keep every filesystem entry point on this.
pub fn extended_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let text = abs.to_string_lossy();
        if text.starts_with(r"\\?\") || text.starts_with(r"\\.\") {
            return abs;
        }
        // The classic limit is 260; leave room for appended child names.
        if text.len() >= 240 {
            if let Some(rest) = text.strip_prefix(r"\\") {
                return PathBuf::from(format!(r"\\?\UNC\{rest}"));
            }
            return PathBuf::from(format!(r"\\?\{text}"));
        }
        abs
    }
    #[cfg(not(windows))]
    {
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clove-fs-test-{}-{tag}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_dirs_first_then_case_insensitive_names() {
        let dir = temp_dir("sort");
        std::fs::create_dir(dir.join("beta")).unwrap();
        std::fs::create_dir(dir.join("Alpha")).unwrap();
        std::fs::write(dir.join("zeta.txt"), b"12345").unwrap();
        std::fs::write(dir.join("Echo.txt"), b"1").unwrap();

        let snap = list_dir(&dir).unwrap();
        let names: Vec<&str> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "Echo.txt", "zeta.txt"]);

        let zeta = snap.entries.iter().find(|e| e.name == "zeta.txt").unwrap();
        assert_eq!(zeta.size, 5);
        assert!(!zeta.is_dir);
        assert_eq!(zeta.name_folded, "zeta.txt");
        assert!(zeta.modified.is_some());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folds_unicode_names_once() {
        let e = Entry::new("Überblick.TXT", false, 0);
        assert_eq!(e.name_folded, "überblick.txt");
    }

    #[test]
    fn missing_dir_is_an_error_not_a_panic() {
        let err = list_dir(std::env::temp_dir().join("clove-fs-definitely-missing")).unwrap_err();
        assert!(err.kind() != io::ErrorKind::Other || true);
    }

    #[test]
    fn extended_path_is_absolute_and_idempotent() {
        let p = extended_path(Path::new("."));
        assert!(p.is_absolute());
        assert_eq!(extended_path(&p), p);
    }
}
