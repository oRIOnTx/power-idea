#!/usr/bin/env bash
# Keeps the newest disk images in dist/ and deletes the older ones (newest = last modified,
# so it works whatever the version format). Run by scripts/dmg.sh after each build.
# Usage: scripts/prune-dmgs.sh [count]   (default: $DMG_KEEP or 3)
set -euo pipefail

cd "$(dirname "$0")/.."
KEEP="${1:-${DMG_KEEP:-3}}"
[[ "$KEEP" =~ ^[1-9][0-9]*$ ]] || { echo "count must be a positive number: $KEEP" >&2; exit 2; }
[[ -d dist ]] || exit 0

# Newest first; only our own images, never anything else in dist/. (bash 3.2: no mapfile.)
# shellcheck disable=SC2012 # ls -t sorts by time; our file names have no spaces or newlines.
{ ls -t dist/Power-IDEA-*.dmg 2>/dev/null || true; } | {
  seen=0
  while IFS= read -r image; do
    seen=$((seen + 1))
    if ((seen > KEEP)); then
      rm -f -- "$image"
      echo "Removed old disk image: $image"
    fi
  done
}
