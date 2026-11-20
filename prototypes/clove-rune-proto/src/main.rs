// Clove Windows — Rune prototype (framework evaluation candidate B)
//
// The same representative slice as the GPUI prototype, in Rune:
//   1. The dithered storage bar — a faithful port of Clove/Features/Storage/
//      StorageBarRenderer.swift (4×4 Bayer ordered dither, 4 pt cell pitch,
//      edge light, value-noise clouds, glints, lit/dim cells, glow), drawn
//      through a replaceable `Pixels` buffer (the future `clove-render`
//      crate's output; Rune only uploads the RGBA).
//   2. A virtualized directory listing (`virtual_rows`) — only mounted rows
//      build. Loaded on a background thread via AppHandle::update.
//   3. Incremental filtering in a text field + keyboard navigation.
//
// NOTE: rune is pinned to `dev` (pre-release); apps pin a rev in real use.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rune::tok::type_;
use rune::{
    col, el, row, text, text_input, App, AppHandle, AppSpec, Color, Cx, El, GrowthSpec, KeyEvent,
    Key, Pixels, ScrollSpec, TextEdit, Tx,
};

use clove_rune_proto::{filter_entries, load_entries, render_storage_bar, Entry, BAR_H, RADIUS};

// ---------------------------------------------------------------------------
// Palette (dark) — Clove/Design/Palette.swift neutrals, as sRGB 0..1.
// ---------------------------------------------------------------------------

const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

const BG: Color = rgba(0.114, 0.118, 0.125, 1.0); // #1D1E20
const CARD: Color = rgba(0.176, 0.180, 0.188, 1.0); // #2D2E30
const TEXT: Color = rgba(0.957, 0.961, 0.965, 1.0); // #F4F5F6
const TEXT_SECONDARY: Color = rgba(0.698, 0.706, 0.722, 1.0); // #B2B4B8

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct State {
    dir: PathBuf,
    entries: Arc<Vec<Entry>>,
    filtered: Vec<usize>,
    query: TextEdit,
    selected: usize,
    status: String,
    bar: Pixels,
    handle: AppHandle<State>,
}

fn fmt_size(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "K", "M", "G"];
    let mut v = size as f64;
    let mut u = 0usize;
    while v >= 1024.0 && u < 3 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{size} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn refilter(s: &mut State) {
    s.filtered = filter_entries(&s.entries, &s.query.text);
    s.selected = s.selected.min(s.filtered.len().saturating_sub(1));
}

fn open_dir(s: &mut State, dir: PathBuf) {
    s.dir = dir;
    s.entries = Arc::new(Vec::new());
    s.status = "loading…".into();
    s.selected = 0;
    s.filtered.clear();
    let h = s.handle.clone();
    let dir2 = s.dir.clone();
    std::thread::spawn(move || {
        let entries = load_entries(&dir2);
        let n = entries.len();
        h.update(move |s, _| {
            s.entries = Arc::new(entries);
            s.status = format!("settled · {n} items");
            refilter(s);
        });
    });
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn row_el(s: &State, idx: usize) -> El {
    let e = &s.entries[idx];
    let name = if e.is_dir {
        format!("▸ {}", e.name)
    } else {
        format!("  {}", e.name)
    };
    row()
        .key(format!("row-{idx}"))
        .h(28.0)
        .px(8.0)
        .center_y()
        .radius(6.0)
        .fill(if idx == s.selected {
            rgba(1.0, 1.0, 1.0, 0.06)
        } else {
            Color::TRANSPARENT
        })
        .child(
            text(name)
                .font(type_::BODY)
                .color(if e.is_dir { TEXT } else { TEXT_SECONDARY })
                .grow(1.0),
        )
        .child(
            text(if e.is_dir { "dir".to_owned() } else { fmt_size(e.size) })
                .font(type_::LABEL)
                .color(TEXT_SECONDARY),
        )
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    col()
        .key("root")
        .w_full()
        .h_full()
        .fill(BG)
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .key("header")
                .w_full()
                .center_y()
                .child(
                    text(s.dir.display().to_string())
                        .key("path")
                        .font(type_::BODY)
                        .color(TEXT)
                        .grow(1.0),
                )
                .child(text(s.status.clone()).key("status").font(type_::LABEL).color(TEXT_SECONDARY)),
        )
        // The dithered storage bar (Clove's signature).
        .child(
            el()
                .key("bar")
                .w_full()
                .h(BAR_H)
                .radius(RADIUS)
                .clip_rounded(RADIUS)
                .pixels_fit(&s.bar, rune::SurfaceFit::Fill),
        )
        .child(
            row()
                .key("filter-row")
                .w_full()
                .h(32.0)
                .px(8.0)
                .radius(6.0)
                .fill(CARD)
                .center_y()
                .child(
                    text_input(&s.query, type_::BODY)
                        .key("filter")
                        .label("Filter files by name")
                        .single_line()
                        .grow(1.0)
                        .color(TEXT)
                        .placeholder(
                            "type to filter · ↑↓ select · enter open · backspace up",
                            TEXT_SECONDARY,
                        )
                        .on_edit(|s: &mut State| &mut s.query)
                        .on_input_change(|s: &mut State, _| refilter(s))
                        .on_input_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| {
                            let n = s.filtered.len();
                            match ev.key {
                                Key::Down if n > 0 => s.selected = (s.selected + 1).min(n - 1),
                                Key::Up => s.selected = s.selected.saturating_sub(1),
                                Key::Enter => {
                                    if let Some(&idx) = s.filtered.get(s.selected) {
                                        if s.entries[idx].is_dir {
                                            let p = s.dir.join(&s.entries[idx].name);
                                            open_dir(s, p);
                                        }
                                    }
                                }
                                Key::Backspace if s.query.text.is_empty() => {
                                    if let Some(p) = s.dir.parent().map(Path::to_path_buf) {
                                        open_dir(s, p);
                                    }
                                }
                                _ => {
                                    let _ = tx;
                                    return false;
                                }
                            }
                            true
                        }),
                ),
        )
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
                            s.filtered.len(),
                            28.0,
                            |s: &State, i| format!("row-{}", s.filtered[i]).into(),
                            |s: &State, i| row_el(s, s.filtered[i]),
                        ),
                ),
        )
}

fn build_state(handle: AppHandle<State>) -> State {
    let dir = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:\\"));
    let width_pts = 936.0; // window 960 − padding 2×12
    let (w, h, rgba_buf) = render_storage_bar(width_pts);
    let bar = Pixels::new();
    bar.set(w as u32, h as u32, rgba_buf);
    State {
        dir,
        entries: Arc::new(Vec::new()),
        filtered: Vec::new(),
        query: TextEdit::new(""),
        selected: 0,
        status: "loading…".into(),
        bar,
        handle,
    }
}

fn main() {
    let app = App::new(AppSpec::<State> {
        ..AppSpec::new("Clove — Rune prototype", (960.0, 680.0), || unreachable!(), view)
    });
    let h = app.handle();
    let app = app.init({
        let h = h.clone();
        move || build_state(h)
    });
    std::thread::spawn(move || {
        let dir = std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\"));
        let entries = load_entries(&dir);
        let n = entries.len();
        h.update(move |s, _| {
            s.entries = Arc::new(entries);
            s.status = format!("settled · {n} items");
            refilter(s);
        });
    });
    app.run();
}
