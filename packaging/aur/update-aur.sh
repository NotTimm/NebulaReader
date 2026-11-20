#!/usr/bin/env bash
# Updates packaging/aur/PKGBUILD and packaging/aur/.SRCINFO for a release tag.
#
# Usage:
#   bash packaging/aur/update-aur.sh 0.2.2
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <version>" >&2
    exit 1
fi

VERSION="$1"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
URL="https://github.com/NotTimm/NebulaReader/archive/refs/tags/v${VERSION}.tar.gz"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

echo "==> Downloading $URL"
curl -fsSL "$URL" -o "$TMP"
SHA="$(sha256sum "$TMP" | awk '{print $1}')"

python3 - "$DIR/PKGBUILD" "$DIR/.SRCINFO" "$VERSION" "$SHA" <<'PY'
import re, sys
pkgbuild, srcinfo, version, sha = sys.argv[1:]

p = open(pkgbuild, encoding="utf-8").read()
p = re.sub(r'^pkgver=.*$', f'pkgver={version}', p, count=1, flags=re.M)
p = re.sub(r'^pkgrel=.*$', 'pkgrel=1', p, count=1, flags=re.M)
p = re.sub(r"^sha256sums=\('[^']*'\)$", f"sha256sums=('{sha}')", p, count=1, flags=re.M)
open(pkgbuild, "w", encoding="utf-8").write(p)

s = open(srcinfo, encoding="utf-8").read()
s = re.sub(r'^\tpkgver = .*$', f'\tpkgver = {version}', s, count=1, flags=re.M)
s = re.sub(r'^\tpkgrel = .*$', '\tpkgrel = 1', s, count=1, flags=re.M)
s = re.sub(
    r'^\tsource = .*$',
    f'\tsource = nebula-reader-{version}.tar.gz::https://github.com/NotTimm/NebulaReader/archive/refs/tags/v{version}.tar.gz',
    s,
    count=1,
    flags=re.M,
)
s = re.sub(r'^\tsha256sums = .*$', f'\tsha256sums = {sha}', s, count=1, flags=re.M)
open(srcinfo, "w", encoding="utf-8").write(s)
PY

echo "Updated PKGBUILD and .SRCINFO -> version $VERSION (sha256 $SHA)"
