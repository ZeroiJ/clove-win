# 2 · Reusability report — what the Rust/Windows port can reuse vs rewrite

Source of truth: the four Phase 0 analyses. Paths are relative to the repo root unless absolute.

## 2.1 The visual identity — fully reusable as *specifications*

The design system is **centrally defined in code**, which makes it portable as data:

| Asset | Location | Reuse verdict |
|---|---|---|
| **Palette** | `Clove\Design\Palette.swift` | Directly portable: one file, OKLCH tokens (one hue 265° for all neutrals), documented WCAG targets, light+dark. Convert to a `Theme` struct in Rust; keep OKLCH as the authoring format. |
| **Storage bar hues** | `Clove\Storage\StorageKind.swift` (P3 dark set + sRGB "paper" light set) | Portable constants (9 kinds: apps/developer/photos/movies/documents/music/system/caches/trash; clearables last). |
| **Vivid hues** | `Palette.Vivid` (8 hues, light/dark hex) | Portable constants (icons, marks, links only). |
| **Dithering algorithm** | `Clove\Features\Storage\StorageBarRenderer.swift` | **Portable algorithm** (see §2.2) — CPU, deterministic, no framework dependency. |
| **Typography** | `Clove\Design\Typography.swift` | System fonts only (SF Pro / SF Mono, monospacedDigit). Windows equivalent: Segoe UI Variable / Cascadia Mono; role system (display/prose/monoLabel/codeLabel) and sizes port 1:1. |
| **Metrics** | `Clove\Design\Metrics.swift` | Portable constants: spacing 4/8/12/16/24, radii (panel 16, row 7, tile 6, control 8; nested 20→14), row paddings 8/5, max reading width 860. |
| **Motion** | `Clove\Design\Motion.swift` + observed springs | Portable: easeOutCubic 0.22 for selection, hover 0.12–0.15s, springs (0.42/0.78 for open, 0.32/0.1 for settle), panel in 0.16s/out 0.11s, press scale 0.96. |
| **Icons** | SF Symbols everywhere; only 12 custom raster assets (app icon, menu-bar glyph) | Windows needs an icon set (Fluent/Segoe MDL2 or embedded SVGs — Rune rasterises SVG icons natively; GPUI supports `Svg` elements). The *monochrome-hierarchical* usage style ports as a convention. |
| **Card/list/motion recipes** | `CalmCard.swift`, `ListPage.swift`, `StorageCard.swift` | Port as layout conventions: "flat fills, a hairline ring, a breath of shadow"; hover/selection are fills, never size changes. |

No baked texture assets exist — the only dithered surface (the storage bar) is generated at
runtime on the CPU. **The entire dithered identity therefore survives as pure math.**

## 2.2 The dithering algorithm (the crown jewel — reusable with a straight port)

`StorageBarRenderer.swift` is framework-independent CPU code (CoreGraphics only for bitmap
allocation; Accelerate only for the glow blur). Faithful port spec:

- **Grid:** 4 pt pitch, bar height 88 pt, corner radius 10; cells inside the rounded rect only
  (per-corner test `dx²+dy² ≤ (r−1)²`); a column belongs to whatever kind covers its centre.
- **Threshold:** `bayer[ (j&3)*4 + (i&3) ]` over the 4×4 Bayer matrix normalised `(v+0.5)/16`.
- **Light level:** `level = -0.04 + 0.66·edgeLight + 0.44·smooth(clamp01((cloud−0.42)/0.38))
  + 0.6·heat + 0.22·focus + 0.55·front`, with
  `edgeLight = exp(−(min(cy, h−cy) − 2)/5)`; `cloud` = 2 octaves of value noise
  (integer hash `x·374761393 ^ y·668265263`, xorshift-multiply; bilinear + smoothstep) drifting
  at `t·0.32`; `heat` decays `exp(−dt/0.42)` when a cell's kind changes; `front = scan·exp(−(end−cx)/22)`.
- **Glints:** cells with `hash > 0.994` flash (re-seeded ~1.6 Hz + per-cell flicker offsets).
- **Cells:** lit ⇒ ~3 pt square, α 242, lerped toward hot (white in dark; colour×0.45 in light);
  dim ⇒ ~1.5 pt square, α 70·keep; empty ⇒ neutral dot α 18 (dark) / 22 (light);
  `keep = 1 − 0.68·dim·(1−focus)` dims unattended kinds.
- **Glow:** 1 px per lit cell → three 5×5 box convolutions (≈ 10 pt Gaussian) → composited low-alpha.
- **Overlay:** triple-stroke outline (6/3.5/1.5 pt, α 0.12/0.28/0.95), 1 pt seams between kinds,
  mote particles on clears (gravity/drag/fade `pow(1−u,1.4)`, cap 1600).
- **Perf contract:** render only while scanning/hovering/clearing; stop after 2 settled frames.
  A Rust port renders the same maths into an RGBA buffer; the GPU only uploads it. (Both
  prototypes implement the settled state of exactly this algorithm — see `prototypes/`.)

## 2.3 Portable logic (near drop-in for Rust) — ~45 files

| Area | Files | Notes |
|---|---|---|
| Search | `Search\SkillSearch.swift`, `LibraryFilter.swift` | Pure scoring functions; port verbatim into `clove-search`. |
| Skill scanning/parsing | `Scanning\SkillScanner.swift`, `SkillFrontmatterParser.swift` | Actor fan-out → Rust tasks; `homeDirectory` parameterised. |
| Agent config parsing | `AgentConfig\` (15 files) | TOML/JSON/markdown parsers, secret redaction — pure logic. |
| Marketplace parsing | `Marketplace\` parsers (`SkillPageParser`, `LeaderboardParser`, `SkillAudit`) | URLSession → `reqwest`/`ureq`; HTML parsing logic portable. |
| Tags | `Tags\TagPersistence.swift` | JSON file. |
| Models | `Models\` | Data model. |
| Markdown | `Components\Markdown\MarkdownBlockParser.swift` (+ `MarkdownDocument`, `MarkdownInline`, `MarkdownBlock`) | Tested parser (tests exist in `CloveTests\`); reusable for the editor + preview. |
| Worktree porcelain | `Worktrees\GitPorcelain.swift`, `WorktreeSafety.swift` | Parse/verdict logic; the `git` invocation plumbing is rewritten. |
| Math islands | `NotchGeometry.swift`, `WorktreeGraph.swift` | Pure geometry. |

## 2.4 macOS-flavoured logic — keep the design, swap the plumbing (~45 files)

| Area | macOS plumbing | Windows replacement |
|---|---|---|
| **Storage scan** (`Storage\`) | `FileManager.enumerator`, allocated-size keys, volume keys, `FileManager.trashItem`, `NSWorkspace` running-app check, `~/.Trash` | `std::fs`/`jwalk`/`ignore` walk on background threads with `IoPriorityHintVeryLow`; sizes-on-disk via `GetCompressedFileSize` (or logical size v1); volume via `GetDiskFreeSpaceExW`; Recycle Bin via `IFileOperation` (`FOF_ALLOWUNDO`); running apps via `EnumProcesses`/window enumeration; kinds remapped: `Program Files`, `%LOCALAPPDATA%`, `%USERPROFILE%\.cache`-style dev dirs, `$Recycle.Bin` |
| **Trash/undo** (`StorageSafety`, `TrashReceipt` pattern) | `trashItem`, put-back | `IFileOperation` Recycle Bin; same re-verify-at-delete-time, receipt, put-back design — **design carries over unchanged** |
| **Skill install** (`Management\`) | symlinks, `~/.agents` layout, `tar`, `trashItem`, lockfile | junctions/symlinks (privilege nuance), same `~/.agents` layout works on Windows, `tar` via `tar` crate |
| **Secrets** (`Secrets\`) | Keychain + Touch ID + zsh profiles | Windows Credential Manager (or DPAPI) + PowerShell profile scan; Touch ID → Windows Hello |
| **Licensing** (`Licensing\`) | Keychain + `Host.localizedName` | Credential Manager + `GetComputerNameEx`; the Dodo REST client is pure logic and ports as-is |
| **Running** (`Running\`) | `ps`, `lsof`, macOS Docker paths | Toolhelp32 snapshots / `GetNamedPipeInfo` / `netstat`-class APIs; Docker Desktop named pipe + WSL; same classification logic |
| **Cleanup transcripts** | transcript paths under `~/Library` | Windows transcript paths ( `%LOCALAPPDATA%` per agent); mtime-cached actor design carries |
| **Updater** (`Release\UpdateService`) | Sparkle | Velopack / Squirrel / WinGet; same signed-manifest concept |
| **Hot keys** (`HotKey\`) | Carbon `RegisterEventHotKey` | `RegisterHotKey` (Chord/keybind model ports) |
| **Login items** (`Support\LaunchAtLogin`) | `SMAppService` | Registry Run key / Task Scheduler |
| **File watching** (`Notch\Agents\FileWatcher`) | FSEvents | `notify` crate (`ReadDirectoryChangesW`) |
| **Open-file panel** (`Support\FolderPicker`) | `NSOpenPanel` (sandbox grant) | `IFileOpenDialog` (no sandbox on Windows — simpler) |

## 2.5 Deeply macOS/UI-bound — rewrite (~230 files ≈ 71%)

`Features\` (138 files), `Components\` views, `Design\` views, `Editor\` views, `App\` shell,
`MenuBar\`, `Notch\` core, `HotKey\`, `Support\` shell bits, `Release\`.

**Notable hard cases with no Windows analogue:**

1. **The notch island** (`Notch\`) — a panel living inside the display notch, with global event
   monitors and a DistributedNotificationCenter handshake with "Shotput". Windows port: a
   top-screen overlay window (Rune's `OverlaySpec` edge-anchored overlays are a near-exact
   fit) with no notch claim; or drop the concept in favour of the tray + panel.
2. **Cross-app text insertion** (`Support\SkillInserter.swift` — AX + synthetic ⌘V): Windows
   equivalent needs UIA + `SendInput`; degradable feature.
3. **Menu-bar access mode** → `Shell_NotifyIcon` tray + popup window (Rune: tray status-item
   support must be checked; GPUI has menus but tray icons are not first-class yet).
4. **Activation-policy juggling** (Dock icon visibility) → minor: window ownership + tray.

## 2.6 Verdict

- **Reusable as-is (logic/spec): ~30 %.** Search, parsers, scanner design, Trash-receipt safety
  model, palette/typography/metrics constants, and the dithering algorithm (the identity).
- **Reusable as design, rewrite plumbing: ~25 %.** Storage scan, install engine, secrets,
  licensing, running, cleanup, updater, hot keys.
- **Rewrite: ~45 % of files (≈ 71 % of the file count is UI-bound)** — the SwiftUI surfaces,
  the notch, the menu-bar shell. These are precisely what the Phase 1 framework comparison
  addresses.
