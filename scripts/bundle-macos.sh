#!/usr/bin/env bash
# Builds a release binary and packages it as "Power IDEA.app" with the app icon.
# Usage: scripts/bundle-macos.sh   (output: target/release/Power IDEA.app)
set -euo pipefail

source "$(dirname "$0")/env.sh"
APP_NAME="Power IDEA"
BUNDLE_ID="dev.power-idea.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
# Build number = commit count, matching the version shown inside the app.
BUILD="$(git rev-list --count HEAD 2>/dev/null || echo 0)"
OUT="target/release/${APP_NAME}.app"
WORK="target/bundle"

cargo build --release -p power-idea

# Icon: rasterize the SVG at every size macOS expects, then pack into .icns.
rm -rf "$WORK" && mkdir -p "$WORK/AppIcon.iconset"
sips -s format png assets/icon.svg --out "$WORK/icon-1024.png" >/dev/null
for size in 16 32 128 256 512; do
  sips -z $size $size "$WORK/icon-1024.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$WORK/icon-1024.png" --out "$WORK/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$WORK/AppIcon.iconset" -o "$WORK/AppIcon.icns"

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp target/release/power-idea "$OUT/Contents/MacOS/power-idea"
cp "$WORK/AppIcon.icns" "$OUT/Contents/Resources/AppIcon.icns"
cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>power-idea</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${BUILD}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSDocumentsFolderUsageDescription</key><string>Power IDEA opens and edits projects stored in your Documents folder.</string>
  <key>NSDesktopFolderUsageDescription</key><string>Power IDEA opens and edits projects stored on your Desktop.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>Power IDEA opens and edits projects stored in Downloads.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>Power IDEA opens projects stored on external drives.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>Power IDEA opens projects stored on network drives.</string>
</dict>
</plist>
PLIST

# Sign the whole bundle so macOS remembers privacy permissions (TCC) for it.
# A stable certificate keeps them across rebuilds; ad-hoc ("-") only within one build.
# Override with CODESIGN_IDENTITY=..., create a local one with scripts/create-signing-cert.sh.
IDENTITY="${CODESIGN_IDENTITY:-}"
if [[ -z "$IDENTITY" ]] && security find-certificate -c "Power IDEA Local" >/dev/null 2>&1; then
  IDENTITY="Power IDEA Local"
fi
IDENTITY="${IDENTITY:--}"
codesign --force --sign "$IDENTITY" --identifier "$BUNDLE_ID" --timestamp=none "$OUT"
codesign --verify --strict "$OUT"
echo "Signed with: $([[ "$IDENTITY" == "-" ]] && echo "ad-hoc (permissions persist per build)" || echo "$IDENTITY")"

echo "Built: $OUT"
