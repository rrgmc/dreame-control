#!/usr/bin/env bash
# Builds Dreame Control for macOS on Apple silicon (arm64) and packages it
# into dist/:
#   DreameControl-<version>-macos-arm64.dmg  (drag the app to Applications)
#   DreameControl-<version>-macos-arm64.zip  (the app alone)
# Usage, from the repository root:  bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0
#
# Signing depends on these environment variables:
#   DREAME_SIGN_IDENTITY    a "Developer ID Application: Name (TEAMID)" identity in the keychain.
#                           Unset, the app is signed ad hoc, and the first start needs
#                           right-click > Open.
#   DREAME_NOTARY_PROFILE   the name of a notarytool keychain profile, made once with
#                             xcrun notarytool store-credentials <name> \
#                               --apple-id <apple-id> --team-id <TEAMID> --password <app-specific>
#                           Set, the DMG is sent to Apple for notarization and the ticket is
#                           stapled to the DMG and to the app. It needs DREAME_SIGN_IDENTITY.
#   DREAME_NOTARY_KEYCHAIN  optional: the keychain that holds the profile, when it is not the
#                           default one. The release workflow sets it.
set -euo pipefail
VERSION="$1"
HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../../assets"
SIGN_IDENTITY="${DREAME_SIGN_IDENTITY:-}"
NOTARY_PROFILE="${DREAME_NOTARY_PROFILE:-}"

# Check the signing settings before the build, so a mistake costs a second and not a build.
if [ -n "$NOTARY_PROFILE" ] && [ -z "$SIGN_IDENTITY" ]; then
  echo "package.sh: DREAME_NOTARY_PROFILE needs DREAME_SIGN_IDENTITY; Apple does not notarize an ad hoc signature." >&2
  exit 1
fi
if [ -n "$SIGN_IDENTITY" ] && ! security find-identity -v -p codesigning | grep -qF "$SIGN_IDENTITY"; then
  echo "package.sh: the keychain has no signing identity named:" >&2
  echo "  $SIGN_IDENTITY" >&2
  echo "'security find-identity -v -p codesigning' lists the ones it has." >&2
  exit 1
fi

TARGET=aarch64-apple-darwin
cargo build --release --locked -p dreame-desktop --target "$TARGET"

mkdir -p dist
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/Dreame Control.app"
DMG="dist/DreameControl-$VERSION-macos-arm64.dmg"
ZIP="dist/DreameControl-$VERSION-macos-arm64.zip"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$TARGET/release/dreame-desktop" "$APP/Contents/MacOS/dreame-desktop"
cp "$ASSETS/icon.icns" "$APP/Contents/Resources/icon.icns"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"

if [ -n "$SIGN_IDENTITY" ]; then
  # Notarization requires the hardened runtime and a secure timestamp.
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
else
  # Apple silicon refuses to run unsigned code; an ad hoc signature is enough to start it.
  codesign --force --deep --sign - "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"

# The DMG is built from a copy of the stage, so the work files below stay out of it.
DMG_ROOT="$STAGE/dmg"
mkdir "$DMG_ROOT"
ditto "$APP" "$DMG_ROOT/Dreame Control.app"
ln -s /Applications "$DMG_ROOT/Applications"
rm -f "$DMG"
hdiutil create -volname "Dreame Control" -srcfolder "$DMG_ROOT" -ov -format UDZO "$DMG"
if [ -n "$SIGN_IDENTITY" ]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"
fi

if [ -n "$NOTARY_PROFILE" ]; then
  KEYCHAIN_ARGS=()
  [ -n "${DREAME_NOTARY_KEYCHAIN:-}" ] && KEYCHAIN_ARGS=(--keychain "$DREAME_NOTARY_KEYCHAIN")
  # One submission covers the DMG and the app inside it. Apple's answer takes a few minutes.
  echo "Notarizing $DMG"
  OUT="$STAGE/notarytool.json"
  if ! xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" \
      ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} --wait --output-format json > "$OUT" \
      || ! grep -q '"status" *: *"Accepted"' "$OUT"; then
    cat "$OUT" >&2
    ID="$(sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' "$OUT" | head -1)"
    if [ -n "$ID" ]; then
      echo "Apple's log of the submission:" >&2
      xcrun notarytool log "$ID" --keychain-profile "$NOTARY_PROFILE" \
        ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} >&2 || true
    fi
    echo "package.sh: notarization failed." >&2
    exit 1
  fi
  cat "$OUT"
  # The stapled ticket lets Gatekeeper accept the file on a Mac without a network connection.
  # The submission also registered the app inside the DMG, so the app takes a ticket too.
  xcrun stapler staple "$DMG"
  xcrun stapler staple "$APP"
  spctl --assess --type execute --verbose=2 "$APP"
fi

# The ZIP is made last, so it holds the app with its stapled ticket.
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
ls -l dist
