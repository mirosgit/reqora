#!/usr/bin/env bash
# Print the Homebrew cask for a released .dmg:
#   packaging/homebrew/render-cask.sh <version> <path/to/reqora_<version>_macos_universal.dmg> [owner/repo]
# The repository defaults to $GITHUB_REPOSITORY (set in GitHub Actions), then to mirosgit/reqora.
set -euo pipefail
[[ $# -ge 2 ]] || { echo "usage: $0 <version> <dmg> [owner/repo]" >&2; exit 1; }
VERSION="$1"
DMG="$2"
REPOSITORY="${3:-${GITHUB_REPOSITORY:-mirosgit/reqora}}"
[[ -f "$DMG" ]] || { echo "No such file: $DMG" >&2; exit 1; }
if command -v sha256sum >/dev/null; then
    SHA256="$(sha256sum "$DMG" | cut -d' ' -f1)"
else
    SHA256="$(shasum -a 256 "$DMG" | cut -d' ' -f1)"
fi
sed -e "s|@VERSION@|$VERSION|g" -e "s|@SHA256@|$SHA256|g" -e "s|@REPOSITORY@|$REPOSITORY|g" \
    "$(dirname "$0")/reqora.rb.in"
