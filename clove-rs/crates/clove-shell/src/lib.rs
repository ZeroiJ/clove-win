//! Delete and trash safety, ported from `Clove/Storage/StorageSafety.swift`.
//!
//! Three ideas carry over unchanged:
//!
//! 1. **Check at the moment of the action, not at scan time.** A path that changed since
//!    the scan must not be able to reach anything outside the folder it was found in, so
//!    every delete re-verifies that the target is a *direct child* of its root.
//! 2. **Links are removed as links.** Nothing here follows a symlink or junction; only the
//!    link itself moves.
//! 3. **A receipt, then put-back.** Each moved item records where it went, so undo is a
//!    plain move rather than a guess.
//!
//! On Windows the trash is the Recycle Bin, reached through `SHFileOperationW` with
//! `FOF_ALLOWUNDO`. The resulting `$R…` path is recovered by reading the matching `$I…`
//! record, which is what makes `Move::trashed` exact instead of hopeful.

use std::path::{Component, Path, PathBuf};

/// One thing moved to the trash, and where it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub original: PathBuf,
    /// Where it landed. `None` when the shell recycled it but the receipt could not be
    /// located — the item is still safely in the Recycle Bin and Explorer can restore it,
    /// but Clove cannot put it back itself.
    pub trashed: Option<PathBuf>,
}

/// What a trash operation did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrashOutcome {
    pub moves: Vec<Move>,
    /// Items that refused to go (guard rejected, or the shell failed).
    pub failures: usize,
}

impl TrashOutcome {
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty() && self.failures == 0
    }
}

/// What a put-back did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub restored: usize,
    /// Gone from the trash, or something new now occupies the original path.
    pub missing: usize,
}

/// `url` is exactly one level inside `root`, with links in its parent resolved.
///
/// Returns `false` when either path cannot be canonicalised, which is the safe answer: a
/// path we cannot verify is a path we do not delete.
pub fn is_direct_child(url: &Path, root: &Path) -> bool {
    let Some(name) = url.file_name() else {
        return false;
    };
    let text = name.to_string_lossy();
    if text.is_empty() || text == "." || text == ".." {
        return false;
    }
    // Reject any component that climbs, before touching the disk.
    if url
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return false;
    }
    let Some(parent) = url.parent() else {
        return false;
    };
    match (parent.canonicalize(), root.canonicalize()) {
        (Ok(parent), Ok(root)) => parent == root,
        _ => false,
    }
}

/// `url` is somewhere inside one of `roots`, and is not a root itself.
pub fn is_inside(url: &Path, roots: &[PathBuf]) -> bool {
    let Ok(url) = url.canonicalize() else {
        return false;
    };
    roots.iter().any(|root| {
        root.canonicalize()
            .map(|root| url.starts_with(&root) && url != root)
            .unwrap_or(false)
    })
}

/// Moves `url`, which must sit directly inside `root`, to the Recycle Bin.
///
/// With `emptying_folder`, a directory stays and its children go one by one — some apps
/// expect their cache folder to still exist. A link goes as a link; what it points at never
/// moves.
pub fn trash(url: &Path, root: &Path, emptying_folder: bool) -> TrashOutcome {
    if !is_direct_child(url, root) {
        return TrashOutcome {
            moves: Vec::new(),
            failures: 1,
        };
    }
    let Ok(meta) = std::fs::symlink_metadata(url) else {
        return TrashOutcome::default();
    };
    let is_link = meta.file_type().is_symlink();
    let is_dir = meta.is_dir();

    if emptying_folder && is_dir && !is_link {
        let mut outcome = TrashOutcome::default();
        let Ok(children) = std::fs::read_dir(url) else {
            outcome.failures += 1;
            return outcome;
        };
        for child in children.flatten() {
            match recycle_one(&child.path()) {
                Some(mv) => outcome.moves.push(mv),
                None => outcome.failures += 1,
            }
        }
        outcome
    } else {
        match recycle_one(url) {
            Some(mv) => TrashOutcome {
                moves: vec![mv],
                failures: 0,
            },
            None => TrashOutcome {
                moves: Vec::new(),
                failures: 1,
            },
        }
    }
}

/// Deletes `url` permanently, with the same direct-child guard.
///
/// This is for caches and build output, where a Recycle Bin entry would be noise. It is
/// never used for anything the user did not explicitly nominate as disposable.
pub fn remove_permanently(url: &Path, root: &Path, emptying_folder: bool) -> usize {
    if !is_direct_child(url, root) {
        return 1;
    }
    let Ok(meta) = std::fs::symlink_metadata(url) else {
        return 0;
    };
    let is_link = meta.file_type().is_symlink();
    if emptying_folder && meta.is_dir() && !is_link {
        let Ok(children) = std::fs::read_dir(url) else {
            return 1;
        };
        let mut failures = 0;
        for child in children.flatten() {
            let path = child.path();
            let ok = if std::fs::symlink_metadata(&path)
                .map(|m| m.is_dir() && !m.file_type().is_symlink())
                .unwrap_or(false)
            {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if ok.is_err() {
                failures += 1;
            }
        }
        return failures;
    }
    let is_link = meta.file_type().is_symlink();
    let is_dir = meta.is_dir();
    let ok = if is_dir && !is_link {
        std::fs::remove_dir_all(url)
    } else {
        std::fs::remove_file(url)
    };
    if ok.is_ok() {
        0
    } else {
        1
    }
}

/// Puts moved things back where they were.
///
/// Anything already gone from the trash, or with something new in its old place, is left
/// alone and counted — never overwritten.
pub fn put_back(moves: &[Move]) -> RestoreOutcome {
    let mut outcome = RestoreOutcome::default();
    for mv in moves {
        let Some(trashed) = mv.trashed.as_ref() else {
            outcome.missing += 1;
            continue;
        };
        if !trashed.exists() || mv.original.exists() {
            outcome.missing += 1;
            continue;
        }
        if let Some(parent) = mv.original.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let restored_ok = if std::fs::rename(trashed, &mv.original).is_ok() {
            true
        } else {
            // Cross-filesystem restore: copy, then remove the trashed payload.
            let copy_ok = if trashed.is_dir() && !trashed.is_symlink() {
                copy_dir_recursive(trashed, &mv.original).is_ok()
            } else if trashed.is_symlink() {
                #[cfg(unix)]
                {
                    std::fs::read_link(trashed)
                        .ok()
                        .and_then(|target| std::os::unix::fs::symlink(&target, &mv.original).ok())
                        .is_some()
                }
                #[cfg(not(unix))]
                {
                    false
                }
            } else {
                std::fs::copy(trashed, &mv.original).is_ok()
            };
            if copy_ok {
                let _ = if trashed.is_dir() && !trashed.is_symlink() {
                    std::fs::remove_dir_all(trashed)
                } else {
                    std::fs::remove_file(trashed)
                };
                true
            } else {
                false
            }
        };
        if restored_ok {
            outcome.restored += 1;
        } else {
            outcome.missing += 1;
        }
    }
    outcome
}

// ---------------------------------------------------------------- recycle bin

/// Recycles one path and recovers where it landed.
fn recycle_one(path: &Path) -> Option<Move> {
    let absolute = std::path::absolute(path).ok()?;

    #[cfg(windows)]
    {
        let since = std::time::SystemTime::now();
        shell_recycle(&absolute)?;
        let trashed = find_trashed(&absolute, since);
        Some(Move {
            original: absolute,
            trashed,
        })
    }

    #[cfg(target_os = "linux")]
    {
        match shell_recycle(&absolute) {
            Some(trashed) => Some(Move {
                original: absolute,
                trashed: Some(trashed),
            }),
            None => None,
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = absolute;
        None
    }
}

#[cfg(windows)]
fn shell_recycle(path: &Path) -> Option<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI,
        FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW,
    };

    // `pFrom` is a list of NUL-terminated paths with an extra NUL at the end.
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.push(0);
    from.push(0);

    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    op.fFlags = (FOF_ALLOWUNDO
        | FOF_SILENT
        | FOF_NOCONFIRMATION
        | FOF_NOCONFIRMMKDIR
        | FOF_NOERRORUI) as u16;

    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted != 0 {
        return None;
    }
    Some(())
}

#[cfg(target_os = "linux")]
fn shell_recycle(path: &Path) -> Option<PathBuf> {
    use std::fs;

    let root = trash_root()?;
    let files_dir = root.join("files");
    let info_dir = root.join("info");
    fs::create_dir_all(&files_dir).ok()?;
    fs::create_dir_all(&info_dir).ok()?;

    let name = path.file_name()?.to_string_lossy().into_owned();
    let unique = unique_trash_name(&files_dir, &info_dir, &name);
    let payload = files_dir.join(&unique);
    let info_path = info_dir.join(format!("{}.trashinfo", unique));

    // Prefer an atomic rename; fall back to copy+delete across filesystems.
    if fs::rename(path, &payload).is_err() {
        if path.is_dir() && !path.is_symlink() {
            copy_dir_recursive(path, &payload).ok()?;
            fs::remove_dir_all(path).ok()?;
        } else {
            fs::copy(path, &payload).ok()?;
            fs::remove_file(path).ok()?;
        }
    }

    // If the move failed, the payload will not exist.
    if !payload.exists() {
        let _ = fs::remove_file(&payload);
        let _ = fs::remove_file(&info_path);
        return None;
    }

    let content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path.display(),
        local_now()
    );
    if fs::write(&info_path, content).is_err() {
        // Best effort: leave the file trashed rather than losing it.
    }

    Some(payload)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn shell_recycle(_path: &Path) -> Option<()> {
    None
}

fn unique_trash_name(files_dir: &Path, info_dir: &Path, name: &str) -> String {
    let payload = files_dir.join(name);
    let info = info_dir.join(format!("{}.trashinfo", name));
    if !payload.exists() && !info.exists() {
        return name.to_string();
    }
    for i in 1.. {
        let candidate = format!("{}.{}", name, i);
        let payload = files_dir.join(&candidate);
        let info = info_dir.join(format!("{}.trashinfo", candidate));
        if !payload.exists() && !info.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::fs;
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() && !ft.is_symlink() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ft.is_symlink() {
            let target = std::fs::read_link(&src_path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn trash_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    Some(base.join("Trash"))
}

fn local_now() -> String {
    use libc::time_t;
    unsafe {
        let mut t: time_t = 0;
        libc::time(&mut t);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let fmt = b"%Y-%m-%dT%H:%M:%S\0";
        libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr().cast(), &tm);
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..len]).to_string()
    }
}

/// The user's Recycle Bin, one directory per SID, on the volume that holds `path`.
///
/// Every user on the machine gets a subdirectory; we scan all of them rather than resolve
/// the current SID, and skip the ones we cannot read.
#[cfg(windows)]
fn recycle_roots(near: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(volume) = near.components().find_map(|c| match c {
        Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
        _ => None,
    }) else {
        return roots;
    };
    // `C:` -> `C:\$Recycle.Bin`
    let base = PathBuf::from(format!("{volume}\\$Recycle.Bin"));
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                roots.push(entry.path());
            }
        }
    }
    roots
}

/// Finds the `$R…` payload for `original`, among `$I…` records written at or after `since`.
#[cfg(windows)]
fn find_trashed(original: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    let target = original.to_string_lossy().to_lowercase();
    let since = since
        .checked_sub(std::time::Duration::from_secs(2))
        .unwrap_or(since);

    // The shell writes the $I record synchronously, but allow a moment on slow volumes.
    for attempt in 0..6 {
        let mut best: Option<(u64, PathBuf)> = None;
        for root in recycle_roots(original) {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let Some(id) = name.strip_prefix("$I") else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    continue;
                };
                let Some(record) = parse_recycle_record(&bytes) else {
                    continue;
                };
                if record.original.to_lowercase() != target || record.deleted < since {
                    continue;
                }
                let payload = root.join(format!("$R{id}"));
                if !payload.exists() {
                    continue;
                }
                let stamp = record.deleted_filetime;
                if best.as_ref().map(|(t, _)| stamp > *t).unwrap_or(true) {
                    best = Some((stamp, payload));
                }
            }
        }
        if let Some((_, payload)) = best {
            return Some(payload);
        }
        if attempt < 5 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    None
}

#[cfg(not(windows))]
fn find_trashed(_original: &Path, _since: std::time::SystemTime) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
struct RecycleRecord {
    original: String,
    deleted: std::time::SystemTime,
    deleted_filetime: u64,
}

/// Parses a `$I…` record: version, size, deletion time, then the original path.
///
/// Version 1 stores a fixed 260-wchar NUL-terminated path; version 2 stores a char count
/// followed by that many wchars.
#[cfg(windows)]
fn parse_recycle_record(bytes: &[u8]) -> Option<RecycleRecord> {
    if bytes.len() < 24 {
        return None;
    }
    let version = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
    let deleted_filetime = u64::from_le_bytes(bytes[16..24].try_into().ok()?);
    let deleted = filetime_to_system_time(deleted_filetime)?;

    let original = match version {
        1 => {
            let end = (24 + 520).min(bytes.len());
            let units: Vec<u16> = bytes[24..end]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }
        2 => {
            let count = u32::from_le_bytes(bytes[24..28].try_into().ok()?) as usize;
            let start: usize = 28;
            let end = start.checked_add(count.checked_mul(2)?)?;
            if end > bytes.len() {
                return None;
            }
            let units: Vec<u16> = bytes[start..end]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                // `count` includes the NUL terminator, so stop at it.
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => return None,
    };
    if original.is_empty() {
        return None;
    }
    Some(RecycleRecord {
        original,
        deleted,
        deleted_filetime,
    })
}

/// Windows FILETIME (100 ns ticks since 1601-01-01 UTC) to `SystemTime`.
#[cfg(windows)]
fn filetime_to_system_time(filetime: u64) -> Option<std::time::SystemTime> {
    const TICKS_PER_SEC: u64 = 10_000_000;
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600; // 1601 -> 1970
    let secs = filetime / TICKS_PER_SEC;
    let nanos = (filetime % TICKS_PER_SEC) as u32 * 100;
    if secs < EPOCH_DIFF_SECS {
        return None;
    }
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::new(secs - EPOCH_DIFF_SECS, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clove-shell-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn direct_child_accepts_one_level_and_rejects_the_rest() {
        let root = temp_dir("child");
        let child = root.join("a.txt");
        std::fs::write(&child, b"x").unwrap();
        let grandchild = root.join("sub").join("b.txt");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(&grandchild, b"x").unwrap();

        assert!(is_direct_child(&child, &root));
        assert!(!is_direct_child(&grandchild, &root), "two levels down");
        assert!(!is_direct_child(&root, &root), "the root itself");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn direct_child_rejects_dot_dot_traversal() {
        let root = temp_dir("traversal");
        let escape = root.join("..").join("somewhere.txt");
        assert!(!is_direct_child(&escape, &root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn direct_child_is_false_for_a_missing_root() {
        let root = std::env::temp_dir().join("clove-shell-missing-root");
        let child = root.join("x");
        assert!(!is_direct_child(&child, &root));
    }

    #[test]
    fn is_inside_uses_components_not_string_prefixes() {
        let root = temp_dir("inside");
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a").join("f"), b"x").unwrap();
        let roots = vec![root.clone()];

        assert!(is_inside(&root.join("a").join("f"), &roots));
        assert!(!is_inside(&root, &roots), "a root is not inside itself");

        // A sibling whose name merely starts with the root's name must not match.
        let sibling = root.with_file_name(format!(
            "{}-sibling",
            root.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("f"), b"x").unwrap();
        assert!(!is_inside(&sibling.join("f"), &roots));

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&sibling).unwrap();
    }

    #[test]
    fn trash_refuses_anything_not_a_direct_child() {
        let root = temp_dir("trash-guard");
        let nested = root.join("sub").join("f");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(&nested, b"x").unwrap();

        let outcome = trash(&nested, &root, false);
        assert_eq!(outcome.failures, 1);
        assert!(outcome.moves.is_empty());
        assert!(nested.exists(), "a refused path must not be touched");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remove_permanently_respects_the_guard() {
        let root = temp_dir("remove-guard");
        let nested = root.join("sub").join("f");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(&nested, b"x").unwrap();

        assert_eq!(remove_permanently(&nested, &root, false), 1);
        assert!(nested.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn remove_permanently_empties_a_folder_but_keeps_it() {
        let root = temp_dir("empty-folder");
        let folder = root.join("cache");
        std::fs::create_dir_all(folder.join("nested")).unwrap();
        std::fs::write(folder.join("a.bin"), b"1").unwrap();
        std::fs::write(folder.join("nested").join("b.bin"), b"2").unwrap();

        assert_eq!(remove_permanently(&folder, &root, true), 0);
        assert!(folder.exists(), "the folder itself stays");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn put_back_restores_and_refuses_to_overwrite() {
        let dir = temp_dir("put-back");
        let trash_dir = dir.join("trash");
        let live = dir.join("live");
        std::fs::create_dir_all(&trash_dir).unwrap();
        std::fs::create_dir_all(&live).unwrap();

        let a_original = live.join("a.txt");
        let a_trashed = trash_dir.join("a.txt");
        std::fs::write(&a_trashed, b"contents of a").unwrap();

        let b_original = live.join("b.txt");
        let b_trashed = trash_dir.join("b.txt");
        std::fs::write(&b_trashed, b"contents of b").unwrap();
        std::fs::write(&b_original, b"something new").unwrap();

        let moves = vec![
            Move {
                original: a_original.clone(),
                trashed: Some(a_trashed.clone()),
            },
            Move {
                original: b_original.clone(),
                trashed: Some(b_trashed.clone()),
            },
            Move {
                original: live.join("c.txt"),
                trashed: None,
            },
        ];
        let outcome = put_back(&moves);

        assert_eq!(outcome.restored, 1);
        assert_eq!(outcome.missing, 2);
        assert_eq!(std::fs::read(&a_original).unwrap(), b"contents of a");
        assert_eq!(
            std::fs::read(&b_original).unwrap(),
            b"something new",
            "an occupied original is never overwritten"
        );
        assert!(b_trashed.exists(), "the un-restored copy stays in the trash");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn put_back_recreates_a_missing_parent() {
        let dir = temp_dir("put-back-parent");
        let trashed = dir.join("x.txt");
        std::fs::write(&trashed, b"x").unwrap();
        let original = dir.join("gone").join("deeper").join("x.txt");

        let outcome = put_back(&[Move {
            original: original.clone(),
            trashed: Some(trashed),
        }]);
        assert_eq!(outcome.restored, 1);
        assert!(original.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
