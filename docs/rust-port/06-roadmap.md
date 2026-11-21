# 6 · Roadmap — Clove for Windows (Rust)

Status: **approved; Phase A complete** (see `07-phase-a-status.md`). Phases are ordered so
every step ends in something runnable, and the two riskiest unknowns (Rune-on-Windows
behaviour, storage-scan performance) are hit early.

## Phase A — Engine crates, headless (no UI) — **done**

Deliverable: `clove-fs`, `clove-search`, `clove-shell`, `clove-render` as tested library
crates; a headless CLI (`clove-cli`) that lists, filters and renders the bar to PNG.
Workspace lives in `clove-rs/`; results are in `07-phase-a-status.md`.

1. **clove-fs**: entry model, `read_dir` listing with metadata + sort, atomic snapshot swap,
   `ReadDirectoryChangesW` watcher, long-path guards. Bench: System32-class listing.
2. **clove-search**: scored fuzzy filter, generation-cancelling worker, **pre-folded name
   cache** (the prototype measured 5–8× faster than per-keystroke lowering — see `04`).
   Bench: 100k entries per keystroke.
3. **clove-render**: the dithered bar ported from the prototype's `lib.rs`, plus dirty-rect
   rendering (only re-render rows whose inputs changed) — the prototype measured ~12 ms per
   full 936-pt frame, which is too much to pay every animation frame; dirty rects and the
   settle-stop contract bring the amortised cost near zero. Tokens module (palette/spacing/
   radii/typography constants).
4. **clove-shell**: trash receipts, re-verify-at-delete, Recycle Bin move (`SHFileOperationW`
   + `FOF_ALLOWUNDO`), receipt resolution via `$I` records, put-back. Integration tests with
   a real recycle → put-back round trip.
5. **clove-cli**: `clove list <dir>`, `clove bar <w> --out out.png`, `clove find <dir> <q>`.
   This is the CI test surface — no window needed.

Exit criteria: engine benches recorded in CI ✅; PNG bar pixel-diffed against the Swift
reference screenshots ⏳ **needs captures from a Mac — moved to Phase C.**

## Phase B — Rune app skeleton

Deliverable: `clove-ui` running the prototype's three surfaces against the real engine crates.

1. Workspace wiring with `[patch.crates-io]` vendored-patches from a **pinned** Rune `dev`
   commit (recorded in the workspace; upgrades are explicit events).
2. Browser surface: virtualized list + background dir load + filter + keyboard navigation
   (the prototype already proves the shape; here it consumes `clove-fs`/`clove-search`).
3. Storage surface: kind scan (3-concurrent, `.utility`-equivalent priority, 30-min cache),
   animated bar via `Pixels` + settle-stop, kind legend.
4. Shell surface: delete-to-trash with receipt, undo toast, error badges.

Exit criteria: daily-driver-able over `C:\Users\<user>`; a11y labels on all rows (Narrator
reads the list).

**Status (2026-11-21): Phase B skeleton complete; real storage scan landed.** Numbers and
findings in `docs/rust-port/08-phase-b-status.md`. Remaining Phase B work: live per-kind
growth in the bar, device-scale-aware rendering, 30-min cache, kind legend, a11y labels.

## Phase C — Identity & polish pass

1. **Prerequisite:** capture the Swift app on a Mac (bar at rest, bar mid-scan, browser,
   storage card) into `img/reference/`. Phase A's pixel-diff exit criterion moved here
   because it cannot be satisfied without them.
2. Pixel-diff the bar and chrome against those captures; fix drift. Known drift to fix:
   bilinear glow upscale, overlay layer (filaments/seams/motes), glow spill pad, 1 px border.
3. Motion: springs on selection, presence transitions, settle-stop (already in Rune's model).
4. Typography scale + spacing audit against `Metrics.swift`/`Typography.swift`.
5. Icons: trace Clove's glyph set to vector paths; pattern helpers for hairlines/nested cards.

## Phase D — Windows integration hardening

1. DPI scaling tests (100/125/150/200%), per-monitor DPI change.
2. IME check (CJK input in the filter box), keyboard layouts.
3. OneDrive placeholders: list without hydrating; explicit intent for offline files.
4. Long paths (`\\?\`), junctions, drive roots, network shares (slow-path policy).
5. Jump list / taskbar integration if the Swift app had equivalents (per Phase 0 inventory).

## Phase E — Performance & release

1. Rune `--profile perf`-style benches wired into CI; launch-to-first-frame budget.
2. Memory budget check (procwatch): entry cache bounds (entries + bytes).
3. Installer (MSIX or NSIS), auto-update decision, telemetry decision (default: none).

## Milestone dates

Not dated until approval; estimate as effort bands instead:

- Phase A: the largest chunk of *new* Rust (~2/5 of total effort).
- Phase B: medium — the prototype de-risks it.
- Phase C/D/E: each small-medium; C and D can interleave with B's tail.

## What we explicitly defer

- Multi-window, tabs, tags: not in the Swift app.
- macOS support in Rust: Windows only (the Swift app remains the Mac product).
- A search index: preserved no-index design; revisit only if benches show a real need.
