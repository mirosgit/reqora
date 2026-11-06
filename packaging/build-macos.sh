#!/usr/bin/env bash
# Build Reqora.app and a .dmg from the release binary. Runs on macOS, after `cargo build --release`.
#   ./packaging/build-macos.sh              → dist/Reqora.app + dist/reqora_<version>_macos_<arch>.dmg
#   ./packaging/build-macos.sh --app-only   → dist/Reqora.app only
#   BIN=path/to/reqora ./packaging/build-macos.sh   package another binary (e.g. a universal one made with lipo)
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == "Darwin" ]] || { echo "This script runs on macOS only." >&2; exit 1; }
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
BIN="${BIN:-target/release/reqora}"
[[ -x "$BIN" ]] || { echo "Build first: cargo build --release" >&2; exit 1; }

APP="dist/Reqora.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/reqora"
cp assets/Reqora.icns "$APP/Contents/Resources/Reqora.icns"
sed "s|@VERSION@|$VERSION|g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
# Ad-hoc signature: Apple Silicon refuses to run unsigned code, and it seals the bundle.
codesign --force --sign - "$APP"
echo "Built $APP"

[[ "${1:-}" == "--app-only" ]] && exit 0

ARCHS="$(lipo -archs "$BIN")"
case "$ARCHS" in
    *" "*) ARCH=universal ;;
    *) ARCH="$ARCHS" ;;
esac
DMG="dist/reqora_${VERSION}_macos_${ARCH}.dmg"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
# hdiutil occasionally fails with "Resource busy" on CI machines, so retry a few times.
for attempt in 1 2 3; do
    hdiutil create -volname Reqora -srcfolder "$STAGE" -fs HFS+ -format UDZO -quiet "$DMG" && break
    [[ $attempt == 3 ]] && exit 1
    sleep 5
done
echo "Built $DMG"
