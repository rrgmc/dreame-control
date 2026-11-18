#!/usr/bin/env bash
# Builds Dreame Control for macOS (Apple silicon and Intel in one universal app) and packages it
# into dist/:
#   DreameControl-<version>-macos-universal.dmg  (drag the app to Applications)
#   DreameControl-<version>-macos-universal.zip  (the app alone)
# The app is signed ad hoc, not with an Apple developer certificate, so the first start needs
# right-click > Open (or System Settings > Privacy & Security > Open Anyway).
# Usage, from the repository root:  bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0
set -euo pipefail
VERSION="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../../assets"

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --locked -p dreame-desktop --target "$t"
done

mkdir -p dist
STAGE="$(mktemp -d)"
APP="$STAGE/Dreame Control.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/dreame-desktop" \
    target/aarch64-apple-darwin/release/dreame-desktop \
    target/x86_64-apple-darwin/release/dreame-desktop
cp "$ASSETS/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
# Apple silicon refuses to run unsigned code; an ad hoc signature is enough to start it.
codesign --force --deep --sign - "$APP"

ditto -c -k --keepParent "$APP" "dist/DreameControl-$VERSION-macos-universal.zip"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Dreame Control" -srcfolder "$STAGE" -ov -format UDZO "dist/DreameControl-$VERSION-macos-universal.dmg"
rm -rf "$STAGE"
ls -l dist
