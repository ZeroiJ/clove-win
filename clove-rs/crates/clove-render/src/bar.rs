//! The dithered storage bar — a port of `Clove/Features/Storage/StorageBarRenderer.swift`.
//!
//! Output is a straight-alpha RGBA buffer composited in the Swift layer order
//! (glow → per-kind fills → cells) over transparency, so the UI draws it on the surface
//! colour. Rendering is incremental: cells are compared against the previous frame and
//! only changed cell rectangles are recomposited, which makes a settled bar (or a bar
//! whose clouds drift slowly) almost free.

use crate::tokens::STORAGE_KIND_RGB;

pub const PITCH: f32 = 4.0;
pub const DEFAULT_HEIGHT: f32 = 88.0;
pub const DEFAULT_SCALE: f32 = 2.0;

/// 4×4 Bayer matrix normalised to (i + 0.5)/16 — `StorageBarRenderer.bayer`.
const BAYER: [f32; 16] = {
    let m = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let mut t = [0f32; 16];
    let mut i = 0;
    while i < 16 {
        t[i] = (m[i] as f32 + 0.5) / 16.0;
        i += 1;
    }
    t
};

/// One storage-kind slice of the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub rgb: [f32; 3],
    /// Fraction of the bar (0..=1). Fractions are used as-is; the bar ends where the
    /// fractions end, and cells past the end render as empty cells (as the Swift renderer).
    pub fraction: f32,
}

impl Segment {
    pub fn new(rgb: [f32; 3], fraction: f32) -> Self {
        Segment { rgb, fraction }
    }

    /// A segment using `StorageKind`'s dark-mode hue.
    pub fn kind(index: usize, fraction: f32) -> Self {
        Segment {
            rgb: STORAGE_KIND_RGB[index % STORAGE_KIND_RGB.len()],
            fraction,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BarSpec {
    pub width_pts: f32,
    pub height_pts: f32,
    pub scale: f32,
    pub segments: Vec<Segment>,
}

impl BarSpec {
    pub fn new(width_pts: f32, segments: Vec<Segment>) -> Self {
        BarSpec {
            width_pts,
            height_pts: DEFAULT_HEIGHT,
            scale: DEFAULT_SCALE,
            segments,
        }
    }

    /// All nine kinds with equal fractions (used by benchmarks).
    pub fn even(width_pts: f32) -> Self {
        let segments = (0..STORAGE_KIND_RGB.len())
            .map(|k| Segment::kind(k, 1.0 / STORAGE_KIND_RGB.len() as f32))
            .collect();
        BarSpec::new(width_pts, segments)
    }

    /// The prototype's uneven fractions, shown until the first storage scan finishes.
    pub fn placeholder(width_pts: f32) -> Self {
        const PLACEHOLDER: [f32; 9] = [0.18, 0.16, 0.11, 0.09, 0.12, 0.07, 0.13, 0.09, 0.05];
        let segments = PLACEHOLDER
            .iter()
            .enumerate()
            .map(|(kind, &fraction)| Segment::kind(kind, fraction))
            .collect();
        BarSpec::new(width_pts, segments)
    }

    pub fn with_height(mut self, height_pts: f32) -> Self {
        self.height_pts = height_pts;
        self
    }

    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
}

/// Incremental CPU renderer for one bar size. Reuse it across frames; construct a new one
/// when the width or the segment layout changes.
pub struct BarRenderer {
    spec: BarSpec,
    w_px: usize,
    h_px: usize,
    cols: usize,
    rows: usize,
    pitch_px: f32,
    radius_px: f32,
    ox_px: f32,
    oy_px: f32,
    lit_px: usize,
    dim_px: usize,
    lit_off: usize,
    dim_off: usize,
    kinds: Vec<i32>,
    thresholds: Vec<f32>,
    edge_light: Vec<f32>,
    fill: Vec<[u8; 4]>,
    buf: Vec<u8>,
    cur_paint: Vec<[u8; 4]>,
    cur_lit: Vec<bool>,
    cur_glow: Vec<[u8; 4]>,
    prev_paint: Vec<[u8; 4]>,
    prev_glow: Vec<[u8; 4]>,
    glow_premul: Vec<[u8; 4]>,
    prev_glow_premul: Vec<[u8; 4]>,
    glint_step: i64,
    glints: Vec<f32>,
    flicker_seed: Vec<f32>,
    last_t: Option<f32>,
    initialized: bool,
    /// Bumped whenever `render` repaints at least one cell. The UI compares it to skip
    /// re-uploading a buffer that did not change.
    frame_seq: u64,
}

impl BarRenderer {
    pub fn new(spec: BarSpec) -> Self {
        let scale = spec.scale.max(0.5);
        let w_px = (spec.width_pts * scale).round().max(1.0) as usize;
        let h_px = (spec.height_pts * scale).round().max(1.0) as usize;
        let cols = (spec.width_pts / PITCH).floor().max(1.0) as usize;
        let rows = (spec.height_pts / PITCH).floor().max(1.0) as usize;
        let pitch_px = PITCH * scale;
        let ox = (spec.width_pts - cols as f32 * PITCH) / 2.0;
        let oy = (spec.height_pts - rows as f32 * PITCH) / 2.0;
        let radius_px = crate::tokens::RADIUS_BAR * scale;

        let lit_px = (3.0 * scale).round().max(2.0) as usize;
        let dim_px = (1.5 * scale).round().max(1.0) as usize;
        let lit_off = ((pitch_px - lit_px as f32) / 2.0).round() as usize;
        let dim_off = ((pitch_px - dim_px as f32) / 2.0).round() as usize;

        // Segment edges in points; cells past the last edge are empty.
        let mut edges = Vec::with_capacity(spec.segments.len() + 1);
        let mut acc = 0.0f32;
        edges.push(0.0);
        for s in &spec.segments {
            acc = (acc + s.fraction.max(0.0)).min(1.0);
            edges.push(acc * spec.width_pts);
        }

        let cell_count = cols * rows;
        let mut kinds = vec![-1i32; cell_count];
        let mut thresholds = vec![0.0f32; cell_count];
        let mut edge_light = vec![0.0f32; cell_count];
        for j in 0..rows {
            let cy = oy + (j as f32 + 0.5) * PITCH;
            let edge = (-(cy.min(spec.height_pts - cy) - 2.0) / 5.0).exp();
            for i in 0..cols {
                let c = j * cols + i;
                let cx = ox + (i as f32 + 0.5) * PITCH;
                let mut kind = -1;
                for k in 0..spec.segments.len() {
                    if cx >= edges[k] && cx < edges[k + 1] {
                        kind = k as i32;
                        break;
                    }
                }
                kinds[c] = kind;
                thresholds[c] = BAYER[(j & 3) * 4 + (i & 3)];
                edge_light[c] = edge;
            }
        }

        // Per-pixel-column fill tint (α 0.12 of the owning kind, Swift's `fills`).
        let mut fill = vec![[0u8; 4]; w_px];
        for (x, slot) in fill.iter_mut().enumerate() {
            let px = x as f32 / scale;
            for k in 0..spec.segments.len() {
                if px >= edges[k] && px < edges[k + 1] {
                    let rgb = spec.segments[k].rgb;
                    *slot = [
                        (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8,
                        31,
                    ];
                    break;
                }
            }
        }

        BarRenderer {
            spec,
            w_px,
            h_px,
            cols,
            rows,
            pitch_px,
            radius_px,
            ox_px: ox * scale,
            oy_px: oy * scale,
            lit_px,
            dim_px,
            lit_off,
            dim_off,
            kinds,
            thresholds,
            edge_light,
            fill,
            buf: vec![0; w_px * h_px * 4],
            cur_paint: vec![[0; 4]; cell_count],
            cur_lit: vec![false; cell_count],
            cur_glow: vec![[0; 4]; cell_count],
            prev_paint: vec![[0; 4]; cell_count],
            prev_glow: vec![[0; 4]; cell_count],
            glow_premul: vec![[0; 4]; cell_count],
            prev_glow_premul: vec![[0; 4]; cell_count],
            glint_step: i64::MIN,
            glints: vec![0.0; cell_count],
            flicker_seed: (0..cell_count)
                .map(|c| hash((c % cols) as i32, (c / cols) as i32) * 7.0)
                .collect(),
            last_t: None,
            initialized: false,
            frame_seq: 0,
        }
    }

    /// A counter that changes only when a `render` call actually repainted a cell.
    ///
    /// The bar settles: clouds drift slowly and glints blink, so many frames produce no
    /// visible change. The UI uses this to upload pixels only when there is something new.
    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }

    pub fn spec(&self) -> &BarSpec {
        &self.spec
    }

    /// `(width, height)` of the buffer in device pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.w_px, self.h_px)
    }

    /// Number of 4 pt cells the bar is made of (`cols × rows`).
    pub fn cell_count(&self) -> usize {
        self.kinds.len()
    }

    /// The latest frame, straight-alpha RGBA, row-major.
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// Renders at time `t` (seconds) and returns the RGBA buffer.
    pub fn render(&mut self, t: f32) -> &[u8] {
        // Settle-stop: the same instant renders to the same pixels, so a paused bar costs
        // nothing. This is what lets the UI stop scheduling frames once the animation ends.
        if self.initialized && self.last_t == Some(t) {
            return &self.buf;
        }
        self.last_t = Some(t);

        let drift = t * 0.32;
        let step = (t * 1.6).floor() as i64;
        let glints_changed = step != self.glint_step;
        if glints_changed {
            self.glint_step = step;
            // Each cell offsets the glint clock by its own seed, so glints twinkle
            // independently instead of the whole bar flashing on the same frame.
            for (c, glint) in self.glints.iter_mut().enumerate() {
                let seed = self.flicker_seed[c];
                *glint = if hash(c as i32, (step as f32 + seed) as i32) > 0.994 {
                    1.0
                } else {
                    0.0
                };
            }
        }

        // 1. Per-cell light, colour, and glow source.
        for c in 0..self.glints.len() {
            let i = c % self.cols;
            let j = c / self.cols;
            let kind = self.kinds[c];
            if kind < 0 {
                self.cur_paint[c] = [255, 255, 255, 18];
                self.cur_lit[c] = false;
                self.glow_premul[c] = [0; 4];
                continue;
            }
            let cloud = noise(i as f32 * 0.045 - drift, j as f32 * 0.11 + drift * 0.3) * 0.65
                + noise(i as f32 * 0.13 + drift * 0.6, j as f32 * 0.3) * 0.35;
            let level = -0.04
                + 0.66 * self.edge_light[c]
                + 0.44 * smooth(clamp01((cloud - 0.42) / 0.38));
            let glint = self.glints[c];
            let rgb = self.spec.segments[kind as usize].rgb;

            if level + glint > self.thresholds[c] {
                // Lit: kind hue lerped toward white by heat/focus/front/glint.
                let wt = (glint * 0.6).min(1.0);
                let lit = [
                    (rgb[0] + (1.0 - rgb[0]) * wt).clamp(0.0, 1.0),
                    (rgb[1] + (1.0 - rgb[1]) * wt).clamp(0.0, 1.0),
                    (rgb[2] + (1.0 - rgb[2]) * wt).clamp(0.0, 1.0),
                ];
                self.cur_lit[c] = true;
                self.cur_paint[c] = [
                    (lit[0] * 255.0).round() as u8,
                    (lit[1] * 255.0).round() as u8,
                    (lit[2] * 255.0).round() as u8,
                    242,
                ];
                let alpha = 255.0 * (0.55 + 0.45 * wt);
                self.glow_premul[c] = [
                    (lit[0] * alpha).round().clamp(0.0, 255.0) as u8,
                    (lit[1] * alpha).round().clamp(0.0, 255.0) as u8,
                    (lit[2] * alpha).round().clamp(0.0, 255.0) as u8,
                    alpha.round().clamp(0.0, 255.0) as u8,
                ];
            } else {
                // Dim: the kind hue at α70.
                self.cur_lit[c] = false;
                self.cur_paint[c] = [
                    (rgb[0] * 255.0).round() as u8,
                    (rgb[1] * 255.0).round() as u8,
                    (rgb[2] * 255.0).round() as u8,
                    70,
                ];
                self.glow_premul[c] = [0; 4];
            }
        }

        // 2. Blur the premultiplied glow only when its source changed. This is the only
        //    writer of `cur_glow`: the blur spreads into dim and empty neighbours, so those
        //    cells must keep their blurred value rather than being cleared per frame.
        let glow_changed = glints_changed
            || self
                .glow_premul
                .iter()
                .zip(&self.prev_glow_premul)
                .any(|(a, b)| a != b);
        if glow_changed || !self.initialized {
            let blurred = box_blur3(&self.glow_premul, self.cols, self.rows);
            for (c, px) in blurred.iter().enumerate() {
                self.cur_glow[c] = if px[3] == 0 {
                    [0; 4]
                } else {
                    let a = px[3] as f32;
                    [
                        (px[0] as f32 * 255.0 / a).round().clamp(0.0, 255.0) as u8,
                        (px[1] as f32 * 255.0 / a).round().clamp(0.0, 255.0) as u8,
                        (px[2] as f32 * 255.0 / a).round().clamp(0.0, 255.0) as u8,
                        px[3],
                    ]
                };
            }
        }

        // 3. Recomposite only cells whose paint or glow changed.
        let mut any_dirty = false;
        for c in 0..self.glints.len() {
            let dirty = !self.initialized
                || self.cur_paint[c] != self.prev_paint[c]
                || self.cur_glow[c] != self.prev_glow[c];
            if dirty {
                self.draw_cell(c);
                any_dirty = true;
            }
        }
        if any_dirty {
            self.frame_seq += 1;
        }

        self.prev_paint.copy_from_slice(&self.cur_paint);
        self.prev_glow.copy_from_slice(&self.cur_glow);
        self.prev_glow_premul.copy_from_slice(&self.glow_premul);
        self.initialized = true;
        &self.buf
    }

    /// The settled bar (t = 0), as the app shows it when nothing is animating.
    pub fn render_settled(&mut self) -> &[u8] {
        self.render(0.0)
    }

    fn draw_cell(&mut self, c: usize) {
        let i = c % self.cols;
        let j = c / self.cols;
        let x0 = (self.ox_px + i as f32 * self.pitch_px).round() as usize;
        let y0 = (self.oy_px + j as f32 * self.pitch_px).round() as usize;
        let x1 = (x0 + self.pitch_px as usize).min(self.w_px);
        let y1 = (y0 + self.pitch_px as usize).min(self.h_px);
        let (off, size) = if self.cur_lit[c] {
            (self.lit_off, self.lit_px)
        } else {
            (self.dim_off, self.dim_px)
        };
        let sx0 = x0 + off;
        let sy0 = y0 + off;
        let sx1 = sx0 + size;
        let sy1 = sy0 + size;

        let w = self.w_px as f32;
        let h = self.h_px as f32;
        let glow = self.cur_glow[c];
        let paint = self.cur_paint[c];

        for y in y0..y1 {
            for x in x0..x1 {
                let idx = (y * self.w_px + x) * 4;
                let inside = inside_rounded(x as f32 + 0.5, y as f32 + 0.5, w, h, self.radius_px);
                if !inside {
                    self.buf[idx..idx + 4].fill(0);
                    continue;
                }
                self.buf[idx..idx + 4].fill(0);
                if glow[3] > 0 {
                    over(
                        &mut self.buf[idx..idx + 4],
                        [glow[0], glow[1], glow[2]],
                        glow[3],
                    );
                }
                let f = self.fill[x];
                if f[3] > 0 {
                    over(&mut self.buf[idx..idx + 4], [f[0], f[1], f[2]], f[3]);
                }
                if x >= sx0 && x < sx1 && y >= sy0 && y < sy1 {
                    over(
                        &mut self.buf[idx..idx + 4],
                        [paint[0], paint[1], paint[2]],
                        paint[3],
                    );
                }
            }
        }
    }
}

fn over(dst: &mut [u8], rgb: [u8; 3], a: u8) {
    if a == 0 {
        return;
    }
    let sa = a as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    let inv = 1.0 / oa;
    for ch in 0..3 {
        let v = (rgb[ch] as f32 * sa + dst[ch] as f32 * da * (1.0 - sa)) * inv;
        dst[ch] = v.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn inside_rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> bool {
    let r = r.min(w * 0.5).min(h * 0.5);
    if r <= 0.0 {
        return true;
    }
    let cx = x.clamp(r, w - r);
    let cy = y.clamp(r, h - r);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Integer hash — `StorageBarRenderer.hash`. Note the `&+`: Swift adds the two
/// multiplied terms, it does not XOR them.
fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374761393)
        .wrapping_add((y as u32).wrapping_mul(668265263));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^= h >> 16;
    (h & 0xffffff) as f32 / 16777216.0
}

/// Value noise — bilinear over four corner hashes with smoothstep.
fn noise(x: f32, y: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let tx = x - xi as f32;
    let ty = y - yi as f32;
    let sx = smooth(tx);
    let sy = smooth(ty);
    let a = hash(xi, yi);
    let b = hash(xi + 1, yi);
    let c = hash(xi, yi + 1);
    let d = hash(xi + 1, yi + 1);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Three separable 5×5 box passes (radius 2), like the renderer's three
/// `vImageBoxConvolve_ARGB8888` passes. Premultiplied in, premultiplied out.
fn box_blur3(src: &[[u8; 4]], w: usize, h: usize) -> Vec<[u8; 4]> {
    let mut a = src.to_vec();
    let mut b = vec![[0u8; 4]; src.len()];
    for _ in 0..3 {
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                let mut n = 0u32;
                for dx in -2i32..=2 {
                    let xx = x as i32 + dx;
                    if (0..w as i32).contains(&xx) {
                        let p = a[y * w + xx as usize];
                        for ch in 0..4 {
                            sum[ch] += p[ch] as u32;
                        }
                        n += 1;
                    }
                }
                let mut out = [0u8; 4];
                for ch in 0..4 {
                    out[ch] = (sum[ch] / n) as u8;
                }
                b[y * w + x] = out;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                let mut n = 0u32;
                for dy in -2i32..=2 {
                    let yy = y as i32 + dy;
                    if (0..h as i32).contains(&yy) {
                        let p = b[yy as usize * w + x];
                        for ch in 0..4 {
                            sum[ch] += p[ch] as u32;
                        }
                        n += 1;
                    }
                }
                let mut out = [0u8; 4];
                for ch in 0..4 {
                    out[ch] = (sum[ch] / n) as u8;
                }
                a[y * w + x] = out;
            }
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(width: f32) -> BarSpec {
        BarSpec::even(width)
    }

    #[test]
    fn dimensions_follow_scale() {
        let mut r = BarRenderer::new(spec(400.0));
        let (w, h) = r.size();
        assert_eq!((w, h), (800, 176));
        assert_eq!(r.render_settled().len(), w * h * 4);
    }

    #[test]
    fn corners_are_transparent_and_interior_is_painted() {
        let mut r = BarRenderer::new(spec(400.0));
        let (w, h) = r.size();
        let buf = r.render_settled();
        let alpha = |x: usize, y: usize| buf[(y * w + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "rounded corner must stay transparent");
        assert_eq!(alpha(w - 1, h - 1), 0);
        assert_eq!(alpha(w - 1, 0), 0);
        assert_eq!(alpha(0, h - 1), 0);

        // Cell squares sit in the middle of each 4 pt cell, so the bar's exact centre can
        // fall in a gap where only the α12 fill tint is present. Lit cells read as opaque.
        let strong = buf.chunks_exact(4).filter(|p| p[3] >= 240).count();
        let faint = buf
            .chunks_exact(4)
            .filter(|p| (1..=60).contains(&p[3]))
            .count();
        assert!(
            strong > (w * h) / 100,
            "lit cells should cover a real area, got {strong} px"
        );
        assert!(
            faint > (w * h) / 100,
            "the α12 fill tint should be visible between cells, got {faint} px"
        );

        let painted = buf.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(painted > (w * h) / 2, "most of the bar is painted: {painted}");
    }

    #[test]
    fn fresh_renderers_are_deterministic() {
        let mut a = BarRenderer::new(spec(320.0));
        let mut b = BarRenderer::new(spec(320.0));
        assert_eq!(a.render(0.4), b.render(0.4));
    }

    #[test]
    fn incremental_frames_equal_a_fresh_render() {
        let mut incremental = BarRenderer::new(spec(936.0));
        for t in [0.0, 0.05, 1.3, 2.7, 9.0] {
            incremental.render(t);
        }
        let mut fresh = BarRenderer::new(spec(936.0));
        let expected = fresh.render(9.0);
        assert_eq!(
            incremental.buffer(),
            expected,
            "dirty-rect compositing must match a from-scratch frame"
        );
    }

    #[test]
    fn same_time_renders_identically_twice() {
        let mut r = BarRenderer::new(spec(300.0));
        let first = r.render(0.0).to_vec();
        let second = r.render(0.0).to_vec();
        assert_eq!(first, second);
    }

    #[test]
    fn segments_change_the_colours_left_to_right() {
        let mut r = BarRenderer::new(spec(360.0));
        let (w, h) = r.size();
        let buf = r.render_settled();
        // Sample the lit/dim squares of the first and last segment; their hues differ.
        let sample = |x: usize| {
            let y = h / 2;
            let p = &buf[(y * w + x) * 4..(y * w + x) * 4 + 4];
            (p[0] as i32, p[1] as i32, p[2] as i32)
        };
        let left = sample(w / 20);
        let right = sample(w - w / 20);
        assert_ne!(left, right, "each kind keeps its own hue: {left:?} vs {right:?}");
    }
}
