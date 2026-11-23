# 10 · File-manager parity plan — make Clove a real file manager

Status: **planning**. This is the next track after the Arch port. It turns Clove from a
storage-bar browser into a full file manager comparable to Nautilus/Dolphin/Explorer.

## Goal

Keep Clove's identity (dithered bar, dark theme, fast filter) while adding the operations
people expect from a daily file manager. Do not lose the no-index, fast-listing design.

## What already exists

- Browse, filter, sort, grid/list/details, tabs, multi-select, rename, properties, preview,
  context menu (partial), trash/undo, storage scan + bar, bookmarks-free quick access.

## Phase FM1 — Operation safety and feedback (1–2 weeks)

- [ ] Conflict dialog: replace / skip / rename / apply-to-all on copy/move.
- [ ] Progress overlay: bytes, files, speed, cancel, per-operation.
- [ ] Multi-level undo stack (not only last trash).
- [ ] Full context menu: Open, Open With, Copy, Cut, Paste, Delete, Rename, New File/Folder,
      Properties, Compress, Extract, Terminal Here, Copy Path, Reveal.
- [ ] New empty file creation.
- [ ] Batch rename with preview.
- [ ] `Open With` via xdg-open/mimeapps.

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

## Open decisions

- Mirror which file manager most closely: Nautilus, Dolphin, or Explorer?
- Split view: side-by-side only, or optional top/bottom?
- Git badges: subtle icon only, or full status column?
- Archive browsing: read-only first, or extract-on-navigate?
- Network support in v1, or defer to v2?