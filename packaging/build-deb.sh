#!/usr/bin/env bash
# Build a .deb package from the release binary: ./packaging/build-deb.sh
set -euo pipefail
cd "$(dirname "$0")/.."
APP_ID=io.github.Reqora
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
ARCH="$(dpkg --print-architecture)"
BIN="target/release/reqora"
[[ -x "$BIN" ]] || { echo "Build first: cargo build --release" >&2; exit 1; }

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
install -Dm755 "$BIN" "$ROOT/usr/bin/reqora"
install -Dm644 assets/$APP_ID.svg "$ROOT/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
for s in 48 64 128 256; do
    install -Dm644 "assets/icon-$s.png" "$ROOT/usr/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"
done
mkdir -p "$ROOT/usr/share/applications"
sed "s|@BIN@|/usr/bin/reqora|" packaging/$APP_ID.desktop > "$ROOT/usr/share/applications/$APP_ID.desktop"
mkdir -p "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<CTRL
Package: reqora
Version: $VERSION
Section: devel
Priority: optional
Architecture: $ARCH
Depends: libc6, libgcc-s1, libegl1 | libgl1, libxkbcommon0
Recommends: xdg-desktop-portal
Maintainer: Reqora contributors <noreply@localhost>
Description: Fast, lightweight open-source API client
 Native GPU-rendered HTTP client: collections, environments with {{variables}},
 history, cURL import/export and Postman collection import/export.
CTRL
mkdir -p dist
OUT="dist/reqora_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$ROOT" "$OUT" >/dev/null
echo "Built $OUT"
