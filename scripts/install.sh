#!/usr/bin/env bash
# Installs or updates Power IDEA in /Applications from a disk image.
# Quits the running app first (it asks about unsaved changes) and starts the new one
# (OPEN=0 to skip starting it; INSTALL_DIR=... to install elsewhere).
# Usage: scripts/install.sh [path/to.dmg]   (default: the newest dist/*.dmg)
set -euo pipefail

cd "$(dirname "$0")/.."
APP_NAME="Power IDEA"
INSTALL_DIR="${INSTALL_DIR:-/Applications}"
TARGET="${INSTALL_DIR}/${APP_NAME}.app"

DMG="${1:-$(ls -t dist/*.dmg 2>/dev/null | head -1 || true)}"
[[ -n "$DMG" && -f "$DMG" ]] || { echo "no disk image; build one with scripts/dmg.sh" >&2; exit 1; }

MOUNT="$(mktemp -d /tmp/power-idea-dmg.XXXXXX)"
cleanup() { hdiutil detach -quiet "$MOUNT" 2>/dev/null || true; rmdir "$MOUNT" 2>/dev/null || true; }
trap cleanup EXIT
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$MOUNT" "$DMG"
SOURCE="$MOUNT/${APP_NAME}.app"
[[ -d "$SOURCE" ]] || { echo "$DMG has no ${APP_NAME}.app" >&2; exit 1; }

NEW="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$SOURCE/Contents/Info.plist") ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$SOURCE/Contents/Info.plist"))"
if [[ -d "$TARGET" ]]; then
  OLD="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$TARGET/Contents/Info.plist" 2>/dev/null || echo '?') ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$TARGET/Contents/Info.plist" 2>/dev/null || echo '?'))"
  echo "Updating ${APP_NAME}: $OLD → $NEW"
else
  echo "Installing ${APP_NAME} $NEW"
fi

# `ps`, not `pgrep`: pgrep sees no processes in some sandboxed shells, and then the old
# app kept running and `open` only brought it to the front instead of restarting it.
running() { ps -axo command= | grep -F "${TARGET}/Contents/MacOS/" >/dev/null; }

# Quit gracefully so unsaved work is handled by the app's own prompt.
if running; then
  osascript -e "tell application \"${APP_NAME}\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 60); do
    running || break
    sleep 0.5
  done
  if running; then
    echo "${APP_NAME} is still running (unsaved changes?). Quit it and run again." >&2
    exit 1
  fi
fi

# Copy next to the old app, then swap, so a failed copy never leaves no app at all.
STAGED="${INSTALL_DIR}/.${APP_NAME}.installing.app"
rm -rf "$STAGED"
ditto "$SOURCE" "$STAGED"
rm -rf "$TARGET"
mv "$STAGED" "$TARGET"
codesign --verify --strict "$TARGET"

echo "Installed: $TARGET"
if [[ "${OPEN:-1}" == 1 ]]; then
  open "$TARGET"
fi
