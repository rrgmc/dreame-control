# dreame-control

Dreame Control, an open replacement for the Dreamehome app: a Rust library for Dreame robot vacuums, with apps on top. Start with [docs/research/README.md](docs/research/README.md).

## Use git worktrees

**Do all work in a git worktree, never in the main checkout.** Several Claude sessions and subagents work on this repository at the same time. A shared checkout lets them overwrite each other's files and commit each other's half-done work.

- Before the first edit, call `EnterWorktree` (or `git worktree add -b <branch> <dir>`). Name the branch after the task.
- When you delegate code changes to subagents, start each one with `isolation: "worktree"`. Read-only agents (research, review) may share a checkout.
- Leave the main checkout on `master`, clean. Merge a worktree's branch into `master` only when the user asks.
- Untracked files that the work needs (`local/env`, `local/refs/`, `local/out/`) live only in the main checkout. Read them from there with an absolute path. Do not copy them into a worktree, and never commit them.

## Layout

| Path | Content |
|---|---|
| `crates/dreame-core` | Shared types: property ids, raw values, device info, errors, app constants |
| `crates/dreame-i18n` | Translations: the current language (en-US, pt-BR) and catalogs keyed by the English text |
| `crates/dreame-cloud` | Dreamehome cloud REST client |
| `crates/dreame-mqtt` | Live updates from the cloud MQTT broker |
| `crates/dreame-model` | Property table, value codecs, enums, capability discovery, typed status, command builders |
| `crates/dreame-map` | Map codec, merge, editable model, renderer, edit commands |
| `crates/dreame` | Facade for apps: login session, cloud robot access, map edit flow (snapshot, apply, read back) |
| `crates/dreame-ffi` | UniFFI bindings for Kotlin and Swift |
| `apps/dreame-cli` | `dreame` command-line tool |
| `apps/dreame-desktop` | Dreame Control for desktop, the end-user app (egui); run with `task app` |
| `apps/dreame-android` | Dreame Control for Android, the end-user app (Compose) on `dreame-ffi`'s `app` API; build with `task android` or `task android:release` |
| `tools/dreame-lab-editor` | Lab map editor (egui) for trying features before they reach the apps |
| `tools/dreame-lab-android` | Android lab app on `dreame-ffi`; build with `task lab:android` |
| `docs/research/` | Protocol, device model and map research |
| `local/` | Git-ignored: credentials (`local/env`), captures and outputs (`local/out/`), reference clones, APK analysis |

## Rules

- Robot access is read-only unless the user approves a specific write. Writes include `set_properties`, actions and map edits.
- Credentials come from `local/env` (`DREAME_USER_EMAIL`, `DREAME_PASSWORD`, `DREAME_REGION`, `DREAME_IP`). Never print or commit them, or the device id, account id, serial, MAC, IP or room names.
- Live tests run only with `DREAME_LIVE=1`. Tests on real captures run only when `DREAME_FIXTURES` points at `local/out/`.
- Do not cite or copy tasshack's paid card code (see docs/research/sources.md).
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` (`task check`) must pass before each commit. The CI workflow runs the same checks on every push to `master` and every pull request.
