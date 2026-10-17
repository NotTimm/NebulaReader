#!/usr/bin/env bash
# Builds "Nebula Reader.app" and a drag-to-Applications .dmg for the Mac
# this runs on (Apple Silicon on CI). Needs Homebrew's libarchive and
# dylibbundler: `brew install libarchive pkg-config dylibbundler`.
#
# libarchive and its compression libraries are copied into the bundle
# (Contents/Frameworks) so the app runs without Homebrew. The app is
# signed ad hoc, not with a Developer ID, so Gatekeeper asks once before
# the first launch (see README).
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need iconutil "it ships with Xcode's command line tools"
need hdiutil
need codesign
need dylibbundler "brew install dylibbundler"

VERSION="$(version)"
ARCH="$(uname -m)" # arm64 or x86_64
# CFBundleShortVersionString must be plain numbers: 0.2.0-beta.1 -> 0.2.0.
SHORT_VERSION="${VERSION%%-*}"

# compress-tools finds libarchive with pkg-config; Homebrew's is keg-only.
if [ -z "${PKG_CONFIG_PATH:-}" ] && command -v brew >/dev/null; then
    export PKG_CONFIG_PATH="$(brew --prefix libarchive)/lib/pkgconfig"
fi
build_release

STAGE="$DIST_DIR/macos"
APP="$STAGE/Nebula Reader.app"
rm -rf "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"

echo "==> Assembling $(basename "$APP")"
install -m0755 "$BIN_PATH" "$APP/Contents/MacOS/$APP_NAME"

ICONSET="$STAGE/NebulaReader.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    cp "$REPO_DIR/res/icons/hicolor/${size}x${size}/apps/$APP_ID.png" "$ICONSET/icon_${size}x${size}.png"
    double=$((size * 2))
    if [ -f "$REPO_DIR/res/icons/hicolor/${double}x${double}/apps/$APP_ID.png" ]; then
        cp "$REPO_DIR/res/icons/hicolor/${double}x${double}/apps/$APP_ID.png" "$ICONSET/icon_${size}x${size}@2x.png"
    fi
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/NebulaReader.icns"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Nebula Reader</string>
    <key>CFBundleDisplayName</key>
    <string>Nebula Reader</string>
    <key>CFBundleIdentifier</key>
    <string>$APP_ID</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundleIconFile</key>
    <string>NebulaReader</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$SHORT_VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.books</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSHumanReadableCopyright</key>
    <string>© Tim Singel, GPL-3.0</string>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist"

echo "==> Bundling shared libraries"
dylibbundler --overwrite-dir --bundle-deps --create-dir \
    --fix-file "$APP/Contents/MacOS/$APP_NAME" \
    --dest-dir "$APP/Contents/Frameworks" \
    --install-path "@executable_path/../Frameworks/"
# Anything still pointing outside the system would break without Homebrew.
leftover="$(otool -L "$APP/Contents/MacOS/$APP_NAME" "$APP"/Contents/Frameworks/*.dylib |
    grep -E '^\s+/(opt|usr/local)/' || true)"
if [ -n "$leftover" ]; then
    echo "error: libraries still linked from outside the bundle:" >&2
    echo "$leftover" >&2
    exit 1
fi

echo "==> Signing (ad hoc)"
# Rewriting install names invalidated the libraries' signatures; Apple
# Silicon refuses to load unsigned code, so sign inside-out.
for lib in "$APP"/Contents/Frameworks/*.dylib; do
    codesign --force --sign - "$lib"
done
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=2 "$APP"

echo "==> Building the disk image"
DMG_ROOT="$STAGE/dmg"
mkdir -p "$DMG_ROOT"
cp -R "$APP" "$DMG_ROOT/"
ln -s /Applications "$DMG_ROOT/Applications"
OUT="$DIST_DIR/NebulaReader-$VERSION-macos-$ARCH.dmg"
rm -f "$OUT"
hdiutil create -volname "Nebula Reader" -srcfolder "$DMG_ROOT" -fs HFS+ -format UDZO -ov "$OUT"
rm -rf "$DMG_ROOT"

echo "==> $OUT"
