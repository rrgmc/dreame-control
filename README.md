# Dreame Control

[![CI](https://github.com/rrgmc/dreame-control/actions/workflows/ci.yml/badge.svg)](https://github.com/rrgmc/dreame-control/actions/workflows/ci.yml)

Dreame Control is an open replacement for the Dreamehome app. It controls Dreame robot vacuums from your computer (Windows, macOS, Linux) or your Android phone. The apps are in English and Brazilian Portuguese.

The apps talk to the robot through the Dreamehome cloud. You sign in with your own Dreamehome account, the same one the official app uses.

Dreame Control is also a Rust library that other programs can use. See [RUST.md](RUST.md).

> [!WARNING]
> This project is not affiliated with or endorsed by Dreame. Use it at your own risk: commands and map edits change the robot's state.

## Supported robots

The only tested robot is the Dreame L40 Ultra AE (`dreame.vacuum.r2579a`). Other Dreame vacuums use the same cloud and the same device model, so they may work in part.

## What the apps do

| Feature | Desktop | Android |
|---|:---:|:---:|
| Live map with the robot, its dock, the rooms and the cleaning path | Yes | Yes |
| Clean the whole home, chosen rooms or a zone drawn on the map | Yes | Yes |
| Start, pause, resume and stop; send the robot to its dock | Yes | Yes |
| Set suction, water and cleaning mode | Yes | Yes |
| Room settings: suction, water, passes, mode, cleaning order, hidden rooms | Yes | Yes |
| Schedules for chosen rooms | Yes | Yes |
| Robot settings, do-not-disturb hours, station actions | Yes | Yes |
| Maintenance: remaining life of brushes, filters and pads | Yes | Yes |
| Cleaning history with the map and path of each job | Yes | Yes |
| Notifications when a job ends or the robot needs help | Yes | Yes |
| Map editing: rename, divide and merge rooms; no-go zones, no-mop zones, virtual walls | Yes | No |
| Drive the robot by hand | Yes | No |
| Photos of the obstacles and pets the robot recognised | Yes | No |

## Install

Download the newest version from the [releases page](https://github.com/rrgmc/dreame-control/releases).

| System | File | How to install |
|---|---|---|
| Windows (x64) | `DreameControl-<version>-windows-x64-setup.exe` | Run it. It installs for your user only and needs no administrator rights. |
| Windows (x64) | `DreameControl-<version>-windows-x64.zip` | Unpack it and run `Dreame Control.exe`. No installation. |
| macOS (Apple silicon and Intel) | `DreameControl-<version>-macos-universal.dmg` | Open it and drag Dreame Control to Applications. |
| Linux (x86_64) | `DreameControl-<version>-linux-x86_64.AppImage` | Make it executable (`chmod +x`), then run it. |
| Linux (x86_64) | `DreameControl-<version>-linux-x86_64.tar.gz` | Unpack it. It holds the program, its icon and a desktop file. |
| Android 8.0 or newer | `dreame-control-<version>.apk` | Open it on the phone and allow the installation. |

### The first start

The packages are not signed with a paid certificate, so each system warns you once:

- **Windows:** SmartScreen says "Windows protected your PC". Choose **More info**, then **Run anyway**.
- **macOS:** right-click the app and choose **Open**. Or open it once, then allow it under **System Settings > Privacy & Security > Open Anyway**.
- **Android:** the phone asks you to allow installations from the app that opened the APK, such as your browser or file manager.

## Use

1. Open the app and sign in with your Dreamehome email and password. Pick your account's region, or leave it on Automatic on the desktop.
2. Pick your robot, if the account has more than one.
3. The app opens on the map of your home. Choose what to clean and press the big button to start.

The apps never store your password. They keep a sign-in token, so you stay signed in after a restart. The desktop app keeps its files in your user's app folder: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS and `~/.config` on Linux.

On Android, the app keeps following a running job in the background. It shows the job in a notification and tells you when the job ends or the robot needs help.

## More

- [BUILDING.md](BUILDING.md): build the apps from source.
- [RUST.md](RUST.md): the Rust library, the `dreame` command-line tool and the tests.
- [docs/research/README.md](docs/research/README.md): how the Dreame cloud, the device model and the map format work.

## Licence

MIT. See [LICENSE](LICENSE).

## Author

Rangel Reale (realerangel@gmail.com)
