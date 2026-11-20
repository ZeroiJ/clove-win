# 7 · Phase A status — engine crates, headless

Status: **complete, with one exit criterion deferred to Phase C** (see "Not done" below).
Everything in this phase is framework-independent: no crate here names a UI toolkit.

## What exists

| Crate | What it does | Tests |
|---|---|---|
| `clove-fs` | `Entry` (name + name folded once), `Snapshot`, `list_dir` (dirs first, then case-insensitive name), `extended_path`, `DirWatcher` (`ReadDirectoryChangesW`) | 8 |
| `clove-search` | `score_name` (subsequence + consecutive/boundary/prefix bonuses), `filter`, `FilterWorker` (generation-cancelling, stale results dropped) | 6 |
| `clove-render` | `BarRenderer` (Bayer dither, edge light, drifting clouds, glints, per-kind fills, premultiplied 3× box-blurred glow), `tokens` (palette/spacing/radii/type/kind hues) | 6 |
| `clove-shell` | `is_direct_child`, `is_inside`, `trash`, `remove_permanently`, `put_back`, `Move` receipts, Recycle Bin `$I`/`$R` handling | 9 + 2 integration |
| `clove-cli` | `list`, `find`, `bar --out`, `bench` — the CI surface, no window | — |

`cargo test` → **31 passing**, including two that recycle a real temp file, resolve its
receipt to the `$R…` payload, and put it back.

## Measured on this machine (release, `cargo run -p clove-cli --release -- bench`)

Listing, best of 5:

| Work | Time |
|---|---|
| `C:\Windows\System32` — 4,738 entries | **6.0 ms** |
| `C:\Users\Sujal` — 48 entries | 0.17 ms |

Filtering, per keystroke:

| Set | Query | Time |
|---|---|---|
| 4,738 entries | `a` (2,220 matches) | **0.29 ms** |
| 4,738 entries | `storage` (38 matches) | 0.29 ms |
| 100,000 synthetic | `a` (100,000 matches) | 10.8 ms |
| 100,000 synthetic | `file-000` (8,146 matches) | 11.3 ms |

The 100k numbers are dominated by sorting 100k matches, not by matching; a real directory
is the 4.7k row. Two fixes made this 3–4× faster than the first cut: the query is split
into chars once per filter (not once per entry), and ties keep listing order so the
comparator never compares strings.

Storage bar, 9 kinds:

| Width | First frame | Animated frame | Settled frame |
|---|---|---|---|
| 512 pt | 8.5 ms | **1.44 ms** | 0.00 ms |
| 936 pt | 17.6 ms | **2.62 ms** | 0.00 ms |
| 1920 pt | 34.2 ms | **5.25 ms** | 0.00 ms |

First frame is the one-time buffer + glow build. Animated frames recomposite only cells
whose paint or glow changed. A settled frame is a no-op by construction (`render` returns
the existing buffer for a repeated timestamp), which is the contract the UI needs to stop
scheduling frames. Compare the prototype's 12.5 ms full 936 pt re-render.

## Fidelity

`clove-render` follows `StorageBarRenderer.swift` rather than the prototype's
approximations. Three things the prototype got wrong, fixed here:

1. **The hash adds, it does not XOR.** `x &* 374761393 &+ y &* 668265263`. XOR produced a
   noise field biased high, which lit most of the bar instead of just the tube edges.
2. **Glints are seeded per cell.** Swift uses `hash(n, Int32(glints + flickerSeed[c]))`
   with `flickerSeed = hash(i, j) * 7`, so glints twinkle independently.
3. **Lit cells lerp toward the `hot` table by heat/focus/front/glint**, not by `level`. At
   rest that is the kind hue; only glints whiten.

Rendered output: `img/clove-render-bar-936.png` (936 × 88 pt @ 2×).

Deliberately not yet: the overlay layer (filaments, seams, motes), glow spill outside the
bar, the 1 px border, scan heat/focus/spill, and bilinear upscaling of the glow (it is
currently flat per cell). Those are Phase C.

## Trash safety

Ported semantics from `StorageSafety.swift`:

- every delete re-verifies the target is a **direct child** of its root, at the moment of
  the action; `..` components are rejected before touching disk; a path that cannot be
  canonicalised is never deleted
- links are removed as links, never followed
- `emptying_folder` keeps the folder and recycles its children
- `put_back` refuses when something new occupies the original, and recreates missing parents
- `Move::trashed` is exact: the `$R…` payload is located by parsing the matching `$I…`
  record (version 1 and 2). **Version 2's char count includes the NUL terminator** — the
  first implementation kept the `\0` and every receipt came back unresolvable.

## Not done (and why)

- **Pixel-diff against Swift reference screenshots** (the Phase A exit criterion). That
  needs captures from a Mac running the real app; this machine can only compare against
  the prototype, which is a deliberate approximation. Deferred to Phase C, where it
  belongs, and listed there as a prerequisite rather than a checkbox.
- `SHFileOperationW` is limited to 260-character paths. Long-path trash is a Phase D item;
  today such a path is counted as a failure rather than silently dropped.
- `clove-shell` is not yet wired into `clove-cli` (no `clove trash` command). It is
  exercised by its own tests, and Phase B calls it from the UI.
