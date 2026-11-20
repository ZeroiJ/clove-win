# 1 · Repository architecture report

**Scope:** full analysis of `clove-win` (fork of keshav-exe/clove, release 3.3, commit `04d1ca3`).
Nothing in the original source was modified; all new work lives under `prototypes/` and `docs/rust-port/`.

## 1.1 What Clove is today

Clove is a **native macOS 26 SwiftUI app** (MIT, single codebase, no Rust yet) that manages the
tooling of AI coding agents. It is *not* a file manager today — file listing exists only in
domain-specific form (storage's biggest-folders list, cleanup lists, the skills browser).

- **322 Swift files** under `Clove\` (31 more in `CloveTests\`), **≈ 24,600 lines**.
- 27 top-level modules; one third-party Swift package (Sparkle 2.9.6); no Metal, no Combine,
  no StoreKit, no QuickLook.

## 1.2 Entry point and shell

| Piece | File(s) | Notes |
|---|---|---|
| `@main` app | `Clove\CloveApp.swift` | 3 scenes: main window (`LibraryRootView`), skill-editor window, Settings |
| App delegate | `Clove\App\AppDelegate.swift` | owns notch, menu-bar panel, hot keys, updater, the shared `AppModel` |
| Central model | `Clove\App\AppModel.swift` | `@Observable` god-object: skills catalog, panel/library query state, all feature stores |
| Two access modes | `Clove\App\AccessMode.swift` | `.notch` (island over the MacBook notch) or `.menuBar` (status item + floating panel) |
| Main window | `Clove\Features\Library\LibraryRootView.swift` | `NavigationSplitView` (sidebar + section page), inspector detail |

## 1.3 Feature inventory

| Module | Files | What it does |
|---|---|---|
| `Scanning\` | 4 | Skill discovery: `SkillScanner` (actor, task-group fan-out, symlink cycles, depth 10), `SkillFrontmatterParser`, `ScanConfiguration`, `ScanRoot` |
| `AgentConfig\` | 15 | Read-only scanning/parsing of agent config: instruction files, Cursor/Windsurf rules, commands, subagents, MCP servers, hooks, permissions (TOML/JSON/markdown parsers, secret redaction) |
| `Models\` | 5 | Catalog model (`SkillCatalog`) |
| `Search\` | 2 | **In-memory scored search** — no index: `SkillSearch::ranked()` per keystroke, weighted tiers (exact 100 / prefix 80 / tag 70 / contains 60 / fuzzy-subsequence 8) |
| `Tags\` | 1 | Tag persistence (one JSON file) |
| `Management\` | 10 | Install/update/remove engine: copies into `~/.agents/skills`, symlinks per agent, `.skill-lock.json` shared with the skills CLI, `TrashReceipt` undo records |
| `Marketplace\` | 13 | skills.sh client (Top/Trending/Hot, search, audits, install), 10-min feed cache, debounced remote search |
| `Cleanup\` | 8 | Unused-skill detection by parsing agent transcripts (actor, mtime-cached), disk leftovers, Trash-with-undo |
| `Storage\` | 4 | **Disk measurement + the dithered bar** (see §1.4) |
| `Secrets\` | 4 | Keychain vault + plaintext-key finder for shell profiles |
| `Running\` | 5 | Live agent processes/dev servers via `ps`/`lsof` every 3 s, Docker (Docker Desktop/OrbStack/Colima/Rancher) |
| `Worktrees\` | 6 | Git worktree browser + safe cleanup (`git worktree list --porcelain`, verdicts, Trash receipt) |
| `Editor\` | 14 | SKILL.md editor with live preview + format validation |
| `Features\` | 138 | All pages/panels: Home, Library, Discover, Cleanup, Storage, Resources, Running, Secrets, Worktrees, Settings, Panel, Onboarding, WhatsNew |
| `Components\` | 36 | Reusable views (markdown renderer, tiles, chips, mock art) |
| `Design\` | 7 | Palette, Typography, Metrics, Motion, card/list systems |
| `Notch\` | 21 | The island over the display notch (deepest macOS coupling; FSEvents file watcher, global event monitors, DistributedNotificationCenter handshake with "Shotput") |
| `MenuBar\` | 5 | `NSStatusItem` + non-activating `NSPanel` |
| `App\` | 11 | App model, settings store, window routing |
| `HotKey\` | 4 | Carbon `RegisterEventHotKey` |
| `Support\` | 7 | Launch-at-login (SMAppService), folder picker, **cross-app text insertion via AX + synthetic ⌘V**, app version |
| `Licensing\` | 4 | Dodo Payments REST (activate/validate/deactivate; key + instance id in Keychain; 7-day revalidation, 45-day offline grace). No StoreKit |
| `Release\` | 1 | Sparkle updater |

## 1.4 The storage visualization (the feature the port preserves first)

Files: `Clove\Storage\StorageKind.swift`, `StorageScanner.swift`, `StorageStore.swift`,
`Clove\Features\Storage\StorageCard.swift`, `StorageBarView.swift`, **`StorageBarRenderer.swift`**
(the dithering engine, 569 lines, CPU + CoreGraphics + Accelerate), `StoragePage.swift`.

Full details in `02-reusability.md` §2. In one paragraph: the bar is **a 4 pt-pitch cell grid,
dithered on the CPU against a 4×4 Bayer matrix**; each cell's light level comes from drifting
value-noise clouds + an edge-light term (bright along top/bottom like a tube) + cooling "heat"
spikes when a cell's kind changes + a hot scan front + hover focus + random glints; cells above
their Bayer threshold draw a lit ~3 pt square (α 242) toward white-hot, below it a dim ~1.5 pt
square (α 70); a 1-px-per-cell glow image is blurred (3× 5×5 box via vImage) and composited;
triple-stroke outline filaments (6/3.5/1.5 pt at α 0.12/0.28/0.95), 1 pt seams, and mote
particles ride on top. Composited on CALayers, `CADisplayLink`-driven, and — key design point —
**the animation stops entirely after 2 settled frames**; the CPU render only runs while
hovering/scanning/clearing.

Scanning engine: `FileManager.enumerator` depth-first walk with allocated-size resource keys,
3 kinds concurrently in a `TaskGroup` at **`.utility` priority**, progress flushed at ≤15 Hz,
cancellation checked every 1024 files, results cached in `storage-scan.json` (0600, backup-excluded)
**fresh for 30 minutes**, UI keeps counts live while the walk runs.

## 1.5 Concurrency and background-work patterns (the behavioural contract to port)

- Stores are `@MainActor @Observable`; heavy work lives in actors or `Task.detached` with explicit priorities (`.utility` for walks, `.userInitiated` for deletes).
- A hidden panel "never scans, polls, or animates" — visibility gates all polling.
- Destructive operations are re-verified at the moment of deletion (path is a direct child of the scanned root, `..` rejected, symlinks never followed), go to Trash first, and return a receipt for put-back undo.
- Everything asks first; whole agent folders like `~/.claude` are never offered.
- Search re-ranks the catalog per keystroke (no index) — cheap at catalog scale, O(entries × tokens).

## 1.6 Third-party / OS dependency summary

One Swift package (Sparkle). Everything else is OS: SwiftUI (198 files), AppKit (40),
Carbon hot keys, Keychain (Security), FSEvents, Accelerate, QuartzCore, ServiceManagement,
LocalAuthentication, CryptoKit (one git-tree hash), Darwin sysctl. Full inventory in
`03-framework-comparison.md` §4 and the coupling classification in `02-reusability.md` §4.
