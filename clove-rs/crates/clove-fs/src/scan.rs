//! Storage measurement by kind — the Windows reading of Clove's `StorageKind` and
//! `StorageScanner` (`Clove/Storage/StorageKind.swift`, `StorageScanner.swift`).
//!
//! Two deliberate differences from the macOS scanner:
//!
//! - **Logical size, not allocated size.** Swift adds `totalFileAllocatedSize`; Windows
//!   would need a `GetCompressedFileSizeW` call per file, doubling the walk. The bar shows
//!   a proportion, and Explorer's Size column is logical too, so the cheaper number is the
//!   one to show. Sparse and compressed files therefore read a little larger here.
//! - **`System` is derived, never walked.** Like Swift, it is the volume's used bytes minus
//!   everything the other kinds counted, so `C:\Windows` never has to be enumerated.
//!
//! The walk is breadth-first over an explicit stack, skips every reparse point (junctions
//! included, so `C:\Users\All Users` cannot loop), ignores unreadable subdirectories, and
//! checks a cancellation flag every 1,024 entries. Three kinds run at once, as on macOS.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::extended_path;

/// Kinds on the bar, in bar order (`StorageKind`'s enum order).
pub const KIND_COUNT: usize = 9;

/// How often a running scan may report partial totals. Matches the macOS scanner's 0.09 s
/// budget closely enough that the UI sees the same counting-up cadence.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Kinds scanned at once, matching `StorageStore.scan`'s three-folder group.
const WORKERS: usize = 3;

/// One part of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum StorageKind {
    Apps = 0,
    Code,
    Photos,
    Movies,
    Documents,
    Music,
    System,
    Caches,
    Trash,
}

impl StorageKind {
    pub const ALL: [StorageKind; KIND_COUNT] = [
        StorageKind::Apps,
        StorageKind::Code,
        StorageKind::Photos,
        StorageKind::Movies,
        StorageKind::Documents,
        StorageKind::Music,
        StorageKind::System,
        StorageKind::Caches,
        StorageKind::Trash,
    ];

    /// Everything a scan walks. `System` is what the volume has left over.
    pub const SCANNED: [StorageKind; KIND_COUNT - 1] = [
        StorageKind::Apps,
        StorageKind::Code,
        StorageKind::Photos,
        StorageKind::Movies,
        StorageKind::Documents,
        StorageKind::Music,
        StorageKind::Caches,
        StorageKind::Trash,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The legend's label. `Code` is the Windows spelling of macOS's `Developer`.
    pub fn title(self) -> &'static str {
        match self {
            StorageKind::Apps => "Apps",
            StorageKind::Code => "Code",
            StorageKind::Photos => "Photos",
            StorageKind::Movies => "Movies",
            StorageKind::Documents => "Documents",
            StorageKind::Music => "Music",
            StorageKind::System => "System",
            StorageKind::Caches => "Caches",
            StorageKind::Trash => "Trash",
        }
    }

    /// Where this kind lives on Windows. Missing folders are simply skipped.
    #[cfg(windows)]
    pub fn roots(self) -> Vec<PathBuf> {
        let Some(home) = home_dir() else {
            return Vec::new();
        };
        let mut roots = Vec::new();
        match self {
            StorageKind::Apps => {
                for name in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(path) = env_path(name) {
                        roots.push(path);
                    }
                }
                roots.push(home.join("AppData").join("Local").join("Programs"));
            }
            StorageKind::Code => {
                for name in ["source", "repos", "Projects", "dev", "code", "GitHub"] {
                    roots.push(home.join(name));
                }
                for name in [
                    ".cargo",
                    ".rustup",
                    ".gradle",
                    ".m2",
                    ".npm",
                    ".nuget",
                    "go",
                    ".bun",
                    ".vscode",
                    ".claude",
                    ".cursor",
                    ".codex",
                    ".ollama",
                ] {
                    roots.push(home.join(name));
                }
            }
            StorageKind::Photos => roots.push(home.join("Pictures")),
            StorageKind::Movies => roots.push(home.join("Videos")),
            StorageKind::Documents => {
                roots.push(home.join("Documents"));
                roots.push(home.join("Desktop"));
                roots.push(home.join("Downloads"));
            }
            StorageKind::Music => roots.push(home.join("Music")),
            StorageKind::System => {}
            StorageKind::Caches => {
                if let Some(local) = env_path("LOCALAPPDATA") {
                    roots.push(local.join("Temp"));
                    roots.push(local.join("Microsoft").join("Windows").join("INetCache"));
                    roots.push(local.join("D3DSCache"));
                    roots.push(local.join("CrashDumps"));
                }
            }
            StorageKind::Trash => {
                let drive = env_path("SystemDrive").unwrap_or_else(|| PathBuf::from("C:"));
                roots.push(PathBuf::from(format!(
                    "{}\\$Recycle.Bin",
                    drive.display().to_string().trim_end_matches('\\')
                )));
            }
        }
        roots
    }
}

impl StorageKind {
    /// Where this kind lives on Arch/Linux. Missing folders are simply skipped.
    #[cfg(target_os = "linux")]
    pub fn roots(self) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let xdg_data = env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"));
    let xdg_cache = env_path("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
    let xdg_pictures = env_path("XDG_PICTURES_DIR").unwrap_or_else(|| home.join("Pictures"));
    let xdg_videos = env_path("XDG_VIDEOS_DIR").unwrap_or_else(|| home.join("Videos"));
    let xdg_documents = env_path("XDG_DOCUMENTS_DIR").unwrap_or_else(|| home.join("Documents"));
    let xdg_music = env_path("XDG_MUSIC_DIR").unwrap_or_else(|| home.join("Music"));
    let xdg_download = env_path("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads"));
    let xdg_desktop = env_path("XDG_DESKTOP_DIR").unwrap_or_else(|| home.join("Desktop"));

    let mut roots = Vec::new();
    match self {
        StorageKind::Apps => {
            roots.push(PathBuf::from("/opt"));
            roots.push(PathBuf::from("/var/lib/flatpak"));
            roots.push(PathBuf::from("/var/lib/snapd"));
            roots.push(home.join(".local").join("share").join("Steam"));
            roots.push(home.join("Applications"));
        }
        StorageKind::Code => {
            for name in [
                "Projects", "projects", "dev", "repos", "src", "code", "GitHub", "workspace",
            ] {
                roots.push(home.join(name));
            }
            for name in [
                ".cargo", ".rustup", ".gradle", ".m2", ".npm", "go", ".local", ".bun", ".vscode",
                ".claude", ".cursor", ".codex", ".ollama", ".config", ".cache",
            ] {
                roots.push(home.join(name));
            }
        }
        StorageKind::Photos => roots.push(xdg_pictures),
        StorageKind::Movies => roots.push(xdg_videos),
        StorageKind::Documents => {
            roots.push(xdg_documents);
            roots.push(xdg_desktop);
            roots.push(xdg_download);
        }
        StorageKind::Music => roots.push(xdg_music),
        StorageKind::System => {}
        StorageKind::Caches => {
            roots.push(xdg_cache);
            roots.push(PathBuf::from("/var/cache/pacman/pkg"));
        }
        StorageKind::Trash => {
            roots.push(xdg_data.join("Trash").join("files"));
        }
    }
    roots
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn roots(self) -> Vec<PathBuf> {
    Vec::new()
}
}

fn home_dir() -> Option<PathBuf> {
    env_path("USERPROFILE").or_else(|| env_path("HOME"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The disk a path lives on, the way Explorer counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub capacity: u64,
    /// Bytes free to the current user (quota-aware).
    pub free: u64,
}

impl Volume {
    pub fn used(self) -> u64 {
        self.capacity.saturating_sub(self.free)
    }
}

/// Capacity and free space for the volume holding `path`.
#[cfg(windows)]
pub fn volume_for(path: impl AsRef<Path>) -> Option<Volume> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    // The API takes a directory; a file's own volume is its parent's.
    let mut probe = path.as_ref().to_path_buf();
    if !probe.is_dir() {
        probe = probe.parent()?.to_path_buf();
    }
    let wide: Vec<u16> = probe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_to_caller = 0u64;
    let mut capacity = 0u64;
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            &mut capacity,
            &mut free,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(Volume {
        capacity,
        free: free_to_caller.min(free),
    })
}

#[cfg(target_os = "linux")]
pub fn volume_for(path: impl AsRef<Path>) -> Option<Volume> {
    let mut probe = path.as_ref().to_path_buf();
    if !probe.is_dir() {
        probe = probe.parent()?.to_path_buf();
    }
    let stat = rustix::fs::statvfs(&probe).ok()?;
    Some(Volume {
        capacity: stat.f_blocks.saturating_mul(stat.f_frsize),
        free: stat.f_bavail.saturating_mul(stat.f_frsize),
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn volume_for(_path: impl AsRef<Path>) -> Option<Volume> {
    None
}

/// A partial total from a running scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindProgress {
    pub kind: StorageKind,
    pub bytes: u64,
    pub files: u64,
    /// The entry being counted, for a ticker under the bar.
    pub current: PathBuf,
}

/// What one kind measured to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindReport {
    pub kind: StorageKind,
    pub bytes: u64,
    pub files: u64,
    /// A root existed but could not be read (the Trash without permission, typically).
    pub restricted: bool,
    pub elapsed: Duration,
}

/// Walks `roots` for `kind`, adding up file sizes.
///
/// `progress` is called at most every [`PROGRESS_INTERVAL`] while the walk is big enough to
/// matter. Setting `cancel` stops at the next directory or 1,024-entry boundary; the
/// returned report is then partial.
pub fn scan_roots(
    kind: StorageKind,
    roots: &[PathBuf],
    cancel: &AtomicBool,
    seen: Option<&std::sync::Mutex<std::collections::HashSet<(u64, u64)>>>,
    mut progress: impl FnMut(KindProgress),
) -> KindReport {
    let started = Instant::now();
    let mut report = KindReport {
        kind,
        bytes: 0,
        files: 0,
        restricted: false,
        elapsed: Duration::ZERO,
    };

    let mut stack: Vec<(PathBuf, bool)> = roots.iter().cloned().map(|root| (root, true)).collect();
    let mut checked = 0u64;
    let mut last_report = Instant::now();

    while let Some((dir, is_root)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let read_path = extended_path(&dir);
        let entries = match std::fs::read_dir(&read_path) {
            Ok(entries) => entries,
            Err(err) => {
                if is_root && err.kind() == io::ErrorKind::PermissionDenied {
                    report.restricted = true;
                }
                continue;
            }
        };

        for item in entries {
            let Ok(item) = item else { continue };
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            // Junctions are symlinks to Rust, and following one can loop or double-count.
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push((item.path(), false));
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            // The Recycle Bin's `$I…` siblings describe the `$R…` payload; counting both
            // would count the metadata and the content.
            if kind == StorageKind::Trash {
                let name = item.file_name();
                let name = name.to_string_lossy();
                if name.len() >= 2 && name[..2].eq_ignore_ascii_case("$I") {
                    continue;
                }
            }

            let metadata = item.metadata().ok();
            #[cfg(target_os = "linux")]
            let size = metadata
                .as_ref()
                .map(|m| {
                    use std::os::unix::fs::MetadataExt;
                    let blocks = m.blocks();
                    if blocks > 0 {
                        blocks * 512
                    } else {
                        m.len()
                    }
                })
                .unwrap_or(0);
            #[cfg(not(target_os = "linux"))]
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            if size == 0 {
                continue;
            }

            #[cfg(target_os = "linux")]
            if let Some(seen) = seen {
                if let Some(meta) = metadata.as_ref() {
                    use std::os::unix::fs::MetadataExt;
                    let key = (meta.dev(), meta.ino());
                    let mut guard = seen.lock().unwrap();
                    if !guard.insert(key) {
                        continue;
                    }
                }
            }
            report.bytes += size;
            report.files += 1;

            checked += 1;
            if checked & 0x3FF == 0 {
                if cancel.load(Ordering::Relaxed) {
                    report.elapsed = started.elapsed();
                    return report;
                }
                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    last_report = Instant::now();
                    progress(KindProgress {
                        kind,
                        bytes: report.bytes,
                        files: report.files,
                        current: item.path(),
                    });
                }
            }
        }
    }

    report.elapsed = started.elapsed();
    report
}

/// Convenience wrapper: scan a kind's real Windows roots.
pub fn scan_kind(
    kind: StorageKind,
    cancel: &AtomicBool,
    progress: impl FnMut(KindProgress),
) -> KindReport {
    scan_roots(kind, &kind.roots(), cancel, None, progress)
}

/// One event from a running [`StorageScan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    Progress(KindProgress),
    Done(KindReport),
    /// Every kind has reported. Sent after the last [`ScanEvent::Done`].
    Finished,
}

/// A three-worker scan of every kind but `System`.
///
/// The UI drains [`StorageScan::try_recv`] once per frame; tools can block on
/// [`StorageScan::wait`]. Dropping the scan cancels it without joining, so the UI thread
/// never waits on a directory read.
pub struct StorageScan {
    events: Receiver<ScanEvent>,
    cancel: Arc<AtomicBool>,
    volume: Option<Volume>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl StorageScan {
    /// Starts scanning the real Windows roots around the user's home folder.
    pub fn start() -> Self {
        let jobs = StorageKind::SCANNED
            .iter()
            .map(|&kind| (kind, kind.roots()))
            .collect();
        let volume_path = home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
        Self::start_with(jobs, volume_path)
    }

    /// Starts scanning explicit `(kind, roots)` jobs. Tests and tools use this to measure a
    /// sandbox instead of the user's disk.
    pub fn start_with(jobs: Vec<(StorageKind, Vec<PathBuf>)>, volume_path: PathBuf) -> Self {
        let volume = volume_for(&volume_path);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::channel();
        let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
        let seen = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let worker_count = WORKERS.min(queue.lock().map(|q| q.len()).unwrap_or(0)).max(1);
        let mut workers = Vec::with_capacity(worker_count);

        for index in 0..worker_count {
            let tx = tx.clone();
            let queue = queue.clone();
            let cancel = cancel.clone();
            let seen = seen.clone();
            let handle = std::thread::Builder::new()
                .name(format!("clove-storage-{index}"))
                .spawn(move || {
                    loop {
                        let next = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                        let Some((kind, roots)) = next else { break };
                        let report = scan_roots(kind, &roots, &cancel, Some(&seen), |progress| {
                            let _ = tx.send(ScanEvent::Progress(progress));
                        });
                        let _ = tx.send(ScanEvent::Done(report));
                    }
                })
                .expect("could not spawn a storage worker");
            workers.push(handle);
        }

        let reporter_cancel = cancel.clone();
        let reporter = std::thread::Builder::new()
            .name("clove-storage-done".into())
            .spawn(move || {
                for worker in workers {
                    let _ = worker.join();
                }
                if !reporter_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(ScanEvent::Finished);
                }
            })
            .expect("could not spawn the storage reporter");
        let workers = vec![reporter];

        StorageScan {
            events,
            cancel,
            volume,
            workers,
        }
    }

    pub fn volume(&self) -> Option<Volume> {
        self.volume
    }

    /// The next event, if one is ready. Never blocks.
    pub fn try_recv(&self) -> Option<ScanEvent> {
        self.events.try_recv().ok()
    }

    /// Blocks up to `timeout` for an event. For tools and tests.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<ScanEvent> {
        self.events.recv_timeout(timeout).ok()
    }

    /// Blocks until every kind has reported and returns the reports in bar order.
    pub fn wait(mut self) -> Vec<KindReport> {
        let mut reports = Vec::with_capacity(StorageKind::SCANNED.len());
        while let Ok(event) = self.events.recv() {
            match event {
                ScanEvent::Done(report) => reports.push(report),
                ScanEvent::Finished => break,
                ScanEvent::Progress(_) => {}
            }
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        reports.sort_by_key(|report| report.kind.index());
        reports
    }
}

impl Drop for StorageScan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // No join: this can run on the UI thread while a worker is inside a slow read.
        // Workers see the flag at their next boundary and exit on their own.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "clove-fs-scan-{}-{tag}-{stamp}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn not_cancelled() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn adds_up_nested_files() {
        let dir = temp_dir("nested");
        std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
        std::fs::write(dir.join("one.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("a").join("two.bin"), vec![0u8; 200]).unwrap();
        std::fs::write(dir.join("a").join("b").join("three.bin"), vec![0u8; 300]).unwrap();

        let report = scan_roots(StorageKind::Documents, &[dir.clone()], &not_cancelled(), None, |_| {});
        assert!(report.bytes >= 600);
        assert_eq!(report.files, 3);
        assert!(!report.restricted);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_root_is_skipped_not_restricted() {
        let dir = temp_dir("missing").join("not-there");
        let report = scan_roots(StorageKind::Documents, &[dir], &not_cancelled(), None, |_| {});
        assert_eq!(report.bytes, 0);
        assert!(!report.restricted);
    }

    #[test]
    fn cancellation_stops_before_counting() {
        let dir = temp_dir("cancel");
        std::fs::write(dir.join("file.bin"), vec![0u8; 64]).unwrap();
        let cancel = AtomicBool::new(true);

        let report = scan_roots(StorageKind::Documents, &[dir.clone()], &cancel, None, |_| {});
        assert_eq!(report.bytes, 0);
        assert_eq!(report.files, 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_skips_receipt_files() {
        let dir = temp_dir("trash");
        std::fs::write(dir.join("$IABC123.txt"), vec![0u8; 120]).unwrap();
        std::fs::write(dir.join("$RABC123.txt"), vec![0u8; 700]).unwrap();

        let report = scan_roots(StorageKind::Trash, &[dir.clone()], &not_cancelled(), None, |_| {});
        assert!(report.bytes >= 700);
        assert_eq!(report.files, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn system_has_no_roots_and_never_walks() {
        assert!(StorageKind::System.roots().is_empty());
    }

    #[test]
    fn reports_are_in_bar_order() {
        let dir = temp_dir("driver");
        std::fs::create_dir_all(dir.join("music")).unwrap();
        std::fs::create_dir_all(dir.join("documents")).unwrap();
        std::fs::write(dir.join("music").join("song.bin"), vec![0u8; 111]).unwrap();
        std::fs::write(dir.join("documents").join("note.bin"), vec![0u8; 222]).unwrap();

        let scan = StorageScan::start_with(
            vec![
                (StorageKind::Music, vec![dir.join("music")]),
                (StorageKind::Documents, vec![dir.join("documents")]),
            ],
            dir.clone(),
        );
        let reports = scan.wait();

        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].kind, StorageKind::Documents);
        assert!(reports[0].bytes >= 222);
        assert_eq!(reports[1].kind, StorageKind::Music);
        assert!(reports[1].bytes >= 111);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn volume_has_room() {
        let dir = temp_dir("volume");
        let volume = volume_for(&dir).expect("a temp dir is on a volume");
        assert!(volume.capacity > 0);
        assert!(volume.free <= volume.capacity);
        assert!(volume.used() <= volume.capacity);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
