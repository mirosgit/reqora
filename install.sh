#!/usr/bin/env bash
# Build Reqora (release) and install it for the current user into ~/.local
#   ./install.sh               build + install
#   ./install.sh --no-build    install an already built target/release/reqora
#   PREFIX=/some/prefix ./install.sh
set -euo pipefail
cd "$(dirname "$0")"

APP_ID=io.github.Reqora
PREFIX="${PREFIX:-$HOME/.local}"
BIN="target/release/reqora"

if [[ "${1:-}" != "--no-build" ]]; then
    CARGO="$(command -v cargo || true)"
    [[ -z "$CARGO" && -x "$HOME/.cargo/bin/cargo" ]] && CARGO="$HOME/.cargo/bin/cargo"
    if [[ -z "$CARGO" ]]; then
        echo "cargo not found. Install Rust first: https://rustup.rs" >&2
        exit 1
    fi
    # Use half of the CPU cores at low priority so the desktop stays responsive.
    JOBS="${JOBS:-$(( $(nproc) > 1 ? $(nproc) / 2 : 1 ))}"
    echo "==> Building release binary ($JOBS jobs, low priority)…"
    nice -n 15 "$CARGO" build --release -j "$JOBS"
fi
[[ -x "$BIN" ]] || { echo "Binary $BIN not found — build first." >&2; exit 1; }

echo "==> Installing into $PREFIX"
# Remove the install from before the rename (the app used to be "Free Postman").
rm -f "$PREFIX/bin/free-postman" "$PREFIX/share/applications/io.github.FreePostman.desktop" \
      "$PREFIX/share/icons/hicolor/scalable/apps/io.github.FreePostman.svg"
for s in 48 64 128 256; do rm -f "$PREFIX/share/icons/hicolor/${s}x${s}/apps/io.github.FreePostman.png"; done
install -Dm755 "$BIN" "$PREFIX/bin/reqora"
install -Dm644 assets/$APP_ID.svg "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"
for s in 48 64 128 256; do
    install -Dm644 "assets/icon-$s.png" "$PREFIX/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"
done
tmp="$(mktemp)"
sed "s|@BIN@|$PREFIX/bin/reqora|" packaging/$APP_ID.desktop > "$tmp"
install -Dm644 "$tmp" "$PREFIX/share/applications/$APP_ID.desktop"
rm -f "$tmp"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "==> Done. Launch \"Reqora\" from the app menu or run: $PREFIX/bin/reqora"
