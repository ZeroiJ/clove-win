<div align="center">

# Clove for Windows

**A fast, standalone file manager for Windows 11, written in Rust.**

Inspired by [Clove](https://github.com/keshav-exe/clove) — the native macOS app I loved but couldn't run because I don't own a Mac. So I ported the idea to Windows 11 and remade the whole thing from scratch in Rust: a single, self-contained `clove.exe` with no runtime dependencies.

[**Download the latest .exe**](https://github.com/ZeroiJ/clove-win/releases/latest) · [Build from source](#build-from-source)

</div>

## What it is

Clove for Windows is a file manager. It browses your drives and folders, lets you filter, open, and delete files, and shows your disk usage as a live, dithered storage bar broken down by kind — apps, code, photos, movies, documents, music, system, caches, and the trash.

It started as a port of Clove's storage view and file browsing to Windows, and grew into a full standalone file manager. Everything runs on background threads; an idle window draws nothing.

## Features

- **File browsing** — virtualized list, background directory loads, live reloads via `ReadDirectoryChangesW`, keyboard navigation (↑↓, Home/End, Enter to open, Backspace to go up).
- **Filter** — type to filter by name; pre-folded names make it sub-millisecond on thousands of entries.
- **Storage bar** — Clove's signature dithered bar, ported byte-for-byte (Bayer 4×4, value noise, glints, dirty-rect repaint, settle-stop). It animates for ~1.5 s after an action, then the window idles at 0 % CPU.
- **Real disk scan** — measures the disk by kind on three worker threads; the bar fills in as kinds finish. Results are cached for 30 minutes so relaunches are instant.
- **Trash** — Delete moves to the Recycle Bin with an exact receipt; Ctrl+Z puts it back. Guarded so a path can never escape its folder.
- **Standalone** — one `clove.exe`, ~19 MB, no installer, no runtime, no account. DirectX 12 GPU only.

## Keyboard

| Key | Action |
|---|---|
| Type | Filter files by name |
| ↑ / ↓ | Move selection |
| Home / End | First / last |
| Enter | Open folder |
| Backspace | Go up |
| Delete | Move to Recycle Bin |
| Ctrl+Z | Undo last trash |
| Ctrl+R | Rescan the disk |
| Esc | Clear filter |

## Download

Grab `clove.exe` from the [latest release](https://github.com/ZeroiJ/clove-win/releases/latest). Windows SmartScreen may warn about an unsigned binary — click **More info → Run anyway**.

## Build from source

Needs Rust 1.99+ (MSVC) and the pinned Rune checkout.

```bash
git clone https://github.com/ZeroiJ/clove-win.git
cd clove-win
# fetch the Rune upstream checkout (git-ignored, required to build)
git clone --depth 1 https://github.com/rune-rs/rune.git prototypes/_rune-upstream
# (pin to the rev recorded in clove-rs/Cargo.toml)

cd clove-rs
cargo build -p clove-ui --release
# binary: clove-rs/target/release/clove.exe
```

Headless engine tests:

```bash
cd clove-rs
cargo test -p clove-fs -p clove-search -p clove-render -p clove-shell
cargo run -p clove-cli -- scan
```

## Project layout

```
clove-rs/
  crates/
    clove-fs/      filesystem engine: listing, folded names, watching, storage scan
    clove-search/  fuzzy filter (pre-folded names)
    clove-render/  dithered storage bar + design tokens (UI-free)
    clove-shell/   trash/restore safety with Recycle Bin receipts
    clove-cli/     headless test surface: list, find, bar, bench, scan
    clove-ui/      the Rune app (the only crate that names a UI framework)
docs/rust-port/    architecture, framework comparison, roadmap, phase reports
Clove/             original macOS app (reference)
```

## Credits

- [Clove](https://github.com/keshav-exe/clove) by keshav-exe — the macOS app whose storage view and file browsing inspired this port. MIT licensed.
- [Rune](https://github.com/rune-rs/rune) — the Rust GPU UI framework the app is built on.

## License

MIT — see [LICENSE](LICENSE).
