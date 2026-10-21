#!/usr/bin/env bash
# Type-checks the app for Windows and macOS from Linux, so code behind
# #[cfg(windows)] / #[cfg(target_os = "macos")] is at least compiled before
# CI. C dependencies (zstd, bzip2, libarchive...) aren't really built for
# the other targets: a stand-in compiler writes empty objects, which is
# enough for `cargo check` (nothing is linked).
#
# Needs: rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin
set -euo pipefail
cd "$(dirname "$0")/.."
fake="$(mktemp -d)"
trap 'rm -rf "$fake"' EXIT
cat > "$fake/cc" <<'CC'
#!/bin/sh
out=""; prev=""
for a in "$@"; do
    [ "$prev" = "-o" ] && out="$a"
    case "$a" in -o?*) out="${a#-o}" ;; esac
    prev="$a"
done
[ -n "$out" ] && : > "$out"
exit 0
CC
cat > "$fake/ar" <<'AR'
#!/bin/sh
for a in "$@"; do case "$a" in *.a) printf '!<arch>\n' > "$a" ;; esac; done
exit 0
AR
chmod +x "$fake/cc" "$fake/ar"
export PKG_CONFIG_ALLOW_CROSS=1
for target in x86_64-pc-windows-gnu aarch64-apple-darwin; do
    echo "==> $target"
    t="${target//-/_}"
    env "CC_$t=$fake/cc" "CXX_$t=$fake/cc" "AR_$t=$fake/ar" cargo check --target "$target" "$@"
done
