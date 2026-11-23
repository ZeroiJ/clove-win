# 9 · Linux port plan — Clove on Arch / Omarchy

Status: **planning — awaiting decisions**. Do not treat unchecked items as approved scope.
Last updated: 2026-11-23.

## Goal

Run the Rust Clove file manager natively on Arch Linux (Omarchy, Hyprland/Wayland), with the
dithered storage bar intact and live during scans. Keep the Windows build working from the same
source tree.

This is a port of the Rust file manager, not a re-port of the 322-file Swift app. SwiftUI and
AppKit do not exist on Linux. The Swift tree supplies the bar algorithm, palette, metrics, and
trash-safety rules as reference; no Swift code is compiled.

The original macOS app is an AI-agent tooling manager with a storage page. The Windows rewrite
became a general file manager. Linux v1 targets the file manager. Mac agent features are a
separate decision (see Open decisions).

## Verified baseline (this laptop, 2026-11-23)

- Omarchy, Wayland session, Hyprland. Rust 1.100.0. AMD Cezanne iGPU + NVIDIA RTX 3050 Mobile.
- Rune `dev` checkout present at `prototypes/_rune-upstream` (git-ignored, 188 MB).
- `clove-search`: 6/6 tests pass.
- `clove-render`: 6/6 tests pass.
- `clove-shell`: 9/9 unit tests pass; 2/2 Recycle Bin integration tests fail on Linux.
- `clove-fs`: 11/15 tests pass. Fails: 3 watcher tests, 1 volume test.
- `clove-fs` non-Windows watcher branch has a compile error (moved `Arc<AtomicBool>`).
- `clove-ui` does not compile on Linux. Root cause found: missing `sctk-adwaita` patch in
  `clove-rs/Cargo.toml`. Rune's own manifest lists it as required for apps that build Rune.

## Scope

### In for v1 (pending approval)

- File browsing: list, filter, sort, grid, tabs, multi-select, rename, properties, preview,
  context menu, hidden files, live directory reload.
- Navigation: This PC / mounts, home and XDG folders, back/forward/up, address bar, breadcrumbs.
- File operations: copy/cut/paste, new folder, trash, permanent delete, undo, open with the
  desktop default application.
- Storage surface: scan by kind, 30-minute cache, legend, Ctrl+R rescan.
- Dithered bar: full animation, live growth during scan, HiDPI, settle-stop at idle.
- Arch packaging: local PKGBUILD, desktop entry, icon, pinned Rune revision.
- Linux tests: engine integration tests, trash round trip, watcher, volume, headless bar diff,
  UI smoke.

### Out for v1 (pending approval)

- macOS-only surfaces: notch, menu bar, Sparkle updater, Keychain, Touch ID.
- Mac agent-tooling modules: skills catalog, marketplace, worktrees, running agents, cleanup,
  secrets, editor. These are a separate product track.
- Global hotkeys (Hyprland configuration), tray icon, file associations, auto-update.
- Windows remaining backlog (conflict dialog, progress overlay, multi-level undo, dual pane,
  archives, terminal here, git badges) unless promoted into Linux v1 by decision.
- X11-first polish. Wayland/Hyprland is the target; X11 should not be broken but is not the
  acceptance environment.
- AUR publication. Local `makepkg` first. AUR is a later step.

## Porting map

| mac/windows thing | Arch/Linux replacement | Status |
|---|---|---|
| SwiftUI / AppKit | Rune UI (already written) | reuse |
| CoreGraphics + Accelerate blur | `clove-render` CPU RGBA + box blur | done |
| `FileManager.enumerator` | `std::fs` walk | reuse |
| Allocated size | `MetadataExt::blocks() * 512` | todo |
| Volume capacity/free | `statvfs` (`rustix`) | todo |
| `ReadDirectoryChangesW` | inotify via `notify` | todo |
| Recycle Bin / `SHFileOperationW` | freedesktop Trash (`files/` + `info/*.trashinfo`) | todo |
| `SHFileOperation` / shell open | `xdg-open` on a worker thread | todo |
| `%APPDATA%` | `~/.config/clove` | todo |
| `%LOCALAPPDATA%` cache | `~/.cache/clove` | todo |
| `C:\`, `SystemDrive`, `USERPROFILE` | `$HOME`, XDG dirs, mount table, `/` | todo |
| Drive letters | mounted filesystems from `/proc/self/mountinfo` | todo |
| “Recycle Bin” | “Trash” | todo |
| Hidden attribute | dotfile names | todo |
| SF Pro / SF Mono | system sans + JetBrains Mono / fallback | todo |
| SF Symbols | bundled SVG icon set | later |
| UIA | AccessKit over AT-SPI | verify |
| Carbon hotkeys | app-local keys; Hyprland binds for global | out |
| SMAppService | `~/.config/autostart/clove.desktop` | later |
| Sparkle | pacman / AUR | out |

## Reuse ledger

- `clove-render`: ~100%, including the crown-jewel bar. No rewrite.
- `clove-search`: 100%.
- `clove-ui`: ~80%. State, views, keys, tabs, sort, grid survive; platform seams change.
- `clove-fs`: ~75%. Listing and scan loop survive; watcher, volume, roots, hidden flags change.
- `clove-shell`: ~55%. Guard, permanent delete, undo survive; trash backend changes.
- Swift: 0% compiled. Reference only.

## Phases

### L0 — Rune on Linux spike (gate)

- [x] Clone Rune `dev` to `prototypes/_rune-upstream`.
- [x] Identify missing `sctk-adwaita` patch as the current Linux build blocker.
- [ ] Add `sctk-adwaita = { path = "../prototypes/_rune-upstream/vendor/sctk-adwaita" }` to
      `clove-rs/Cargo.toml` `[patch.crates-io]`.
- [ ] `cargo build -p clove-ui` succeeds on Linux.
- [ ] Window opens on Hyprland/Wayland; text, keyboard, scroll, resize work.
- [ ] `Pixels` bar displays correctly at scale 1.0 and 1.25/1.5.
- [ ] Runs on AMD iGPU and NVIDIA dGPU; record which adapter wgpu picks.
- [ ] Headless `--snap` renders a valid PNG.
- [ ] Idle window settles to 0% CPU after bar animation.
- [ ] Gate decision recorded: continue with Rune, or stop and reassess UI framework.

### L1 — Filesystem engine

- [ ] Fix non-Windows watcher stub compile error.
- [ ] Implement `statvfs` volume query (`capacity`, free to user).
- [ ] Choose volume under `$HOME` for the bar; handle separate `/home` filesystem.
- [ ] Implement inotify watcher via `notify`; coalesce events; reload on overflow.
- [ ] Add fallback reload when watch setup fails.
- [ ] Detect dotfiles as hidden on Unix.
- [ ] Use allocated size (`st_blocks * 512`).
- [ ] Deduplicate hardlinks by `(dev, ino)`.
- [ ] Never follow symlinks; skip `/proc`, `/sys`, `/dev`, `/run`; stay on one filesystem.
- [ ] Linux scan roots: `/opt`, Flatpak, Steam, XDG folders, `~/.cache`, `~/.cargo`,
      `~/.rustup`, pacman cache, Trash. `System` remains derived.
- [ ] Linux tests: watcher change/rename/overflow, volume, hidden, hardlink dedupe.

### L2 — Trash, open, paths

- [ ] Implement freedesktop Trash: move to `~/.local/share/Trash/files`.
- [ ] Write `info/<name>.trashinfo` with percent-encoded original path and deletion date.
- [ ] Handle filename collisions.
- [ ] Handle cross-filesystem moves (same-filesystem rename; copy+delete fallback).
- [ ] Return exact trash path as the undo receipt.
- [ ] `put_back`: move file back, remove `.trashinfo`, never overwrite.
- [ ] Count Trash size from `files/` only; skip `info/`.
- [ ] Add `xdg-open` for files on a worker thread.
- [ ] Resolve XDG folders and mounts; build This PC entries.
- [ ] Integration tests: trash → receipt → put back; collision; missing receipt.
- [ ] Update all user-facing “Recycle Bin” text to “Trash”.

### L3 — UI Linux wiring

- [ ] Add `clove-ui` platform module: home, XDG dirs, config/cache dirs, mounts, open.
- [ ] Start in `$HOME`; keep `CLOVE_START_DIR` for headless tests.
- [ ] Settings at `~/.config/clove/settings.json`.
- [ ] Storage cache at `~/.cache/clove/storage-scan.txt`.
- [ ] This PC shows Home, XDG folders, `/`, and real mounted volumes.
- [ ] Backspace at `/` returns to This PC.
- [ ] Hidden toggle filters dotfiles correctly.
- [ ] Open selected file with default app; Enter still opens folders.
- [ ] Remove Windows-only quick-access assumptions from `this_pc_entries`/`this_pc_resolve`.
- [ ] Keep Windows branches behind `cfg(windows)`; Windows build stays green.

### L4 — Live dithered bar

- [ ] Feed scan `Progress` events into the bar (currently only kind `Done` updates it).
- [ ] Add `BarRenderer` segment-update API that preserves heat, glints, and glow state.
- [ ] Smooth growth so the bar fills visibly during a scan instead of jumping.
- [ ] Use real Rune surface scale for `BarSpec::scale`; remove hardcoded 2.0.
- [ ] Rebuild correctly on window resize.
- [ ] Keep animation budget: ~30 fps while scanning/clearing, then stop.
- [ ] Verify idle CPU is 0% after settle.
- [ ] Pixel-diff headless bar output against `docs/rust-port/img/clove-render-bar-936.png`.
- [ ] Verify the scan front, seams, and heat are visible while kinds grow.

### L5 — Arch packaging

- [ ] Add `packaging/arch/PKGBUILD`.
- [ ] Pin Rune revision in a repo file; document the required patches.
- [ ] Package fetches the matching Rune tarball into `prototypes/_rune-upstream`.
- [ ] Build with `cargo build -p clove-ui --release`.
- [ ] Install `/usr/bin/clove`.
- [ ] Install `.desktop` entry with working `StartupWMClass`.
- [ ] Install hicolor icons from the existing macOS icon set.
- [ ] Install licence.
- [ ] `makepkg -si` installs and launches from the app launcher.
- [ ] Document local build and package build in README.

### L6 — Hardening and daily-driver check

- [ ] Hyprland scale matrix: 1.0, 1.25, 1.5; multi-monitor; tiled and floating; fullscreen.
- [ ] Clipboard file-list interop with Thunar, Dolphin, Nautilus.
- [ ] Trash round trip on same and different filesystems.
- [ ] Large directory (10k+ entries) navigation and filtering.
- [ ] Permission-denied directories and files.
- [ ] Btrfs used/free sanity; separate `/home` and `/` filesystems.
- [ ] Removable drive appears/disappears cleanly.
- [ ] Scan cancellation and Ctrl+R.
- [ ] Linux UI smoke script.
- [ ] Engine tests green on Linux.
- [ ] README updated: Linux section, build deps, package install, known limits.
- [ ] Use as daily driver for one full day without a blocker.

## Known code gaps to fix during the port

- [ ] Context menu “Open” is a no-op for files; “Copy” is a no-op.
- [ ] Enter opens folders only; no default-app launch for files.
- [ ] Non-Windows watcher emits `Failed` and does not watch.
- [ ] Non-Windows volume query returns `None`.
- [ ] Hidden/system flags are always false on Unix.
- [ ] Bar consumes kind totals only, not live progress.
- [ ] Bar scale fixed at 2.0.
- [ ] Settings and cache use Windows environment variables only.
- [ ] Start directory falls back to `C:\`.
- [ ] Trash is Windows-only.

## Risks

1. Rune on Linux is less exercised than Windows; the `sctk-adwaita` patch is the known blocker,
   but more compile/runtime issues may follow.
2. Wayland client-side decorations, rounded corners, and fractional scaling under Hyprland.
3. Hybrid GPU adapter selection on the AMD/NVIDIA laptop.
4. Freedesktop Trash edge cases: collisions, cross-filesystem, permissions.
5. Btrfs capacity versus counted bytes; subvolumes and snapshots.
6. Wayland clipboard file lists depend on data-control support and the target file manager.
7. Rune is pre-release; the revision must be pinned, not floating `dev`.
8. Scan cost on Arch: `/usr` is large; kinds must be scoped so the bar fills promptly.

## Open decisions

- [ ] v1 scope: Rust file manager parity only, or also Mac agent-tooling modules?
- [ ] Windows backlog: after Linux parity, or pull some items into Linux v1?
- [ ] Branch model: cross-platform `main` or `linux-port` branch merged when green?
- [ ] Desktop target: Hyprland/Wayland only, or broader X11/other-DE support?
- [ ] Packaging: local PKGBUILD only, or AUR too?
- [ ] Rune fallback policy if the L0 spike fails.
- [ ] Live bar growth: must-have for v1?
- [ ] Keep Windows releases shipping during the port?
