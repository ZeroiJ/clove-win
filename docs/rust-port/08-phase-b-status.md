# Phase B status — the Rune app on the real engine crates

## What landed

- `clove-ui` (`bin/clove`) now consumes `clove-fs`, `clove-search`, `clove-render`, and
  `clove-shell` instead of inlined copies. It still builds only with the pinned Rune `dev`
  checkout in `prototypes/_rune-upstream`.
- Real storage scan: `clove-fs::scan` (new module) walks the disk by `StorageKind` on three
  worker threads and reports per-kind totals, matching the macOS scanner's 3-folder grouping
  and 0.09 s progress cadence. The bar keeps the prototype's placeholder fractions until the
  first scan finishes, then rebuilds once with real `bytes / capacity` fractions.
- `BarRenderer::frame_seq()` (Phase A) keeps the UI from re-uploading pixels when no cell
  repainted; `poll` returns `true` only when something visible changed.
- `BarSpec::placeholder` centralises the pre-scan fractions shared by the app and the CLI.
- `clove` CLI gained a `scan` subcommand that prints per-kind bytes, files, timing, and the
  volume total — the headless surface for measuring the scan.
- Headless smoke test (`crates/clove-ui/scripts/smoke.ps1`) drives the real app with Rune's
  `--script` and proves that typing reaches the filter and that Delete reaches the trash
  handler. `CLOVE_START_DIR` lets the script target a sandbox.

## Measured (release, 2026-11-21)

- Launch to visible: **696 / 281 / 283 ms** (cold / warm / warm), against the prototype's
  775 / 597 / 565 ms.
- Idle CPU over 3 s: **0 ms** — the bar animates for 1.5 s after a navigation, then the
  window draws nothing.
- Working set: **105.2 MB**.
- Release build: 5 m 03 s (thin LTO, `codegen-units = 1`).
- Engine: `C:\Windows\System32` lists in 6.0 ms (4,738 entries); filtering 4,738 entries
  takes 0.27–0.29 ms per keystroke; 100k entries filter in 10.8–11.3 ms; the 936 pt bar
  renders its first frame in 17.6 ms, animated frames in 2.6 ms, settled frames in 0.000 ms.

## Findings

- Rune's `Color` is linear light. Clove's tokens are sRGB bytes, so `clove-ui` decodes
  sRGB→linear on the way in. Feeding sRGB straight through rendered the near-black
  `#1D1E20` as mid grey — caught via the headless snapshot in `img/clove-ui-browser.png`.
- The macOS `StorageKind.developer` is spelled `Code` in the token set and the Windows roots.
- The Recycle Bin's `$I` records store a UTF-16 char count that **includes** the NUL
  terminator; `clove-shell` accounts for it. Real Recycle Bin round-trip tests pass.
- `GetDiskFreeSpaceExW` (quota-aware free) gives the volume total the bar divides by;
  `System` is derived as `used - counted`, so `C:\Windows` is never enumerated.

## Open in Phase B

- The scan does not yet count up live in the bar: the placeholder stays until the scan
  finishes, then the bar swaps once. Per-kind live growth needs a renderer that accepts
  changing segment sizes, which is Phase C work.
- The bar is rebuilt on window resize via `on_layout`, but device scale is fixed at 2.0.
- No 30-minute cache or persisted snapshot yet (in-memory only, once per launch).

## Tests

- `clove-fs`: 15 (8 listing/watch + 7 scan)
- `clove-search`: 6
- `clove-render`: 6
- `clove-shell`: 11 (9 unit + 2 Recycle Bin integration)
- `clove-cli`: builds; `scan` runs end to end
- `clove-ui`: builds; headless smoke test passes (filter + trash)

## Next

Phase C (identity and polish): needs Mac captures of the real app. Fix the bilinear glow
upscale, the overlay layer (filaments, seams, motes), glow spill pad, 1 px border, and a
typography/spacing audit. Live per-kind growth lands with the renderer changes there.
