#!/usr/bin/env bash
# Remove Reqora installed by install.sh. Your data (collections, history) is kept
# unless you pass --purge. On Windows use uninstall.ps1 instead.
#   Linux: app in ~/.local, data in ~/.local/share/reqora
#   macOS: app in ~/Applications, data in ~/Library/Application Support/Reqora
set -euo pipefail

if [[ "$(uname -s)" == "Darwin" ]]; then
    rm -rf "${APPS_DIR:-$HOME/Applications}/Reqora.app"
    DATA="$HOME/Library/Application Support/Reqora"
else
    APP_ID=io.github.Reqora
    PREFIX="${PREFIX:-$HOME/.local}"
    rm -f "$PREFIX/bin/reqora" \
          "$PREFIX/share/applications/$APP_ID.desktop" \
          "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    for s in 48 64 128 256; do rm -f "$PREFIX/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"; done
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
    DATA="${XDG_DATA_HOME:-$HOME/.local/share}/reqora"
fi

if [[ "${1:-}" == "--purge" ]]; then
    rm -rf "$DATA"
    echo "Removed app and data."
else
    echo "Removed app. Data kept in $DATA (use --purge to delete)."
fi
