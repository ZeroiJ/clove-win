# 5 · Proposed architecture — Clove for Windows (Rust-native)

Status: **proposal — awaiting user approval before any full implementation.**
Companion documents: `01-repository-architecture.md` (what the Swift app does),
`02-reusability.md` (what ports as-is), `03-framework-comparison.md` (why Rune),
`04-prototypes-and-benchmarks.md` (measured evidence), `06-roadmap.md` (delivery order).

---

## 1. Design goals, in priority order

1. **Speed.** Directory listing and keystroke-filter feel instant on Windows-sized folders
   (10k+ entries); every visible surface (list, storage bar, search) budgets its frame.
2. **Identity preservation.** Clove's dithered storage bar, monochrome palette, typography,
   spacing and motion survive the port pixel-for-pixel. Rendering of identity-bearing art is
   framework-independent code, not UI-framework widgets.
3. **Engine/UI separation.** The filesystem engine never imports a UI framework. It is
   testable headlessly and, if the UI framework ever has to change (GPUI fallback case), it
   survives untouched.
4. **Safety.** Deletion follows the Swift app's model exactly: move to a receipt-tracked trash,
   re-verify each path immediately before the delete, keep receipts for undo.
5. **Windows-native.** UIA accessibility, ClearType text, DX12, long paths, OneDrive placeholders.

## 2. Workspace layout

```
clove/                      # new workspace (does not touch the Swift tree)
├── crates/
│   ├── clove-fs/           # filesystem engine (no UI deps)
│   ├── clove-search/       # scoring/filter engine (no UI deps)
│   ├── clove-render/       # identity rendering: dithered bar, patterns, shades (no UI deps)
│   ├── clove-shell/        # trash/receipt engine, recycle bin, file ops (no UI deps)
│   └── clove-ui/           # Rune app: views, state, animation wiring (the only Rune dep)
├── xtask/                  # headless benchmark + screenshot harness
└── Cargo.toml              # workspace; carries the [patch.crates-io] block (see §7)
```

Dependency direction is one-way and enforced by review (and optionally by `cargo-deny`):

```
clove-ui → clove-render, clove-fs, clove-search, clove-shell
clove-shell → clove-fs
everything else: std + a few leaf crates only
```

`clove-ui` is the *only* crate that may name `rune`. This is the same layering the GPUI
prototype validated implicitly, and it is what keeps the documented GPUI-fallback cheap: a
framework change replaces ~1 of 5 crates.

## 3. `clove-fs` — filesystem engine

Ported from the Swift scanner layer (`Clove/Storage/`, `Clove/Core/`), restructured for
Rust concurrency.

### 3.1 Directory listing
- One producer thread per open directory using `std::fs::read_dir` behind a small
  channel/actor; results stream into a `Vec<Entry>` snapshot swapped atomically
  (`arc-swap`), so the UI never blocks on I/O.
- Entries carry: name, kind (dir/file/symlink), size (client-side sum for dirs, `GetFileInformation`
  style metadata for files), modified time, hidden/system flags from `FILE_ATTRIBUTE` bits
  (long-path safe: all paths handled as `\\?\` where needed).
- Re-scan on directory-change notification (ReadDirectoryChangesW watcher per open folder)
  instead of polling — this is a Windows upgrade over the Swift timer-based refresh.

### 3.2 Storage kind scan
- Same domain model as `StorageKind.swift`: a fixed ordered set of kinds with hues.
- Concurrency contract preserved from the Swift app: **scan at most 3 kinds concurrently at
  `.utility` priority**; results cached **30 minutes**; partial results render as soon as
  each kind lands (the bar animates from whatever is known, exactly like the original).
- Windows implementation notes: kinds map onto known-folder locations + size heuristics
  (e.g. Apps → `Program Files`/UWP `WindowsApps` where readable, Photos → `Pictures`,
  Videos → `Videos`, Mail → Outlook/WindowsMail stores, etc.). Sizes via walk with
  progress callbacks; inaccessible paths contribute "unknown" and are skipped, never crash.

### 3.3 Error & edge policy
- Permission errors are per-entry, recorded, surfaced in the UI as a muted badge; never abort
  a scan.
- Symlinks/junctions are listed but never followed implicitly (matches Trash-reverify rules).

## 4. `clove-search` — filter/search

Preserves the original design decision (found in Phase 0): **no index; per-keystroke scored
filtering over the in-memory entry list**.

- Input: `&[Entry]` + query; output: scored, ordered subset (returns indices, not copies).
- Scoring: fuzzy-subsequence base score + bonuses (word-boundary match, prefix match,
  path-segment match), stable tie-break on original order. Same feel as the Swift version.
- **Cache case-folded (lowercased) names per directory load.** The prototype benchmarked the
  difference: 14.2–14.7 ms per keystroke over 100k entries when folding inside the filter vs
  1.8–3.2 ms with names pre-folded — a 5–8× win that keeps a keystroke at ~2 ms (see `04`).
- Runs on a worker thread with **cancel-on-new-keystroke** (generation counter); results
  swap in atomically. For 100k entries a full filter pass is microseconds-scale in Rust, so
  an index is deliberately *not* built (less memory, no invalidation bugs).
- Unicode: case-folding via ` unicase`-style folding; grapheme-safe highlight ranges.

## 5. `clove-render` — the identity, framework-free

This crate is the core answer to "does Clove still look like Clove on Windows". It contains
**no Rune, no GPUI, no draw calls** — only pure CPU pixel/geometry functions with Rust-idiomatic
APIs, consumed by `clove-ui` (Rune `Pixels` / canvas, or GPUI images in fallback).

### 5.1 Dithered storage bar (port of `StorageBarRenderer.swift`)
Full algorithm spec is in `02-reusability.md` §Dither; the crate's contract:
- `render_bar(spec) -> RgbaBuffer` where `spec` = width, height (88 pt), per-kind fractions,
  per-kind hues (P3 floats from `StorageKind.swift`, converted once to sRGB),
  animation phase `t ∈ [0,1]`, edgeLight params.
- Internal pieces, all pure functions: 4×4 Bayer threshold (`(v+0.5)/16`), value-noise clouds,
  edge light `exp(−(min(cy,h−cy)−2)/5)`, glint hash (`x·374761393 ^ y·668265263` xorshift-multiply,
  glint when hash > 0.994), 4 pt cell pitch, lit cell ≈3 pt α242 toward white, dim cell ≈1.5 pt α70,
  glow = 1px-per-cell mask blurred 3× (5×5 box).
- **Dirty-rectangle API**: `render_bar` recomputes only changed rows/columns so Rune's
  `Pixels` can upload per-dirty-row without touching GPU cost. The "stop after 2 settled
  frames" contract lives in `clove-ui`, not here.
- The prototypes already render this bit-identical logic; `04` shows the GPUI output.

### 5.2 Shared identity tokens (port of `Palette/Typography/Metrics.swift`)
- `tokens.rs`: bg `#1D1E20`, surface `#2D2E30`, ink `#F4F5F6`, inkSecondary `#B2B4B8`,
  hairline `#3C3D3F`, accent `#6BADFF`; spacing 4/8/12/16/24; radii 16/7/6/8 (nested 20→14).
- Typographic scale from `Typography.swift` as weight/size/leading constants (Rune renders
  them via its DirectWrite stack; GPUI fallback maps onto its font API).
- Icons: the small monochrome glyph set is traced to simple vector paths (Rune canvas /
  GPUI path) so it stays crisp at any scale; patterns (hairlines, nested cards) become
  helper functions returning paint instructions, not views.

## 6. `clove-shell` — deletion, trash, file ops

Port of the Trash-receipt system (`02-reusability.md` §Trash):
- **Receipt-first**: every delete creates a receipt (JSON, in app data) listing original paths,
  restore targets, timestamps, before any destructive act.
- **Re-verify at delete**: each path is re-checked (still exists, same identity) immediately
  before its move into the recycle bin; mismatch → abort that item, never silently delete.
- Windows implementation: `IFileOperation`/`SHFileOperation` recycle-bin move (with undo
  metadata), long-path guards, OneDrive placeholder handling (never offline a cloud file
  without explicit user intent).
- Restore = read receipt, verify current state, move back.

## 7. `clove-ui` — the Rune app

- State: small retained model structs per surface (browser, storage, search); background work
  handled via Rune async tasks + channel-fed snapshots from `clove-fs`.
- Key elements used (validated in the Rune prototype): `Pixels` for the bar
  (`pixels_fit(SurfaceFit::Fill)`), `virtual_rows(count, 28.0, key_fn, build_fn)` inside a
  scroll region for the list, `text_input` for the filter, springs + `live_only()` for bar
  animation and settle-stop.
- Accessibility: roles/labels on every interactive element (list rows = rows/selection state,
  bar = progress with description) — reaching UIA on Windows via Rune's vendored AccessKit.
- Motion: analytic springs for selection, presence transitions for panels, and the
  settle-stop wrapper that halts bar re-rendering after 2 stable frames.

## 8. Build & dependency strategy

- **`[patch.crates-io]` is mandatory in the app workspace** (Rune's own `Cargo.toml` documents
  this): `wgpu-hal`, `winit`, `parley_engine`, `accesskit_windows` from the pinned Rune rev
  (shallow clone in `prototypes/_rune-upstream` during prototyping; a vendored copy or exact
  commit pin ships with the real workspace). `tiny-skia-path` is *not* patched (patching it
  breaks usvg — observed).
- Rune pinned to a specific `dev` commit; upgrades are explicit, reviewed events (pre-release
  framework, breaking changes expected).
- Toolchain: stable MSVC; no nightly requirements observed in the prototypes.

## 9. Risk register

| Risk | Mitigation |
|---|---|
| Rune pre-release volatility; no external PRs accepted | Pin commits; `clove-ui` is the only exposed surface; GPUI fallback path documented in `03` |
| Windows quirks (DPI, IME, taskbar) | Rune's vendored winit/wgpu-hal patches target exactly these; verify in Phase 4 |
| Storage-scan cost on huge drives | 3-kind concurrency cap + 30-min cache (inherited) + incremental rescan on watcher events |
| Recycle-bin edge cases (non-empty restore names, cross-volume moves) | Receipt model already covers identity re-verify; add integration tests per edge case |
| Headless CI without GPU | Rune headless PNG/JSON output + `cargo test` for engine crates; UI logic kept thin |

## 10. What is deliberately *not* in scope of this architecture

- Multi-pane/tabs, tags, or any features absent from the Swift app (no scope creep).
- A search index (the original app's no-index decision is preserved and benchmarked in `04`).
- macOS back-support in Rust; Windows is the only target.

## 11. Approval gate

Nothing beyond prototypes gets implemented until the architecture above (and the Rune choice
in `03`) is approved. The roadmap in `06` sequences the crates so `clove-fs`/`clove-render`
land first with headless tests — both are useful even if the UI decision is revisited.
