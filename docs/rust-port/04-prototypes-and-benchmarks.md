# 4 · Prototypes & benchmarks — measured evidence

Date: 2026-11-15. Machine: Windows 11, AMD Ryzen (integrated Radeon, DX12), Rust 1.99.1 MSVC.
Everything below was **measured on this machine** by building and running both prototypes;
nothing is extrapolated from README claims. Engine numbers (bar renderer, listing, filter)
are identical code in both prototypes, so they are framework-neutral facts; the
framework-specific numbers (runtime frames, launch) are labelled per framework.

Source locations:

- Engine (shared, framework-independent): `prototypes/clove-rune-proto/src/lib.rs` —
  `render_storage_bar`, `box_blur3`, `hash`, `noise`, `load_entries`, `filter_entries`.
  This file is the seed of the proposed `clove-render` + `clove-fs` crates.
- Benchmarks: `prototypes/clove-rune-proto/src/bin/bench.rs` (engine + headless Rune runtime).
- GPUI prototype: `prototypes/clove-gpui-proto/src/main.rs` (same engine, GPUI front end).
- Rune prototype UI: `prototypes/clove-rune-proto/src/main.rs` (same engine, Rune front end).

## 1. What was built

Both prototypes implement the same representative slice of Clove:

1. **The dithered storage bar** — faithful CPU port of `StorageBarRenderer.swift`:
   4×4 Bayer ordered dither at `(v+0.5)/16`, 4 pt cell pitch (2× device scale), 88 pt height,
   radius 10, edge light `exp(−(min(cy,h−cy)−2)/5)`, two-octave value-noise clouds, glints
   (hash > 0.994), lit cells ≈3 pt α242 toward white, dim cells ≈1.5 pt α70, and the glow
   layer (1 px/cell mask, three 5×5 box-blur passes — the renderer's `vImageBoxConvolve`).
   All 9 `StorageKind` hues (P3 → sRGB) with the prototype's placeholder fractions.
   - GPUI: rendered into a PNG blob → `Image::from_bytes`, re-rendered when the window
     resizes by >24 pt.
   - Rune: rendered into a `Pixels` RGBA buffer, uploaded by the framework
     (`.pixels_fit(&s.bar, SurfaceFit::Fill)`).
2. **Virtualized directory listing** (GPUI `uniform_list`, Rune `virtual_rows`) with
   28 pt rows, selection highlight, background thread loading, and a text filter with
   keyboard navigation (↑↓ select, Enter open, Backspace up).
3. **Incremental filtering** per keystroke over the in-memory entries (the Swift app's
   no-index design).

Both build and run. Screenshots:

- GPUI, live window: `img/gpui-proto-live.png`
- Rune, headless snapshot (`--snap`, no GUI session): `img/rune-proto-headless.png`

Rune's headless mode also printed its own perf line in debug:
`gpu init 442.7 ms, first frame 445.97 ms`.

## 2. Engine benchmarks (framework-neutral, release build)

From `cargo run --release --bin bench` (medians; µs):

| Metric | Value |
|---|---|
| Bar render, 512 pt wide (incl. glow blur) | **5.7 ms** |
| Bar render, 936 pt (prototype window) | **12.5 ms** |
| Bar render, 1920 pt | **26.0 ms** |
| `load_entries` on a 48-item folder | **174 µs** |
| `load_entries` on `C:\Windows\System32` (4,734 items, metadata+sort) | **15.7 ms** |
| Filter 100k synthetic entries, empty query (copy-all) | 1.8 ms |
| …query `"a"` | 14.2 ms |
| …query `"app"` | 14.4 ms |
| …query `"readme"` | 14.7 ms |
| …query `"entry-099"` | 24.0 ms |
| Same filter, **names pre-folded** (lowercased once at load), `"app"` | **1.8 ms** |
| Same, pre-folded, `"readme"` | **3.2 ms** |

### Findings that shape the architecture

1. **The full-bar render is the heaviest per-frame item.** ~12.5 ms per full 936-pt frame
   means a naive "re-render every animation frame" bar caps around 80 fps and eats a full
   core. Mitigation is already specified: the settle-stop contract (Swift bar stops after
   2 settled frames) plus **dirty-rect rendering** — during animation only the cloud-drift
   columns change; the edge light and static parts can be cached per width. The prototype
   renders the whole buffer every call; the production `clove-render` must do dirty rects
   (and consider rendering the glow layer only when a cell toggles).
2. **Case-folding inside the filter is 5–8× of the filter cost.** Folding every entry name
   on every keystroke costs 12–13 ms over 100k entries; folding once at directory load
   makes a keystroke ~2 ms. `clove-search` will store a folded-name sidecar per loaded
   directory (bounded by the directory's entry count, freed on close).
3. **Directory listing is I/O-bound**, as expected: 174 µs for a user folder, 15.7 ms for
   System32 — always on the background thread (both prototypes already do this), never the
   frame loop (Rune's own rule: no file I/O during frames).

## 3. Rune runtime (headless, release build)

`Runtime::new` with the prototype's real view over 100,000 synthetic entries:

| Metric | Value |
|---|---|
| First frame (initial build, CPU scene, headless, no GPU upload) | 55.0 ms |
| State change shrinking the list (100k → fewer) + rebuild frame | 31.9 ms |
| **Selection change (keyboard nav) + frame, 100k rows** | **2.9 ms** |

Findings:

- Virtualization works as advertised: with 100k rows, only the ~20 visible rows are built
  per frame, so a selection keystroke costs 2.9 ms — a 120 Hz-class margin.
- The 55 ms first frame is dominated by one-time text-stack/font setup (it does not recur;
  subsequent structural frames are ~32 ms only when the row count collapses wholesale, and
  2.9 ms when the visible window is stable).
- Rune's a11y enforcement is real: the debug build **refused to pass silently** until the
  filter input got a screen-reader label (`El::label`); it prints
  `a focusable text field has no name a screen reader can say` at startup otherwise.

## 4. Launch-to-visible-window (release exes, 3 runs each)

Method: `Start-Process` → poll `MainWindowHandle` + `IsWindowVisible` every 2 ms; stopwatch
from spawn to the first visible window. First run of each includes cold file cache.

| Framework | Runs (ms) | Notes |
|---|---|---|
| Rune | **775 / 597 / 565** | Rune debug headless self-reported `gpu init 443 ms, first frame 446 ms` |
| GPUI | **2714 / 939 / 872** | first run cold; steady-state ~0.9 s |

Both are prototype-grade numbers (unoptimised app logic, first-frame font/text setup), not
release budgets; the difference is consistent with Rune's DX12 pipeline warm-up being
explicitly prewarmed in its headless mode. Phase E sets the real budget.

## 5. Compile footprint (this machine, prototype scale)

| | GPUI 0.2.2 prototype | Rune dev prototype |
|---|---|---|
| Release build of the app crate only (deps cached) | fast (~11 s dev-equivalent) | fast (incremental) |
| Full release build, this machine | **8 m 15 s** | **3 m 11 s** |
| Notable dep friction | `proc-macro-error2` future-incompat warning | `[patch.crates-io]` block **required** (documented); patching `tiny-skia-path` breaks usvg (verified — do not) |

## 6. What the prototypes prove / don't prove

**Proven on this machine:**
- The dithered bar ports faithfully to both frameworks; in Rune it rides a first-class
  `Pixels` primitive; in GPUI it needs a PNG-blob workaround.
- Virtualized lists behave at 100k scale in Rune's headless runtime (2.9 ms keynav frames).
- The no-index filter is viable *only with* a folded-name cache (measured).
- Both apps load directories on background threads and never block the UI.

**Not yet proven (roadmap items):**
- Animated-bar frame budget with dirty-rect rendering (the prototype re-renders fully).
- Screen-reader walkthrough (Narrator) on either framework — Rune's UIA path is wired
  (label warning proves the pipeline), GPUI's Windows a11y is documented-missing.
- IME/CJK input, per-monitor DPI, OneDrive placeholders.
- The placeholders in the bar (equal segment fractions) need real scan proportions from
  the `clove-fs` storage scan (Phase A).
