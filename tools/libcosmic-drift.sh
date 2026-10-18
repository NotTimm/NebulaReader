#!/usr/bin/env bash
# How far the NotTimm/libcosmic fork is behind pop-os/libcosmic, and whether
# Nebula's Cargo.toml pins the fork's latest commit. Read-only apart from a
# `git fetch` in the fork clone. See "The libcosmic fork" in
# docs/DEVELOPING.md for how to sync.
#
#   tools/libcosmic-drift.sh [path to a NotTimm/libcosmic clone]
#
# Defaults to ../libcosmic next to this checkout.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
fork="${1:-$here/../libcosmic}"
if [ ! -d "$fork/.git" ] && [ ! -f "$fork/.git" ]; then
    echo "No libcosmic clone at $fork." >&2
    echo "Clone it with: git clone git@github.com:NotTimm/libcosmic.git $fork" >&2
    exit 1
fi
cd "$fork"

git remote get-url upstream >/dev/null 2>&1 ||
    git remote add upstream https://github.com/pop-os/libcosmic.git
git fetch -q origin
git fetch -q upstream

base="$(git merge-base origin/master upstream/master)"
behind="$(git rev-list --count origin/master..upstream/master)"
pinned="$(grep -oE 'rev = "[0-9a-f]{40}"' "$here/Cargo.toml" | head -1 | grep -oE '[0-9a-f]{40}')"
fork_head="$(git rev-parse origin/master)"

echo "Fork master:      $(git log -1 --format='%h %cs %s' origin/master)"
echo "Synced up to:     $(git log -1 --format='%h %cs %s' "$base")"
echo "Upstream master:  $(git log -1 --format='%h %cs %s' upstream/master)"
echo "Upstream commits not in the fork: $behind"
if [ "$pinned" = "$fork_head" ]; then
    echo "Cargo.toml pins the fork's master."
else
    echo "Cargo.toml pins $(git log -1 --format='%h %cs %s' "$pinned" 2>/dev/null || echo "$pinned"), not the fork's master."
fi

old_iced="$(git ls-tree "$base" iced | awk '{print $3}')"
new_iced="$(git ls-tree upstream/master iced | awk '{print $3}')"
if [ "$old_iced" != "$new_iced" ]; then
    echo "Upstream moved its iced pin: ${old_iced:0:8} -> ${new_iced:0:8} (re-vendor iced when syncing)."
fi

if [ "$behind" -gt 0 ]; then
    echo
    echo "Upstream fixes since the last sync:"
    git log --format='  %h %cs %s' "$base..upstream/master" | grep -E ' fix|revert|Revert' | head -30 || true
fi
