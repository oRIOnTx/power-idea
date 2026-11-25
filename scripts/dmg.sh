#!/usr/bin/env bash
# Builds a drag-to-install disk image: "Power IDEA.app" next to an Applications link.
# Usage: scripts/dmg.sh [--skip-build] [--install]
#   --skip-build  package the existing target/release/Power IDEA.app
#   --install     install / update /Applications right after (scripts/install.sh)
# Output: dist/Power-IDEA-<version>-<build>.dmg; images older than the newest 3 are deleted
#         (DMG_KEEP=<n> keeps more; scripts/prune-dmgs.sh)
set -euo pipefail

cd "$(dirname "$0")/.."
APP_NAME="Power IDEA"
APP="target/release/${APP_NAME}.app"
SKIP_BUILD=0
INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --install) INSTALL=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [[ $SKIP_BUILD == 0 ]]; then
  scripts/bundle-macos.sh
fi
[[ -d "$APP" ]] || { echo "no $APP; run without --skip-build" >&2; exit 1; }

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")"
BUILD="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Contents/Info.plist")"
DMG="dist/Power-IDEA-${VERSION}-${BUILD}.dmg"
STAGE="target/dmg"

rm -rf "$STAGE" && mkdir -p "$STAGE" dist
# ditto keeps the code signature and extended attributes intact.
ditto "$APP" "$STAGE/${APP_NAME}.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create -quiet -volname "${APP_NAME} ${VERSION}" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
rm -rf "$STAGE"

# Sign the image too when a real (non ad-hoc) identity signed the app.
IDENTITY="${CODESIGN_IDENTITY:-}"
if [[ -z "$IDENTITY" ]] && security find-identity -v -p codesigning | grep -q '"Power IDEA Local"'; then
  IDENTITY="Power IDEA Local"
fi
if [[ -n "$IDENTITY" ]]; then
  codesign --force --sign "$IDENTITY" --timestamp=none "$DMG"
fi
hdiutil verify -quiet "$DMG"

echo "Disk image: $DMG ($(du -h "$DMG" | cut -f1 | tr -d ' '))"
# Only the newest images stay in dist/ (DMG_KEEP, default 3).
scripts/prune-dmgs.sh

if [[ $INSTALL == 1 ]]; then
  scripts/install.sh "$DMG"
fi
