#!/usr/bin/env bash
# Builds Dreame Control for macOS on Apple silicon (arm64) and packages it
# into dist/:
#   DreameControl-<version>-macos-arm64.dmg  (drag the app to Applications)
#   DreameControl-<version>-macos-arm64.zip  (the app alone)
# The app is signed ad hoc, not with an Apple developer certificate, so the first start needs
# right-click > Open (or System Settings > Privacy & Security > Open Anyway).
# Usage, from the repository root:  bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0
set -euo pipefail
VERSION="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../../assets"

TARGET=aarch64-apple-darwin
cargo build --release --locked -p dreame-desktop --target "$TARGET"

mkdir -p dist
STAGE="$(mktemp -d)"
APP="$STAGE/Dreame Control.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/dreame-desktop" "$APP/Contents/MacOS/dreame-desktop"
cp "$ASSETS/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
# Apple silicon refuses to run unsigned code; an ad hoc signature is enough to start it.
codesign --force --deep --sign - "$APP"

ditto -c -k --keepParent "$APP" "dist/DreameControl-$VERSION-macos-arm64.zip"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Dreame Control" -srcfolder "$STAGE" -ov -format UDZO "dist/DreameControl-$VERSION-macos-arm64.dmg"
rm -rf "$STAGE"
ls -l dist
