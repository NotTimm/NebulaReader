#!/usr/bin/env bash
# Updates packaging/homebrew/nebula-reader.rb for a release version.
#
# Usage:
#   bash packaging/homebrew/update-cask.sh 0.2.2 [path/to/NebulaReader-0.2.2-macos-arm64.dmg]
#
# If no local .dmg path is given, the script downloads the release .dmg from
# GitHub Releases to compute its SHA-256.
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "Usage: $0 <version> [path-to-dmg]" >&2
    exit 1
fi

VERSION="$1"
CASK="$(dirname "${BASH_SOURCE[0]}")/nebula-reader.rb"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

if [ "$#" -eq 2 ]; then
    DMG="$2"
    [ -f "$DMG" ] || { echo "File not found: $DMG" >&2; exit 1; }
    SHA="$(sha256_of "$DMG")"
else
    TMP="$(mktemp)"
    trap 'rm -f "$TMP"' EXIT
    URL="https://github.com/NotTimm/NebulaReader/releases/download/v${VERSION}/NebulaReader-${VERSION}-macos-arm64.dmg"
    echo "==> Downloading $URL"
    curl -fsSL "$URL" -o "$TMP"
    SHA="$(sha256_of "$TMP")"
fi

python3 - "$CASK" "$VERSION" "$SHA" <<'PY'
import re, sys
path, version, sha = sys.argv[1:]
text = open(path, encoding="utf-8").read()
text, n1 = re.subn(r'version "[^"]+"', f'version "{version}"', text, count=1)
text, n2 = re.subn(r'sha256 "[^"]+"', f'sha256 "{sha}"', text, count=1)
if n1 != 1 or n2 != 1:
    sys.exit("Could not find version/sha256 lines in cask")
open(path, "w", encoding="utf-8").write(text)
PY

echo "Updated $CASK -> version $VERSION (sha256 $SHA)"
