#!/usr/bin/env bash
# Capture the store screenshots (library, series, reader, book) on a COSMIC
# Wayland session.
#
# Uses `cosmic-screenshot` (ships with COSMIC), which goes through
# xdg-desktop-portal-cosmic. In the default interactive mode COSMIC's own
# screenshot picker opens: choose "Window", click the Nebula Reader window and
# save. That gives a window-only PNG with no desktop around it.
#
#   res/screenshots/capture.sh                 # all four, interactive
#   res/screenshots/capture.sh reader book     # just these
#   res/screenshots/capture.sh --profile ~/cc-demo   # throwaway library
#   res/screenshots/capture.sh --full          # whole screen, crop later
#   res/screenshots/capture.sh --self-test     # check capture works, keep nothing
#
# Options:
#   --profile DIR  Run the app with XDG_DATA_HOME/XDG_CACHE_HOME under DIR, so
#                  the shots show a demo library instead of your real one.
#                  (Config stays put so the COSMIC theme still applies.)
#   --bin PATH     Binary to run (default: nebula-reader on PATH, else
#                  target/release/nebula-reader in this repo).
#   --no-launch    Don't start the app; use the window that's already open.
#   --full         Non-interactive full-screen capture (every monitor). You
#                  then have to crop each PNG to the window yourself.
#   --self-test    Take one non-interactive capture into a temp dir, print its
#                  size, delete it, exit. Doesn't launch the app.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="$HERE"
MAX_BYTES=$((3 * 1024 * 1024))

ALL_SHOTS=(library series reader book)
declare -A HOWTO=(
  [library]="Library view: the main grid of series with the Continue Reading shelf visible. Clear the search box and scroll to the top."
  [series]="Series page: open a series that has a matched description, publisher and several issues, some read and one in progress."
  [reader]="Comic reader: open a page with strong art. Single or dual page, not zoomed, no info or settings drawer open."
  [book]="Book reader: an EPUB in the middle of a chapter with body text (not a cover or title page). Default font and theme are fine."
)

profile="" bin="" launch=1 full=0 selftest=0 shots=()
while (($#)); do
  case "$1" in
    --profile) profile="${2:?--profile needs a directory}"; shift 2 ;;
    --bin) bin="${2:?--bin needs a path}"; shift 2 ;;
    --no-launch) launch=0; shift ;;
    --full) full=1; shift ;;
    --self-test) selftest=1; shift ;;
    -h|--help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    library|series|reader|book) shots+=("$1"); shift ;;
    *) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
  esac
done
((${#shots[@]})) || shots=("${ALL_SHOTS[@]}")

die() { echo "error: $*" >&2; exit 1; }

command -v cosmic-screenshot >/dev/null ||
  die "cosmic-screenshot not found (Fedora: sudo dnf install cosmic-screenshot)"
[[ "${XDG_SESSION_TYPE:-}" == wayland ]] || echo "warning: not a Wayland session" >&2
[[ "${XDG_CURRENT_DESKTOP:-}" == *COSMIC* ]] || echo "warning: not a COSMIC session; the portal picker may differ" >&2

# Prints "WxH bytes" for a PNG.
describe() {
  local dims bytes
  bytes=$(stat -c %s "$1")
  if command -v identify >/dev/null; then
    dims=$(identify -format '%wx%h' "$1" 2>/dev/null || echo '?')
  else
    dims=$(file -b "$1" | grep -oE '[0-9]+ x [0-9]+' | tr -d ' ' || echo '?')
  fi
  echo "$dims $bytes"
}

# One capture. Prints the path of the new PNG on stdout, or nothing.
# $1 = directory for non-interactive saves.
capture() {
  local tmpdir="$1" marker path
  marker="$tmpdir/.marker"; : >"$marker"
  if ((full)) || ((selftest)); then
    path=$(cosmic-screenshot --interactive=false --modal=false --notify=false \
             --save-dir "$tmpdir" 2>/dev/null | tail -n1) || true
  else
    path=$(cosmic-screenshot --interactive=true --notify=false 2>/dev/null | tail -n1) || true
  fi
  path="${path#file://}"
  if [[ -n "$path" && -f "$path" ]]; then echo "$path"; return; fi

  # Picker saved somewhere without telling us: look for a fresh file in the
  # usual Screenshots folder.
  local pics
  pics="$(xdg-user-dir PICTURES 2>/dev/null || echo "$HOME/Pictures")"
  path=$(find "$pics" "$pics/Screenshots" "$tmpdir" -maxdepth 1 -name '*.png' \
           -newer "$marker" -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -n1 | cut -d' ' -f2-) || true
  if [[ -n "$path" && -f "$path" ]]; then echo "$path"; return; fi

  # "Copy to clipboard" in the picker.
  if command -v wl-paste >/dev/null &&
     wl-paste --list-types 2>/dev/null | grep -qx image/png; then
    wl-paste --type image/png >"$tmpdir/clip.png" 2>/dev/null &&
      [[ -s "$tmpdir/clip.png" ]] && echo "$tmpdir/clip.png"
  fi
}

TMP="$(mktemp -d -t nebula-reader-shots.XXXXXX)"
APP_PID=""
cleanup() {
  rm -rf "$TMP"
  if [[ -n "$APP_PID" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ((selftest)); then
  p=$(capture "$TMP")
  [[ -n "$p" ]] || die "capture produced no file"
  read -r dims bytes < <(describe "$p")
  echo "ok: cosmic-screenshot captured $dims ($bytes bytes); deleted"
  exit 0
fi

# ── launch the app ──────────────────────────────────────────────────────────
if ((launch)); then
  if [[ -z "$bin" ]]; then
    if command -v nebula-reader >/dev/null; then bin="$(command -v nebula-reader)"
    elif [[ -x "$REPO/target/release/nebula-reader" ]]; then bin="$REPO/target/release/nebula-reader"
    else die "nebula-reader not found; build it (just build-release) or pass --bin"
    fi
  fi
  if pgrep -x nebula-reader >/dev/null; then
    echo "Nebula Reader is already running. Close it first so the window opens at"
    echo "its default size, or re-run with --no-launch to use it as is."
    exit 1
  fi
  env_args=()
  if [[ -n "$profile" ]]; then
    mkdir -p "$profile/data" "$profile/cache"
    real_key="${XDG_DATA_HOME:-$HOME/.local/share}/nebula-reader/comicvine_api_key.txt"
    if [[ -f "$real_key" && ! -f "$profile/data/nebula-reader/comicvine_api_key.txt" ]]; then
      mkdir -p "$profile/data/nebula-reader"
      cp "$real_key" "$profile/data/nebula-reader/"
    fi
    env_args=(XDG_DATA_HOME="$profile/data" XDG_CACHE_HOME="$profile/cache")
    echo "Using demo profile $profile (add library folders there; it persists between runs)."
  fi
  env "${env_args[@]}" "$bin" >"$TMP/app.log" 2>&1 &
  APP_PID=$!
  sleep 2
  kill -0 "$APP_PID" 2>/dev/null || { cat "$TMP/app.log" >&2; die "app exited on startup"; }
  echo "Started $bin (pid $APP_PID) at its default 1024x768 window size."
  echo "Don't resize or maximize it, so all shots come out the same size."
fi

echo
if ((full)); then
  echo "Full-screen mode: each shot captures every monitor. Crop to the window afterwards."
else
  echo "For each shot COSMIC's screenshot picker opens: pick \"Window\", click the"
  echo "Nebula Reader window, then Save (Copy to clipboard also works)."
fi

saved=()
for name in "${shots[@]}"; do
  echo
  echo "── $name.png ──"
  echo "${HOWTO[$name]}"
  if ((full)); then
    read -r -p "Set it up, then press Enter; capture happens 3 s later (s = skip): " ans
  else
    read -r -p "Set it up, then press Enter to open the picker (s = skip): " ans
  fi
  [[ "$ans" == s* ]] && { echo "skipped"; continue; }
  ((full)) && sleep 3

  while :; do
    p=$(capture "$TMP")
    if [[ -n "$p" ]]; then break; fi
    read -r -p "No screenshot came back. Retry? [Y/n] " ans
    [[ "$ans" == [nN]* ]] && { p=""; break; }
  done
  [[ -n "$p" ]] || { echo "skipped"; continue; }

  dest="$OUT/$name.png"
  if command -v magick >/dev/null; then
    # Strip metadata (timestamps etc.) and compress harder than the portal does.
    magick "$p" -strip -define png:compression-level=9 "$dest"
  else
    cp "$p" "$dest"
  fi
  # Only remove the original if it was ours, not the user's Pictures folder.
  [[ "$p" == "$TMP"/* ]] && rm -f "$p"

  read -r dims bytes < <(describe "$dest")
  echo "saved $dest  $dims  $((bytes / 1024)) KiB"
  ((bytes > MAX_BYTES)) && echo "  warning: over 3 MB; Flathub prefers smaller (try a less busy page)"
  saved+=("$name")
done

echo
if ((${#saved[@]})); then
  echo "Saved: ${saved[*]} in $OUT"
  ((full)) && echo "Remember to crop them to the window (e.g. in GIMP) before committing."
  echo "Check them against res/screenshots/README.md before committing."
else
  echo "Nothing saved."
fi
