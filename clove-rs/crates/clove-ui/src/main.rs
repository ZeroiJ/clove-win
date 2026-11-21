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

struct State {
    dir: PathBuf,
    snapshot: Arc<Snapshot>,
    filtered: Vec<usize>,
    query: TextEdit,
    selected: usize,
    status: String,
    loading: bool,
    /// Receipt from the last trash, so Ctrl+Z can put it back.
    undo: Option<Vec<clove_shell::Move>>,
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
}

impl State {
    fn wake_bar(&mut self) {
        self.bar_until = self.bar_time + 1.5;
    }
}

// ---------------------------------------------------------------- engine glue

fn refilter(s: &mut State) {
    s.filtered = clove_search::filter(&s.snapshot.entries, &s.query.text);
    s.selected = s.selected.min(s.filtered.len().saturating_sub(1));
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
        BarSpec::new(s.bar_width, segments).with_height(BAR_H)
    } else {
        BarSpec::placeholder(s.bar_width).with_height(BAR_H)
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
                s.dir = dir;
                // A reload triggered by the watcher must not wipe a notice the user is
                // still reading ("moved … to the Recycle Bin · Ctrl+Z to undo").
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

/// Opens `dir`: arms a watcher first (so no change is missed), loads it, then reloads on
/// every change until the user navigates away.
fn open_dir(s: &mut State, dir: PathBuf) {
    let mine = s.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let generation = s.generation.clone();
    let handle = s.handle.clone();

    s.dir = dir.clone();
    s.snapshot = Arc::new(Snapshot::new(dir.clone(), Vec::new()));
    s.filtered.clear();
    s.selected = 0;
    s.loading = true;
    s.status = "loading…".into();

    std::thread::spawn(move || {
        let watcher = DirWatcher::new(&dir).ok();
        load_into(&handle, &generation, mine, dir.clone());

        let Some(watcher) = watcher else { return };
        while generation.load(Ordering::Relaxed) == mine {
            // A timeout keeps stale threads exiting promptly after navigation.
            if watcher.recv_timeout(Duration::from_millis(250)).is_some() {
                load_into(&handle, &generation, mine, dir.clone());
            }
        }
    });
}

fn delete_selected(s: &mut State) {
    let Some(&index) = s.filtered.get(s.selected) else {
        return;
    };
    let entry = s.snapshot.entries[index].clone();
    let path = s.dir.join(&entry.name);
    let root = s.dir.clone();
    let handle = s.handle.clone();

    s.status = format!("moving {} to the Recycle Bin…", entry.name);
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
            s.status = format!("moved {name} to the Recycle Bin · Ctrl+Z to undo");
            s.undo = Some(moves);
            s.wake_bar();
        });
    });
}

fn undo_delete(s: &mut State) {
    let Some(moves) = s.undo.take() else {
        return;
    };
    let handle = s.handle.clone();
    s.status = "restoring…".into();

    std::thread::spawn(move || {
        let outcome = clove_shell::put_back(&moves);
        handle.update(move |s, _| {
            s.status = format!(
                "restored {} · {} could not be restored",
                outcome.restored, outcome.missing
            );
            s.wake_bar();
        });
    });
}

// ---------------------------------------------------------------- input

fn handle_key(s: &mut State, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    // Ctrl+Z (Cmd+Z on macOS) restores the last trashed selection. It has to be claimed
    // before the focused filter field turns it into a text undo.
    if ev.is_command('z') {
        undo_delete(s);
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
            s.selected = (s.selected + 1).min(n - 1);
            true
        }
        Key::Up => {
            s.selected = s.selected.saturating_sub(1);
            true
        }
        Key::Home if n > 0 => {
            s.selected = 0;
            true
        }
        Key::End if n > 0 => {
            s.selected = n - 1;
            true
        }
        Key::Enter => {
            if let Some(&index) = s.filtered.get(s.selected) {
                if s.snapshot.entries[index].is_dir {
                    let path = s.dir.join(&s.snapshot.entries[index].name);
                    open_dir(s, path);
                    return true;
                }
            }
            false
        }
        Key::Backspace if s.query.text.is_empty() => {
            if let Some(parent) = s.dir.parent().map(Path::to_path_buf) {
                open_dir(s, parent);
                return true;
            }
            false
        }
        Key::Delete => {
            delete_selected(s);
            true
        }
        Key::Escape => {
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
    if let Some(scan) = &s.scan {
        while let Some(event) = scan.try_recv() {
            match event {
                ScanEvent::Progress(progress) => {
                    s.storage_bytes[progress.kind.index()] = progress.bytes;
                }
                ScanEvent::Done(report) => {
                    s.storage_bytes[report.kind.index()] = report.bytes;
                    new_done = true;
                }
                ScanEvent::Finished => finished = true,
            }
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
    let selected = index == s.selected;
    row()
        .key(format!("row-{index}"))
        .h(ROW_H)
        .px(8.0)
        .center_y()
        .radius(tokens::RADIUS_ROW)
        .fill(if selected { selection() } else { Color::TRANSPARENT })
        .child(
            text(if entry.is_dir {
                format!("{}", entry.name)
            } else {
                entry.name.clone()
            })
            .font(type_::BODY)
            .color(if entry.is_dir { ink() } else { ink_secondary() })
            .grow(1.0),
        )
        .child(
            text(if entry.is_dir {
                "folder".to_owned()
            } else {
                format_size(entry.size)
            })
            .font(type_::LABEL)
            .color(ink_secondary()),
        )
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let count = s.filtered.len();
    let total = s.snapshot.len();

    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(bg())
        .padding(12.0)
        .gap(8.0)
        // Path + status.
        .child(
            row()
                .key("header")
                .w_full()
                .h(24.0)
                .center_y()
                .child(
                    text(s.dir.display().to_string())
                        .key("path")
                        .font(type_::BODY)
                        .color(ink())
                        .grow(1.0),
                )
                .child(
                    text(if !s.status.is_empty() {
                        s.status.clone()
                    } else if count == total {
                        format!("{total} items")
                    } else {
                        format!("{count} of {total}")
                    })
                    .key("status")
                    .font(type_::LABEL)
                    .color(ink_secondary()),
                ),
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
        // Virtualized listing: only mounted rows build.
        .child(
            col()
                .key("viewport")
                .scroll(ScrollSpec { stick_bottom: false })
                .grow(1.0)
                .min_h(0.0)
                .child(
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
                        ),
                ),
        )
}

// ---------------------------------------------------------------- main

fn storage_cache_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Clove").join("storage-scan.txt"))
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
    let dir = start_dir();

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
        BarSpec::new(936.0, segments).with_height(BAR_H)
    } else {
        BarSpec::placeholder(936.0).with_height(BAR_H)
    };
    let mut bar_renderer = BarRenderer::new(spec);
    bar_renderer.render(0.0);
    let (w, h) = bar_renderer.size();
    let bar = Pixels::new();
    bar.set(w as u32, h as u32, bar_renderer.buffer().to_vec());

    State {
        dir: dir.clone(),
        snapshot: Arc::new(Snapshot::new(dir, Vec::new())),
        filtered: Vec::new(),
        query: TextEdit::new(""),
        selected: 0,
        status: "loading…".into(),
        loading: true,
        undo: None,
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
    std::thread::spawn(move || {
        let dir = start_dir();
        boot.update(move |s, _| open_dir(s, dir));
    });

    app.run();
}
