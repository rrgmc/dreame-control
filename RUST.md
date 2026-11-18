# The Rust library

Dreame Control is built on a Rust library for Dreame robot vacuums. The desktop app, the Android app and the `dreame` command-line tool all use it. Other programs can use it too.

- Library design: [docs/research/rust-library-design.md](docs/research/rust-library-design.md)
- Protocol, device model and map research: [docs/research/README.md](docs/research/README.md)
- Building the apps: [BUILDING.md](BUILDING.md)

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

Most programs need only `dreame`, the facade. The crates are not published on crates.io yet, so depend on them through git:

```toml
[dependencies]
dreame = { git = "https://github.com/rrgmc/dreame-control" }
```

## Apps and tools

| Program | Path | Content | Run or build |
|---|---|---|---|
| `dreame-cli` | `apps/dreame-cli` | The `dreame` command-line tool | `task dreame -- <args>` |
| `dreame-desktop` | `apps/dreame-desktop` | Dreame Control for desktop (`egui`) | `task app` |
| `dreame-android` | `apps/dreame-android` | Dreame Control for Android (Compose) | `task android` |
| `dreame-lab-editor` | `tools/dreame-lab-editor` | Lab map editor | `task lab:editor` |
| `dreame-lab-android` | `tools/dreame-lab-android` | Android lab app | `task lab:android` |

Lab tools try features against a real robot before they reach the apps.

## The `dreame` tool

Reading commands change nothing on the robot. Commands that write (`control`, `set`, and the `map` commands that edit, back up or restore) print what they would send. They send it only with `--apply`.

Put your account in `local/env`. Git ignores the `local/` folder.

```
DREAME_USER_EMAIL=you@example.com
DREAME_PASSWORD=...
DREAME_REGION=us
```

`DREAME_REGION` is optional. Without it, the tool tries every region. The login saves a refresh token in `local/session.json`, so later commands do not need the password.

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

The [CI workflow](.github/workflows/ci.yml) runs clippy (warnings fail) and the tests on every push to `master` and every pull request. It also builds the Android app.

Two kinds of tests are off by default:

- Live tests against the real cloud run only with `DREAME_LIVE=1`.
- Tests on real captures run only when `DREAME_FIXTURES` points at a folder of captures (`local/out/`).

## Tasks

`Taskfile.yml` wraps the common commands for [go-task](https://taskfile.dev). Run `task --list` to see them.

- `task check` runs clippy and the tests. Every commit must pass it.
- `task test:fixtures` and `task test:live` set the variables above.
- `task dreame -- <args>` runs the tool with any arguments.
