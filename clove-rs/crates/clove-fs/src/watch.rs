//! Live directory watching.
//!
//! The browser surface must reflect the disk without polling it. On Windows this is
//! `ReadDirectoryChangesW` on a background thread; the UI drains [`DirWatcher::try_recv`]
//! once per frame and reloads the listing when something changed.
//!
//! Signals are intentionally coarse — "this directory changed", not a replay of every
//! event. The UI reloads the whole listing anyway (a `read_dir` of 5k entries is ~6 ms), and
//! coalescing here keeps a 10k-file extraction from queueing 10k reloads.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

#[cfg(windows)]
use crate::extended_path;

/// What the watcher saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchSignal {
    /// Something in the directory changed; reload the listing.
    Changed,
    /// The change buffer overflowed, so events were lost. Reload the listing.
    Overflow,
    /// The watcher stopped because of an error; reload and re-arm.
    Failed,
}

/// A live watch on one directory.
///
/// Dropping it stops the watcher thread (within ~100 ms; it polls a stop flag between
/// blocking waits rather than sharing a Win32 event with the UI thread).
pub struct DirWatcher {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<WatchSignal>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DirWatcher {
    /// Starts watching `dir` (non-recursive). Fails if the directory cannot be opened.
    ///
    /// Returns only once the first read is armed, so a change made immediately after this
    /// call cannot be missed.
    pub fn new(dir: impl AsRef<Path>) -> io::Result<Self> {
        let path = dir.as_ref().to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::channel();

        #[cfg(any(windows, unix))]
        let thread = {
            let handle = imp::open(&path)?;
            let stop = stop.clone();
            let (armed_tx, armed) = mpsc::sync_channel::<io::Result<()>>(1);
            let thread = std::thread::Builder::new()
                .name("clove-fs-watch".into())
                .spawn(move || imp::run(handle, stop, tx, armed_tx))?;
            match armed.recv() {
                Ok(Ok(())) => thread,
                Ok(Err(err)) => {
                    let _ = thread.join();
                    return Err(err);
                }
                Err(_) => {
                    let _ = thread.join();
                    return Err(io::Error::other("watcher thread stopped before arming"));
                }
            }
        };

        #[cfg(all(not(windows), not(unix)))]
        let thread = {
            // No portable watcher without a dependency; report once and stop.
            let _ = tx.send(WatchSignal::Failed);
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("clove-fs-watch".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                })?
        };

        Ok(DirWatcher {
            path,
            stop,
            events,
            thread: Some(thread),
        })
    }

    /// The directory being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The newest signal, if any. Coalesced: at most one per batch of changes.
    pub fn try_recv(&self) -> Option<WatchSignal> {
        let mut newest = None;
        while let Ok(signal) = self.events.try_recv() {
            newest = Some(signal);
        }
        newest
    }

    /// Blocks up to `timeout` for a signal. For tests and tools.
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<WatchSignal> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for DirWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_IO_PENDING, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
        FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

    /// A raw handle wrapper so it can move into the watcher thread.
    pub(super) struct Handles {
        dir: HANDLE,
        event: HANDLE,
    }

    // The handles are only used by the owning watcher thread.
    unsafe impl Send for Handles {}

    impl Drop for Handles {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.dir);
                CloseHandle(self.event);
            }
        }
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn open(dir: &Path) -> io::Result<Handles> {
        let path = extended_path(dir);
        let dir_handle = unsafe {
            CreateFileW(
                wide(&path).as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if dir_handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
        if event.is_null() {
            let err = io::Error::last_os_error();
            unsafe { CloseHandle(dir_handle) };
            return Err(err);
        }
        Ok(Handles {
            dir: dir_handle,
            event,
        })
    }

    pub(super) fn run(
        handles: Handles,
        stop: Arc<AtomicBool>,
        tx: mpsc::Sender<WatchSignal>,
        armed: mpsc::SyncSender<io::Result<()>>,
    ) {
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_ATTRIBUTES
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut first = true;

        while !stop.load(Ordering::Relaxed) {
            // Auto-reset events are reset by a successful wait, but a cancelled read leaves
            // the event signalled. Reset explicitly so the next wait is meaningful.
            unsafe { ResetEvent(handles.event) };

            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = handles.event;

            let issued = unsafe {
                ReadDirectoryChangesW(
                    handles.dir,
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    0, // not recursive: the browser only shows one level
                    filter,
                    std::ptr::null_mut(),
                    &mut overlapped,
                    None,
                )
            };
            if issued == 0 {
                let err = unsafe { GetLastError() };
                if err != ERROR_IO_PENDING {
                    let error = io::Error::from_raw_os_error(err as i32);
                    if first {
                        let _ = armed.send(Err(error));
                    } else {
                        let _ = tx.send(WatchSignal::Failed);
                    }
                    return;
                }
            }
            if first {
                first = false;
                if armed.send(Ok(())).is_err() {
                    return;
                }
            }

            // Poll rather than wait forever, so `stop` is honoured without sharing a Win32
            // event with the UI thread. 100 ms is imperceptible for a directory refresh.
            match unsafe { WaitForSingleObject(handles.event, 100) } {
                WAIT_OBJECT_0 => {
                    let mut bytes = 0u32;
                    let ok = unsafe { GetOverlappedResult(handles.dir, &overlapped, &mut bytes, 0) };
                    if ok == 0 {
                        let _ = tx.send(WatchSignal::Failed);
                        return;
                    }
                    // A zero-length result means the buffer overflowed and events were lost.
                    let signal = if bytes == 0 {
                        WatchSignal::Overflow
                    } else {
                        WatchSignal::Changed
                    };
                    if tx.send(signal).is_err() {
                        return;
                    }
                }
                WAIT_TIMEOUT => {
                    // Cancel the pending read and drain it before reusing the OVERLAPPED.
                    unsafe { CancelIoEx(handles.dir, &overlapped) };
                    let mut bytes = 0u32;
                    unsafe { GetOverlappedResult(handles.dir, &overlapped, &mut bytes, 1) };
                }
                _ => {
                    let _ = tx.send(WatchSignal::Failed);
                    return;
                }
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use rustix::fd::OwnedFd;
    use rustix::fs::inotify;

    pub(super) type Handles = OwnedFd;

    pub(super) fn open(dir: &Path) -> io::Result<Handles> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        inotify::add_watch(&fd, dir, inotify::WatchFlags::ALL_EVENTS)?;
        Ok(fd)
    }

    pub(super) fn run(
        handles: Handles,
        stop: Arc<AtomicBool>,
        tx: mpsc::Sender<WatchSignal>,
        armed: mpsc::SyncSender<io::Result<()>>,
    ) {
        if armed.send(Ok(())).is_err() {
            return;
        }
        let mut buf = vec![std::mem::MaybeUninit::uninit(); 64 * 1024];
        let mut reader = inotify::Reader::new(handles, &mut buf);
        while !stop.load(Ordering::Relaxed) {
            match reader.next() {
                Ok(event) => {
                    let signal = if event.events().contains(inotify::ReadFlags::QUEUE_OVERFLOW) {
                        WatchSignal::Overflow
                    } else {
                        WatchSignal::Changed
                    };
                    if tx.send(signal).is_err() {
                        return;
                    }
                    while !reader.is_buffer_empty() {
                        if let Ok(ev) = reader.next() {
                            let s = if ev.events().contains(inotify::ReadFlags::QUEUE_OVERFLOW) {
                                WatchSignal::Overflow
                            } else {
                                WatchSignal::Changed
                            };
                            let _ = tx.send(s);
                        } else {
                            break;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => {
                    let _ = tx.send(WatchSignal::Failed);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clove-fs-watch-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reports_a_new_file() {
        let dir = temp_dir("new-file");
        let watcher = DirWatcher::new(&dir).unwrap();
        assert_eq!(watcher.path(), dir.as_path());

        std::fs::write(dir.join("hello.txt"), b"hi").unwrap();
        let signal = watcher.recv_timeout(Duration::from_secs(5));
        assert_eq!(signal, Some(WatchSignal::Changed));

        drop(watcher);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_a_rename() {
        let dir = temp_dir("rename");
        std::fs::write(dir.join("before.txt"), b"x").unwrap();
        let watcher = DirWatcher::new(&dir).unwrap();

        std::fs::rename(dir.join("before.txt"), dir.join("after.txt")).unwrap();
        assert_eq!(
            watcher.recv_timeout(Duration::from_secs(5)),
            Some(WatchSignal::Changed)
        );

        drop(watcher);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dropping_stops_the_thread_promptly() {
        let dir = temp_dir("drop");
        let watcher = DirWatcher::new(&dir).unwrap();
        let started = std::time::Instant::now();
        drop(watcher);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "drop took {:?}",
            started.elapsed()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_directory_is_an_error() {
        let err = DirWatcher::new(std::env::temp_dir().join("clove-fs-watch-definitely-missing"));
        assert!(err.is_err());
    }
}
