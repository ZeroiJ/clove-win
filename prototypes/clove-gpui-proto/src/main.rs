// Clove Windows — GPUI prototype (framework evaluation candidate A)
//
// Minimal but representative slice of the ported Clove identity:
//   1. The dithered storage bar — a faithful port of Clove/Features/Storage/
//      StorageBarRenderer.swift (4×4 Bayer ordered dither, 4 pt cell pitch,
//      edge light, lit/dim cells, rounded-rect mask, hot colour ramp).
//   2. A virtualized directory listing (gpui::uniform_list) loaded on a
//      background task — never blocking the UI thread.
//   3. Incremental case-insensitive filtering + keyboard navigation.
//
// The storage-bar rendering lives in this file as the nucleus of the future
// framework-independent `clove-render` crate; the filesystem bits mirror the
// future `clove-fs` crate.

use gpui::{
    div, img, prelude::*, px, rgb, size, uniform_list, App, Application, Bounds, Context,
    UniformListScrollHandle,
    Image, ImageFormat, KeyDownEvent, Render, SharedString, Window, WindowBounds, WindowOptions,
};
use image::{codecs::png::PngEncoder, ImageEncoder};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Clove palette — from Clove/Design/Palette.swift (Vivid) and
// Clove/Storage/StorageKind.swift dark-mode bar hues (Display P3 → sRGB here).
// ---------------------------------------------------------------------------

const BG: u32 = 0x121214;
const PANEL: u32 = 0x1b1c1f;
const TEXT: u32 = 0xe8e6e3;
const TEXT_DIM: u32 = 0x8b8b90;
const SEL: u32 = 0x2b2d33;
const BAR_BG: u32 = 0x0e0e10;

/// (srgb, name) per StorageKind bar hue, dark mode, in enum order.
const KINDS: [(u32, &str); 9] = [
    (0x599cff, "Apps"),      // blue
    (0x6fdb74, "Code"),      // green
    (0xf166af, "Photos"),    // pink
    (0xa272ff, "Movies"),    // violet
    (0x54d2f0, "Documents"), // cyan
    (0xf7d042, "Music"),     // yellow
    (0xb2bac8, "System"),    // neutral
    (0xfa8e3b, "Caches"),    // orange
    (0xf05348, "Trash"),     // red
];

// ---------------------------------------------------------------------------
// StorageBarRenderer — faithful Rust port of the CPU dithering engine.
// Framework-independent: produces raw RGBA pixels. This is the seed of
// `clove-render`.
// ---------------------------------------------------------------------------

const PITCH: f32 = 4.0; // cell pitch, points
const RADIUS: f32 = 10.0; // bar corner radius
const HOT: (u8, u8, u8) = (255, 255, 255); // "hot" cells in dark mode

/// 4×4 Bayer matrix, normalised to (i + 0.5)/16 — matches StorageBarRenderer.bayer.
const BAYER: [f32; 16] = {
    let m = [
        0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5,
    ];
    let mut t = [0f32; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = (m[i] as f32 + 0.5) / 16.0;
        i += 1;
    }
    t
};

/// Integer hash — port of StorageBarRenderer.hash (no sin, xorshift-multiply).
fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xffffff) as f32 / 16777216.0
}

/// Value noise — bilinear over four corner hashes with smoothstep.
fn noise(x: f32, y: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let a = hash(xi, yi);
    let b = hash(xi + 1, yi);
    let c = hash(xi, yi + 1);
    let d = hash(xi + 1, yi + 1);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    (
        lerp_u8(a.0, b.0, t),
        lerp_u8(a.1, b.1, t),
        lerp_u8(a.2, b.2, t),
    )
}

/// Proportions per kind (equal here; the real bar uses scan results).
const SEG_FRACS: [f32; 9] = [0.18, 0.16, 0.11, 0.09, 0.12, 0.07, 0.13, 0.09, 0.05];

/// Renders the settled storage bar exactly as StorageBarRenderer would at
/// t=0, no heat/focus/scan-front: clouds + edge light only.
///
/// Returns (width, height, RGBA pixels) at `scale` device pixels per point.
fn render_storage_bar(width_pts: f32, scale: f32) -> (usize, usize, Vec<u8>) {
    let w_px = (width_pts * scale).round() as usize;
    let h_pts = 88.0; // StorageBarRenderer.height
    let h_px = (h_pts * scale).round() as usize;
    let pitch = PITCH * scale;
    let _ = pitch;
    let radius = RADIUS * scale;
    let rows = (h_pts / PITCH).round() as usize; // 22
    let cols = (width_pts / PITCH).round() as usize;

    // Cumulative segment edges in points, like the renderer's `edges`.
    let total: f32 = SEG_FRACS.iter().sum();
    let mut edges = vec![0.0f32; KINDS.len() + 1];
    let mut acc = 0.0;
    for (k, f) in SEG_FRACS.iter().enumerate() {
        acc += f / total;
        edges[k + 1] = acc * width_pts;
    }

    let mut buf = vec![0u8; w_px * h_px * 4];
    // Base: near-black track under the dithered cells.
    for i in 0..(w_px * h_px) {
        let (r, g, b) = hex_rgb(BAR_BG);
        buf[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
    }

    let put = |buf: &mut [u8], cx: f32, cy: f32, half: f32, c: (u8, u8, u8), a: u8| {
        let x0 = ((cx - half).round() as isize).max(0) as usize;
        let x1 = ((cx + half).round() as isize).min(w_px as isize - 1) as usize;
        let y0 = ((cy - half).round() as isize).max(0) as usize;
        let y1 = ((cy + half).round() as isize).min(h_px as isize - 1) as usize;
        let af = a as f32 / 255.0;
        let cc = [c.0, c.1, c.2];
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = (y * w_px + x) * 4;
                for ch in 0..3 {
                    let d = buf[i + ch] as f32;
                    buf[i + ch] = (cc[ch] as f32 * af + d * (1.0 - af)) as u8;
                }
                buf[i + 3] = 255;
            }
        }
    };

    for row in 0..rows {
        for col in 0..cols {
            let cx_pt = (col as f32 + 0.5) * PITCH;
            let cy_pt = (row as f32 + 0.5) * PITCH;
            let cx = cx_pt * scale;
            let cy = cy_pt * scale;

            // Rounded-rect mask: distance from the nearest corner centre,
            // matching the renderer's `dx²+dy² ≤ (r−1)²` corner test.
            let hw = w_px as f32;
            let dxr = if cx < radius { radius - cx } else if cx > hw - radius { cx - (hw - radius) } else { 0.0 };
            let dyr = if cy < radius { radius - cy } else if cy > h_px as f32 - radius { cy - (h_px as f32 - radius) } else { 0.0 };
            let in_bar = dxr * dxr + dyr * dyr <= (radius - scale) * (radius - scale);
            if !in_bar {
                continue;
            }

            // Which kind owns this column?
            let mut kind = 0usize;
            for k in 0..KINDS.len() {
                if cx_pt >= edges[k] && cx_pt < edges[k + 1] {
                    kind = k;
                    break;
                }
            }

            // Edge light: bright gathers along top and bottom.
            let d_edge = (cy_pt).min(h_pts - cy_pt);
            let edge_light = (-(d_edge - 2.0) / 5.0).exp();

            // Drifting value-noise clouds, two octaves (drift = 0 at t=0).
            let cloud = noise(cx_pt * 0.045, cy_pt * 0.11) * 0.65 + noise(cx_pt * 0.13, cy_pt * 0.3) * 0.35;
            let smooth_c = ((cloud - 0.42) / 0.38).clamp(0.0, 1.0);
            let smooth_c = smooth_c * smooth_c * (3.0 - 2.0 * smooth_c);

            let mut level = -0.04 + 0.66 * edge_light + 0.44 * smooth_c;

            // Settled: no heat (0), no scan front (0), no hover focus (0).
            let glint_p = hash(col as i32, row as i32);
            let glint = if glint_p > 0.994 { (glint_p - 0.994) / 0.006 } else { 0.0 };
            level += glint * 0.5;

            let threshold = BAYER[((row & 3) * 4) + (col & 3)];
            let color = hex_rgb(KINDS[kind].0);
            if level + glint > threshold {
                // Lit cell: ~3 pt square, alpha 242, toward white-hot.
                let wt = (level * 0.95 + glint * 0.6).min(1.0);
                let c = lerp_rgb(color, HOT, wt * 0.8);
                put(&mut buf, cx, cy, 1.5 * scale, c, 242);
            } else {
                // Dim cell: ~1.5 pt square, alpha 70.
                let dim = (
                    (color.0 as f32 * 0.55) as u8,
                    (color.1 as f32 * 0.55) as u8,
                    (color.2 as f32 * 0.55) as u8,
                );
                put(&mut buf, cx, cy, 0.75 * scale, dim, 70);
            }
        }
    }

    // Empty-tail cells: the SEG_FRACS sum to 1.0 here, so no empty zone; in
    // the real app empty cells draw white dots at alpha 18 (dark mode).

    (w_px, h_px, buf)
}

fn hex_rgb(v: u32) -> (u8, u8, u8) {
    ((v >> 16) as u8, ((v >> 8) & 0xff) as u8, (v & 0xff) as u8)
}

/// Encode the bar to PNG bytes so GPUI can treat it as an image asset.
fn bar_png(width_pts: f32) -> Option<Arc<Image>> {
    let (w, h, rgba) = render_storage_bar(width_pts, 2.0); // 2× for crispness
    let flat = image::RgbaImage::from_raw(w as u32, h as u32, rgba)?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(flat.as_raw(), w as u32, h as u32, image::ColorType::Rgba8.into())
        .ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
}

// ---------------------------------------------------------------------------
// Filesystem model — mirrors the future `clove-fs` crate surface.
// ---------------------------------------------------------------------------

struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}

struct Model {
    dir: PathBuf,
    entries: Arc<Vec<Entry>>,
    filtered: Vec<usize>,
    query: String,
    selected: usize,
    loading: bool,
    status: String,
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

impl Model {
    fn new() -> Self {
        let dir = std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\"));
        Model {
            dir,
            entries: Arc::new(Vec::new()),
            filtered: Vec::new(),
            query: String::new(),
            selected: 0,
            loading: true,
            status: "loading…".into(),
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

struct CloveProto {
    model: Model,
    bar: Option<Arc<Image>>,
    bar_w: f32,
    list_scroll: UniformListScrollHandle,
}

impl CloveProto {
    fn load_dir(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.model.dir = dir.clone();
        self.model.entries = Arc::new(Vec::new());
        self.model.loading = true;
        self.model.status = "loading…".into();
        self.model.refilter();
        cx.spawn(async move |this, cx| {
            let mut entries: Vec<Entry> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let size = if is_dir {
                        0
                    } else {
                        e.metadata().map(|m| m.len()).unwrap_or(0)
                    };
                    entries.push(Entry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        is_dir,
                        size,
                    });
                }
            }
            entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            });
            this.update(cx, |m, _| {
                let n = entries.len();
                m.model.entries = Arc::new(entries);
                m.model.loading = false;
                m.model.status = format!("settled · {n} items");
                m.model.refilter();
            })
            .ok();
        })
        .detach();
    }

    fn row(&self, idx: usize, selected: bool) -> gpui::Div {
        let e = &self.model.entries[idx];
        let label = if e.is_dir {
            format!("▸ {}", e.name)
        } else {
            format!("  {}", e.name)
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .px_2()
            .py_0p5()
            .when(selected, |d| d.bg(rgb(SEL)))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(if e.is_dir { TEXT } else { TEXT_DIM }))
                    .child(SharedString::from(label)),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(TEXT_DIM))
                    .child(SharedString::from(if e.is_dir {
                        "dir".into()
                    } else {
                        fmt_size(e.size)
                    })),
            )
    }
}

impl Render for CloveProto {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width: f32 = window.viewport_size().width.into();
        if self.bar.is_none() || (self.bar_w - width).abs() > 24.0 {
            self.bar = bar_png(width - 24.0);
            self.bar_w = width;
        }
        if self.model.loading {
            let dir = self.model.dir.clone();
            self.load_dir(dir, cx);
        }

        let filtered = self.model.filtered.clone();
        let n = filtered.len();
        let sel = self.model.selected;
        let entity = cx.entity();

        let header = SharedString::from(format!(
            "{} — {}",
            self.model.dir.display(),
            self.model.status
        ));

        div()
            .id("root")
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .p_3()
            .gap_2()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                match key {
                    "down" => {
                        this.model.selected = (this.model.selected + 1).min(this.model.filtered.len().saturating_sub(1));
                        cx.notify();
                    }
                    "up" => {
                        this.model.selected = this.model.selected.saturating_sub(1);
                        cx.notify();
                    }
                    "backspace" => {
                        if let Some(parent) = this.model.dir.parent() {
                            let p = parent.to_path_buf();
                            this.load_dir(p, cx);
                        }
                    }
                    "enter" => {
                        if let Some(&idx) = this.model.filtered.get(this.model.selected) {
                            let e = &this.model.entries[idx];
                            if e.is_dir {
                                let p = this.model.dir.join(&e.name);
                                this.load_dir(p, cx);
                            }
                        }
                    }
                    _ => {
                        // Incremental filter: printable chars append to the query.
                        if let Some(c) = &ev.keystroke.key_char {
                            let c = c.chars().next().unwrap_or('\0');
                            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.' {
                                this.model.query.push(c);
                                this.model.refilter();
                                cx.notify();
                            }
                        }
                    }
                }
                window.refresh();
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(13.0)).child(header))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(TEXT_DIM))
                            .child(SharedString::from(format!("{n} shown"))),
                    ),
            )
            // The dithered storage bar (Clove's signature).
            .child(
                match self.bar.clone() {
                    Some(b) => img(b)
                        .w_full()
                        .h(px(88.0))
                        .rounded(px(RADIUS as f32))
                        .into_any_element(),
                    None => div().h(px(88.0)).w_full().bg(rgb(BAR_BG)).into_any_element(),
                },
            )
            // Incremental filter readout
            .child(
                div()
                    .bg(rgb(PANEL))
                    .rounded(px(6.0))
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(rgb(TEXT_DIM))
                    .child(SharedString::from(if self.model.query.is_empty() {
                        "type to filter · ↑↓ select · enter open · backspace up".to_string()
                    } else {
                        format!("filter: {}", self.model.query)
                    })),
            )
            // Virtualized list
            .child(
                uniform_list("entries", n, move |range, _window, app_cx| {
                    let rows: Vec<usize> = range.clone().collect();
                    let mut built = Vec::new();
                    entity.update(app_cx, |this, _cx| {
                        for row in rows {
                            let idx = filtered[row];
                            built.push(this.row(idx, row == sel));
                        }
                    });
                    built
                })
                .track_scroll(self.list_scroll.clone())
                .flex_1()
                .w_full()
                .overflow_hidden(),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Clove — GPUI prototype".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| CloveProto {
                    model: Model::new(),
                    bar: None,
                    bar_w: 0.0,
                    list_scroll: UniformListScrollHandle::new(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
