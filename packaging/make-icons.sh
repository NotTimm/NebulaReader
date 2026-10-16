#!/usr/bin/env bash
# Generates every icon the app and its packages use from two masters:
#
#   res/icons/source/book.{svg,png}       closed book: the app icon
#   res/icons/source/book-open.{svg,png}  open book: window icon while reading
#
# A PNG master should be square-ish and large (1024px+). If it has a solid
# background instead of transparency (as AI image generators tend to give),
# the background is keyed out (see KEY_SAT below). Outputs are committed, so
# building the app never needs these tools:
#
#   res/icons/hicolor/<N>x<N>/apps/com.tsingel.CosmicComic.png  (Linux)
#   res/icons/com.tsingel.CosmicComic.ico                       (Windows exe)
#   res/icons/com.tsingel.CosmicComic-open.ico                  (Windows, reading)
#   res/icons/window/book.png, book-open.png                   (runtime icon)
#
# Needs ImageMagick, plus rsvg-convert for SVG masters.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need convert "sudo dnf install ImageMagick  # or: sudo apt install imagemagick"

ICONS="$REPO_DIR/res/icons"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Background keying for PNG/JPG masters: a pixel counts as background when
# its HSL saturation and lightness are both under these percentages.
KEY_SAT="${ICON_KEY_SAT:-18}"
KEY_LIGHT="${ICON_KEY_LIGHT:-32}"

# Renders master $1 to a transparent, square 1024px PNG at $2.
master() {
    local name="$1" out="$2" src
    if [ -f "$ICONS/source/$name.svg" ]; then
        need rsvg-convert "sudo dnf install librsvg2-tools  # or: sudo apt install librsvg2-bin"
        rsvg-convert -w 1024 -h 1024 "$ICONS/source/$name.svg" -o "$out"
        return
    fi
    src=""
    for ext in png jpg jpeg webp; do
        [ -f "$ICONS/source/$name.$ext" ] && { src="$ICONS/source/$name.$ext"; break; }
    done
    [ -n "$src" ] || { echo "error: no res/icons/source/$name.{svg,png,jpg,webp}" >&2; exit 1; }

    local opaque
    opaque="$(convert "$src" -format '%[fx:p{0,0}.a]' info:)"
    if [ "$opaque" = "1" ]; then
        # Solid background (and its drop shadow): remove pixels that are both
        # dark and unsaturated and connected to the image border. Matching on
        # saturation rather than colour distance keeps dark-but-coloured
        # outlines that sit close to a dark grey background.
        local t="$WORK/$name"
        convert "$src" -resize 1024x1024 "$t-src.png"
        convert "$t-src.png" -colorspace HSL -channel G -separate +channel \
            -threshold "$KEY_SAT%" -negate "$t-lowsat.png"
        convert "$t-src.png" -colorspace HSL -channel B -separate +channel \
            -threshold "$KEY_LIGHT%" -negate "$t-dark.png"
        convert "$t-lowsat.png" "$t-dark.png" -compose multiply -composite \
            -fill red -draw "color 0,0 floodfill" -draw "color 1023,0 floodfill" \
            -draw "color 0,1023 floodfill" -draw "color 1023,1023 floodfill" \
            -fill black +opaque red -fill white -opaque red -negate \
            -morphology Open Disk:1 -blur 0x1 "$t-alpha.png"
        convert "$t-src.png" "$t-alpha.png" -alpha off -compose CopyOpacity -composite \
            "$t-keyed.png"
        src="$t-keyed.png"
    fi
    # Trim to the artwork, then centre it on a square canvas with a small margin.
    convert "$src" -trim +repage -resize 940x940 -background none \
        -gravity center -extent 1024x1024 "$out"
}

master book "$WORK/book.png"
master book-open "$WORK/book-open.png"

echo "==> Linux icon theme sizes"
rm -rf "$ICONS/hicolor"
for s in 16 24 32 48 64 128 256 512; do
    dir="$ICONS/hicolor/${s}x${s}/apps"
    mkdir -p "$dir"
    convert "$WORK/book.png" -filter Lanczos -resize "${s}x${s}" "$dir/$APP_ID.png"
done

echo "==> Windows .ico (closed book, and open book for while reading)"
convert "$WORK/book.png" -filter Lanczos \
    -define icon:auto-resize=256,128,64,48,40,32,24,20,16 "$ICONS/$APP_ID.ico"
convert "$WORK/book-open.png" -filter Lanczos \
    -define icon:auto-resize=256,128,64,48,40,32,24,20,16 "$ICONS/$APP_ID-open.ico"

echo "==> Runtime window icons"
mkdir -p "$ICONS/window"
convert "$WORK/book.png" -filter Lanczos -resize 128x128 "$ICONS/window/book.png"
convert "$WORK/book-open.png" -filter Lanczos -resize 128x128 "$ICONS/window/book-open.png"

echo "==> Done. Preview: res/icons/hicolor/*/apps/$APP_ID.png"
