#!/usr/bin/env bash
# Builds Dreame Control for Linux (x86_64) and packages it into dist/:
#   DreameControl-<version>-linux-x86_64.AppImage  (one file: make it executable and run it)
#   DreameControl-<version>-linux-x86_64.tar.gz    (the program, icon and desktop file)
# Build on the oldest system the app should run on (the release uses Ubuntu 22.04), since the
# program links against that system's C library.
# Usage, from the repository root:  bash apps/dreame-desktop/packaging/linux/package.sh 0.2.0
set -euo pipefail
VERSION="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../../assets"

cargo build --release --locked -p dreame-desktop
mkdir -p dist
STAGE="$(mktemp -d)"

# The tarball.
TAR="$STAGE/dreame-control-$VERSION"
mkdir -p "$TAR"
cp target/release/dreame-desktop "$TAR/"
cp "$ASSETS/icon-256.png" "$TAR/dreame-control.png"
cp "$HERE/dreame-control.desktop" "$TAR/"
tar -C "$STAGE" -czf "dist/DreameControl-$VERSION-linux-x86_64.tar.gz" "dreame-control-$VERSION"

# The AppImage.
APPDIR="$STAGE/AppDir"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp target/release/dreame-desktop "$APPDIR/usr/bin/"
cp "$HERE/dreame-control.desktop" "$APPDIR/dreame-control.desktop"
cp "$HERE/dreame-control.desktop" "$APPDIR/usr/share/applications/"
cp "$ASSETS/icon-256.png" "$APPDIR/dreame-control.png"
cp "$ASSETS/icon-256.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/dreame-control.png"
ln -s usr/bin/dreame-desktop "$APPDIR/AppRun"
TOOL="$STAGE/appimagetool"
curl -fsSL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x "$TOOL"
ARCH=x86_64 "$TOOL" --appimage-extract-and-run "$APPDIR" "dist/DreameControl-$VERSION-linux-x86_64.AppImage"

rm -rf "$STAGE"
ls -l dist
