# Clove for Windows — Rust workspace

Phases A and B of the port (see `../docs/rust-port/06-roadmap.md`). Everything except
`clove-ui` is framework-independent: no engine crate imports a UI toolkit.

| Crate | Role |
|---|---|
| `clove-fs` | Entry model, directory listing, sort, folded names, long paths, `ReadDirectoryChangesW` watching |
| `clove-search` | Scored per-keystroke filter + cancellable worker (no index, as the Swift app) |
| `clove-render` | Clove's dithered storage bar as pure CPU rendering + design tokens |
| `clove-shell` | Delete/trash safety: direct-child guards, receipts, Recycle Bin moves, put-back |
| `clove-cli` | Headless test surface: `list`, `find`, `bar --out x.png`, `bench` |
| `clove-ui` | The Rune app (`bin/clove`): browser, filter field, bar, trash + undo |

## Commands

```sh
cargo test                     # engine tests (clove-ui is not a default member)
cargo run -p clove-cli -- list .
cargo run -p clove-cli -- find . storage
cargo run -p clove-cli -- bar 936 --out bar.png
cargo run -p clove-cli --release -- bench

cargo run -p clove-ui --release            # the app
cargo run -p clove-ui -- --snap out.png    # headless frame, no window (debug build)
```

`clove-ui` builds Rune from the shallow clone at `../prototypes/_rune-upstream`, so that
clone must exist (`git clone --depth 1 -b dev https://github.com/anomalyco/rune`). The
`[patch.crates-io]` block in `Cargo.toml` is required by Rune's README; in production Rune
becomes a git dependency pinned to a rev and the vendored crates come with it.

## The app (Phase B)

- Listing, filtering and trashing run off the UI thread; nothing touches the disk inside a
  frame or a key handler.
- The bar is a `Pixels` buffer the renderer owns. `BarRenderer::frame_seq()` changes only
  when a cell actually repaints, so a settled bar costs no upload and no draw. The bar
  animates for 1.5 s after navigation or an action, then holds still.
- Rune's `Color` is **linear light**. Clove's tokens are sRGB bytes, so `clove-ui` decodes
  them on the way in; feeding sRGB straight through renders `#1D1E20` as mid grey.
- Ctrl+Z restores the last trashed selection; `clove_shell::put_back` refuses to overwrite
  anything that has appeared at the original path.

## Fidelity notes

`clove-render` follows `Clove/Features/Storage/StorageBarRenderer.swift`: 4 pt cells, 4×4
Bayer threshold, edge light, drifting value-noise clouds, glints, lit cells (3 pt, α242)
lerped toward the hot table, dim cells (1.5 pt, α70), empty cells (white, α18), per-kind
fill tints (α12), and the glow layer (premultiplied, three 5×5 box passes). The output is
straight-alpha RGBA composited in the Swift layer order (glow → fills → cells), so the UI
places it over the surface color.

Not yet in this slice (Phase C fidelity pass): the overlay layer (filaments, seams, motes),
glow spill outside the bar (6-cell pad), the 1 px border, and scan heat/focus/spill
animation. The renderer's `render(t)` already drifts the clouds and flickers glints.

## Fidelity notes: trash

`clove-shell` ports `Clove/Storage/StorageSafety.swift` semantics: every delete re-verifies
that the target is a direct child of its root, links are removed as links, and each moved
item records where it went so `put_back` is a plain move. On Windows the trash is the real
Recycle Bin (`SHFileOperationW` + `FOF_ALLOWUNDO`), and the `$R…` payload path is recovered
from the matching `$I…` record — that is what makes `Move::trashed` exact.

`SHFileOperationW` is subject to the classic 260-character path limit; long-path trash is a
Phase D item (`06-roadmap.md`), and a path the shell refuses is counted as a failure rather
than silently dropped.
