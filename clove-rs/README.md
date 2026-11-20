# Clove for Windows — Rust workspace

Phase A of the port (see `../docs/rust-port/06-roadmap.md`). The filesystem engine and the
identity renderer are framework-independent: nothing here imports a UI framework.

| Crate | Role |
|---|---|
| `clove-fs` | Entry model, directory listing, sort, folded names, long paths, `ReadDirectoryChangesW` watching |
| `clove-search` | Scored per-keystroke filter + cancellable worker (no index, as the Swift app) |
| `clove-render` | Clove's dithered storage bar as pure CPU rendering + design tokens |
| `clove-shell` | Delete/trash safety: direct-child guards, receipts, Recycle Bin moves, put-back |
| `clove-cli` | Headless test surface: `list`, `find`, `bar --out x.png`, `bench` |

## Commands

```sh
cargo test                     # engine tests
cargo run -p clove-cli -- list .
cargo run -p clove-cli -- find . storage
cargo run -p clove-cli -- bar 936 --out bar.png
cargo run -p clove-cli --release -- bench
```

`clove-ui` (Rune) joins the workspace in Phase B; it is the only crate that may depend on a
UI framework.

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
