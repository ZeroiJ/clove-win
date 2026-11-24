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
    col, el, row, text, text_input, App, AppHandle, AppSpec, Color, Cx, DragItem, El, GrowthSpec,
    Key, KeyEvent, Pixels, ScrollSpec, TextEdit, Tx,
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
    Rename { from: PathBuf, to: PathBuf },
    Copy { created: Vec<PathBuf> },
    Move { pairs: Vec<(PathBuf, PathBuf)> },
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
    git_branch: Option<String>,
    git_dirty: bool,
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
    properties_checksum: Option<String>,
    properties_mime: Option<String>,
    properties_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    renaming: Option<usize>,
    rename_text: TextEdit,
    context_menu: Option<usize>,
    drop_hover: bool,
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
    thumbnail_bytes: usize,
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
    show_shortcuts: bool,
    keybinds: Vec<(String, String)>,
    customizing: Option<usize>,
    custom_key: Option<String>,
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
    refresh_git(s);
    if s.tabs.is_empty() { s.tabs.push(s.location.clone()); }
    if s.active_tab < s.tabs.len() { s.tabs[s.active_tab] = s.location.clone(); }
    save_tabs(s);
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
                    reload(s);
                });
            }
            UndoAction::Rename { from, to } => {
                let _ = std::fs::rename(&to, &from);
                handle.update(move |s, _| {
                    s.status = "undo rename".into();
                    s.wake_bar();
                    reload(s);
                });
            }
            UndoAction::Copy { created } => {
                for p in &created {
                    if p.is_dir() { let _ = std::fs::remove_dir_all(p); } else { let _ = std::fs::remove_file(p); }
                }
                handle.update(move |s, _| {
                    s.status = "undo copy".into();
                    s.wake_bar();
                    reload(s);
                });
            }
            UndoAction::Move { pairs } => {
                for (src, dest) in &pairs {
                    let _ = std::fs::rename(dest, src);
                }
                handle.update(move |s, _| {
                    s.status = "undo move".into();
                    s.wake_bar();
                    reload(s);
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
fn unique_link_path(parent: &Path, target: &Path, kind: &str) -> PathBuf {
    let stem = target.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "link".into());
    let ext = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut dest = parent.join(format!("{} ({}{}){}", stem, kind, "", ext));
    let mut n = 1u32;
    while dest.exists() && n < 1000 {
        dest = parent.join(format!("{} ({} {})({}){}", stem, kind, n, "", ext));
        n += 1;
    }
    dest
}

fn create_symlink(target: &Path, dest: &Path) -> bool {
    #[cfg(unix)]
    { return std::os::unix::fs::symlink(target, dest).is_ok(); }
    #[cfg(windows)]
    {
        if target.is_dir() { return std::os::windows::fs::symlink_dir(target, dest).is_ok(); }
        return std::os::windows::fs::symlink_file(target, dest).is_ok();
    }
    #[cfg(not(any(unix, windows)))]
    { let _ = (target, dest); false }
}

fn is_archive(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
        || lower.ends_with(".tar") || lower.ends_with(".tar.bz2") || lower.ends_with(".tar.xz")
        || lower.ends_with(".7z") || lower.ends_with(".rar")
}

fn extract_archive(src: &Path, target: &Path) -> bool {
    let _ = std::fs::create_dir_all(target);
    let lower = src.to_string_lossy().to_lowercase();
    let res = if lower.ends_with(".zip") {
        std::process::Command::new("unzip").args(["-o", &src.to_string_lossy(), "-d", &target.to_string_lossy()]).status()
    } else if lower.ends_with(".7z") {
        std::process::Command::new("7z").args(["x", "-y", &format!("-o{}", target.to_string_lossy()), &src.to_string_lossy()]).status()
    } else if lower.ends_with(".rar") {
        std::process::Command::new("unrar").args(["x", "-y", &src.to_string_lossy(), &target.to_string_lossy()]).status()
    } else {
        std::process::Command::new("tar").args(["xf", &src.to_string_lossy(), "-C", &target.to_string_lossy()]).status()
    };
    res.map(|r| r.success()).unwrap_or(false)
}

fn run_progress(cmd: &mut std::process::Command, cancel: &std::sync::atomic::AtomicBool, handle: &AppHandle<State>) -> bool {
    // GNU tar prints verbose names on stderr; unzip and 7z use stdout.
    let use_stderr = cmd.get_program().to_string_lossy() == "tar";
    if use_stderr {
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());
    } else {
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());
    }
    let mut child = match cmd.spawn() { Ok(c) => c, Err(_) => return false };
    let stream: Option<Box<dyn std::io::Read + Send>> = if use_stderr {
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)
    } else {
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>)
    };
    let h = handle.clone();
    let reader = std::thread::spawn(move || {
        let mut count = 0usize;
        if let Some(out) = stream {
            use std::io::BufRead;
            for _ in std::io::BufReader::new(out).lines() {
                count += 1;
                let c = count;
                h.update(move |s, _| {
                    if let Some(op) = &mut s.op_progress { op.done = c; }
                });
            }
        }
        count
    });
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => { let _ = reader.join(); return status.success(); }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => { let _ = reader.join(); return false; }
        }
    }
}

fn count_files(path: &Path) -> usize {
    if path.is_dir() {
        std::fs::read_dir(path).map(|rd| rd.flatten().map(|e| count_files(&e.path())).sum()).unwrap_or(0)
    } else { 1 }
}

fn archive_entry_count(path: &Path) -> usize {
    let lower = path.to_string_lossy().to_lowercase();
    let out = if lower.ends_with(".zip") {
        std::process::Command::new("unzip").args(["-Z1", &path.to_string_lossy()]).output().ok()
    } else {
        std::process::Command::new("tar").args(["tf", &path.to_string_lossy()]).output().ok()
    };
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0)
}

fn file_checksum(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256Hasher::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Some(hasher.finish_hex())
}

struct Sha256Hasher {
    state: [u32; 8],
    bit_len: u64,
    buffer: Vec<u8>,
}

impl Sha256Hasher {
    fn new() -> Self {
        Self {
            state: [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19],
            bit_len: 0,
            buffer: Vec::new(),
        }
    }
    fn update(&mut self, data: &[u8]) {
        self.bit_len += (data.len() as u64) * 8;
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let chunk: [u8; 64] = self.buffer[..64].try_into().unwrap();
            self.buffer.drain(..64);
            self.compress(chunk);
        }
    }
    fn finish_hex(mut self) -> String {
        let mut padded = self.buffer.clone();
        padded.push(0x80);
        while (padded.len() % 64) != 56 { padded.push(0); }
        padded.extend_from_slice(&self.bit_len.to_be_bytes());
        while padded.len() >= 64 {
            let chunk: [u8; 64] = padded[..64].try_into().unwrap();
            padded.drain(..64);
            self.compress(chunk);
        }
        let mut out = String::with_capacity(64);
        for w in self.state { out.push_str(&format!("{:08x}", w)); }
        out
    }
    fn compress(&mut self, chunk: [u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
            0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
            0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
            0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
            0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
            0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
            0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
            0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i*4..i*4+4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let [mut a,mut b,mut c,mut d,mut e,mut f,mut g,mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h=g; g=f; f=e; e=d.wrapping_add(t1); d=c; c=b; b=a; a=t1.wrapping_add(t2);
        }
        self.state[0]=self.state[0].wrapping_add(a);
        self.state[1]=self.state[1].wrapping_add(b);
        self.state[2]=self.state[2].wrapping_add(c);
        self.state[3]=self.state[3].wrapping_add(d);
        self.state[4]=self.state[4].wrapping_add(e);
        self.state[5]=self.state[5].wrapping_add(f);
        self.state[6]=self.state[6].wrapping_add(g);
        self.state[7]=self.state[7].wrapping_add(h);
    }
}

fn collect_files_recursive(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else { return; };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { collect_files_recursive(&p, out); }
        else { out.push(p); }
    }
}

fn find_duplicates(root: &Path) -> Vec<(String, Vec<PathBuf>)> {
    let mut files = Vec::new();
    collect_files_recursive(root, &mut files);
    let mut by_size: std::collections::HashMap<u64, Vec<PathBuf>> = std::collections::HashMap::new();
    for p in files {
        if let Ok(md) = p.metadata() { by_size.entry(md.len()).or_default().push(p); }
    }
    let mut out = Vec::new();
    for (_sz, group) in by_size {
        if group.len() < 2 { continue; }
        let mut by_hash: std::collections::HashMap<String, Vec<PathBuf>> = std::collections::HashMap::new();
        for p in group {
            if let Some(h) = file_checksum(&p) { by_hash.entry(h).or_default().push(p); }
        }
        for (h, g) in by_hash { if g.len() > 1 { out.push((h, g)); } }
    }
    out
}

fn compare_dirs(a: &Path, b: &Path) -> (usize, usize, usize) {
    fn walk(root: &Path, base: &Path, out: &mut std::collections::HashMap<String, u64>) {
        let Ok(rd) = std::fs::read_dir(root) else { return; };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { walk(&p, base, out); }
            else if let Ok(rel) = p.strip_prefix(base) {
                out.insert(rel.to_string_lossy().to_string(), e.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }
    let mut ma = std::collections::HashMap::new();
    let mut mb = std::collections::HashMap::new();
    walk(a, a, &mut ma);
    walk(b, b, &mut mb);
    let mut only_a = 0usize; let mut only_b = 0usize; let mut diff = 0usize;
    for (k, va) in &ma {
        match mb.get(k) { None => only_a += 1, Some(vb) if vb != va => diff += 1, _ => {} }
    }
    for k in mb.keys() { if !ma.contains_key(k) { only_b += 1; } }
    (only_a, only_b, diff)
}

fn props_permissions(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = path.metadata() {
            let m = md.permissions().mode();
            return format!("{:03o}", m & 0o777);
        }
    }
    #[cfg(not(unix))]
    { let _ = path; }
    "unknown".to_string()
}

fn props_owner(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(md) = path.metadata() {
            return format!("uid {}", md.uid());
        }
    }
    #[cfg(not(unix))]
    { let _ = path; }
    "unknown".to_string()
}

fn dir_size(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> u64 {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) { return 0; }
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(path) {
        for entry in rd.flatten() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) { return total; }
            let p = entry.path();
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    total += dir_size(&p, cancel);
                } else {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}

fn refresh_git(s: &mut State) {
    let dir = match &s.location {
        Location::Folder(d) => d.clone(),
        Location::ThisPc => return,
    };
    // find repo root
    let out = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output();
    let root = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => { s.git_branch = None; s.git_dirty = false; return; }
    };
    if root.is_empty() { s.git_branch = None; s.git_dirty = false; return; }
    let branch_out = std::process::Command::new("git")
        .args(["-C", &root, "rev-parse", "--abbrev-ref", "HEAD"])
        .output();
    let branch = branch_out.ok().and_then(|o| if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None });
    let dirty_out = std::process::Command::new("git")
        .args(["-C", &root, "status", "--porcelain"])
        .output();
    let dirty = dirty_out.map(|o| !o.stdout.is_empty()).unwrap_or(false);
    s.git_branch = branch;
    s.git_dirty = dirty;
}

fn thumbnail_external(path: &Path) -> Option<image::DynamicImage> {
    // shortcut: external ffmpeg/pdftoppm, silent fallback when absent.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "mov" | "avi") {
        let out = std::process::Command::new("ffmpeg")
            .args(["-y", "-ss", "1", "-i"]).arg(path)
            .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
            .output().ok()?;
        if !out.status.success() || out.stdout.is_empty() { return None; }
        return image::load_from_memory(&out.stdout).ok();
    }
    if ext == "pdf" {
        let prefix = platform::cache_dir().join("clove").join("thumbs").join(format!("pdf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(prefix.parent()?);
        let status = std::process::Command::new("pdftoppm")
            .args(["-png", "-singlefile", "-f", "1", "-l", "1"])
            .arg(path).arg(&prefix).status().ok()?;
        if !status.success() { return None; }
        let img = image::open(prefix.with_extension("png")).ok();
        let _ = std::fs::remove_file(prefix.with_extension("png"));
        return img;
    }
    None
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
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "mp4" | "mkv" | "webm" | "mov" | "avi" | "pdf") {
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
            if let Some(img) = image::open(&path2).ok().or_else(|| thumbnail_external(&path2)) {
                let thumb = img.thumbnail(64, 64);
                let rgba8 = thumb.to_rgba8();
                dims = Some((rgba8.width(), rgba8.height()));
                rgba = Some(rgba8.into_raw());
            }
            handle.update(move |s, _| {
                s.thumbnail_pending.remove(&path2);
                if let (Some(d), Some(r)) = (dims, rgba) {
                    let size = r.len();
                    s.thumbnail_bytes += size;
                    s.thumbnails.insert(path2, (d.0, d.1, r));
                    // shortcut: simple eviction by dropping random keys, replace with LRU when thumb cache grows
                    const MAX_THUMB_BYTES: usize = 64 * 1024 * 1024;
                    while s.thumbnail_bytes > MAX_THUMB_BYTES {
                        let Some(k) = s.thumbnails.keys().next().cloned() else { break; };
                        if let Some((_, _, v)) = s.thumbnails.remove(&k) { s.thumbnail_bytes = s.thumbnail_bytes.saturating_sub(v.len()); }
                    }
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

fn handle_drop_files(s: &mut State, paths: &[std::path::PathBuf], _tx: &mut Tx) {
    if let Location::Folder(dir) = s.location.clone() {
        if !paths.is_empty() {
            s.status = format!("dropped {} file(s)", paths.len());
            run_paste(s, dir, paths.to_vec(), false, 0);
        }
    }
    s.drop_hover = false;
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
        let mut created: Vec<PathBuf> = Vec::new();
        let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
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
            if res.is_ok() {
                moved += 1;
                if cut { pairs.push((src.clone(), dest.clone())); } else { created.push(dest.clone()); }
            } else { failures += 1; }
            let done = i + 1;
            handle.update(move |s, _| {
                if let Some(op) = &mut s.op_progress {
                    op.done = done;
                    op.failures = failures;
                }
            });
        }
        let undo_action = if cut {
            Some(UndoAction::Move { pairs })
        } else if !created.is_empty() {
            Some(UndoAction::Copy { created })
        } else { None };
        handle.update(move |s, _| {
            s.op_progress = None;
            s.status = format!("pasted {} · skipped {} · failed {}", moved, skipped, failures);
            s.wake_bar();
            if let Some(a) = undo_action { s.undo_stack.push(a); }
            if cut { s.clipboard.clear(); s.clipboard_cut = false; }
            reload(s);
        });
    });
}


fn ev_is_command(ev: &KeyEvent, c: char) -> bool {
    matches!(ev.key, Key::Command(k) if k.to_ascii_lowercase() == c.to_ascii_lowercase())
}

fn ev_char(ev: &KeyEvent) -> Option<char> {
    match &ev.key {
        Key::Command(c) => Some(c.to_ascii_lowercase()),
        Key::Text(t) => t.chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    }
}

fn ev_name(ev: &KeyEvent) -> String {
    match &ev.key {
        Key::Function(n) => format!("f{n}"),
        Key::Enter => "enter".into(),
        Key::Backspace => "backspace".into(),
        Key::Delete => "delete".into(),
        Key::Escape => "escape".into(),
        Key::Tab => "tab".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        Key::PageUp => "pageup".into(),
        Key::PageDown => "pagedown".into(),
        Key::Command(c) => c.to_ascii_lowercase().to_string(),
        Key::Text(c) => c.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn ev_canonical(ev: &KeyEvent) -> String {
    let mut parts = Vec::new();
    if ev.mods.ctrl { parts.push("ctrl"); }
    if ev.mods.alt { parts.push("alt"); }
    if ev.mods.shift { parts.push("shift"); }
    if ev.mods.cmd { parts.push("cmd"); }
    let name = ev_name(ev);
    if name.is_empty() { return String::new(); }
    parts.push(&name);
    // leak-free: build owned
    let mut out = String::new();
    for (i,p) in parts.iter().enumerate() {
        if i>0 { out.push('+'); }
        out.push_str(p);
    }
    out
}

fn kb_canonical(kb: &str) -> String {
    let b = kb.trim().to_ascii_lowercase();
    let mut ctrl=false; let mut alt=false; let mut shift=false; let mut cmd=false;
    let mut key = String::new();
    for part in b.split('+') {
        match part.trim() {
            "ctrl"|"control" => ctrl=true,
            "alt" => alt=true,
            "shift" => shift=true,
            "cmd"|"meta"|"super"|"command" => cmd=true,
            k => key = k.to_string(),
        }
    }
    if key.is_empty() { return String::new(); }
    let mut out = String::new();
    if ctrl { out.push_str("ctrl+"); }
    if alt { out.push_str("alt+"); }
    if shift { out.push_str("shift+"); }
    if cmd { out.push_str("cmd+"); }
    out.push_str(&key);
    out
}

fn key_is_bound(s: &State, ev: &KeyEvent) -> Option<String> {
    let evc = ev_canonical(ev);
    if evc.is_empty() { return None; }
    for (action, kb) in &s.keybinds {
        if kb_canonical(kb) == evc { return Some(action.clone()); }
    }
    None
}

fn handle_action(s: &mut State, action: &str) -> bool {
    match action {
        "undo" => { undo_delete(s); true }
        "copy" => { do_copy(s); true }
        "cut" => {
            do_copy(s);
            s.clipboard_cut = true;
            let n = s.clipboard.len();
            if n > 0 { s.status = format!("cut {n} item(s)"); }
            true
        }
        "paste" => { do_paste(s); true }
        "new-file" => { do_new_file(s); true }
        "new-folder" => { do_new_folder(s); true }
        "rename" => { start_rename(s); true }
        "delete" => { delete_selected(s); true }
        "delete-permanent" => { do_permanent_delete(s); true }
        "properties" => { open_properties(s); true }
        "search" => { s.search_mode = !s.search_mode; save_settings(s); true }
        "hidden" => {
            s.show_hidden = !s.show_hidden;
            refilter(s);
            save_settings(s);
            s.status = if s.show_hidden { "showing hidden files".into() } else { "hidden files hidden".into() };
            true
        }
        "shortcuts" => { s.show_shortcuts = !s.show_shortcuts; true }
        "new-tab" => { s.tabs.push(s.location.clone()); s.active_tab = s.tabs.len() - 1; save_tabs(s); true }
        "close-tab" => {
            if s.tabs.len() > 1 {
                s.tabs.remove(s.active_tab);
                if s.active_tab >= s.tabs.len() { s.active_tab = s.tabs.len() - 1; }
                let target = s.tabs[s.active_tab].clone();
                navigate(s, target);
                sync_address(s);
                save_tabs(s);
            }
            true
        }
        "next-tab" => {
            if !s.tabs.is_empty() {
                s.active_tab = (s.active_tab + 1) % s.tabs.len();
                let target = s.tabs[s.active_tab].clone();
                navigate(s, target);
                sync_address(s);
            }
            true
        }
        "view-details" => { s.view_mode = 0; save_settings(s); true }
        "view-list" => { s.view_mode = 1; save_settings(s); true }
        "view-grid" => { s.view_mode = 2; save_settings(s); s.status = "view: grid".into(); load_thumbnails(s); true }
        "conflict-mode" => {
            s.conflict_policy = if s.conflict_policy == 0 { 1 } else { 0 };
            s.status = if s.conflict_policy == 0 { "conflicts: auto-rename".into() } else { "conflicts: ask".into() };
            true
        }
        _ => false,
    }
}

fn do_copy(s: &mut State) {
    let mut paths = Vec::new();
    for &idx in &s.selected {
        if let Some(e) = s.snapshot.entries.get(idx) {
            if let Some(p) = entry_path(&s.location, e) { paths.push(p); }
        }
    }
    if paths.is_empty() {
        if let Some(&idx) = s.filtered.get(s.cursor) {
            if let Some(e) = s.snapshot.entries.get(idx) {
                if let Some(p) = entry_path(&s.location, e) { paths.push(p); }
            }
        }
    }
    if !paths.is_empty() {
        s.clipboard = paths.clone();
        s.clipboard_cut = false;
        let p2 = paths.clone();
        std::thread::spawn(move || { if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set().file_list(&p2); } });
        s.status = format!("copied {} item(s)", paths.len());
    }
}

fn do_new_file(s: &mut State) {
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
}

fn do_new_folder(s: &mut State) {
    if let Location::Folder(root) = s.location.clone() {
        let mut name = "New folder".to_string();
        let mut n = 1u32;
        while root.join(&name).exists() { n += 1; name = format!("New folder {n}"); }
        match std::fs::create_dir(root.join(&name)) {
            Ok(_) => { s.status = format!("created {name}"); reload(s); }
            Err(_) => { s.status = "could not create folder".into(); }
        }
    }
}

fn start_rename(s: &mut State) {
    if let Some(&idx) = s.filtered.get(s.cursor) {
        s.renaming = Some(idx);
        if let Some(e) = s.snapshot.entries.get(idx) { s.rename_text = TextEdit::new(&e.name); }
    }
}

fn do_permanent_delete(s: &mut State) {
    let mut paths = Vec::new();
    for &idx in &s.selected {
        if let Some(e) = s.snapshot.entries.get(idx) {
            if let Some(p) = entry_path(&s.location, e) { paths.push(p); }
        }
    }
    if paths.is_empty() {
        if let Some(&idx) = s.filtered.get(s.cursor) {
            if let Some(e) = s.snapshot.entries.get(idx) {
                if let Some(p) = entry_path(&s.location, e) { paths.push(p); }
            }
        }
    }
    if paths.is_empty() { return; }
    let handle = s.handle.clone();
    s.status = "deleting permanently...".into();
    std::thread::spawn(move || {
        let mut done = 0usize; let mut fail = 0usize;
        for p in &paths {
            let res = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            if res.is_ok() { done += 1; } else { fail += 1; }
        }
        handle.update(move |s, _| {
            s.status = format!("permanent delete: {done} ok, {fail} failed");
            reload(s);
        });
    });
}

fn open_properties(s: &mut State) {
    if let Some(&idx) = s.filtered.get(s.cursor) {
        if let Some(e) = s.snapshot.entries.get(idx) {
            if let Some(p) = entry_path(&s.location, e) {
                s.properties_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                s.properties_cancel = cancel.clone();
                s.properties_path = Some(p.clone());
                s.properties_size = None;
                s.properties_loading = true;
                s.properties_checksum = None;
                s.properties_mime = None;
                let path = p;
                let handle = s.handle.clone();
                std::thread::spawn(move || {
                    let size = if path.is_dir() { dir_size(&path, &cancel) } else { path.metadata().map(|m| m.len()).unwrap_or(0) };
                    let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                    let checksum = if !cancelled && path.is_file() { file_checksum(&path) } else { None };
                    let mime = std::process::Command::new("file")
                        .args(["-b", "--mime-type"])
                        .arg(&path)
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    handle.update(move |s, _| {
                        if cancelled { s.properties_loading = false; s.status = "properties: cancelled".into(); return; }
                        s.properties_size = Some(size);
                        s.properties_checksum = checksum;
                        s.properties_mime = mime;
                        s.properties_loading = false;
                    });
                });
            }
        }
    }
}

fn keybinds_help() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("undo", "ctrl+z", "Restore last trash"),
        ("copy", "ctrl+c", "Copy selection"),
        ("cut", "ctrl+x", "Cut selection"),
        ("paste", "ctrl+v", "Paste into folder"),
        ("new-file", "ctrl+alt+n", "New empty file"),
        ("new-folder", "ctrl+shift+n", "New folder"),
        ("rename", "f2", "Rename"),
        ("delete", "delete", "Trash selection"),
        ("delete-permanent", "shift+delete", "Permanently delete"),
        ("properties", "ctrl+i", "Properties"),
        ("search", "ctrl+f", "Search in folder"),
        ("hidden", "f7", "Toggle hidden files"),
        ("shortcuts", "ctrl+/", "Keyboard shortcuts"),
        ("new-tab", "ctrl+t", "New tab"),
        ("close-tab", "ctrl+w", "Close tab"),
        ("next-tab", "f6", "Next tab"),
        ("view-details", "ctrl+1", "Details view"),
        ("view-list", "ctrl+2", "List view"),
        ("view-grid", "ctrl+3", "Grid view"),
        ("conflict-mode", "ctrl+shift+c", "Conflict mode toggle"),
    ]
}

fn default_keybinds() -> Vec<(String, String)> {
    vec![
        ("undo".to_string(), "ctrl+z".to_string()),
        ("copy".to_string(), "ctrl+c".to_string()),
        ("cut".to_string(), "ctrl+x".to_string()),
        ("paste".to_string(), "ctrl+v".to_string()),
        ("new-file".to_string(), "ctrl+alt+n".to_string()),
        ("new-folder".to_string(), "ctrl+shift+n".to_string()),
        ("rename".to_string(), "f2".to_string()),
        ("delete".to_string(), "delete".to_string()),
        ("delete-permanent".to_string(), "shift+delete".to_string()),
        ("properties".to_string(), "ctrl+i".to_string()),
        ("search".to_string(), "ctrl+f".to_string()),
        ("hidden".to_string(), "f7".to_string()),
        ("shortcuts".to_string(), "ctrl+/".to_string()),
        ("new-tab".to_string(), "ctrl+t".to_string()),
        ("close-tab".to_string(), "ctrl+w".to_string()),
        ("next-tab".to_string(), "f6".to_string()),
        ("view-details".to_string(), "ctrl+1".to_string()),
        ("view-list".to_string(), "ctrl+2".to_string()),
        ("view-grid".to_string(), "ctrl+3".to_string()),
        ("conflict-mode".to_string(), "ctrl+shift+c".to_string()),
    ]
}

fn keybinds_path() -> Option<PathBuf> {
    Some(platform::config_dir().join("clove").join("keybinds.txt"))
}

fn save_keybinds(s: &State) {
    if let Some(path) = keybinds_path() {
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        let mut lines = Vec::new();
        for (a, k) in &s.keybinds { lines.push(format!("{a}: {k}")); }
        let _ = std::fs::write(path, lines.join("\n"));
    }
}

fn load_keybinds() -> Vec<(String, String)> {
    let mut out = default_keybinds();
    if let Some(path) = keybinds_path() {
        if let Ok(text) = std::fs::read_to_string(path) {
            let mut loaded = Vec::new();
            for line in text.lines() {
                if let Some((a, k)) = line.split_once(": ") {
                    let a = a.trim().to_string();
                    let k = k.trim().to_string();
                    if !a.is_empty() && !k.is_empty() { loaded.push((a, k)); }
                }
            }
            if !loaded.is_empty() { out = loaded; }
        }
    }
    out
}

fn handle_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    // Custom keybinds: if customizing, capture key; else match custom or default.
    if s.customizing.is_some() {
        if ev.key == Key::Function(10) || ev.key == Key::Escape || (ev.key == Key::Command('q') && ev.mods.ctrl) {
            s.customizing = None;
            s.custom_key = None;
            return true;
        }
        let key_str = match &ev.key {
            Key::Function(n) => format!("f{}", n),
            Key::Enter => "enter".into(),
            Key::Backspace => "backspace".into(),
            Key::Delete => "delete".into(),
            Key::Escape => "escape".into(),
            Key::Command(c) => {
                let mut parts = vec!["cmd".to_string()];
                if ev.mods.ctrl { parts.push("ctrl".into()); }
                if ev.mods.alt { parts.push("alt".into()); }
                if ev.mods.shift { parts.push("shift".into()); }
                parts.push(c.clone().to_string());
                parts.join("+")
            }
            Key::Text(c) => c.clone().to_string(),
            _ => { if let Key::Command(c) = ev.key { c.to_string() } else { return true; } }
        };
        if !key_str.is_empty() && key_str != "f10" && key_str != "escape" {
            if let Some(i) = s.customizing {
                if i < s.keybinds.len() {
                    s.keybinds[i].1 = key_str;
                    save_keybinds(s);
                }
            }
        }
        s.customizing = None;
        s.custom_key = None;
        return true;
    }
    if let Some(action) = key_is_bound(s, ev) {
        return handle_action(s, &action);
    }
    // Shortcuts overlay default.
    if ev.key == Key::Function(1) { s.show_shortcuts = !s.show_shortcuts; return true; }
    // Ctrl+Z (Cmd+Z on macOS) restores the last trashed selection. It has to be claimed
    // before the focused filter field turns it into a text undo.
    if ev.key == Key::Command('z') && ev.mods.ctrl {
        undo_delete(s);
        return true;
    }
    // Ctrl+H toggles hidden files.
    if ev.key == Key::Command('h') && ev.mods.ctrl {
        s.show_hidden = !s.show_hidden;
        refilter(s);
        save_settings(s);
        s.status = if s.show_hidden { "showing hidden files".into() } else { "hidden files hidden".into() };
        return true;
    }
    if ev_is_command(ev, '/') || ev.key == Key::Function(1) {
        s.show_shortcuts = !s.show_shortcuts;
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
    // Ctrl+P preview pane.
    if ev_is_command(ev, 'p') { s.preview_open = !s.preview_open; return true; }
    // Ctrl+R re-measures disk.
    if ev_is_command(ev, 'r') {
        if let Some(path) = storage_cache_path() { let _ = std::fs::remove_file(path); }
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
                    if is_archive(&path) {
                        // shortcut: cache-backed extraction, not libarchive virtual FS; upgrade if no-extract reads are needed.
                        let stem = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "archive".into());
                        let target = platform::cache_dir().join("clove").join("archives").join(stem);
                        if target.is_dir() {
                            navigate(s, Location::Folder(target));
                            sync_address(s);
                            return true;
                        }
                        let handle = s.handle.clone();
                        let src = path.clone();
                        let dest = target.clone();
                        s.status = "extracting...".into();
                        std::thread::spawn(move || {
                            let ok = extract_archive(&src, &dest);
                            handle.update(move |s, _| {
                                if ok {
                                    navigate(s, Location::Folder(target));
                                    sync_address(s);
                                } else {
                                    s.status = "extract failed".into();
                                }
                            });
                        });
                    } else {
                        platform::open_path(&path);
                        s.status = format!("opening {}", s.snapshot.entries[index].name);
                    }
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
            if s.show_shortcuts { s.show_shortcuts = false; return true; }
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
                            // Collision policy: if target exists, append (n).
                            let mut final_path = parent.join(&new_name);
                            if final_path.exists() {
                                let stem = final_path.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| new_name.clone());
                                let ext = final_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                                let mut n = 1u32;
                                loop {
                                    let cand = parent.join(format!("{stem} ({n}){ext}"));
                                    if !cand.exists() { final_path = cand; break; }
                                    n += 1;
                                    if n > 10_000 { break; }
                                }
                            }
                            let final_name = final_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(new_name.clone());
                            let handle = s.handle.clone();
                            let old = old_path.clone();
                            let to = final_path.clone();
                            std::thread::spawn(move || {
                                let res = std::fs::rename(&old, &to);
                                handle.update(move |s, _| {
                                    if res.is_ok() {
                                        s.status = format!("renamed to {}", final_name);
                                        s.undo_stack.push(UndoAction::Rename { from: old.clone(), to: to.clone() });
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
    // Tab completion.
    if ev.key == Key::Tab {
        let text = s.address.text.trim().to_string();
        if text.is_empty() || text.eq_ignore_ascii_case("This PC") { return true; }
        let path = PathBuf::from(&text);
        let (parent, prefix) = if text.ends_with('/') || text.ends_with(std::path::MAIN_SEPARATOR) {
            (path.clone(), String::new())
        } else {
            (path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("/")), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
        };
        let mut matches: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&parent) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix) {
                    let suffix = if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { "/" } else { "" };
                    matches.push(format!("{}{}", name, suffix));
                }
            }
        }
        if matches.len() == 1 {
            let new = parent.join(matches[0].trim_end_matches('/'));
            s.address.text = new.to_string_lossy().to_string();
            if matches[0].ends_with('/') { s.address.text.push('/'); }
        } else if matches.len() > 1 {
            // complete common prefix
            let first = &matches[0];
            let mut common = first.clone();
            for m in &matches[1..] {
                while !m.starts_with(&common) && !common.is_empty() {
                    common.pop();
                }
            }
            if common.len() > prefix.len() {
                let base = if prefix.is_empty() { text.clone() } else { text[..text.len()-prefix.len()].to_string() };
                s.address.text = format!("{}{}", base, common);
            }
            s.status = format!("{} matches", matches.len());
        }
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
                .label("Rename")
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
    let drag_paths: Vec<PathBuf> = if selected {
        s.selected.iter().filter_map(|&i| s.snapshot.entries.get(i).and_then(|e| entry_path(&s.location, e))).collect()
    } else {
        entry_path(&s.location, entry).into_iter().collect()
    };
    if !drag_paths.is_empty() {
        r = r.drag_out(move |_: &State| {
            let mut item = DragItem::new();
            for p in &drag_paths { item = item.file(p.clone()); }
            Some(item)
        });
    }
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

fn sidebar_el(s: &State) -> El {
    let mut col = col()
        .key("sidebar")
        .w(180.0)
        .h_full()
        .padding(8.0)
        .gap(4.0)
        .fill(surface());
    col = col.child(text("Bookmarks").font(type_::LABEL).color(ink_secondary()));
    for (name, path) in platform::quick_access() {
        let target = path.clone();
        col = col.child(
            el()
                .key(format!("sb-bm-{}", name))
                .w_full()
                .h(22.0)
                .px(6.0)
                .center_y()
                .radius(tokens::RADIUS_CONTROL)
                .child(text(name).font(type_::BODY).color(ink()))
                .on_press(move |s: &mut State| {
                    navigate(s, Location::Folder(target.clone()));
                    sync_address(s);
                }),
        );
    }
    col = col.child(text("Mounts").font(type_::LABEL).color(ink_secondary()));
    for (name, path) in platform::mount_entries() {
        let target = path.clone();
        let label = if name == "/" { "/".to_string() } else { name.clone() };
        col = col.child(
            el()
                .key(format!("sb-mnt-{}", name))
                .w_full()
                .h(22.0)
                .px(6.0)
                .center_y()
                .radius(tokens::RADIUS_CONTROL)
                .child(text(label).font(type_::BODY).color(ink()))
                .on_press(move |s: &mut State| {
                    navigate(s, Location::Folder(target.clone()));
                    sync_address(s);
                }),
        );
    }
    col
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

    row()
        .key("root")
        .w_full()
        .h_full()
        .fill(bg())
        .gap(8.0)
        .on_drop_files(|s: &mut State, _f, _| { s.drop_hover = false; })
        .child(sidebar_el(s))
        .child(
            col()
                .key("main")
                .grow(1.0)
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
                        .label("Back")
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
                        .label("Forward")
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
                        .label("Up one folder")
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
                .child(if let Some(b) = &s.git_branch {
                    text(format!("⌥ {}{}", b, if s.git_dirty { " ●" } else { "" }))
                        .key("git")
                        .font(type_::LABEL)
                        .color(ink_secondary())
                } else {
                    text("").key("git-off").font(type_::LABEL).color(ink_secondary())
                })
                .child(
                    text(if let Some(op) = &s.op_progress {
                        if op.total == 0 {
                            format!("{} {} · {} failed", op.label, op.done, op.failures)
                        } else {
                            format!("{} {}/{} · {} failed", op.label, op.done, op.total, op.failures)
                        }
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
                    el().key("op-cancel-btn").label("Cancel operation").w(60.0).h(18.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
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
                .fill(if s.drop_hover { selection() } else { Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } })
                .on_drop_hover(|s: &mut State, over, _| { s.drop_hover = over; })
                .on_drop_files(handle_drop_files)
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
                        .child(text(format!("MIME: {}", s.properties_mime.clone().unwrap_or_else(|| if loading { "detecting...".to_string() } else { "unknown".to_string() }))).font(type_::LABEL).color(ink_secondary()))
                        .child(text(format!("Permissions: {}", props_permissions(&path))).font(type_::LABEL).color(ink_secondary()))
                        .child(text(format!("Owner: {}", props_owner(&path))).font(type_::LABEL).color(ink_secondary()))
                        .child(text(if loading {
                            "Size: calculating...".to_string()
                        } else if let Some(sz) = size {
                            format!("Size: {}", format_size(sz))
                        } else {
                            "Size: unknown".to_string()
                        }).font(type_::LABEL).color(ink_secondary()))
                        .child(text(format!("Checksum: {}", s.properties_checksum.clone().unwrap_or_else(|| if loading { "calculating...".to_string() } else { "n/a".to_string() }))).font(type_::LABEL).color(ink_secondary()))
                        .child(
                            row().gap(8.0)
                                .child(
                                    el()
                                        .key("props-close")
                                        .label("Close properties")
                                        .w(80.0).h(28.0)
                                        .radius(tokens::RADIUS_CONTROL)
                                        .fill(bg())
                                        .center()
                                        .child(text("Close").font(type_::BODY).color(ink()))
                                        .on_press(|s: &mut State| {
                                            s.properties_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                            s.properties_path = None;
                                        }),
                                )
                                .child(if loading {
                                    el()
                                        .key("props-cancel")
                                        .label("Cancel properties")
                                        .w(80.0).h(28.0)
                                        .radius(tokens::RADIUS_CONTROL)
                                        .fill(bg())
                                        .center()
                                        .child(text("Cancel").font(type_::BODY).color(ink()))
                                        .on_press(|s: &mut State| {
                                            s.properties_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                            s.properties_loading = false;
                                            s.status = "properties: cancelled".into();
                                        })
                                } else {
                                    el().key("props-cancel-off").w(0.0).h(0.0)
                                }),
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
        // Shortcuts overlay.
        .child(if s.show_shortcuts {
            col().key("shortcuts-overlay").w_full().h_full().center().child(
                col().key("shortcuts-card").w(520.0).padding(16.0).gap(6.0).radius(tokens::RADIUS_BAR).fill(surface())
                    .child(text("Keyboard Shortcuts").font(type_::TITLE).color(ink()))
                    .child(col().gap(2.0).children({
                        let help = keybinds_help();
                        s.keybinds.iter().map(|(action, kb)| {
                            let desc = help.iter().find(|(a, _, _)| a == action).map(|(_, _, d)| *d).unwrap_or("");
                            row().gap(8.0)
                                .child(text(kb.clone()).font(type_::LABEL).color(ink()).w(120.0))
                                .child(text(desc.to_string()).font(type_::LABEL).color(ink_secondary()))
                        }).collect::<Vec<_>>()
                    }))
                    .child(text("Customize: ~/.config/clove/keybinds.txt").font(type_::LABEL).color(ink_secondary()))
                    .child(el().key("sc-close").label("Close shortcuts").w(80.0).h(28.0).radius(tokens::RADIUS_CONTROL).fill(bg()).center()
                        .child(text("Close").font(type_::BODY).color(ink()))
                        .on_press(|s: &mut State| { s.show_shortcuts = false; }))
            )
        } else { col().key("shortcuts-off").w(0.0).h(0.0) })
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
            let (target, paths, _cut) = s.pending_paste.clone().unwrap();
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
                                s.op_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                                let cancel = s.op_cancel.clone();
                                let total = paths.iter().map(|p| count_files(p)).sum::<usize>().max(1);
                                s.op_progress = Some(FileOpProgress { label: "Compressing".into(), done: 0, total, bytes_done: 0, bytes_total: 0, failures: 0, cancelled: false });
                                std::thread::spawn(move || {
                                    let mut args = vec!["czvf".to_string(), dest.to_string_lossy().to_string()];
                                    for p in &paths { args.push(p.to_string_lossy().to_string()); }
                                    let mut cmd = std::process::Command::new("tar");
                                    cmd.args(&args);
                                    let ok = run_progress(&mut cmd, &cancel, &handle);
                                    let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                                    handle.update(move |s, _| {
                                        s.op_progress = None;
                                        s.status = if cancelled { "compress cancelled".into() } else if ok { "compressed".into() } else { "compress failed".into() };
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
                                        s.op_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                                        let cancel = s.op_cancel.clone();
                                        let total = archive_entry_count(&p);
                                        s.op_progress = Some(FileOpProgress { label: "Extracting".into(), done: 0, total, bytes_done: 0, bytes_total: 0, failures: 0, cancelled: false });
                                        std::thread::spawn(move || {
                                            let mut cmd = if p.to_string_lossy().ends_with(".zip") {
                                                let mut c = std::process::Command::new("unzip");
                                                c.args(["-o", &p.to_string_lossy(), "-d", &target.to_string_lossy()]);
                                                c
                                            } else {
                                                let mut c = std::process::Command::new("tar");
                                                c.args(["xvf", &p.to_string_lossy(), "-C", &target.to_string_lossy()]);
                                                c
                                            };
                                            let ok = run_progress(&mut cmd, &cancel, &handle);
                                            let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
                                            handle.update(move |s, _| {
                                                s.op_progress = None;
                                                s.status = if cancelled { "extract cancelled".into() } else if ok { "extracted".into() } else { "extract failed".into() };
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
                        .child(ctx_item("Copy Checksums", move |s: &mut State| {
                            let mut paths = Vec::new();
                            for &i2 in &s.selected { if let Some(e) = s.snapshot.entries.get(i2) { if let Some(p) = entry_path(&s.location, e) { if p.is_file() { paths.push(p); } } } }
                            if paths.is_empty() { if let Some(e) = s.snapshot.entries.get(idx) { if let Some(p) = entry_path(&s.location, e) { if p.is_file() { paths.push(p); } } } }
                            let handle = s.handle.clone();
                            s.status = "checksumming...".into();
                            std::thread::spawn(move || {
                                let mut lines = Vec::new();
                                for p in &paths { if let Some(h) = file_checksum(p) { lines.push(format!("{}  {}", h, p.display())); } }
                                let report = lines.join("\n");
                                handle.update(move |s, _| {
                                    if let Ok(mut cb) = arboard::Clipboard::new() { let _ = cb.set_text(report.clone()); }
                                    s.status = format!("{} checksums copied", lines.len());
                                });
                            });
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Find Duplicates", move |s: &mut State| {
                            let root = match &s.location { Location::Folder(d) => d.clone(), _ => { s.status = "open a folder first".into(); s.context_menu = None; return; } };
                            let handle = s.handle.clone();
                            s.op_progress = Some(FileOpProgress { label: "Scanning dupes".into(), done: 0, total: 1, bytes_done: 0, bytes_total: 0, failures: 0, cancelled: false });
                            std::thread::spawn(move || {
                                let dups = find_duplicates(&root);
                                let groups = dups.len();
                                let files: usize = dups.iter().map(|(_, g)| g.len()).sum();
                                handle.update(move |s, _| {
                                    s.op_progress = None;
                                    s.status = format!("duplicates: {} groups, {} files", groups, files);
                                });
                            });
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Compare Folders", move |s: &mut State| {
                            let mut dirs: Vec<PathBuf> = Vec::new();
                            for &i2 in &s.selected { if let Some(e) = s.snapshot.entries.get(i2) { if e.is_dir { if let Some(p) = entry_path(&s.location, e) { dirs.push(p); } } } }
                            if dirs.len() == 2 {
                                let a = dirs[0].clone(); let b = dirs[1].clone();
                                let handle = s.handle.clone();
                                s.status = "comparing...".into();
                                std::thread::spawn(move || {
                                    let (oa, ob, diff) = compare_dirs(&a, &b);
                                    handle.update(move |s, _| { s.status = format!("compare: only A {}, only B {}, different {}", oa, ob, diff); });
                                });
                            } else {
                                s.status = "select exactly 2 folders".into();
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Create Symlink", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    if let Some(parent) = path.parent() {
                                        let dest = unique_link_path(parent, &path, "symlink");
                                        let res = create_symlink(&path, &dest);
                                        s.status = if res { "symlink created".into() } else { "symlink failed".into() };
                                        reload(s);
                                    }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Create Hardlink", move |s: &mut State| {
                            if let Some(entry) = s.snapshot.entries.get(idx) {
                                if let Some(path) = entry_path(&s.location, entry) {
                                    if path.is_file() {
                                        if let Some(parent) = path.parent() {
                                            let dest = unique_link_path(parent, &path, "hardlink");
                                            let res = std::fs::hard_link(&path, &dest).is_ok();
                                            s.status = if res { "hardlink created".into() } else { "hardlink failed".into() };
                                            reload(s);
                                        }
                                    } else { s.status = "hardlink: files only".into(); }
                                }
                            }
                            s.context_menu = None;
                        }))
                        .child(ctx_item("Properties", move |s: &mut State| {
                            open_properties(s);
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
        )
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

fn tabs_path() -> Option<PathBuf> {
    Some(platform::config_dir().join("clove").join("tabs.txt"))
}

fn save_tabs(s: &State) {
    if let Some(path) = tabs_path() {
        if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
        let mut lines = Vec::new();
        for loc in &s.tabs {
            match loc {
                Location::ThisPc => lines.push("thispc".to_string()),
                Location::Folder(p) => lines.push(p.to_string_lossy().to_string()),
            }
        }
        let _ = std::fs::write(path, lines.join("\n"));
    }
}

fn load_tabs() -> Vec<Location> {
    let Some(path) = tabs_path() else { return vec![Location::ThisPc]; };
    let text = std::fs::read_to_string(path).ok().unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        if line.eq_ignore_ascii_case("thispc") {
            out.push(Location::ThisPc);
        } else {
            let p = PathBuf::from(line);
            if p.exists() { out.push(Location::Folder(p)); }
        }
    }
    if out.is_empty() { out.push(Location::ThisPc); }
    out
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
    let git_branch = None;
    let git_dirty = false;

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
        git_branch,
        git_dirty,
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
        properties_checksum: None,
        properties_mime: None,
        properties_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        renaming: None,
        rename_text: TextEdit::new(""),
        context_menu: None,
        drop_hover: false,
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
        thumbnail_bytes: 0,
        tabs: load_tabs(),
        active_tab: 0,
        show_hidden,
        show_shortcuts: false,
        keybinds: load_keybinds(),
        customizing: None,
        custom_key: None,
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn checksum_known() {
        let dir = std::env::temp_dir().join("clove_test_ck");
        let _ = fs::create_dir_all(&dir);
        let f = dir.join("a.txt");
        fs::write(&f, b"hello").unwrap();
        let h = file_checksum(&f).unwrap();
        assert_eq!(h.len(), 64);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compare_dirs_basic() {
        let root = std::env::temp_dir().join("clove_test_cmp");
        let a = root.join("a"); let b = root.join("b");
        let _ = fs::create_dir_all(&a); let _ = fs::create_dir_all(&b);
        fs::write(a.join("same.txt"), b"x").unwrap();
        fs::write(b.join("same.txt"), b"x").unwrap();
        fs::write(a.join("onlya.txt"), b"a").unwrap();
        fs::write(b.join("onlyb.txt"), b"b").unwrap();
        let (oa, ob, diff) = compare_dirs(&a, &b);
        assert_eq!(oa, 1);
        assert_eq!(ob, 1);
        assert_eq!(diff, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_duplicates_basic() {
        let root = std::env::temp_dir().join("clove_test_dup");
        let _ = fs::create_dir_all(&root);
        fs::write(root.join("a.txt"), b"same").unwrap();
        fs::write(root.join("b.txt"), b"same").unwrap();
        fs::write(root.join("c.txt"), b"diff").unwrap();
        let dups = find_duplicates(&root);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].1.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod perf_tests {
    use std::fs;

    #[test]
    #[ignore = "100k-file smoke; run manually with --ignored"]
    fn perf_100k_listing() {
        let root = std::env::temp_dir().join("clove_perf_100k");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for i in 0..100_000 {
            fs::write(root.join(format!("f{i:06}")), b"").unwrap();
        }
        let start = std::time::Instant::now();
        let n = fs::read_dir(&root).unwrap().count();
        let elapsed = start.elapsed();
        assert_eq!(n, 100_000);
        println!("100k listing: {elapsed:?}");
        let _ = fs::remove_dir_all(&root);
    }
}
