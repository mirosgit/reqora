#!/usr/bin/env bash
# Remove Reqora from ~/.local. Your data (collections, history) is kept
# in ~/.local/share/reqora unless you pass --purge.
set -euo pipefail
APP_ID=io.github.Reqora
PREFIX="${PREFIX:-$HOME/.local}"
rm -f "$PREFIX/bin/reqora" \
      "$PREFIX/share/applications/$APP_ID.desktop" \
      "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
for s in 48 64 128 256; do rm -f "$PREFIX/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"; done
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
if [[ "${1:-}" == "--purge" ]]; then
    rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/reqora"
    echo "Removed app and data."
else
    echo "Removed app. Data kept in ${XDG_DATA_HOME:-$HOME/.local/share}/reqora (use --purge to delete)."
fi
