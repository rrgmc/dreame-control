# Dreame Control

[![CI](https://github.com/rrgmc/dreame-control/actions/workflows/ci.yml/badge.svg)](https://github.com/rrgmc/dreame-control/actions/workflows/ci.yml)

An open replacement for the Dreamehome app: a Rust library for Dreame robot vacuums, with a desktop app (Windows, macOS, Linux), an Android app and a command-line tool on top. The apps are in English and Brazilian Portuguese.

It talks to the robot through the Dreamehome cloud, with your own Dreamehome account. It can show the live map and status, start and stop cleaning, change settings and schedules, and edit the map (rooms, zones, walls, carpets, furniture).

The first and only tested robot is the Dreame L40 Ultra AE (`dreame.vacuum.r2579a`). Other Dreame vacuums use the same cloud and property model and may work in part.

This project is not affiliated with or endorsed by Dreame. Use it at your own risk: commands and map edits change the robot's state.

- Downloads: [releases](https://github.com/rrgmc/dreame-control/releases) (desktop installers and the Android APK)
- Research: [docs/research/README.md](docs/research/README.md)
- Library design: [docs/research/rust-library-design.md](docs/research/rust-library-design.md)
- Licence: MIT, see [LICENSE](LICENSE)

## Crates

| Crate | Content |
|---|---|
| `dreame-core` | Shared types: property ids, raw values, device info, errors, app constants |
| `dreame-i18n` | Translations for the text apps show: language switch and catalogs keyed by the English text (en-US, pt-BR) |
| `dreame-cloud` | Async Dreamehome cloud client: login, token refresh, devices, property reads, commands, file downloads |
| `dreame-mqtt` | Live updates from the cloud MQTT broker, with a pinned-root certificate check |
| `dreame-model` | Property table, value codecs, open enums, capability discovery, typed status, command builders |
| `dreame-map` | Map codec, P-frame merge, editable map model, PNG renderer, map edit commands, recovery archive edits |
| `dreame` | Facade for apps: login session, cloud robot access, map edit flow (snapshot, apply, read back) |
| `dreame-ffi` | UniFFI bindings for Kotlin and Swift |

## Apps

| App | Content | Run or build |
|---|---|---|
| `dreame-cli` | The `dreame` command-line tool | `task dreame -- <args>` |
| `dreame-desktop` | Dreame Control for desktop, the end-user app (`egui`) | `task app`; packages: see [apps/dreame-desktop/packaging/README.md](apps/dreame-desktop/packaging/README.md) |
| `dreame-android` | Dreame Control for Android, the end-user app (Compose) | `task android` or `task android:release` |

The apps are in `apps/`. Both end-user apps are in English and Brazilian Portuguese.

Pushing a version tag runs the release workflow. It builds the desktop packages for Windows, macOS and Linux and the signed Android APK, and attaches them to the GitHub release.

## Lab tools

Lab tools try features against a real robot before they reach the apps. They are in `tools/`.

| Tool | Content | Run or build |
|---|---|---|
| `dreame-lab-editor` | Lab map editor | `task lab:editor` |
| `dreame-lab-android` | Android lab app | `task lab:android` |

## The `dreame` tool

Reading commands change nothing on the robot. Commands that write (`control`, `set`, and the `map` commands that edit, back up or restore) print what they would send and send it only with `--apply`.

Put your account in `local/env` (git-ignored):

```
DREAME_USER_EMAIL=you@example.com
DREAME_PASSWORD=...
DREAME_REGION=us
```

`DREAME_REGION` is optional; without it the tool tries every region. The login saves a refresh token in `local/session.json`, so later commands do not need the password.

```bash
cargo run -p dreame-cli -- login
```

```bash
cargo run -p dreame-cli -- devices
```

```bash
cargo run -p dreame-cli -- props 2.1 3.1
```

```bash
cargo run -p dreame-cli -- status
```

```bash
cargo run -p dreame-cli -- watch --seconds 600
```

```bash
cargo run -p dreame-cli -- map --which saved --out saved.png --floor-plan --names
```

`props --cached` reads the cloud cache instead of asking the robot. `map --which current` renders the last frame the robot uploaded.

## Tests

```bash
cargo test --workspace
```

The [CI workflow](.github/workflows/ci.yml) runs clippy (warnings fail) and the tests on every push to `master` and every pull request, and builds the Android app.

Live tests against the real cloud run only with `DREAME_LIVE=1`. Tests on real captures run only when `DREAME_FIXTURES` points at a folder of captures (`local/out/`).

## Tasks

`Taskfile.yml` wraps the common commands for [go-task](https://taskfile.dev). Run `task --list` to see them. `task check` runs clippy and the tests. `task test:fixtures` and `task test:live` set the variables above. `task dreame -- <args>` runs the tool with any arguments.
