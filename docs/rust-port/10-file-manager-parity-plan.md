# 10 · File-manager parity plan — make Clove a real file manager

Status: **FM1 active**. This track turns Clove from a storage-bar browser into a full file manager comparable to Nautilus (Omarchy default), while keeping Clove identity.

## Goal

Keep Clove's identity (dithered bar, dark theme, fast filter) while adding the operations
people expect from a daily file manager. Do not lose the no-index, fast-listing design.

## What already exists

- Browse, filter, sort, grid/list/details, tabs, multi-select, rename, properties, preview,
  context menu (partial), trash/undo, storage scan + bar, bookmarks-free quick access.

## Phase FM1 — Operation safety and feedback (1–2 weeks)  [mostly complete]

- [x] Conflict handling: paste auto-renames by default; ask-mode modal with Replace/Skip/Rename/Cancel.
- [x] Progress overlay: paste reports progress, cancel via Esc/button.
- [x] Multi-level undo stack for trash (not only last trash).
- [x] Context menu: Open, Open With (app chooser), Copy, Cut, Paste, Copy Path, New File, New Folder, Terminal Here, Reveal, Delete (undo-aware), Rename, Batch Rename, Compress, Extract, Properties.
- [x] New empty file creation (context menu + Ctrl+?).
- [x] Batch rename with preview modal.
- [x] `Open With` via xdg-mime + .desktop chooser.

## Phase FM2 — Navigation and layout (1–2 weeks)  [mostly complete]

- [x] Bookmarks + mounts sidebar (quick access + /proc/mounts; custom reorder TODO).
- [ ] Split view / dual pane: intentionally skipped per decision.
- [x] Mounts view: local disks + removable via /proc/mounts.
- [ ] Network locations: SMB/NFS via gio/gvfs (deferred, feature-gated).
- [x] Breadcrumb/address editing + Tab path completion.
- [x] Tabs restore on launch.

## Phase FM3 — Content and metadata (1–2 weeks)  [partial]

- [x] Thumbnail cache: images+svg in-memory with 64MB byte-cap eviction. No video/PDF (needs ffmpeg/poppler deps).
- [x] Archive extract-on-navigate (tar/zip/7z/rar via system tools). Virtual browsing deferred.
- [x] Git badge: branch + dirty icon in header.
- [x] Extended properties: perms, owner (uid), size, SHA256 checksum.
- [~] Folder sizes on demand: properties computes dir size in background. No cancel/progress UI yet.

## Phase FM4 — Power features (1–2 weeks)  [mostly complete]

- [x] Terminal here.
- [x] Compare folders (select 2 dirs, name+size diff).
- [x] Bulk checksum copy/verify report (SHA256, clipboard).
- [x] Symlink/hardlink creation.
- [x] Compress/extract actions with progress status.
- [x] Duplicate finder (size+SHA256, current folder tree).

## Phase FM5 — Quality and platform (ongoing)  [partial]

- [ ] Drag and drop internal/external (Rune/framework support not wired; deferred).
- [x] Keyboard shortcut audit overlay (Ctrl+/ or F1). File-based customization not implemented.
- [~] Accessibility: labels/placeholders on inputs; full AT-SPI pass deferred.
- [~] Performance: virtualized listing already used; thumbnail 64MB cap added; no 100k benchmark run.
- [~] Cross-platform regression tests: helper unit tests added (checksum/compare/dupes); trash/open/mount integration tests still limited.

## Architecture notes

- Keep `clove-fs`, `clove-search`, `clove-render`, `clove-shell` UI-free.
- Add `clove-archive`, `clove-git`, `clove-preview` only if they stay UI-free.
- UI state grows: panes, bookmarks, undo stack, job queue. Put the job queue on a worker
  thread and report progress through the existing `AppHandle::update` channel.
- Reuse the existing storage scan for folder sizes and duplicate detection.
- Do not add a search index; keep per-keystroke filtering.

## Suggested order

1. FM1 first: it fixes the most dangerous gaps (conflicts, progress, undo).
2. FM2 next: dual pane and bookmarks change daily use the most.
3. FM3 after that: thumbnails and git badges are visible wins.
4. FM4/FM5 last: power features and polish.

## Decisions (resolved)

- **Mirror**: Nautilus (Omarchy file manager). Live dithered bar stays always at top.
- **Split view**: no, not now. Revisit post-FM2.
- **Git badges**: icons only, subtle; no full status column in FM1.
- **Archives**: extract on navigate (FM3).
- **Network support**: defer to FM2 optional/feature-gated (gvfs/gio/udisks integration later). Not FM1 blocker; local + mounted volumes first.