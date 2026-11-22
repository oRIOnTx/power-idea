#!/usr/bin/env bash
# Captures one IDE window for the README: click the window after running it.
# Usage: scripts/screenshot.sh <name>   (saves docs/screenshots/<name>.png)
# Names used by README.md: editor, git-log, debug, database, welcome
set -euo pipefail

NAME="${1:?usage: scripts/screenshot.sh <name>}"
cd "$(dirname "$0")/.."
mkdir -p docs/screenshots
echo "Click the Power IDEA window to capture it as docs/screenshots/${NAME}.png"
# -w: pick a window, -o: no drop shadow, -x: no sound
screencapture -w -o -x "docs/screenshots/${NAME}.png"
echo "Saved docs/screenshots/${NAME}.png"
