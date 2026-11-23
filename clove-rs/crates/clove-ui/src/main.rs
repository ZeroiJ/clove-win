//! Clove for Windows — the Rune app.
//!
//! Phase B skeleton: the browser surface and the storage bar, running on the real engine
//! crates (`clove-fs`, `clove-search`, `clove-render`, `clove-shell`) rather than the
//! prototype's inlined copies.
//!
//! Threading contract, per Rune's own guidance and the measured numbers in
//! `docs/rust-port/07-phase-a-status.md`:
//!
//! - directory reads, trashing, and put-back run on worker threads and land through
//!   `AppHandle::update`; nothing touches the disk inside a frame or a key handler
//! - filtering runs inline, because 4.7k entries filter in 0.29 ms; `clove_search::FilterWorker`
//!   exists for the pathological-directory case but is not worth the latency here
//! - the bar only animates while `bar_until` is in the future, so an idle window draws nothing

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rune::tok::type_;
use rune::{
    col, el, row, text, text_input, App, AppHandle, AppSpec, Color, Cx, El, GrowthSpec, Key,
    KeyEvent, Pixels, ScrollSpec, TextEdit, Tx,
};

mod platform;

use clove_fs::{list_dir, DirWatcher, Entry, Snapshot};
use clove_fs::scan::{ScanEvent, StorageKind, StorageScan, KIND_COUNT};
use clove_render::tokens::{self, Rgb};
use clove_render::{BarRenderer, BarSpec, Segment};

// ---------------------------------------------------------------- palette

/// Clove's tokens are sRGB bytes — what a PNG or a GPU pixel buffer wants. Rune's `Color`
/// is *linear* light, so every token is decoded on the way in; handing it sRGB directly
/// renders the whole UI one gamma step too light (the near-black `#1D1E20` becomes mid grey).
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn color(rgb: Rgb, a: f32) -> Color {
    let c = rgb.to_f32();
    Color {
        r: srgb_to_linear(c[0]),
        g: srgb_to_linear(c[1]),
        b: srgb_to_linear(c[2]),
        a,
    }
}

fn bg() -> Color {
    color(tokens::BG, 1.0)
}
fn surface() -> Color {
    color(tokens::SURFACE, 1.0)
}
fn ink() -> Color {
    color(tokens::INK, 1.0)
}
fn ink_secondary() -> Color {
    color(tokens::INK_SECONDARY, 1.0)
}
fn hairline() -> Color {
    color(tokens::HAIRLINE, 1.0)
}
/// A 6 % white wash over the surface, as Clove's selected row.
fn selection() -> Color {
    Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 0.06,
    }
}

const ROW_H: f32 = 28.0;
const BAR_H: f32 = 88.0;

// ---------------------------------------------------------------- state

#[derive(Clone, Debug, PartialEq, Eq)]
enum Location {
    ThisPc,
    Folder(PathBuf),
}

#[derive(Clone, Debug)]
enum UndoAction {
    Trash { moves: Vec<clove_shell::Move> },
}

#[derive(Clone, Debug)]
struct FileOpProgress {
    label: String,
    done: usize,
    total: usize,
    bytes_done: u64,
    bytes_total: u64,
    failures: usize,
    cancelled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConflictChoice {
    Replace,
    Skip,
    Rename,
    ReplaceAll,
    SkipAll,
}

#[derive(Clone, Debug)]
struct PendingConflict {
    src: PathBuf,
    dest: PathBuf,
    is_dir: bool,
    apply_all: Option<ConflictChoice>,
}

struct State {
    location: Location,
    history_back: Vec<Location>,
    history_forward: Vec<Location>,
    snapshot: Arc<Snapshot>,
    filtered: Vec<usize>,
    query: TextEdit,
    address: TextEdit,
    /// Selected entry indices (into the snapshot).
    selected: std::collections::HashSet<usize>,
    /// Keyboard cursor position in `filtered` (index into filtered vec).
    cursor: usize,
    anchor: Option<usize>,
    clipboard: Vec<PathBuf>,
    clipboard_cut: bool,
    sort_by: u8,
    sort_desc: bool,
    folders_first: bool,
    view_mode: u8,
    preview_open: bool,
    search_mode: bool,
    search_query: TextEdit,
    search_results: Vec<PathBuf>,
    properties_path: Option<PathBuf>,
    properties_size: Option<u64>,
    properties_loading: bool,
    renaming: Option<usize>,
    rename_text: TextEdit,
    context_menu: Option<usize>,
    // Conflict resolution.
    conflict_policy: u8, // 0 auto-rename, 1 ask
    pending_paste: Option<(PathBuf, Vec<PathBuf>, bool)>,
    // Batch rename modal.
    batch_rename_open: bool,
    // Open With modal.
    open_with_open: bool,
    open_with_path: Option<PathBuf>,
    open_with_apps: Vec<(String, PathBuf)>,
    batch_rename_mode: u8,
    batch_rename_find: TextEdit,
    batch_rename_replace: TextEdit,
    batch_rename_prefix: TextEdit,
    batch_rename_suffix: TextEdit,
    thumbnails: std::collections::HashMap<PathBuf, (u32, u32, Vec<u8>)>,
    thumbnail_pending: std::collections::HashSet<PathBuf>,
    tabs: Vec<Location>,
    active_tab: usize,
    status: String,
    loading: bool,
    /// Multi-level undo stack for file ops.
    undo_stack: Vec<UndoAction>,
    /// Active file operation progress (copy/move/delete/extract).
    op_progress: Option<FileOpProgress>,
    op_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    bar: Pixels,
    bar_renderer: BarRenderer,
    bar_time: f32,
    /// Animate the bar until this clock value; `poll` stops asking for frames after it.
    bar_until: f32,
    /// Focus the filter field on the first frames, so typing filters without a click.
    focus_attempts: u8,
    /// Bumped on every navigation so stale load/watch threads stop applying.
    generation: Arc<AtomicU64>,
    handle: AppHandle<State>,
    // --- storage scan ---
    storage_bytes: [u64; KIND_COUNT],
    storage_capacity: u64,
    storage_used: u64,
    scan: Option<StorageScan>,
    scan_started: bool,
    storage_ready: bool,
    storage_has_data: bool,
    bar_width: f32,
    ui_scale: f32,
    show_hidden: bool,
}

impl State {
    fn wake_bar(&mut self) {
        self.bar_until = self.bar_time + 1.5;
    }
}

// ---------------------------------------------------------------- engine glue

fn trigger_search(s: &mut State) {
    let query = s.search_query.text.trim().to_lowercase();
    if query.is_empty() {
        s.search_results.clear();
        return;
    }
    let root = match &s.location {
        Location::Folder(d) => d.clone(),
        _ => return,
    };
    let handle = s.handle.clone();
    std::thread::spawn(move || {
        let mut results = Vec::new();
        walk_dir(&root, &query, &mut results);
        handle.update(move |s, _| {
            s.search_results = results;
        });
    });
}

fn walk_dir(dir: &Path, query: &str, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for item in rd.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_lowercase();
            if name.contains(query) {
                out.push(path.clone());
                if out.len() >= 500 { return; }
            }
            if item.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk_dir(&path, query, out);
                if out.len() >= 500 { return; }
            }
        }
    }
}

fn refilter(s: &mut State) {
    let mut idx = clove_search::filter(&s.snapshot.entries, &s.query.text);
    if !s.show_hidden {
        idx.retain(|&i| {
            let e = &s.snapshot.entries[i];
            !e.hidden && !e.system
        });
    }
    // Sort when not filtering by query (search ranking wins otherwise).
    if s.query.text.trim().is_empty() {
        let sort_by = s.sort_by;
        let desc = s.sort_desc;
        let folders_first = s.folders_first;
        idx.sort_by(|&a, &b| {
            let ea = &s.snapshot.entries[a];
            let eb = &s.snapshot.entries[b];
            if folders_first {
                let fc = eb.is_dir.cmp(&ea.is_dir);
                if fc != std::cmp::Ordering::Equal {
                    return fc;
                }
            }
            let ord = match sort_by {
                1 => ea.size.cmp(&eb.size),
                2 => ea.name.rsplit('.').next().unwrap_or("").cmp(&eb.name.rsplit('.').next().unwrap_or("")),
                3 => ea.modified.cmp(&eb.modified),
                _ => ea.name_folded.cmp(&eb.name_folded),
            };
            if desc { ord.reverse() } else { ord }
        });
    }
    s.filtered = idx;
    if s.filtered.is_empty() {
        s.cursor = 0;
    } else {
        s.cursor = s.cursor.min(s.filtered.len() - 1);
    }
}

fn sync_address(s: &mut State) {
    match &s.location {
        Location::ThisPc => s.address.text = "This PC".to_string(),
        Location::Folder(p) => s.address.text = p.display().to_string(),
    }
}

fn rebuild_bar(s: &mut State) {
    let spec = if s.storage_has_data {
        let total: u64 = s.storage_bytes.iter().sum();
        let capacity = if s.storage_capacity > 0 {
            s.storage_capacity
        } else {
            total.max(1)
        };
        let segments = s
            .storage_bytes
            .iter()
            .enumerate()
            .map(|(kind, &bytes)| Segment::kind(kind, bytes as f32 / capacity as f32))
            .collect();
        BarSpec::new(s.bar_width, segments)
            .with_height(BAR_H)
            .with_scale(s.ui_scale)
    } else {
        BarSpec::placeholder(s.bar_width)
            .with_height(BAR_H)
            .with_scale(s.ui_scale)
    };
    let mut renderer = BarRenderer::new(spec);
    renderer.render(s.bar_time);
    let (w, h) = renderer.size();
    s.bar.set(w as u32, h as u32, renderer.buffer().to_vec());
    s.bar_renderer = renderer;
    s.wake_bar();
}

/// Reads `dir` and swaps it in, unless a newer navigation has superseded this load.
fn load_into(
    handle: &AppHandle<State>,
    generation: &Arc<AtomicU64>,
    mine: u64,
    dir: PathBuf,
) {
    let result = list_dir(&dir);
    let generation = generation.clone();
    handle.update(move |s, _| {
        if generation.load(Ordering::Relaxed) != mine {
            return;
        }
        match result {
            Ok(snapshot) => {
                s.snapshot = Arc::new(snapshot);
                s.location = Location::Folder(dir);
                sync_address(s);
                // tab update not available here easily; skip
                if s.loading {
                    s.status.clear();
                }
                s.loading = false;
                s.wake_bar();
                refilter(s);
            }
            Err(err) => {
                s.status = format!("cannot read: {err}");
                s.loading = false;
            }
        }
    });
}

/// Navigates to `location`: records history, swaps the snapshot, arms a watcher for folders.
fn navigate(s: &mut State, location: Location) {
    if location == s.location {
        return;
    }
    s.history_back.push(s.location.clone());
    s.history_forward.clear();

    s.location = location.clone();
    sync_address(s);
    if s.tabs.is_empty() { s.tabs.push(s.location.clone()); }
    if s.active_tab < s.tabs.len() { s.tabs[s.active_tab] = s.location.clone(); }
    s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), Vec::new()));
    s.filtered.clear();
    s.selected.clear();
    s.cursor = 0;
    s.anchor = None;
    s.loading = true;
    s.status = "loading...".into();
    s.query.text.clear();

    let mine = s.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let generation = s.generation.clone();
    let handle = s.handle.clone();

    match location {
        Location::ThisPc => {
            let entries = this_pc_entries();
            handle.update(move |s, _| {
                if generation.load(Ordering::Relaxed) != mine {
                    return;
                }
                s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), entries));
                s.loading = false;
                s.status.clear();
                refilter(s);
            });
        }
        Location::Folder(dir) => {
            std::thread::spawn(move || {
                let watcher = DirWatcher::new(&dir).ok();
                load_into(&handle, &generation, mine, dir.clone());

                let Some(watcher) = watcher else { return };
                while generation.load(Ordering::Relaxed) == mine {
                    if watcher.recv_timeout(Duration::from_millis(250)).is_some() {
                        load_into(&handle, &generation, mine, dir.clone());
                    }
                }
            });
        }
    }
}

fn go_back(s: &mut State) {
    let Some(prev) = s.history_back.pop() else { return };
    s.history_forward.push(s.location.clone());
    let target = prev;
    s.location = target.clone();
    sync_address(s);
    if !s.tabs.is_empty() && s.active_tab < s.tabs.len() { s.tabs[s.active_tab] = s.location.clone(); }
    s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), Vec::new()));
    s.filtered.clear();
    s.selected.clear();
    s.cursor = 0;
    s.anchor = None;
    s.loading = true;
    s.status = "loading...".into();
    let mine = s.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let generation = s.generation.clone();
    let handle = s.handle.clone();
    match target {
        Location::ThisPc => {
            let entries = this_pc_entries();
            handle.update(move |s, _| {
                if generation.load(Ordering::Relaxed) != mine { return; }
                s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), entries));
                s.loading = false;
                s.status.clear();
                refilter(s);
            });
        }
        Location::Folder(dir) => {
            std::thread::spawn(move || {
                load_into(&handle, &generation, mine, dir);
            });
        }
    }
}

fn go_forward(s: &mut State) {
    let Some(next) = s.history_forward.pop() else { return };
    s.history_back.push(s.location.clone());
    let target = next;
    s.location = target.clone();
    sync_address(s);
    if !s.tabs.is_empty() && s.active_tab < s.tabs.len() { s.tabs[s.active_tab] = s.location.clone(); }
    s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), Vec::new()));
    s.filtered.clear();
    s.selected.clear();
    s.cursor = 0;
    s.anchor = None;
    s.loading = true;
    s.status = "loading...".into();
    let mine = s.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let generation = s.generation.clone();
    let handle = s.handle.clone();
    match target {
        Location::ThisPc => {
            let entries = this_pc_entries();
            handle.update(move |s, _| {
                if generation.load(Ordering::Relaxed) != mine { return; }
                s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), entries));
                s.loading = false;
                s.status.clear();
                refilter(s);
            });
        }
        Location::Folder(dir) => {
            std::thread::spawn(move || {
                load_into(&handle, &generation, mine, dir);
            });
        }
    }
}

fn reload(s: &mut State) {
    let mine = s.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let generation = s.generation.clone();
    let handle = s.handle.clone();
    match s.location.clone() {
        Location::ThisPc => {
            let entries = this_pc_entries();
            handle.update(move |s, _| {
                if generation.load(Ordering::Relaxed) != mine { return; }
                s.snapshot = Arc::new(Snapshot::new(PathBuf::new(), entries));
                s.loading = false;
                s.status.clear();
                refilter(s);
            });
        }
        Location::Folder(dir) => {
            std::thread::spawn(move || {
                load_into(&handle, &generation, mine, dir);
            });
        }
    }
}

fn this_pc_entries() -> Vec<Entry> {
    let mut entries = Vec::new();
    for (name, path) in platform::quick_access() {
        if path.exists() {
            entries.push(Entry::new(name, true, 0));
        }
    }
    #[cfg(windows)]
    {
        for i in 0..26u8 {
            let letter = (b'A' + i) as char;
            let root = PathBuf::from(format!("{letter}:\\"));
            if root.exists() {
                entries.push(Entry::new(format!("{letter}:"), true, 0));
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        for (label, path) in platform::mount_entries() {
            if path.exists() {
                entries.push(Entry::new(label, true, 0));
            }
        }
    }
    entries
}

fn this_pc_resolve(entry: &Entry) -> Option<PathBuf> {
    let name = entry.name.as_str();
    if name.len() == 2 && name.ends_with(':') {
        let letter = name.chars().next().unwrap();
        return Some(PathBuf::from(format!("{letter}:\\\\")));
    }
    let home = platform::home_dir();
    match name {
        "Desktop" => Some(home.join("Desktop")),
        "Documents" => Some(home.join("Documents")),
        "Downloads" => Some(home.join("Downloads")),
        "Pictures" => Some(home.join("Pictures")),
        "Music" => Some(home.join("Music")),
        "Videos" => Some(home.join("Videos")),
        _ => {
            // Mount points on Linux are stored under their own path.
            if name.starts_with('/') {
                Some(PathBuf::from(name))
            } else {
                None
            }
        }
    }
}

fn entry_path(location: &Location, entry: &Entry) -> Option<PathBuf> {
    match location {
        Location::ThisPc => this_pc_resolve(entry),
        Location::Folder(dir) => Some(dir.join(&entry.name)),
    }
}


fn delete_selected(s: &mut State) {
    let Location::Folder(root) = s.location.clone() else { return };
    let Some(&index) = s.filtered.get(s.cursor) else {
        return;
    };
    let entry = s.snapshot.entries[index].clone();
    let Some(path) = entry_path(&s.location, &entry) else { return };
    let handle = s.handle.clone();

    s.status = format!("moving {} to the Trash...", entry.name);
    s.wake_bar();

    std::thread::spawn(move || {
        let outcome = clove_shell::trash(&path, &root, false);
        let name = entry.name.clone();
        let moves = outcome.moves;
        let failures = outcome.failures;
        handle.update(move |s, _| {
            if moves.is_empty() {
                if failures > 0 {
                    s.status = format!("could not move {name}");
                }
                return;
            }
            s.status = format!("moved {name} to the Trash . Ctrl+Z to undo");
            s.undo_stack.push(UndoAction::Trash { moves });
            s.wake_bar();
        });
    });
}


fn undo_delete(s: &mut State) {
    let Some(action) = s.undo_stack.pop() else {
        s.status = "nothing to undo".into();
        return;
    };
    let handle = s.handle.clone();
    s.status = "restoring…".into();

    std::thread::spawn(move || {
        match action {
            UndoAction::Trash { moves } => {
                let outcome = clove_shell::put_back(&moves);
                handle.update(move |s, _| {
                    s.status = format!(
                        "restored {} · {} could not be restored",
                        outcome.restored, outcome.missing
                    );
                    s.wake_bar();
                });
            }
        }
    });
}

fn batch_rename_targets(s: &State) -> Vec<usize> {
    if s.selected.is_empty() {
        if let Some(&idx) = s.filtered.get(s.cursor) { vec![idx] } else { vec![] }
    } else {
        s.selected.iter().copied().collect()
    }
}

fn batch_rename_preview(s: &State) -> Vec<(usize, String)> {
    let targets = batch_rename_targets(s);
    let mut out = Vec::new();
    let mut n = 1u32;
    let start = s.batch_rename_suffix.text.parse::<u32>().ok().unwrap_or(1); // reuse suffix? no
    let _ = start;
    for &idx in &targets {
        let Some(entry) = s.snapshot.entries.get(idx) else { continue };
        let old = entry.name.clone();
        let new_name = match s.batch_rename_mode {
            0 => {
                let find = s.batch_rename_find.text.as_str();
                let repl = s.batch_rename_replace.text.as_str();
                if find.is_empty() { old.clone() } else { old.replace(find, repl) }
            }
            1 => format!("{}{}", s.batch_rename_prefix.text, old),
            2 => {
                // suffix before extension
                if let Some(dot) = old.rfind('.') {
                    format!("{}{}{}", &old[..dot], s.batch_rename_suffix.text, &old[dot..])
                } else {
                    format!("{}{}", old, s.batch_rename_suffix.text)
                }
            }
            3 => {
                let prefix = s.batch_rename_prefix.text.as_str();
                let suffix = s.batch_rename_suffix.text.as_str();
                format!("{}{}{}", prefix, n, suffix)
            }
            _ => old.clone(),
        };
        if s.batch_rename_mode == 3 { n += 1; }
        if new_name != old && !new_name.is_empty() {
            out.push((idx, new_name));
        }
    }
    out
}

fn open_with_candidates(path: &Path) -> Vec<(String, PathBuf)> {
    // Determine mime type via xdg-mime if available.
    let mime = std::process::Command::new("xdg-mime")
        .args(["query", "filetype"])
        .arg(path)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let mut dirs = vec![PathBuf::from("/usr/share/applications"), PathBuf::from("/usr/local/share/applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("desktop") { continue; }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            // parse Name and MimeType
            let mut name = String::new();
            let mut mimes = String::new();
            let mut hidden = false;
            let mut no_display = false;
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("Name=") { name = v.to_string(); }
                if let Some(v) = line.strip_prefix("MimeType=") { mimes = v.to_string(); }
                if line == "Hidden=true" || line == "NoDisplay=true" { hidden = true; }
                if line.starts_with("NoDisplay=") { no_display = line.ends_with("true"); }
            }
            if hidden || no_display { continue; }
            if name.is_empty() { name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(); }
            let matches = if mime.is_empty() {
                true
            } else {
                mimes.split(';').any(|m| m.trim() == mime)
            };
            if !matches { continue; }
            if seen.insert(p.clone()) {
                out.push((name, p));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn launch_desktop_app(desktop: &Path, file: &Path) {
    // Try gtk-launch first (handles Exec parsing + field codes).
    let app_id = desktop.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if std::process::Command::new("gtk-launch").arg(&app_id).arg(file).spawn().is_ok() {
        return;
    }
    // Fallback: parse Exec line and substitute %f/%F/%u/%U.
    if let Ok(text) = std::fs::read_to_string(desktop) {
        for line in text.lines() {
            if let Some(exec) = line.strip_prefix("Exec=") {
                let mut cmd = exec.replace("%F", &file.to_string_lossy()).replace("%f", &file.to_string_lossy())
                    .replace("%U", &file.to_string_lossy()).replace("%u", &file.to_string_lossy());
                // strip remaining field codes
                for code in ["%i", "%c", "%k"] { cmd = cmd.replace(code, ""); }
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.is_empty() { break; }
                let mut c = std::process::Command::new(parts[0]);
                c.args(&parts[1..]);
                let _ = c.spawn();
                break;
            }
        }
    }
}
fn apply_batch_rename(s: &mut State) {
    let previews = batch_rename_preview(s);
    if previews.is_empty() { s.status = "nothing to rename".into(); return; }
    let handle = s.handle.clone();
    let items: Vec<(PathBuf, PathBuf)> = previews.iter().filter_map(|(idx, new_name)| {
        let entry = s.snapshot.entries.get(*idx)?;
        let old = entry_path(&s.location, entry)?;
        let parent = old.parent()?.to_path_buf();
        Some((old, parent.join(new_name)))
    }).collect();
    s.status = format!("renaming {} items...", items.len());
    std::thread::spawn(move || {
        let mut done = 0usize;
        let mut fail = 0usize;
        for (old, newp) in &items {
            if std::fs::rename(old, newp).is_ok() { done += 1; } else { fail += 1; }
        }
        handle.update(move |s, _| {
            s.status = format!("renamed {} · {} failed", done, fail);
            s.batch_rename_open = false;
            reload(s);
        });
    });
}
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            let p = entry.path();
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    total += dir_size(&p);
                } else {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}

fn load_thumbnails(s: &mut State) {
    let entries: Vec<(usize, PathBuf)> = s
        .snapshot
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if e.is_dir { return None; }
            let path = entry_path(&s.location, e)?;
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp") {
                return None;
            }
            if s.thumbnails.contains_key(&path) || s.thumbnail_pending.contains(&path) {
                return None;
            }
            Some((i, path))
        })
        .collect();
    for (_, path) in entries {
        s.thumbnail_pending.insert(path.clone());
        let handle = s.handle.clone();
        let path2 = path.clone();
        std::thread::spawn(move || {
            let mut rgba = None;
            let mut dims = None;
            if let Ok(img) = image::open(&path2) {
                let thumb = img.thumbnail(64, 64);
                let rgba8 = thumb.to_rgba8();
                dims = Some((rgba8.width(), rgba8.height()));
                rgba = Some(rgba8.into_raw());
            }
            handle.update(move |s, _| {
                s.thumbnail_pending.remove(&path2);
                if let (Some(d), Some(r)) = (dims, rgba) {
                    s.thumbnails.insert(path2, (d.0, d.1, r));
                }
            });
        });
    }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

fn do_paste(s: &mut State) {
    let target = match &s.location {
        Location::Folder(d) => d.clone(),
        _ => { s.status = "paste not supported here".into(); return; }
    };
    let mut paths = s.clipboard.clone();
    if paths.is_empty() {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(fl) = cb.get().file_list() {
                paths = fl;
                s.clipboard_cut = false;
            }
        }
    }
    if paths.is_empty() {
        s.status = "clipboard empty".into();
        return;
    }
    if s.conflict_policy == 1 {
        let has_collision = paths.iter().any(|src| {
            src.file_name().map(|n| target.join(n).exists()).unwrap_or(false)
        });
        if has_collision {
            s.pending_paste = Some((target, paths, s.clipboard_cut));
            s.status = "resolve conflicts".into();
            return;
        }
    }
    run_paste(s, target, paths, s.clipboard_cut, 0);
}

fn run_paste(s: &mut State, target: PathBuf, paths: Vec<PathBuf>, cut: bool, mode: u8) {
    let handle = s.handle.clone();
    let cancel = s.op_cancel.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let total = paths.len();
    s.op_progress = Some(FileOpProgress {
        label: "Pasting".into(),
        done: 0,
        total,
        bytes_done: 0,
        bytes_total: 0,
        failures: 0,
        cancelled: false,
    });
    s.status = "pasting...".into();
    std::thread::spawn(move || {
        let mut moved = 0usize;
        let mut skipped = 0usize;
        let mut failures = 0usize;
        for (i, src) in paths.iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                handle.update(move |s, _| {
                    s.op_progress = None;
                    s.status = format!("cancelled · pasted {} · skipped {} · failed {}", moved, skipped, failures);
                    s.wake_bar();
                    reload(s);
                });
                return;
            }
            let Some(name) = src.file_name() else { failures += 1; continue; };
            let mut dest = target.join(name);
            if dest.exists() {
                match mode {
                    1 => { /* replace: keep dest */ }
                    2 => { skipped += 1; continue; }
                    _ => {
                        let stem = dest.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
                        let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                        let mut n = 1u32;
                        loop {
                            let cand = target.join(format!("{stem} ({n}){ext}"));
                            if !cand.exists() { dest = cand; break; }
                            n += 1;
                            if n > 10_000 { skipped += 1; break; }
                        }
                        if n > 10_000 { continue; }
                    }
                }
            }
            if mode == 1 && dest.exists() {
                if dest.is_dir() { let _ = std::fs::remove_dir_all(&dest); } else { let _ = std::fs::remove_file(&dest); }
            }
            let res = if src.is_dir() {
                if cut {
                    std::fs::rename(src, &dest).or_else(|_| { copy_dir(src, &dest).and_then(|_| std::fs::remove_dir_all(src)) })
                } else {
                    copy_dir(src, &dest)
                }
            } else if cut {
                std::fs::rename(src, &dest).or_else(|_| { std::fs::copy(src, &dest).and_then(|_| std::fs::remove_file(src)) })
            } else {
                std::fs::copy(src, &dest).map(|_| ())
            };
            if res.is_ok() { moved += 1; } else { failures += 1; }
            let done = i + 1;
            handle.update(move |s, _| {
                if let Some(op) = &mut s.op_progress {
                    op.done = done;
                    op.failures = failures;
                }
            });
        }
        handle.update(move |s, _| {
            s.op_progress = None;
            s.status = format!("pasted {} · skipped {} · failed {}", moved, skipped, failures);
            s.wake_bar();
            if cut { s.clipboard.clear(); s.clipboard_cut = false; }
            reload(s);
        });
    });
}


fn handle_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    // Ctrl+Z (Cmd+Z on macOS) restores the last trashed selection. It has to be claimed
    // before the focused filter field turns it into a text undo.
    if ev.is_command('z') {
        undo_delete(s);
        return true;
    }
    // Ctrl+H toggles hidden files.
    if ev.is_command('h') {
        s.show_hidden = !s.show_hidden;
        refilter(s);
        save_settings(s);
        s.status = if s.show_hidden { "showing hidden files".into() } else { "hidden files hidden".into() };
        return true;
    }
    // Ctrl+C / Ctrl+X: copy/cut selected entries.
    if ev.is_command('c') || ev.is_command('x') {
        let cut = ev.is_command('x');
        let mut paths = Vec::new();
        for &idx in &s.selected {
            if let Some(entry) = s.snapshot.entries.get(idx) {
                if let Some(p) = entry_path(&s.location, entry) {
                    paths.push(p);
                }
            }
        }
        if paths.is_empty() {
            if let Some(&idx) = s.filtered.get(s.cursor) {
                if let Some(entry) = s.snapshot.entries.get(idx) {
                    if let Some(p) = entry_path(&s.location, entry) {
                        paths.push(p);
                    }
                }
            }
        }
        if !paths.is_empty() {
            s.clipboard = paths.clone();
            s.clipboard_cut = cut;
            let paths2 = paths.clone();
            std::thread::spawn(move || {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set().file_list(&paths2);
                }
            });
            s.status = format!("{} {} item(s)", if cut { "cut" } else { "copied" }, paths.len());
        }
        return true;
    }
    // Ctrl+V: paste.
    if ev.is_command('v') {
        do_paste(s);
        return true;
    }
    // Ctrl+Shift+N: new folder.
    if ev.is_command('n') && ev.mods.shift {
        if let Location::Folder(root) = s.location.clone() {
            let handle = s.handle.clone();
            s.status = "creating folder...".into();
            std::thread::spawn(move || {
                let mut name = "New folder".to_string();
                let mut dest = root.join(&name);
                let mut i = 2;
                while dest.exists() {
                    name = format!("New folder ({})", i);
                    dest = root.join(&name);
                    i += 1;
                }
                let _ = std::fs::create_dir(&dest);
                handle.update(move |s, _| {
                    s.status = format!("created {}", name);
                    reload(s);
                });
            });
        }
        return true;
    }
    // F3 cycles sort field, F4 toggles direction, F5 toggles folders-first.
    if ev.key == Key::Function(3) {
        s.sort_by = (s.sort_by + 1) % 4;
        refilter(s);
        save_settings(s);
        let names = ["name", "size", "type", "modified"];
        s.status = format!("sort by {}", names[s.sort_by as usize]);
        return true;
    }
    if ev.key == Key::Function(4) {
        s.sort_desc = !s.sort_desc;
        refilter(s);
        save_settings(s);
        s.status = if s.sort_desc { "sort: descending".into() } else { "sort: ascending".into() };
        return true;
    }
    if ev.key == Key::Function(5) {
        s.folders_first = !s.folders_first;
        refilter(s);
        save_settings(s);
        s.status = if s.folders_first { "folders first".into() } else { "folders mixed".into() };
        return true;
    }
    // Ctrl+1/2/3 view modes.
    if ev.is_command('1') { s.view_mode = 0; save_settings(s); s.status = "view: details".into(); return true; }
    if ev.is_command('2') { s.view_mode = 1; save_settings(s); s.status = "view: list".into(); return true; }
    if ev.is_command('3') {
        s.view_mode = 2;
        save_settings(s);
        s.status = "view: grid".into();
        load_thumbnails(s);
        return true;
    }
    // Ctrl+P preview pane.
    if ev.is_command('p') { s.preview_open = !s.preview_open; return true; }
    // Ctrl+F search within folder.
    if ev.is_command('f') {
        s.search_mode = !s.search_mode;
        if s.search_mode { s.status = "search (esc to exit)".into(); }
        return true;
    }
    // F2 rename (placeholder: sets status; full inline edit deferred).
    if ev.key == Key::Function(2) {
        if let Some(&idx) = s.filtered.get(s.cursor) {
            if let Some(entry) = s.snapshot.entries.get(idx) {
                s.renaming = Some(idx);
                s.rename_text = TextEdit::new(&entry.name);
            }
        }
        return true;
    }
    // Ctrl+I properties.
    if ev.is_command('i') {
        if let Some(&idx) = s.filtered.get(s.cursor) {
            if let Some(entry) = s.snapshot.entries.get(idx) {
                if let Some(p) = entry_path(&s.location, entry) {
                    s.properties_path = Some(p);
                    s.properties_size = None;
                    s.properties_loading = true;
                    let path = s.properties_path.clone().unwrap();
                    let handle = s.handle.clone();
                    std::thread::spawn(move || {
                        let size = if path.is_dir() { dir_size(&path) } else { path.metadata().map(|m| m.len()).unwrap_or(0) };
                        handle.update(move |s, _| {
                            s.properties_size = Some(size);
                            s.properties_loading = false;
                        });
                    });
                }
            }
        }
        return true;
    }
    // Tabs.
    if ev.is_command('t') {
        s.tabs.push(s.location.clone());
        s.active_tab = s.tabs.len() - 1;
        s.status = format!("tab {}", s.active_tab + 1);
        return true;
    }
    if ev.is_command('w') {
        if s.tabs.len() > 1 {
            s.tabs.remove(s.active_tab);
            if s.active_tab >= s.tabs.len() { s.active_tab = s.tabs.len() - 1; }
            let target = s.tabs[s.active_tab].clone();
            navigate(s, target);
            sync_address(s);
        }
        return true;
    }
    // Ctrl+Tab next tab (approx via command tab not standard; use F6).
    if ev.key == Key::Function(6) {
        if !s.tabs.is_empty() {
            s.active_tab = (s.active_tab + 1) % s.tabs.len();
            let target = s.tabs[s.active_tab].clone();
            navigate(s, target);
            sync_address(s);
        }
        return true;
    }
    // Ctrl+R (Cmd+R) re-measures the disk.
    if ev.is_command('r') {
        if let Some(path) = storage_cache_path() {
            let _ = std::fs::remove_file(path);
        }
        s.storage_bytes = [0; KIND_COUNT];
        s.storage_capacity = 0;
        s.storage_used = 0;
        s.storage_ready = false;
        s.storage_has_data = false;
        s.scan = None;
        s.scan_started = false;
        s.status = "measuring disk…".into();
        s.wake_bar();
        rebuild_bar(s);
        return true;
    }
    let n = s.filtered.len();
    match ev.key {
        Key::Down if n > 0 => {
            s.cursor = (s.cursor + 1).min(n - 1);
            if ev.mods.shift {
                if let Some(&idx) = s.filtered.get(s.cursor) { s.selected.insert(idx); }
            } else {
                s.selected.clear();
                if let Some(&idx) = s.filtered.get(s.cursor) { s.selected.insert(idx); }
            }
            true
        }
        Key::Up => {
            s.cursor = s.cursor.saturating_sub(1);
            if n > 0 {
                if ev.mods.shift {
                    if let Some(&idx) = s.filtered.get(s.cursor) { s.selected.insert(idx); }
                } else {
                    s.selected.clear();
                    if let Some(&idx) = s.filtered.get(s.cursor) { s.selected.insert(idx); }
                }
            }
            true
        }
        Key::Home if n > 0 => {
            s.cursor = 0;
            s.selected.clear();
            if let Some(&idx) = s.filtered.get(0) { s.selected.insert(idx); }
            true
        }
        Key::End if n > 0 => {
            s.cursor = n - 1;
            s.selected.clear();
            if let Some(&idx) = s.filtered.get(n - 1) { s.selected.insert(idx); }
            true
        }
        Key::Enter => {
            if let Some(&index) = s.filtered.get(s.cursor) {
                if s.snapshot.entries[index].is_dir {
                    if let Some(path) = entry_path(&s.location, &s.snapshot.entries[index]) {
                        navigate(s, Location::Folder(path));
                        sync_address(s);
                        return true;
                    }
                } else if let Some(path) = entry_path(&s.location, &s.snapshot.entries[index]) {
                    platform::open_path(&path);
                    s.status = format!("opening {}", s.snapshot.entries[index].name);
                    return true;
                }
            }
            false
        }
        Key::Backspace if s.query.text.is_empty() => {
            match &s.location {
                Location::Folder(dir) => {
                    if let Some(parent) = dir.parent().map(Path::to_path_buf) {
                        navigate(s, Location::Folder(parent));
                        sync_address(s);
                        return true;
                    }
                    // at drive root -> This PC
                    navigate(s, Location::ThisPc);
                    sync_address(s);
                    return true;
                }
                Location::ThisPc => false,
            }
        }
        Key::Delete => {
            if ev.mods.shift {
                // permanent delete
                let mut paths = Vec::new();
                for &idx in &s.selected {
                    if let Some(entry) = s.snapshot.entries.get(idx) {
                        if let Some(p) = entry_path(&s.location, entry) { paths.push(p); }
                    }
                }
                if paths.is_empty() {
                    if let Some(&idx) = s.filtered.get(s.cursor) {
                        if let Some(entry) = s.snapshot.entries.get(idx) {
                            if let Some(p) = entry_path(&s.location, entry) { paths.push(p); }
                        }
                    }
                }
                let handle = s.handle.clone();
                s.status = "deleting permanently...".into();
                std::thread::spawn(move || {
                    let mut done = 0usize; let mut fail = 0usize;
                    for p in &paths {
                        let res = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
                        if res.is_ok() { done += 1; } else { fail += 1; }
                    }
                    handle.update(move |s, _| {
                        s.status = format!("deleted {} · {} failed", done, fail);
                        reload(s);
                    });
                });
            } else {
                delete_selected(s);
            }
            true
        }
        Key::Escape => {
            if s.op_progress.is_some() {
                s.op_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                return true;
            }
            if s.context_menu.is_some() {
                s.context_menu = None;
                return true;
            }
            if s.properties_path.is_some() {
                s.properties_path = None;
                return true;
            }
            if s.search_mode {
                s.search_mode = false;
                return true;
            }
            if s.query.text.is_empty() {
                false
            } else {
                s.query.text.clear();
                refilter(s);
                true
            }
        }
        _ => false,
    }
}

fn handle_rename_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if ev.key == Key::Enter {
        if let Some(idx) = s.renaming {
            if let Some(entry) = s.snapshot.entries.get(idx) {
                if let Some(old_path) = entry_path(&s.location, entry) {
                    let new_name = s.rename_text.text.trim().to_string();
                    if !new_name.is_empty() && new_name != entry.name {
                        if let Some(parent) = old_path.parent() {
                            let new_path = parent.join(&new_name);
                            let handle = s.handle.clone();
                            let old = old_path.clone();
                            std::thread::spawn(move || {
                                let res = std::fs::rename(&old, &new_path);
                                handle.update(move |s, _| {
                                    if res.is_ok() {
                                        s.status = format!("renamed to {}", new_name);
                                    } else {
                                        s.status = "rename failed".into();
                                    }
                                    s.renaming = None;
                                    reload(s);
                                });
                            });
                        }
                    } else {
                        s.renaming = None;
                    }
                }
            }
        }
        return true;
    }
    if ev.key == Key::Escape {
        s.renaming = None;
        return true;
    }
    false
}

fn handle_address_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if ev.key == Key::Enter {
        let text = s.address.text.trim().to_string();
        if text.eq_ignore_ascii_case("This PC") || text.is_empty() {
            navigate(s, Location::ThisPc);
            sync_address(s);
            return true;
        }
        let path = PathBuf::from(&text);
        if path.exists() {
            if path.is_dir() {
                navigate(s, Location::Folder(path));
                sync_address(s);
                return true;
            }
        }
        s.status = format!("not found: {text}");
        return true;
    }
    false
}

// ---------------------------------------------------------------- bar animation

/// Rune calls this at the start of every frame. Returns true only when visible state moved.
fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    // Clove's filter field owns the keyboard from the start. `Tx::focus` resolves after the
    // frame's view is built, so the first attempt lands on frame one; a second covers a
    // window that had not built its tree yet.
    if s.focus_attempts < 2 {
        s.focus_attempts += 1;
        tx.focus("filter", false);
    }

    s.bar_time = clock as f32;

    // Start the storage scan once, after the first frame is on its way. The scan runs on
    // its own threads; this only spawns them and records the volume's capacity.
    if !s.scan_started {
        s.scan_started = true;
        let scan = StorageScan::start();
        if let Some(volume) = scan.volume() {
            s.storage_capacity = volume.capacity;
            s.storage_used = volume.used();
        }
        s.scan = Some(scan);
        tx.wake_after(Duration::from_millis(100));
    }

    let mut changed = false;
    let mut finished = false;
    let mut new_done = false;
    let events: Vec<ScanEvent> = if let Some(scan) = &s.scan {
        let mut v = Vec::new();
        while let Some(event) = scan.try_recv() {
            v.push(event);
        }
        v
    } else {
        Vec::new()
    };
    for event in events {
        match event {
            ScanEvent::Progress(progress) => {
                s.storage_bytes[progress.kind.index()] = progress.bytes;
                s.storage_has_data = true;
                let total: u64 = s.storage_bytes.iter().sum();
                let capacity = if s.storage_capacity > 0 {
                    s.storage_capacity
                } else {
                    total.max(1)
                };
                let segments = s
                    .storage_bytes
                    .iter()
                    .enumerate()
                    .map(|(kind, &bytes)| Segment::kind(kind, bytes as f32 / capacity as f32))
                    .collect();
                if s.bar_renderer.set_segments(segments) {
                    s.wake_bar();
                    changed = true;
                }
            }
            ScanEvent::Done(report) => {
                s.storage_bytes[report.kind.index()] = report.bytes;
                s.storage_has_data = true;
                let total: u64 = s.storage_bytes.iter().sum();
                let capacity = if s.storage_capacity > 0 {
                    s.storage_capacity
                } else {
                    total.max(1)
                };
                let segments = s
                    .storage_bytes
                    .iter()
                    .enumerate()
                    .map(|(kind, &bytes)| Segment::kind(kind, bytes as f32 / capacity as f32))
                    .collect();
                if s.bar_renderer.set_segments(segments) {
                    s.wake_bar();
                    changed = true;
                }
                new_done = true;
            }
            ScanEvent::Finished => finished = true,
        }
    }
    if new_done {
        s.storage_has_data = true;
        rebuild_bar(s);
        changed = true;
    }
    if finished {
        s.scan = None;
        let counted: u64 = s
            .storage_bytes
            .iter()
            .enumerate()
            .filter(|(kind, _)| *kind != StorageKind::System.index())
            .map(|(_, &bytes)| bytes)
            .sum();
        s.storage_bytes[StorageKind::System.index()] = s.storage_used.saturating_sub(counted);
        s.storage_ready = true;
        s.storage_has_data = true;
        rebuild_bar(s);
        changed = true;
        // Persist for the 30-minute cache so the next launch skips the walk.
        let bytes = s.storage_bytes;
        let capacity = s.storage_capacity;
        let used = s.storage_used;
        std::thread::spawn(move || save_storage_cache(bytes, capacity, used));
    } else if s.scan.is_some() {
        tx.wake_after(Duration::from_millis(100));
    }

    if s.bar_time >= s.bar_until {
        return changed;
    }

    let before = s.bar_renderer.frame_seq();
    s.bar_renderer.render(s.bar_time);
    let dirty = s.bar_renderer.frame_seq() != before;
    if dirty {
        let (w, h) = s.bar_renderer.size();
        s.bar.set(w as u32, h as u32, s.bar_renderer.buffer().to_vec());
    }

    if s.bar_time < s.bar_until {
        tx.wake_after(Duration::from_millis(33));
    }
    dirty || changed
}

// ---------------------------------------------------------------- view

fn human_gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        format!("{:.1} TB", bytes as f64 / 1e12)
    } else {
        format!("{:.1} GB", bytes as f64 / 1e9)
    }
}

fn color_f32(rgb: [f32; 3], a: f32) -> Color {
    Color {
        r: srgb_to_linear(rgb[0]),
        g: srgb_to_linear(rgb[1]),
        b: srgb_to_linear(rgb[2]),
        a,
    }
}

fn legend_el(s: &State) -> El {
    let mut legend = row()
        .key("legend")
        .w_full()
        .h(20.0)
        .gap(12.0)
        .center_y();
    for k in 0..KIND_COUNT {
        let bytes = s.storage_bytes[k];
        let label = if s.storage_has_data {
            format!("{} {}", tokens::STORAGE_KIND_NAMES[k], human_gb(bytes))
        } else {
            tokens::STORAGE_KIND_NAMES[k].to_string()
        };
        legend = legend.child(
            row()
                .key(format!("legend-{k}"))
                .gap(4.0)
                .center_y()
                .child(
                    el()
                        .w(8.0)
                        .h(8.0)
                        .radius(2.0)
                        .fill(color_f32(tokens::STORAGE_KIND_RGB[k], 1.0)),
                )
                .child(text(label).font(type_::LABEL).color(ink_secondary())),
        );
    }
    legend
}

fn format_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn row_el(s: &State, index: usize) -> El {
    let entry: &Entry = &s.snapshot.entries[index];
    let selected = s.selected.contains(&index);
    let renaming = s.renaming == Some(index);
    let mut r = row()
        .key(format!("row-{index}"))
        .h(ROW_H)
        .px(8.0)
        .center_y()
        .radius(tokens::RADIUS_ROW)
        .fill(if selected { selection() } else { Color::TRANSPARENT });
    if renaming {
        r = r.child(
            text_input(&s.rename_text, type_::BODY)
                .key(format!("rename-{index}"))
                .single_line()
                .grow(1.0)
                .color(ink())
                .on_edit(|s: &mut State| &mut s.rename_text)
                .on_input_key(handle_rename_key),
        );
    } else {
        r = r.child(
            text(if entry.is_dir {
                format!("{}", entry.name)
            } else {
                entry.name.clone()
            })
            .font(type_::BODY)
            .color(if entry.is_dir { ink() } else { ink_secondary() })
            .grow(1.0),
        );
    }
    r = r.child(
        text(if entry.is_dir {
            "folder".to_owned()
        } else {
            format_size(entry.size)
        })
        .font(type_::LABEL)
        .color(ink_secondary()),
    );
    let idx2 = index;
    r = r.on_secondary_press(move |s: &mut State, _, _| {
        s.context_menu = Some(idx2);
    });
    r
}

fn location_label(loc: &Location) -> String {
    match loc {
        Location::ThisPc => "This PC".to_string(),
        Location::Folder(p) => p.display().to_string(),
    }
}

fn breadcrumbs_el(s: &State) -> El {
    let mut row = row().key("breadcrumbs").w_full().h(20.0).gap(4.0).center_y();
    let this_pc_color = if matches!(s.location, Location::ThisPc) { ink() } else { ink_secondary() };
    row = row.child(
        el()
            .key("bc-thispc")
            .radius(tokens::RADIUS_CONTROL)
            .px(6.0)
            .child(text("This PC").font(type_::LABEL).color(this_pc_color))
            .on_press(|s: &mut State| { navigate(s, Location::ThisPc); sync_address(s); }),
    );
    if let Location::Folder(path) = &s.location {
        let mut acc = PathBuf::new();
        for comp in path.components() {
            acc.push(comp);
            let label = comp.as_os_str().to_string_lossy().to_string();
            if label.is_empty() || label == "\\" || label == "/" {
                continue;
            }
            let target = acc.clone();
            let color = ink_secondary();
            row = row.child(
                el()
                    .key(format!("bc-{}", label))
                    .radius(tokens::RADIUS_CONTROL)
                    .px(6.0)
                    .child(text(label).font(type_::LABEL).color(color))
                    .on_press(move |s: &mut State| {
                        navigate(s, Location::Folder(target.clone()));
                        sync_address(s);
                    }),
            );
        }
    }
    row
}

fn colhdr_item(label: &'static str, field: u8, s: &State) -> El {
    let active = s.sort_by == field;
    let arrow = if active { if s.sort_desc { " ↓" } else { " ↑" } } else { "" };
    el()
        .key(format!("colhdr-{field}"))
        .grow(if field == 0 { 1.0 } else { 0.0 })
        .w(if field == 0 { 0.0 } else { 80.0 })
        .h(20.0)
        .center_y()
        .child(text(format!("{}{}", label, arrow)).font(type_::LABEL).color(if active { ink() } else { ink_secondary() }))
        .on_press(move |st: &mut State| {
            if st.sort_by == field {
                st.sort_desc = !st.sort_desc;
            } else {
                st.sort_by = field;
                st.sort_desc = false;
            }
            refilter(st);
            save_settings(st);
        })
}

fn ctx_item(label: &'static str, on: impl Fn(&mut State) + 'static) -> El {
    let on = std::rc::Rc::new(on);
    let on2 = on.clone();
    el()
        .key(format!("ctx-{}", label))
        .w_full()
        .h(24.0)
        .px(8.0)
        .center_y()
        .radius(tokens::RADIUS_CONTROL)
        .child(text(label).font(type_::BODY).color(ink()))
        .on_press(move |s: &mut State| on2(s))
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let scale = cx.env().scale;
    if (scale - s.ui_scale).abs() > 0.01 {
        let handle = s.handle.clone();
        handle.update(move |s, _| {
            if (s.ui_scale - scale).abs() > 0.01 {
                s.ui_scale = scale;
                rebuild_bar(s);
            }
        });
    }
    let count = s.filtered.len();
    let total = s.snapshot.len();

    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(bg())
        .padding(12.0)
        .gap(8.0)
        // Tab bar.
        .child(
            row()
                .key("tabs")
                .w_full()
                .h(24.0)
                .gap(4.0)
                .center_y()
                .children((0..s.tabs.len()).map(|i| {
                    let active = i == s.active_tab;
                    el()
                        .key(format!("tab-{i}"))
                        .px(8.0)
                        .h(20.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .fill(if active { selection() } else { surface() })
                        .child(text(format!("Tab {}", i + 1)).font(type_::LABEL).color(ink()))
                        .on_press(move |s: &mut State| {
                            if i < s.tabs.len() {
                                s.active_tab = i;
                                let target = s.tabs[i].clone();
                                navigate(s, target);
                                sync_address(s);
                            }
                        })
                })),
        )
        // Toolbar: Back / Forward / Up + address bar.
        .child(
            row()
                .key("toolbar")
                .w_full()
                .h(32.0)
                .gap(6.0)
                .center_y()
                .child(
                    el()
                        .key("btn-back")
                        .w(28.0).h(28.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .fill(surface())
                        .center()
                        .child(text("◀").font(type_::BODY).color(if s.history_back.is_empty() { ink_secondary() } else { ink() }))
                        .on_press(|s: &mut State| { go_back(s); sync_address(s); }),
                )
                .child(
                    el()
                        .key("btn-fwd")
                        .w(28.0).h(28.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .fill(surface())
                        .center()
                        .child(text("▶").font(type_::BODY).color(if s.history_forward.is_empty() { ink_secondary() } else { ink() }))
                        .on_press(|s: &mut State| { go_forward(s); sync_address(s); }),
                )
                .child(
                    el()
                        .key("btn-up")
                        .w(28.0).h(28.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .fill(surface())
                        .center()
                        .child(text("↑").font(type_::BODY).color(ink()))
                        .on_press(|s: &mut State| {
                            if let Location::Folder(dir) = s.location.clone() {
                                if let Some(parent) = dir.parent().map(Path::to_path_buf) {
                                    navigate(s, Location::Folder(parent));
                                } else {
                                    navigate(s, Location::ThisPc);
                                }
                                sync_address(s);
                            }
                        }),
                )
                .child(
                    text_input(&s.address, type_::BODY)
                        .key("address")
                        .label("Address")
                        .single_line()
                        .grow(1.0)
                        .color(ink())
                        .placeholder("This PC", ink_secondary())
                        .on_edit(|s: &mut State| &mut s.address)
                        .on_input_key(handle_address_key),
                ),
        )
        // Breadcrumbs.
        .child(breadcrumbs_el(s))
        // Location title + status.
        .child(
            row()
                .key("header")
                .w_full()
                .h(20.0)
                .center_y()
                .child(
                    text(location_label(&s.location))
                        .key("path")
                        .font(type_::TITLE)
                        .color(ink())
                        .grow(1.0),
                )
                .child(
                    text(if let Some(op) = &s.op_progress {
                        format!("{} {}/{} · {} failed", op.label, op.done, op.total, op.failures)
                    } else if !s.status.is_empty() {
                        s.status.clone()
                    } else if count == total {
                        format!("{total} items")
                    } else {
                        format!("{count} of {total}")
                    })
                    .key("status")
                    .font(type_::LABEL)
                    .color(ink_secondary()),
                )
                .child(if s.op_progress.is_some() {
                    el().key("op-cancel-btn").w(60.0).h(18.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                        .child(text("Cancel").font(type_::LABEL).color(ink()))
                        .on_press(|s: &mut State| { s.op_cancel.store(true, std::sync::atomic::Ordering::Relaxed); })
                } else {
                    el().key("op-cancel-off").w(0.0).h(0.0)
                }),
        )
        // The signature: Clove's dithered storage bar.
        .child(
            el()
                .key("bar")
                .w_full()
                .h(BAR_H)
                .radius(tokens::RADIUS_BAR)
                .clip_rounded(tokens::RADIUS_BAR)
                .on_layout(|s: &mut State, [_, _, w, _]| {
                    if (s.bar_width - w).abs() >= 0.5 {
                        s.bar_width = w;
                        rebuild_bar(s);
                    }
                })
                .pixels_fit(&s.bar, rune::SurfaceFit::Fill),
        )
        // Kind legend under the bar.
        .child(legend_el(s))
        // Filter field.
        .child(
            row()
                .key("filter-row")
                .w_full()
                .h(32.0)
                .px(8.0)
                .radius(tokens::RADIUS_CONTROL)
                .fill(surface())
                .center_y()
                .child(
                    text_input(&s.query, type_::BODY)
                        .key("filter")
                        .label("Filter files by name")
                        .single_line()
                        .grow(1.0)
                        .color(ink())
                        .placeholder("filter · ↑↓ select · enter open · del trash · ctrl+z undo", ink_secondary())
                        .on_edit(|s: &mut State| &mut s.query)
                        .on_input_change(|s: &mut State, _| refilter(s))
                        .on_input_key(handle_key),
                ),
        )
        .child(row().key("hairline").w_full().h(1.0).fill(hairline()))
        // Search within folder (Ctrl+F).
        .child(if s.search_mode {
            col()
                .key("search-row")
                .w_full()
                .h(32.0)
                .px(8.0)
                .radius(tokens::RADIUS_CONTROL)
                .fill(surface())
                .center_y()
                .child(
                    text_input(&s.search_query, type_::BODY)
                        .key("search")
                        .label("Search recursively")
                        .single_line()
                        .grow(1.0)
                        .color(ink())
                        .placeholder("search files in this folder...", ink_secondary())
                        .on_edit(|s: &mut State| &mut s.search_query)
                        .on_input_change(|s: &mut State, _| trigger_search(s)),
                )
        } else {
            col().key("search-off").w_full().h(0.0)
        })
        // Column headers (Details view).
        .child(if s.view_mode == 0 {
            row()
                .key("colhdr")
                .w_full()
                .h(20.0)
                .px(8.0)
                .center_y()
                .gap(8.0)
                .child(colhdr_item("Name", 0, s))
                .child(colhdr_item("Size", 1, s))
                .child(colhdr_item("Type", 2, s))
                .child(colhdr_item("Modified", 3, s))
        } else {
            row().key("colhdr-off").w_full().h(0.0)
        })
        // Virtualized listing: only mounted rows build.
        .child(
            col()
                .key("viewport")
                .scroll(ScrollSpec { stick_bottom: false })
                .grow(1.0)
                .min_h(0.0)
                .child(if s.search_mode {
                    let mut col = col().key("search-results").w_full().gap(2.0);
                    for (i, p) in s.search_results.iter().take(200).enumerate() {
                        col = col.child(
                            text(p.display().to_string())
                                .key(format!("sr-{i}"))
                                .font(type_::BODY)
                                .color(ink_secondary()),
                        );
                    }
                    if s.search_results.is_empty() {
                        col = col.child(text("no results").font(type_::LABEL).color(ink_secondary()));
                    } else {
                        col = col.child(text(format!("{} results", s.search_results.len())).font(type_::LABEL).color(ink_secondary()));
                    }
                    col
                } else if s.view_mode == 2 {
                    let mut row = row().key("grid").w_full().flex_wrap().gap(8.0);
                    for i in 0..count {
                        let idx = s.filtered[i];
                        let entry = &s.snapshot.entries[idx];
                        let name = entry.name.clone();
                        let path = entry_path(&s.location, entry);
                        let thumb = path.as_ref().and_then(|p| s.thumbnails.get(p));
                        let mut card = col()
                            .key(format!("card-{idx}"))
                            .w(96.0)
                            .h(112.0)
                            .padding(6.0)
                            .gap(4.0)
                            .radius(tokens::RADIUS_CONTROL)
                            .fill(surface())
                            .center_y();
                        if let Some((w, h, rgba)) = thumb {
                            let px = Pixels::new();
                            px.set(*w, *h, rgba.clone());
                            card = card.child(el().w(64.0).h(64.0).pixels_fit(&px, rune::SurfaceFit::Fit));
                        } else {
                            card = card.child(el().w(64.0).h(64.0).fill(bg()).child(text(if entry.is_dir { "📁" } else { "📄" }).font(type_::TITLE).color(ink())));
                        }
                        card = card.child(text(name).font(type_::LABEL).color(ink_secondary()));
                        row = row.child(card);
                    }
                    row
                } else {
                    col()
                        .key("rows")
                        .w_full()
                        .gap(2.0)
                        .growth(GrowthSpec::default())
                        .virtual_rows(
                            count,
                            ROW_H,
                            |s: &State, i| format!("row-{}", s.filtered[i]).into(),
                            |s: &State, i| row_el(s, s.filtered[i]),
                        )
                }),
        )
        // Preview pane (toggle Ctrl+P).
        .child(if s.preview_open {
            let txt = if let Some(&idx) = s.filtered.get(s.cursor) {
                if let Some(entry) = s.snapshot.entries.get(idx) {
                    if let Some(path) = entry_path(&s.location, entry) {
                        if path.is_file() {
                            std::fs::read_to_string(&path)
                                .map(|t| t.chars().take(1500).collect::<String>())
                                .unwrap_or_else(|_| "[binary or unreadable]".to_string())
                        } else {
                            format!("[folder: {}]", path.display())
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            col()
                .key("preview")
                .w_full()
                .h(120.0)
                .padding(8.0)
                .radius(tokens::RADIUS_CONTROL)
                .fill(surface())
                .child(text(txt).font(type_::BODY).color(ink_secondary()))
        } else {
            col().key("preview-off").w_full().h(0.0)
        })
        // Status bar.
        .child(
            row()
                .key("statusbar")
                .w_full()
                .h(20.0)
                .center_y()
                .child(
                    text(if count == 0 {
                        "0 items".to_string()
                    } else {
                        format!("{count} of {total} items")
                    })
                    .key("status-count")
                    .font(type_::LABEL)
                    .color(ink_secondary())
                    .grow(1.0),
                )
                .child(
                    text(if s.show_hidden { "hidden: on" } else { "" })
                        .font(type_::LABEL)
                        .color(ink_secondary()),
                ),
        )
        // Properties dialog (Ctrl+I).
        .child(if s.properties_path.is_some() {
            let path = s.properties_path.clone().unwrap();
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
            let size = s.properties_size;
            let loading = s.properties_loading;
            col()
                .key("props-overlay")
                .w_full()
                .h_full()
                .center()
                .child(
                    col()
                        .key("props-card")
                        .w(420.0)
                        .padding(16.0)
                        .gap(8.0)
                        .radius(tokens::RADIUS_BAR)
                        .fill(surface())
                        .child(text(format!("Properties: {}", name)).font(type_::TITLE).color(ink()))
                        .child(text(format!("Location: {}", path.display())).font(type_::LABEL).color(ink_secondary()))
                        .child(text(format!("Type: {}", if path.is_dir() { "Folder" } else { "File" })).font(type_::LABEL).color(ink_secondary()))
                        .child(text(if loading {
                            "Size: calculating...".to_string()
                        } else if let Some(sz) = size {
                            format!("Size: {}", format_size(sz))
                        } else {
                            "Size: unknown".to_string()
                        }).font(type_::LABEL).color(ink_secondary()))
                        .child(
                            el()
                                .key("props-close")
                                .w(80.0).h(28.0)
                                .radius(tokens::RADIUS_CONTROL)
                                .fill(bg())
                                .center()
                                .child(text("Close").font(type_::BODY).color(ink()))
                                .on_press(|s: &mut State| { s.properties_path = None; }),
                        ),
                )
        } else {
            col().key("props-off").w(0.0).h(0.0)
        })
        // Batch rename modal.
        .child(if s.batch_rename_open {
            let previews = batch_rename_preview(s);
            col()
                .key("batchrename-overlay")
                .w_full()
                .h_full()
                .center()
                .child(
                    col()
                        .key("batchrename-card")
                        .w(460.0)
                        .padding(16.0)
                        .gap(8.0)
                        .radius(tokens::RADIUS_BAR)
                        .fill(surface())
                        .child(text("Batch Rename").font(type_::TITLE).color(ink()))
                        .child(row().gap(4.0)
                            .child(ctx_item("Replace", |_| {})) // placeholder spacer
                        )
                        .child(text("Mode: 0=replace 1=prefix 2=suffix 3=numbering").font(type_::LABEL).color(ink_secondary()))
                        .child(text_input(&s.batch_rename_find, type_::BODY).key("br-find").on_edit(|s: &mut State| &mut s.batch_rename_find))
                        .child(text_input(&s.batch_rename_replace, type_::BODY).key("br-repl").on_edit(|s: &mut State| &mut s.batch_rename_replace))
                        .child(text_input(&s.batch_rename_prefix, type_::BODY).key("br-pref").on_edit(|s: &mut State| &mut s.batch_rename_prefix))
                        .child(text_input(&s.batch_rename_suffix, type_::BODY).key("br-suff").on_edit(|s: &mut State| &mut s.batch_rename_suffix))
                        .child(text(format!("Preview: {} items", previews.len())).font(type_::LABEL).color(ink_secondary()))
                        .child(
                            col().gap(2.0).children(previews.iter().take(8).map(|(_, n)| text(n.clone()).font(type_::BODY).color(ink())).collect::<Vec<_>>())
                        )
                        .child(row().gap(8.0)
                            .child(el().key("br-apply").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                                .child(text("Apply").font(type_::BODY).color(ink()))
                                .on_press(|s: &mut State| { apply_batch_rename(s); }))
                            .child(el().key("br-cancel").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                                .child(text("Cancel").font(type_::BODY).color(ink()))
                                .on_press(|s: &mut State| { s.batch_rename_open = false; })))
                )
        } else {
            col().key("batchrename-off").w(0.0).h(0.0)
        })
        // Open With modal.
        .child(if s.open_with_open {
            col()
                .key("openwith-overlay")
                .w_full()
                .h_full()
                .center()
                .child(
                    col()
                        .key("openwith-card")
                        .w(320.0)
                        .padding(16.0)
                        .gap(6.0)
                        .radius(tokens::RADIUS_BAR)
                        .fill(surface())
                        .child(text("Open With").font(type_::TITLE).color(ink()))
                        .child(if s.open_with_apps.is_empty() {
                            text("No applications found").font(type_::LABEL).color(ink_secondary())
                        } else {
                            let apps = s.open_with_apps.clone();
                            let target = s.open_with_path.clone();
                            col().gap(2.0).children(apps.into_iter().map(|(name, desktop)| {
                                let d = desktop.clone();
                                let target2 = target.clone();
                                let name2 = name.clone();
                                el()
                                    .key(format!("ow-{}", name))
                                    .w_full()
                                    .h(24.0)
                                    .px(8.0)
                                    .center_y()
                                    .radius(tokens::RADIUS_CONTROL)
                                    .child(text(name).font(type_::BODY).color(ink()))
                                    .on_press(move |s: &mut State| {
                                        if let Some(p) = &target2 {
                                            launch_desktop_app(&d, p);
                                            s.status = format!("opened with {}", name2);
                                        }
                                        s.open_with_open = false;
                                    })
                            }).collect::<Vec<_>>())
                        })
                        .child(el().key("ow-close").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                            .child(text("Close").font(type_::BODY).color(ink()))
                            .on_press(|s: &mut State| { s.open_with_open = false; }))
                )
        } else {
            col().key("openwith-off").w(0.0).h(0.0)
        })
        // Conflict resolution modal.
        .child(if s.pending_paste.is_some() {
            let (target, paths, cut) = s.pending_paste.clone().unwrap();
            let collisions: Vec<String> = paths.iter().filter_map(|src| {
                let name = src.file_name()?.to_string_lossy().to_string();
                if target.join(&name).exists() { Some(name) } else { None }
            }).collect();
            col()
                .key("conflict-overlay")
                .w_full()
                .h_full()
                .center()
                .child(
                    col()
                        .key("conflict-card")
                        .w(360.0)
                        .padding(16.0)
                        .gap(8.0)
                        .radius(tokens::RADIUS_BAR)
                        .fill(surface())
                        .child(text("Resolve Conflicts").font(type_::TITLE).color(ink()))
                        .child(text(format!("{} item(s) collide", collisions.len())).font(type_::LABEL).color(ink_secondary()))
                        .child(col().gap(2.0).children(collisions.into_iter().take(6).map(|n| text(n).font(type_::BODY).color(ink())).collect::<Vec<_>>()))
                        .child(row().gap(6.0)
                            .child(el().key("cf-replace").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                                .child(text("Replace").font(type_::BODY).color(ink()))
                                .on_press(move |s: &mut State| {
                                    let (t, p, c) = s.pending_paste.take().unwrap();
                                    run_paste(s, t, p, c, 1);
                                }))
                            .child(el().key("cf-skip").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                                .child(text("Skip").font(type_::BODY).color(ink()))
                                .on_press(move |s: &mut State| {
                                    let (t, p, c) = s.pending_paste.take().unwrap();
                                    run_paste(s, t, p, c, 2);
                                }))
                            .child(el().key("cf-rename").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                                .child(text("Rename").font(type_::BODY).color(ink()))
                                .on_press(move |s: &mut State| {
                                    let (t, p, c) = s.pending_paste.take().unwrap();
                                    run_paste(s, t, p, c, 0);
                                }))
                        )
                        .child(el().key("cf-cancel").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                            .child(text("Cancel").font(type_::BODY).color(ink()))
                            .on_press(|s: &mut State| { s.pending_paste = None; s.status = "cancelled".into(); }))
                )
        } else {
            col().key("conflict-off").w(0.0).h(0.0)
        })
        // Context menu (right-click).
        .child(if s.context_menu.is_some() {
            let idx = s.context_menu.unwrap();
            col()
                .key("ctx-overlay")
                .w_full()
                .h_full()
                .center()
                .child(
                    col()
                        .key("ctx-card")
                        .w(220.0)
                        .padding(8.0)
                        .gap(4.0)
                        .radius(tokens::RADIUS_CONTROL)
                        .fill(surface())
                        .child(ctx_item("Open", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    if entry.is_dir {
                                        navigate(s, Location::Folder(path));
                                        sync_address(s);
                                    } else {
                                        platform::open_path(&path);
                                        s.status = format!("opening {}", entry.name);
                                    }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Open With...", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    let apps = open_with_candidates(&path);
                                    s.open_with_path = Some(path);
                                    s.open_with_apps = apps;
                                    s.open_with_open = true;
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Copy", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    s.clipboard = vec![path.clone()];
                                    s.clipboard_cut = false;
                                    let p2 = path.clone();
                                    std::thread::spawn(move || { if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set().file_list(&[p2]); } });
                                    s.status = "copied 1 item".into();
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Cut", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    s.clipboard = vec![path.clone()];
                                    s.clipboard_cut = true;
                                    let p2 = path.clone();
                                    std::thread::spawn(move || { if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set().file_list(&[p2]); } });
                                    s.status = "cut 1 item".into();
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Paste", move |s: &mut State| {
                            do_paste(s);
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Reveal in Folder", move |s: &mut State| {
                            let mut parent_opt = None;
                            let mut name = String::new();
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    if let Some(parent) = path.parent() {
                                        parent_opt = Some(parent.to_path_buf());
                                        name = entry.name.clone();
                                    }
                                }
                            }
                            if let Some(parent) = parent_opt {
                                navigate(s, Location::Folder(parent));
                                sync_address(s);
                                s.status = format!("revealed {}", name);
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Copy Path", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    let text = path.to_string_lossy().to_string();
                                    std::thread::spawn(move || { if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(text); } });
                                    s.status = "path copied".into();
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("New File", move |s: &mut State| {
                            if let Location::Folder(root) = s.location.clone() {
                                let mut name = "New file".to_string();
                                let mut n = 1u32;
                                while root.join(&name).exists() { n += 1; name = format!("New file {n}"); }
                                let dest = root.join(&name);
                                match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
                                    Ok(_) => { s.status = format!("created {name}"); reload(s); }
                                    Err(_) => { s.status = "could not create file".into(); }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("New Folder", move |s: &mut State| {
                            if let Location::Folder(root) = s.location.clone() {
                                let mut name = "New folder".to_string();
                                let mut n = 1u32;
                                while root.join(&name).exists() { n += 1; name = format!("New folder {n}"); }
                                let dest = root.join(&name);
                                match std::fs::create_dir(&dest) {
                                    Ok(_) => { s.status = format!("created {name}"); reload(s); }
                                    Err(_) => { s.status = "could not create folder".into(); }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Terminal Here", move |s: &mut State| {
                            let dir = match &s.location {
                                Location::Folder(d) => d.clone(),
                                Location::ThisPc => platform::home_dir(),
                            };
                            platform::open_terminal_here(&dir);
                            s.status = "opening terminal".into();
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Delete", move |s: &mut State| {
                            if let Some(e) = s.snapshot.entries.get(idx) {
                                if let Some(p) = entry_path(&s.location, e) {
                                    let root = match &s.location { Location::Folder(d) => d.clone(), _ => { s.context_menu = None; return; } };
                                    let name = e.name.clone();
                                    let handle = s.handle.clone();
                                    std::thread::spawn(move || {
                                        let outcome = clove_shell::trash(&p, &root, false);
                                        handle.update(move |s, _| {
                                            if !outcome.moves.is_empty() {
                                                s.undo_stack.push(UndoAction::Trash { moves: outcome.moves });
                                                s.status = format!("moved {name} to Trash · Ctrl+Z to undo");
                                            }
                                            reload(s);
                                        });
                                    });
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Rename (F2)", move |s: &mut State| {
                            s.renaming = Some(idx);
                            if let Some(e) = s.snapshot.entries.get(idx) {
                                s.rename_text = TextEdit::new(&e.name);
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Batch Rename...", move |s: &mut State| {
                            s.batch_rename_open = true;
                            s.batch_rename_find = TextEdit::new("");
                            s.batch_rename_replace = TextEdit::new("");
                            s.batch_rename_prefix = TextEdit::new("");
                            s.batch_rename_suffix = TextEdit::new("");
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Compress", move |s: &mut State| {
                            // Collect selected paths (or cursor).
                            let mut paths = Vec::new();
                            for &idx in &s.selected {
                                if let Some(e) = s.snapshot.entries.get(idx) {
                                    if let Some(p) = entry_path(&s.location, e) { paths.push(p); }
                                }
                            }
                            if paths.is_empty() {
                                if let Some(entry) = s.snapshot.entries.get(idx) {
                                    if let Some(p) = entry_path(&s.location, entry) { paths.push(p); }
                                }
                            }
                            if !paths.is_empty() {
                                let target = match &s.location { Location::Folder(d) => d.clone(), _ => PathBuf::from(".") };
                                let name = paths.first().and_then(|p| p.file_name()).map(|n| format!("{}.tar.gz", n.to_string_lossy())).unwrap_or_else(|| "archive.tar.gz".into());
                                let dest = target.join(name);
                                let handle = s.handle.clone();
                                s.status = "compressing...".into();
                                std::thread::spawn(move || {
                                    // Use tar with -C parent and relative names.
                                    let mut args = vec!["czf".to_string(), dest.to_string_lossy().to_string()];
                                    for p in &paths {
                                        args.push(p.to_string_lossy().to_string());
                                    }
                                    let res = std::process::Command::new("tar").args(&args).status();
                                    handle.update(move |s, _| {
                                        s.status = if res.map(|r| r.success()).unwrap_or(false) { "compressed".into() } else { "compress failed".into() };
                                        reload(s);
                                    });
                                });
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Extract Here", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    let lower = path.to_string_lossy().to_lowercase();
                                    let is_archive = lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".tar") || lower.ends_with(".tar.bz2") || lower.ends_with(".tar.xz") || lower.ends_with(".7z");
                                    if is_archive {
                                        let target = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
                                        let handle = s.handle.clone();
                                        let p = path.clone();
                                        s.status = "extracting...".into();
                                        std::thread::spawn(move || {
                                            let res = if p.to_string_lossy().ends_with(".zip") {
                                                std::process::Command::new("unzip").args(["-o", &p.to_string_lossy(), "-d", &target.to_string_lossy()]).status()
                                            } else {
                                                std::process::Command::new("tar").args(["xf", &p.to_string_lossy(), "-C", &target.to_string_lossy()]).status()
                                            };
                                            handle.update(move |s, _| {
                                                s.status = if res.map(|r| r.success()).unwrap_or(false) { "extracted".into() } else { "extract failed".into() };
                                                reload(s);
                                            });
                                        });
                                    } else {
                                        s.status = "not an archive".into();
                                    }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Properties", move |s: &mut State| {
                            if let Some(e) = s.snapshot.entries.get(idx) {
                                if let Some(p) = entry_path(&s.location, e) {
                                    s.properties_path = Some(p);
                                    s.properties_size = None;
                                    s.properties_loading = true;
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Toggle Conflict Policy", move |s: &mut State| {
                            s.conflict_policy = if s.conflict_policy == 0 { 1 } else { 0 };
                            s.status = if s.conflict_policy == 0 { "conflicts: auto-rename".into() } else { "conflicts: ask".into() };
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Close", move |s: &mut State| {
                            s.context_menu = None;
                        })),
                )
        } else {
            col().key("ctx-off").w(0.0).h(0.0)
        })
}

// ---------------------------------------------------------------- main

fn settings_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Clove").join("settings.json"))
    }
    #[cfg(not(windows))]
    {
        Some(platform::config_dir().join("clove").join("settings.json"))
    }
}

fn load_settings() -> Option<(u8, bool, bool, u8, bool)> {
    let path = settings_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    // Very small manual parser: key:value pairs.
    let mut sort_by = 0u8;
    let mut sort_desc = false;
    let mut folders_first = true;
    let mut view_mode = 0u8;
    let mut show_hidden = false;
    for line in text.lines() {
        let line = line.trim().trim_matches(|c| c == '{' || c == '}' || c == ',');
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().trim_matches('"');
            let v = v.trim().trim_matches(',').trim();
            match k {
                "sort_by" => sort_by = v.parse().unwrap_or(0),
                "sort_desc" => sort_desc = v == "true",
                "folders_first" => folders_first = v == "true",
                "view_mode" => view_mode = v.parse().unwrap_or(0),
                "show_hidden" => show_hidden = v == "true",
                _ => {}
            }
        }
    }
    Some((sort_by, sort_desc, folders_first, view_mode, show_hidden))
}

fn save_settings(s: &State) {
    if let Some(path) = settings_path() {
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        let text = format!(
            "{{\n  \"sort_by\": {},\n  \"sort_desc\": {},\n  \"folders_first\": {},\n  \"view_mode\": {},\n  \"show_hidden\": {}\n}}",
            s.sort_by, s.sort_desc, s.folders_first, s.view_mode, s.show_hidden
        );
        let _ = std::fs::write(path, text);
    }
}

fn storage_cache_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Clove").join("storage-scan.txt"))
    }
    #[cfg(not(windows))]
    {
        Some(platform::cache_dir().join("clove").join("storage-scan.txt"))
    }
}

fn load_storage_cache() -> Option<([u64; KIND_COUNT], u64, u64)> {
    let path = storage_cache_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut parts = text.trim().split(',');
    let scanned_at: u64 = parts.next()?.parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now.saturating_sub(scanned_at) > 30 * 60 {
        return None;
    }
    let capacity: u64 = parts.next()?.parse().ok()?;
    let used: u64 = parts.next()?.parse().ok()?;
    let mut bytes = [0u64; KIND_COUNT];
    for slot in bytes.iter_mut() {
        *slot = parts.next()?.parse().ok()?;
    }
    Some((bytes, capacity, used))
}

fn save_storage_cache(bytes: [u64; KIND_COUNT], capacity: u64, used: u64) {
    let Some(path) = storage_cache_path() else { return };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut line = format!("{now},{capacity},{used}");
    for b in bytes {
        line.push_str(&format!(",{b}"));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, line);
}

fn start_dir() -> PathBuf {
    // `CLOVE_START_DIR` lets a headless `--script` run open a sandbox directory, which is how
    // the filter/trash/undo paths are exercised without a window.
    if let Ok(dir) = std::env::var("CLOVE_START_DIR") {
        return PathBuf::from(dir);
    }
    // A file manager opens on the main drive, not the user's profile.
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    PathBuf::from(format!("{}\\", drive.trim_end_matches('\\')))
}

fn build_state(handle: AppHandle<State>) -> State {
    let location = Location::ThisPc;

    let (sort_by, sort_desc, folders_first, view_mode, show_hidden) =
        load_settings().unwrap_or((0, false, true, 0, false));

    let cached = load_storage_cache();
    let (storage_bytes, storage_capacity, storage_used, storage_has_data, storage_ready, scan_started) =
        if let Some((bytes, capacity, used)) = cached {
            (
                bytes,
                capacity,
                used,
                true,
                true,
                true, // skip the scan
            )
        } else {
            ([0u64; KIND_COUNT], 0, 0, false, false, false)
        };

    let spec = if storage_has_data {
        // Build from cached numbers immediately.
        let total: u64 = storage_bytes.iter().sum();
        let capacity = if storage_capacity > 0 {
            storage_capacity
        } else {
            total.max(1)
        };
        let segments = storage_bytes
            .iter()
            .enumerate()
            .map(|(kind, &bytes)| Segment::kind(kind, bytes as f32 / capacity as f32))
            .collect();
        BarSpec::new(936.0, segments)
            .with_height(BAR_H)
            .with_scale(2.0)
    } else {
        BarSpec::placeholder(936.0)
            .with_height(BAR_H)
            .with_scale(2.0)
    };
    let mut bar_renderer = BarRenderer::new(spec);
    bar_renderer.render(0.0);
    let (w, h) = bar_renderer.size();
    let bar = Pixels::new();
    bar.set(w as u32, h as u32, bar_renderer.buffer().to_vec());

    let pc_entries = this_pc_entries();
    let filtered: Vec<usize> = (0..pc_entries.len())
        .filter(|&i| show_hidden || !(pc_entries[i].hidden || pc_entries[i].system))
        .collect();
    let mut selected = std::collections::HashSet::new();
    if let Some(&i) = filtered.first() {
        selected.insert(i);
    }

    State {
        location,
        history_back: Vec::new(),
        history_forward: Vec::new(),
        snapshot: Arc::new(Snapshot::new(PathBuf::new(), pc_entries)),
        filtered,
        query: TextEdit::new(""),
        address: TextEdit::new("This PC"),
        selected,
        cursor: 0,
        anchor: None,
        clipboard: Vec::new(),
        clipboard_cut: false,
        sort_by,
        sort_desc,
        folders_first,
        view_mode,
        preview_open: false,
        search_mode: false,
        search_query: TextEdit::new(""),
        search_results: Vec::new(),
        properties_path: None,
        properties_size: None,
        properties_loading: false,
        renaming: None,
        rename_text: TextEdit::new(""),
        context_menu: None,
        conflict_policy: 0,
        pending_paste: None,
        batch_rename_open: false,
        open_with_open: false,
        open_with_path: None,
        open_with_apps: Vec::new(),
        batch_rename_mode: 0,
        batch_rename_find: TextEdit::new(""),
        batch_rename_replace: TextEdit::new(""),
        batch_rename_prefix: TextEdit::new(""),
        batch_rename_suffix: TextEdit::new(""),
        thumbnails: std::collections::HashMap::new(),
        thumbnail_pending: std::collections::HashSet::new(),
        tabs: vec![Location::ThisPc],
        active_tab: 0,
        show_hidden,
        status: String::new(),
        loading: false,
        undo_stack: Vec::new(),
        op_progress: None,
        op_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        bar,
        bar_renderer,
        bar_time: 0.0,
        bar_until: 2.5,
        focus_attempts: 0,
        generation: Arc::new(AtomicU64::new(0)),
        handle,
        storage_bytes,
        storage_capacity,
        storage_used,
        scan: None,
        scan_started,
        storage_ready,
        storage_has_data,
        bar_width: 936.0,
        ui_scale: 2.0,
    }
}

fn main() {
    let app = App::new(AppSpec {
        poll: Some(poll),
        on_key: Some(handle_key),
        ..AppSpec::new("Clove", (960.0, 680.0), || unreachable!(), view)
    });
    let handle = app.handle();
    let app = app.init({
        let handle = handle.clone();
        move || build_state(handle)
    });

    let boot = app.handle();
    if let Ok(dir) = std::env::var("CLOVE_START_DIR") {
        let dir = PathBuf::from(dir);
        std::thread::spawn(move || {
            boot.update(move |s, _| {
                navigate(s, Location::Folder(dir));
            });
        });
    }

    app.run();
}
