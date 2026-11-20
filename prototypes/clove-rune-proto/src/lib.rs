//! Engine code of the Rune prototype, shared between the UI (`main.rs`) and the
//! headless benchmark (`bin/bench.rs`). Framework-independent on purpose: this
//! is the seed of the future `clove-render` and `clove-fs` crates, and it must
//! not depend on the UI framework beyond a couple of numeric constants.

use std::path::Path;

// ---------------------------------------------------------------------------
// Clove palette — Clove/Design/Palette.swift OKLCH neutrals (dark) and
// Clove/Storage/StorageKind.swift dark-mode bar hues (P3 → 0..1 sRGB here).
// ---------------------------------------------------------------------------

/// rgb 0..1 per StorageKind bar hue, dark mode, in enum order.
pub const KIND_RGB: [[f32; 3]; 9] = [
    [0.349, 0.612, 1.000], // Apps
    [0.436, 0.855, 0.453], // Code
    [0.946, 0.404, 0.691], // Photos
    [0.639, 0.449, 1.000], // Movies
    [0.332, 0.826, 0.946], // Documents
    [0.969, 0.818, 0.265], // Music
    [0.698, 0.729, 0.784], // System
    [0.982, 0.558, 0.233], // Caches
    [0.941, 0.328, 0.284], // Trash
];

pub const KIND_NAMES: [&str; 9] = [
    "Apps", "Code", "Photos", "Movies", "Documents", "Music", "System", "Caches", "Trash",
];

// ---------------------------------------------------------------------------
// StorageBarRenderer — faithful Rust port of the CPU dithering engine,
// framework-independent. Produces raw RGBA; each framework uploads it its
// own way.
// ---------------------------------------------------------------------------

pub const PITCH: f32 = 4.0; // cell pitch, points
pub const BAR_H: f32 = 88.0; // points
pub const RADIUS: f32 = 10.0; // corner radius
pub const SCALE: f32 = 2.0; // device pixels per point

/// 4×4 Bayer matrix, normalised to (i + 0.5)/16 — matches StorageBarRenderer.bayer.
pub const BAYER: [f32; 16] = {
    let m = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let mut t = [0f32; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = (m[i] as f32 + 0.5) / 16.0;
        i += 1;
    }
    t
};

/// Integer hash — port of StorageBarRenderer.hash.
pub fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263);
    h ^= h >> 13;
    h = h.wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xffffff) as f32 / 16777216.0
}

/// Value noise — bilinear over four corner hashes with smoothstep.
pub fn noise(x: f32, y: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let tx = x - xi as f32;
    let ty = y - yi as f32;
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

/// Proportions per kind (equal here; the real bar uses scan results).
pub const SEG_FRACS: [f32; 9] = [0.18, 0.16, 0.11, 0.09, 0.12, 0.07, 0.13, 0.09, 0.05];

/// Renders the settled bar (t = 0, no heat/focus/scan front): clouds + edge
/// light + glints only, plus the soft glow layer (3× 5×5 box blur, as the
/// renderer's vImage passes).
///
/// Returns (w, h, RGBA).
pub fn render_storage_bar(width_pts: f32) -> (usize, usize, Vec<u8>) {
    let w_px = (width_pts * SCALE).round() as usize;
    let h_px = (BAR_H * SCALE).round() as usize;
    let pitch = PITCH * SCALE;
    let radius = RADIUS * SCALE;
    let rows = (BAR_H / PITCH).round() as usize;
    let cols = (width_pts / PITCH).round() as usize;

    let mut edges = vec![0.0f32; KIND_RGB.len() + 1];
    let total: f32 = SEG_FRACS.iter().sum();
    let mut acc = 0.0;
    for (k, f) in SEG_FRACS.iter().enumerate() {
        acc += f / total;
        edges[k + 1] = acc * width_pts;
    }

    let mut buf = vec![0u8; w_px * h_px * 4];
    let mut glow = vec![0u8; cols * rows]; // 1 px per cell, as the renderer's glow image

    let put = |buf: &mut [u8], cx: f32, cy: f32, half: f32, c: [f32; 3], a: f32| {
        let x0 = ((cx - half).round() as isize).clamp(0, w_px as isize - 1) as usize;
        let x1 = ((cx + half).round() as isize).clamp(0, w_px as isize - 1) as usize;
        let y0 = ((cy - half).round() as isize).clamp(0, h_px as isize - 1) as usize;
        let y1 = ((cy + half).round() as isize).clamp(0, h_px as isize - 1) as usize;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = (y * w_px + x) * 4;
                for ch in 0..3 {
                    let d = buf[i + ch] as f32;
                    buf[i + ch] = (c[ch] * 255.0 * a + d * (1.0 - a)) as u8;
                }
                buf[i + 3] = 255;
            }
        }
    };

    for gy_row in 0..rows {
        for gc in 0..cols {
            let cx_pt = (gc as f32 + 0.5) * PITCH;
            let cy_pt = (gy_row as f32 + 0.5) * PITCH;
            let cx = cx_pt * SCALE;
            let cy = cy_pt * SCALE;

            // Rounded-rect mask: distance from the nearest corner centre.
            let hw = w_px as f32;
            let dxr = if cx < radius {
                radius - cx
            } else if cx > hw - radius {
                cx - (hw - radius)
            } else {
                0.0
            };
            let dyr = if cy < radius {
                radius - cy
            } else if cy > h_px as f32 - radius {
                cy - (h_px as f32 - radius)
            } else {
                0.0
            };
            if dxr * dxr + dyr * dyr > (radius - SCALE) * (radius - SCALE) {
                continue;
            }

            // Which kind owns this column?
            let mut kind = 0usize;
            for k in 0..KIND_RGB.len() {
                if cx_pt >= edges[k] && cx_pt < edges[k + 1] {
                    kind = k;
                    break;
                }
            }

            // Edge light gathers along top and bottom like a tube.
            let d_edge = cy_pt.min(BAR_H - cy_pt);
            let edge_light = (-(d_edge - 2.0) / 5.0).exp();

            // Drifting value-noise clouds, two octaves (drift = 0 at t=0).
            let cloud =
                noise(cx_pt * 0.045, cy_pt * 0.11) * 0.65 + noise(cx_pt * 0.13, cy_pt * 0.3) * 0.35;
            let s = ((cloud - 0.42) / 0.38).clamp(0.0, 1.0);
            let smooth_c = s * s * (3.0 - 2.0 * s);

            let mut level = -0.04 + 0.66 * edge_light + 0.44 * smooth_c;

            // Settled: no heat (0), no scan front (0), no hover (0). Glints flash.
            let g = hash(gc as i32, gy_row as i32);
            let glint = if g > 0.994 { (g - 0.994) / 0.006 } else { 0.0 };
            level += glint * 0.5;

            let threshold = BAYER[((gy_row & 3) * 4) + (gc & 3)];
            let kc = KIND_RGB[kind];
            if level + glint > threshold {
                // Lit cell: ~3 pt square, alpha 242, toward white-hot; glow source.
                let wt = (level * 0.95 + glint * 0.6).min(1.0) * 0.8;
                let lit = [
                    kc[0] + (1.0 - kc[0]) * wt,
                    kc[1] + (1.0 - kc[1]) * wt,
                    kc[2] + (1.0 - kc[2]) * wt,
                ];
                put(&mut buf, cx, cy, 1.5 * SCALE, lit, 242.0 / 255.0);
                glow[gy_row * cols + gc] = 255;
            } else {
                // Dim cell: ~1.5 pt square, alpha 70.
                let dim = [kc[0] * 0.55, kc[1] * 0.55, kc[2] * 0.55];
                put(&mut buf, cx, cy, 0.75 * SCALE, dim, 70.0 / 255.0);
            }
        }
    }

    // Glow: blur the 1 px/cell image three times (5×5 box), then composite
    // upscaled over the bar at low alpha, as CALayer's glowLayer does.
    let blurred = box_blur3(&glow, cols, rows, 2);
    let cell_w = pitch;
    let cell_h = h_px as f32 / rows as f32;
    for gy_row in 0..rows {
        for gc in 0..cols {
            let v = blurred[gy_row * cols + gc] as f32 / 255.0;
            if v <= 0.01 {
                continue;
            }
            let cx = (gc as f32 + 0.5) * cell_w;
            let cy = (gy_row as f32 + 0.5) * cell_h;
            let half = cell_w.max(cell_h);
            put(&mut buf, cx, cy, half, [0.55, 0.62, 0.78], v * 0.16);
        }
    }

    (w_px, h_px, buf)
}

/// Three horizontal+vertical 5×5 box passes (radius 2), matching the
/// renderer's three vImageBoxConvolve_ARGB8888 passes.
pub fn box_blur3(src: &[u8], w: usize, h: usize, r: i32) -> Vec<u8> {
    let mut a = src.to_vec();
    let mut b = vec![0u8; src.len()];
    for _ in 0..3 {
        // horizontal
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                for dx in -r..=r {
                    let xx = x as i32 + dx;
                    if (0..w as i32).contains(&xx) {
                        sum += a[y * w + xx as usize] as u32;
                        n += 1;
                    }
                }
                b[y * w + x] = (sum / n) as u8;
            }
        }
        // vertical
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                for dy in -r..=r {
                    let yy = y as i32 + dy;
                    if (0..h as i32).contains(&yy) {
                        sum += b[yy as usize * w + x] as u32;
                        n += 1;
                    }
                }
                a[y * w + x] = (sum / n) as u8;
            }
        }
    }
    a
}

// ---------------------------------------------------------------------------
// Filesystem model — mirrors the future `clove-fs` crate surface.
// ---------------------------------------------------------------------------

pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

pub fn load_entries(dir: &Path) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
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
    entries
}

/// Per-keystroke scored filter (substring here), over the in-memory entries.
/// This mirrors the Swift app's decision: no index; filter on every keystroke.
pub fn filter_entries(entries: &[Entry], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}
