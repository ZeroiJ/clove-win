# 10 · File-manager parity plan — make Clove a real file manager

Status: **FM1 active**. This track turns Clove from a storage-bar browser into a full file manager comparable to Nautilus (Omarchy default), while keeping Clove identity.

## Goal

Keep Clove's identity (dithered bar, dark theme, fast filter) while adding the operations
people expect from a daily file manager. Do not lose the no-index, fast-listing design.

## What already exists

- Browse, filter, sort, grid/list/details, tabs, multi-select, rename, properties, preview,
  context menu (partial), trash/undo, storage scan + bar, bookmarks-free quick access.

## Phase FM1 — Operation safety and feedback (1–2 weeks)

- [~] Conflict handling: paste auto-renames on collision; replace/skip/rename/apply-all dialog still TODO.
- [~] Progress overlay: `op_progress` state/status skeleton added; full job queue + cancel UI still TODO.
- [x] Multi-level undo stack for trash (not only last trash).
- [x] Context menu: Open, Open With (app chooser), Copy, Cut, Paste, Copy Path, New File, New Folder, Terminal Here, Reveal, Delete (undo-aware), Rename, Batch Rename, Compress, Extract, Properties.
- [x] New empty file creation (context menu + Ctrl+?).
- [x] Batch rename with preview modal.
- [x] `Open With` via xdg-mime + .desktop chooser.

## Phase FM2 — Navigation and layout (1–2 weeks)

- [ ] Persistent bookmarks sidebar (user folders + custom paths, drag to reorder).
- [ ] Split view / dual pane with independent navigation and cross-pane copy/move.
- [ ] Mounts view: local disks, removable media, network mounts from `/proc/mounts`/udisks2.
- [ ] Network locations: SMB/NFS via gio/gvfs (optional, feature-gated).
- [ ] Breadcrumb editing and path completion.
- [ ] Tabs restore on launch.

## Phase FM3 — Content and metadata (1–2 weeks)

- [ ] Thumbnail cache: images, video frames, PDF first page, with eviction by bytes.
- [ ] Archive browsing as virtual folders (zip/tar/7z) via libarchive.
- [ ] Git badges: repo root detection, branch, dirty/clean, ahead/behind.
- [ ] Extended properties: permissions, owner, MIME type, checksums.
- [ ] Folder sizes on demand (background, cancellable).

## Phase FM4 — Power features (1–2 weeks)

- [ ] Terminal here (launch default terminal in the current folder).
- [ ] Compare folders.
- [ ] Bulk checksum verify.
- [ ] Symlink/hardlink creation.
- [ ] Compress/extract with progress.
- [ ] Duplicate finder (optional, reuses scan engine).

## Phase FM5 — Quality and platform (ongoing)

- [ ] Drag and drop internal/external.
- [ ] Keyboard shortcut audit and customization.
- [ ] Accessibility (AT-SPI) pass.
- [ ] Performance: 100k-entry folders, thumbnail memory bounds.
- [ ] Cross-platform regression tests for trash/open/mounts.

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