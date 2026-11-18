#!/usr/bin/env bash
# Builds Dreame Control for macOS on Apple silicon (arm64) and packages it
# into dist/:
#   DreameControl-<version>-macos-arm64.dmg  (drag the app to Applications)
#   DreameControl-<version>-macos-arm64.zip  (the app alone)
# Usage, from the repository root:
#   bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0              signed ad hoc
#   bash apps/dreame-desktop/packaging/macos/package.sh --notarize 0.2.0   signed and notarized
#
# --notarize fills in the two settings below that are not set: the Developer ID Application
# identity found in the keychain, and the notarytool keychain profile "mac-apps". Make that
# profile once, from an App Store Connect API key:
#   xcrun notarytool store-credentials mac-apps --key <file.p8> --key-id <key id> --issuer <issuer id>
#
# Signing depends on these environment variables:
#   DREAME_SIGN_IDENTITY    a "Developer ID Application: Name (TEAMID)" identity in the keychain.
#                           Unset, the app is signed ad hoc, and the first start needs
#                           right-click > Open.
# Notarization needs DREAME_SIGN_IDENTITY and an App Store Connect API key, given in one of two ways:
#   DREAME_NOTARY_KEY       the path of the key's .p8 file,
#   DREAME_NOTARY_KEY_ID    the key's ID and
#   DREAME_NOTARY_ISSUER    its issuer ID. The release workflow sets these three.
# or
#   DREAME_NOTARY_PROFILE   the name of a notarytool keychain profile that holds the key, made once with
#                             xcrun notarytool store-credentials <name> \
#                               --key <file.p8> --key-id <key id> --issuer <issuer id>
# With either, the DMG is sent to Apple for notarization and the ticket is stapled to the DMG
# and to the app.
set -euo pipefail
NOTARIZE=0
if [ "${1:-}" = --notarize ]; then
  NOTARIZE=1
  shift
fi
VERSION="${1:?usage: package.sh [--notarize] <version>}"
if [ "$NOTARIZE" -eq 1 ]; then
  if [ -z "${DREAME_SIGN_IDENTITY:-}" ]; then
    DREAME_SIGN_IDENTITY="$(security find-identity -v -p codesigning \
      | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -1)"
    if [ -z "$DREAME_SIGN_IDENTITY" ]; then
      echo "package.sh: --notarize needs a Developer ID Application identity, and the keychain has none." >&2
      exit 1
    fi
  fi
  if [ -z "${DREAME_NOTARY_KEY:-}" ]; then
    DREAME_NOTARY_PROFILE="${DREAME_NOTARY_PROFILE:-mac-apps}"
  fi
fi
HERE="$(cd "$(dirname "$0")" && pwd)"
ASSETS="$HERE/../../assets"
SIGN_IDENTITY="${DREAME_SIGN_IDENTITY:-}"
NOTARY_ARGS=()
if [ -n "${DREAME_NOTARY_KEY:-}" ]; then
  NOTARY_ARGS=(--key "$DREAME_NOTARY_KEY" --key-id "${DREAME_NOTARY_KEY_ID:?needs DREAME_NOTARY_KEY_ID}" \
    --issuer "${DREAME_NOTARY_ISSUER:?needs DREAME_NOTARY_ISSUER}")
elif [ -n "${DREAME_NOTARY_PROFILE:-}" ]; then
  NOTARY_ARGS=(--keychain-profile "$DREAME_NOTARY_PROFILE")
fi

# Check the signing settings before the build, so a mistake costs a second and not a build.
if [ ${#NOTARY_ARGS[@]} -gt 0 ] && [ -z "$SIGN_IDENTITY" ]; then
  echo "package.sh: notarization needs DREAME_SIGN_IDENTITY; Apple does not notarize an ad hoc signature." >&2
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

if [ ${#NOTARY_ARGS[@]} -gt 0 ]; then
  # One submission covers the DMG and the app inside it. Apple's answer takes a few minutes.
  echo "Notarizing $DMG"
  OUT="$STAGE/notarytool.json"
  if ! xcrun notarytool submit "$DMG" "${NOTARY_ARGS[@]}" --wait --output-format json > "$OUT" \
      || ! grep -q '"status" *: *"Accepted"' "$OUT"; then
    cat "$OUT" >&2
    ID="$(sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' "$OUT" | head -1)"
    if [ -n "$ID" ]; then
      echo "Apple's log of the submission:" >&2
      xcrun notarytool log "$ID" "${NOTARY_ARGS[@]}" >&2 || true
    fi
    echo "package.sh: notarization failed." >&2
    exit 1
  fi
  cat "$OUT"
  # The stapled ticket lets Gatekeeper accept the file on a Mac without a network connection.
  # The submission also registered the app inside the DMG, so the app takes a ticket too.
  # Apple's ticket server sometimes drops the connection, so each staple is tried a few times.
  for FILE in "$DMG" "$APP"; do
    ATTEMPT=1
    until xcrun stapler staple "$FILE"; do
      if [ "$ATTEMPT" -ge 5 ]; then
        echo "package.sh: stapling $FILE failed $ATTEMPT times." >&2
        exit 1
      fi
      ATTEMPT=$((ATTEMPT + 1))
      echo "Stapling failed; trying again in 30 seconds." >&2
      sleep 30
    done
  done
  spctl --assess --type execute --verbose=2 "$APP"
fi

# The ZIP is made last, so it holds the app with its stapled ticket.
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
ls -l dist
