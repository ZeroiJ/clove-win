# Changelog

All notable changes to Clove are documented in this file.

## [Unreleased]

### Added
- Static website in [`site/`](site/index.html) deployed live at `https://clove-linux.vercel.app`, with interactive Bayer 4×4 storage bar specimen, AUR quick-copy installer, Light/Dark scheme toggle, and upstream attribution.
- Captured real `v1.6.2` desktop preview at [`docs/clove-linux-preview.png`](docs/clove-linux-preview.png).

### Changed
- Rewrote [`README.md`](README.md) for Linux / Arch / Omarchy (`clove-linux` AUR, keyboard shortcuts, source build, upstream attribution) and updated GitHub About metadata + homepage URL.

## [1.6.2] - 2026-11-26

### Changed
- Increased the bottom animated Bayer 4×4 storage bar height (`16px` -> `44px`, corner radius `8px`) so the dithered visualization is large and legible.
- Removed the storage bar Hide button and menu toggle so the storage bar stays permanently accessible.
- Simplified expanded storage details: placed the category breakdown (`<Kind> <Size>` with color dots) directly beneath the storage bar without card wrappers or extra headings, with an inline rescan control on the storage row.
- Added `CHANGELOG.md` to track releases going forward.
- Bumped `packaging/arch/PKGBUILD`, `packaging/aur/PKGBUILD`, and `packaging/aur/.SRCINFO` to `1.6.2`.

### Fixed
- Automated AUR publishing by chaining [`.github/workflows/aur.yml`](.github/workflows/aur.yml) directly after the `release` job via `workflow_call` (avoiding GitHub Actions' `GITHUB_TOKEN` release-event suppression), removing redundant Rune checkout cloning, and syncing `pkgver` / `.SRCINFO` to the pushed release tag.

## [1.6.1] - 2026-11-26

### Added
- Dynamic GTK/Omarchy icon-theme loader (`Yaru-red` / `Yaru` / `Adwaita` / `hicolor`) and `~/.config/gtk-3.0/bookmarks` parser.
- Nautilus-inspired `Recent`, `Starred`, and `Network` sidebar locations with persistent starred paths.
- Monochrome 16×16 symbolic SVG icons (`icons.rs`) matching Adwaita and GNOME Files.
- Dedicated `ui.rs` layout with translucent dark glass panes, centered Nautilus PathBar pill, view/sort/icon-size split menu, main menu popover, right-click context menu, `192×192` media/PDF thumbnails, and inline search/location editing.
- Replaced application icon assets across `AppIcon.appiconset` with the halftone blue butterfly icon and added attribution in `README.md`.

### Changed
- Redesigned headerbar, sidebar, and borderless `96px` GridView to match GNOME Files (Nautilus 51) under Omarchy's dark glass theme, removing forced initial selection highlights.
- Moved Clove's animated Bayer 4×4 storage bar into a compact footer dock cleanly anchored at the bottom of the window.
- Bumped `packaging/arch/PKGBUILD`, `packaging/aur/PKGBUILD`, and `packaging/aur/.SRCINFO` to `1.6.1`.

### Fixed
- Fixed `/proc/self/mountinfo` filesystem type parsing by splitting at `" - "` so virtual/pseudo/boot/btrfs-subvolume mounts no longer flood the sidebar.

## [0.1.6] - 2026-11-24

### Changed
- Initial Omarchy UI pass, Yaru folder icon loading, and Linux-focused GitHub Actions release packaging.

## [0.1.5] - 2026-11-24

### Added
- File manager copy/cut/paste conflict prompts, customizable keyboard shortcuts overlay, grid view, and item context menu.

## [0.1.4] - 2026-11-24

### Added
- Background progress tracking for file operations, thumbnail byte-cap eviction + SVG thumbnails, batch rename, SHA-256 checksums, duplicate scanner, and folder diffing.

## [0.1.0] - 2026-11-23

### Added
- Pure-Rust file manager workspace (`clove-fs`, `clove-search`, `clove-render`, `clove-shell`, `clove-cli`, `clove-ui`) powered by Rune and Clove's animated Bayer 4×4 dithered storage bar.
