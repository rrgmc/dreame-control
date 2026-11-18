# Building Dreame Control

This page explains how to build the desktop app and the Android app from source. To use the apps, download them from the [releases page](https://github.com/rrgmc/dreame-control/releases) instead.

## What you need

| Tool | For | Notes |
|---|---|---|
| [Rust](https://rustup.rs) | Everything | Version 1.80 or newer |
| [go-task](https://taskfile.dev) | The `task` commands | Optional: each task is a plain command in `Taskfile.yml` |
| Linux build libraries | The desktop app on Linux | See below |
| Android SDK and NDK 28.2.13676358 | The Android app | Android Studio installs both |
| [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) | The Android app | `cargo install cargo-ndk` |
| Java 21 | The Android app | Android Studio includes one |

On Ubuntu or Debian, the desktop app needs these libraries:

```bash
sudo apt-get install -y libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libwayland-dev libgl1-mesa-dev libdbus-1-dev pkg-config
```

For the Android app, add the Rust targets:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
```

## Desktop app

Run it:

```bash
task app
```

Build an optimized program into `target/release`:

```bash
task app:release
```

### Packages

Each script builds a release program and writes the packages into `dist/`.

| System | Command | Needs |
|---|---|---|
| Windows | `task app:package -- 0.2.0` | Inno Setup 6 |
| macOS | `bash apps/dreame-desktop/packaging/macos/package.sh 0.2.0` | Xcode command line tools, the Rust targets `aarch64-apple-darwin` and `x86_64-apple-darwin` |
| Linux | `bash apps/dreame-desktop/packaging/linux/package.sh 0.2.0` | `curl` and the libraries above |

## Android app

The Android build has two steps. First it builds the Rust library for arm64 phones and the x86_64 emulator and generates its Kotlin bindings. Then Gradle builds the APK. The tasks below run both steps.

On Windows, the tasks look for the Android SDK in `%LOCALAPPDATA%\Android\Sdk`. Set `ANDROID_HOME` if yours is elsewhere. If `cargo ndk` cannot find the NDK, set `ANDROID_NDK_HOME` to its folder.

Build a debug APK:

```bash
task android
```

Build a release APK:

```bash
task android:release
```

The APKs are in `apps/dreame-android/app/build/outputs/apk/`.

### Signing a release APK

Without a signing key, the release APK is signed with the debug key. To sign it with your own key, create `apps/dreame-android/keystore.properties`. Git ignores this file.

```properties
storeFile=release.jks
storePassword=...
keyAlias=...
keyPassword=...
```

`storeFile` is relative to `apps/dreame-android`. Instead of the file, you can set the variables `DREAME_KEYSTORE_FILE`, `DREAME_KEYSTORE_PASSWORD`, `DREAME_KEY_ALIAS` and `DREAME_KEY_PASSWORD`.

## Checks

Every commit must pass clippy and the tests:

```bash
task check
```

See [RUST.md](RUST.md) for the tests that need a real account or real captures.

## Releases

Pushing a version tag such as `v0.2.0` runs the release workflow. It builds the desktop packages for Windows, macOS and Linux and the signed Android APK, and publishes them as a GitHub release. See [apps/dreame-desktop/packaging/README.md](apps/dreame-desktop/packaging/README.md) for the signing secrets the workflow needs.
