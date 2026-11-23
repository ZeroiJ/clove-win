#!/usr/bin/env bash
# Build Clove for the current Arch user and install it into ~/.local.
# Use this before a packaged build is available. No sudo required.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [[ ! -d prototypes/_rune-upstream ]]; then
  echo "Rune checkout missing at prototypes/_rune-upstream" >&2
  echo "Clone Rune dev there first; see README." >&2
  exit 1
fi

cargo build --manifest-path clove-rs/Cargo.toml --release -p clove-ui

bindir="${HOME}/.local/bin"
appdir="${HOME}/.local/share/applications"
icondir="${HOME}/.local/share/icons/hicolor"

install -Dm755 clove-rs/target/release/clove "${bindir}/clove"
install -Dm644 packaging/arch/clove.desktop "${appdir}/clove.desktop"

for icon in 16 32 128 256 512; do
  install -Dm644 "Clove/Assets.xcassets/AppIcon.appiconset/icon_${icon}x${icon}.png" \
    "${icondir}/${icon}x${icon}/apps/clove.png"
done

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "${appdir}" >/dev/null 2>&1 || true
fi

echo "Installed: ${bindir}/clove"
echo "Desktop entry: ${appdir}/clove.desktop"
