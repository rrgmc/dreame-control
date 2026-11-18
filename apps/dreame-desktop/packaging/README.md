# Releasing Dreame Control

A release is built only when a version tag is pushed. Ordinary pushes and pull requests run the
[CI workflow](../../../.github/workflows/ci.yml) instead, which checks the code but publishes nothing.

Before the first release, add the Android signing key to the repository's secrets (Settings >
Secrets and variables > Actions): `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS` and, if it differs from the keystore password, `ANDROID_KEY_PASSWORD`. Without
them the Android job fails and no release is published.

Also add the macOS signing secrets. The macOS app is signed with a Developer ID certificate and
notarized by Apple, so Gatekeeper opens it without a warning:

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATE_P12_BASE64` | The "Developer ID Application" certificate with its private key, exported from Keychain Access as a `.p12` file, base64-encoded: `base64 -i cert.p12 \| pbcopy` |
| `MACOS_CERTIFICATE_PASSWORD` | The password given to the `.p12` file on export |
| `APPLE_NOTARY_APPLE_ID` | The Apple ID of the developer account |
| `APPLE_NOTARY_PASSWORD` | An app-specific password for that Apple ID, made at account.apple.com under Sign-In and Security |

The workflow reads the team id from the certificate's name. Without these secrets the macOS job
fails and no release is published.

```bash
git tag v0.2.0
git push origin v0.2.0
```

The [release workflow](../../../.github/workflows/release.yml) then builds the desktop app on Windows, macOS and Linux and the signed Android app (`dreame-android`), and publishes a GitHub release named after the tag. The package files take the tag's number (`v0.2.0` gives `0.2.0`).

## What a release holds

| System | File | How to install |
|---|---|---|
| Windows (x64) | `DreameControl-<v>-windows-x64-setup.exe` | Run it. It installs for the current user, so it needs no administrator rights, and adds a Start menu entry. |
| Windows (x64) | `DreameControl-<v>-windows-x64.zip` | Unpack it and run `Dreame Control.exe`. |
| macOS (Apple silicon) | `DreameControl-<v>-macos-arm64.dmg` | Open it and drag Dreame Control to Applications. Signed and notarized. |
| macOS (Apple silicon) | `DreameControl-<v>-macos-arm64.zip` | The app alone, signed and notarized. |
| Linux (x86_64) | `DreameControl-<v>-linux-x86_64.AppImage` | `chmod +x` the file, then run it. |
| Linux (x86_64) | `DreameControl-<v>-linux-x86_64.tar.gz` | The program, its icon and a desktop file. |
| Android | `dreame-control-<v>.apk` | Open it on the phone and allow the install. It is signed with the release key from the repository secrets listed in the workflow. |

## The first start

The macOS app is signed with a Developer ID certificate and notarized, so it opens without a warning. The other packages are not signed with a paid code-signing certificate, so those systems warn once:

- **Windows:** SmartScreen says "Windows protected your PC". Choose **More info**, then **Run anyway**.
- **Linux:** the AppImage needs the execute permission (`chmod +x`). It uses the system's graphics and keyboard libraries (OpenGL, xkbcommon), which every desktop has.

## Building a package locally

Each script builds a release binary and writes the packages into `dist/` at the repository root.

| System | Command | Needs |
|---|---|---|
| Windows | `task app:package -- 0.2.0`, or `powershell -File apps/dreame-desktop/packaging/windows/package.ps1 0.2.0` | Inno Setup 6 |
| macOS | `bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0` | Xcode command line tools, the Rust target `aarch64-apple-darwin` |

A local macOS build is signed ad hoc unless `DREAME_SIGN_IDENTITY` names a Developer ID Application
identity in the keychain. Set `DREAME_NOTARY_PROFILE` as well to notarize it; the header of
`package.sh` says how to make that profile.
| Linux | `bash apps/dreame-desktop/packaging/linux/package.sh 0.2.0` | `curl`, and the libraries listed in the workflow |

## Cost

The repository is public, so the standard GitHub runners cost nothing. One release takes roughly 10 to 20 minutes per system.

## The icon

`apps/dreame-desktop/assets/make_icons.py` draws the icon and writes `icon-256.png`, `icon-1024.png`, `icon.ico` and `icon.icns`. It uses the Python standard library only. Run it again after changing the drawing, and commit the files it writes.
