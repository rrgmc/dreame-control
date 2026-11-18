# Rust library design

This document proposes the structure of the Rust library. Its inputs are [protocols.md](protocols.md), [device-model.md](device-model.md), [maps.md](maps.md) and the verified results in [l40-ultra-ae.md](l40-ultra-ae.md). It is a starting point for the implementing agent, not a frozen specification.

## Goals

1. Control every feature the Dreamehome app offers for Dreame robot vacuums. The first target is the user's L40 Ultra AE (`dreame.vacuum.r2579a`).
2. Work with other Dreame-style robots, and with the MOVA and TROUVER brands that share the same cloud. Model differences live in data, not in code paths.
3. Run inside Android and iOS apps and a desktop app. A desktop map editor is a main use case.
4. Make map editing better than the official app, within what the robot accepts.

Out of scope: devices other than robot vacuums. The app also supports other devices (pet feeders, washers and more), but the library ignores them.

## Main constraints from the research

- **Cloud only, for now.** The L40 Ultra AE has no open local port (verified). Every command goes through the Dreamehome cloud, and live updates arrive through the cloud's MQTT broker. Local miIO, the Mi cloud and Valetudo are out of scope for now (see Decisions).
- **App constants can change.** Client credentials, the password salt, the signing secret and header values come from the app and may change with an app update. They must be configuration with defaults, not scattered literals.
- **The server is lenient today.** A plain `rustls` client with signed bodies works (verified). tasshack copies the Android TLS fingerprint, which suggests some accounts saw blocking. Keep the HTTP client behind a trait so a fingerprint-copying client can replace it later.
- **Values are messy.** Properties mix integers, booleans, strings, JSON inside strings, packed bit fields and base64 names. Meanings and enum labels change with firmware. The library must keep raw values and treat labels as hints.
- **Maps are large and incremental.** The robot sends full I-frames and partial P-frames. The library must merge them and keep state.

## Crate layout

One Cargo workspace. The crate names below are suggestions.

```
dreame-control/
  crates/
    dreame-core/        # shared types: ids, values, errors, config
    dreame-cloud/       # Dreamehome cloud: auth, REST, signing, OSS download
    dreame-mqtt/        # cloud MQTT push client
    dreame-miio/        # not built: local miIO transport (UDP 54321) and Mi cloud
    dreame-model/       # MIoT profile, codecs, capability detection, typed API
    dreame-map/         # map codec, merge, geometry, render, edit commands
    dreame-i18n/        # translations of app text (en-US, pt-BR)
    dreame/             # facade: Robot, Session, event stream
    dreame-ffi/         # UniFFI bindings for Kotlin and Swift
  apps/
    dreame-cli/         # CLI for research and testing (replaces the prototype tools)
    dreame-desktop/     # desktop app with the map editor
    dreame-android/     # Android end-user app on dreame-ffi
  tools/
    dreame-lab-editor/  # desktop lab for trying features
    dreame-lab-android/ # Android lab app on dreame-ffi
```

Dependencies, from each crate's `Cargo.toml`:

- `dreame-core` and `dreame-i18n` use no other crate of the workspace.
- `dreame-cloud` uses `core`. `dreame-mqtt` and `dreame-model` use `core` and `cloud`.
- `dreame-map` uses `core` and `i18n`. It does not depend on any transport, so the desktop editor can open saved files offline.
- The `dreame` facade uses all of the crates above.
- `dreame-ffi`, `dreame-cli`, `dreame-desktop` and `dreame-lab-editor` use the facade and some lower crates directly. The Android apps use `dreame-ffi`.

### dreame-core

- `PropId { siid: u32, piid: u32 }`, `ActionId { siid, aiid }`, `EventId { siid, eiid }`.
- `RawValue`: `Int(i64) | Bool(bool) | Str(String) | Json(serde_json::Value)`. The cloud cache returns strings and the relay returns typed values, so normalise both into this type.
- `DeviceInfo { did, model, firmware, name, bind_domain, mac, online, local_ip }`.
- `Error` enum: `Auth`, `Region`, `Http`, `Api { code, msg }`, `NoAck` (cloud code `80001`), `Decode`, `Unsupported`.
- `AppConstants` struct with defaults: Basic credentials, password salt, signing secret, `dreame-meta` version, user agent, tenant id. Load overrides from configuration.

### dreame-cloud

- `Region` enum (`Us`, `Eu`, `Cn`, `Ru`, `Sg`, `Kr`, `By`, plus `Custom(host)`) and brand (`Dreame`, `Mova`, `Trouver`) giving the host. When the user's region is unknown, try regions in order and use the `region` field of the token response.
- `Auth`: password login (salted MD5), refresh, expiry tracking. Persist only the refresh token. A trait `TokenStore` lets apps keep it in the Android Keystore or iOS Keychain.
- `Signer`: the `spliced` + MD5 body signature. It has a unit test against the worked example in protocols.md. The prototype client passed it before the library existed.
- `CloudClient` methods:
  - `list_devices`, `device_info`, `otc_info`
  - `get_properties(did, &[PropId])` (relayed), `set_properties`, `action(did, ActionId, in)`
  - `cached_properties(did, &[PropId])` (cloud cache; keys sent as one comma-separated string)
  - `property_history`, `get_user_data`, `set_user_data`
  - `download_url(object)`, `permanent_download_url(object)`, `fetch(url)`
- Retry policy: retry once on `success` without `result`. Treat `80001` as "unknown outcome" and confirm through MQTT or a later read. Back off on HTTP 429.
- HTTP behind a small trait (`HttpTransport`), with `reqwest` + `rustls` as the default.

### dreame-mqtt

- Its own small MQTT 3.1.1 client (`packet.rs`) over `tokio-rustls`, with no MQTT crate. The TLS verifier in `tls.rs` pins the Dreame private root CA from tasshack (see protocols.md §11.1a).
- Username is the `uid`; the password is the access token. Reconnect with the new token after each refresh.
- Subscribes to `/status/<did>/<uid>/<model>/<region>/` and decodes `properties_changed`, `event_occured`, `props`, `_otc.info` and map-list updates into a typed `Push` enum.
- Exposes an async `Stream<Item = Push>`.

### dreame-miio (not built)

Out of scope for now (see Decisions). This sketch stays for a later robot that needs it.


- The miIO UDP packet codec (header, AES-128-CBC with `md5(token)`, checksum) and the Mi cloud login with RC4 signing.
- Implements the same `Transport` trait as the cloud, so `dreame-model` does not care which one is used.

### dreame-model

- **Profile**: a data table of every known property and action, with name, ids, access and codec. Ship it as a Rust `const` table generated from a TOML file. Allow per-model overrides keyed by model id.
- **Capability discovery**: on connect, read the whole profile in batches (15 per relayed call, or the cloud cache, which accepts at least 60 keys). Record which properties answer. Combine this with an optional model table for features that presence cannot show, gated by firmware build number. tasshack's `DEVICE_INFO` table is the data source.
- **Codecs**: one module per composite value. See device-model.md for the layouts. The list:
  - cleaning mode group (4.23), settings store (4.50), AI obstacle flags (4.22)
  - DND, off-peak charging, schedules, shortcuts
  - per-room clean settings, custom-clean payloads, station operations (`"op,flag"`)
  - fault list, stream status
- **Open enums**: `enum State { Known(StateKind), Unknown(i64) }`, and the same pattern for status, errors, suction and water levels.
- **Typed API**: `Robot::start_clean`, `clean_rooms(&[RoomId], settings)`, `clean_zones`, `pause`, `dock`, `set_suction`, consumables, DND, schedules, station actions, and so on. Each method checks capabilities first and returns `Error::Unsupported` when the model lacks the feature.
- **Raw escape hatch**: `Robot::raw_get`, `raw_set` and `raw_action` for anything the profile does not cover yet.

### dreame-map

This crate matters most for the user. It must work without a network.

- `envelope`: URL-safe base64, optional AES-256-CBC (key from SHA-256 of the key string, IV per model), zlib. Include an encode path for tests.
- `header`: the 27-byte header.
- `grid`: pixel decoders A to E plus Wi-Fi and clean-map modes. Choose by map version and JSON flags. On r2579a, live frames use B and saved maps use E (verified).
- `trailer`: serde structs for known keys (`seg_inf`, `vw`, `vws`, `walls_info`, `carpet_polygon`, `funiture_info`, `cleanareaorder`, `delsr`, and so on), each with an `extra` catch-all map so unknown firmware keys survive a round trip.
- `merge`: P-frame merge, frame queueing, and live-plus-saved composition.
- `model`: a clean, editable in-memory map:
  - `Map { rooms, walls (floor plan), carpets, furniture, restrictions (no-go, no-mop, virtual walls, thresholds), dock, robot, path }`
  - in millimetre world coordinates, with transforms to and from pixels
- `render`: a raster renderer to an RGBA buffer (for mobile and thumbnails) and a vector export (SVG) for the desktop editor.
- `edit`: typed builders for every map edit command in maps.md (merge, split, rename, retype, order, per-room settings, zones, walls, carpets, furniture, map select, rename and delete). Each builder returns the action 6.2 piid-4 JSON. A diff function turns "edited `Map` versus original `Map`" into the command list. The editor can then let the user edit freely and send only valid commands.
- `validate`: checks before sending, such as the room count limit, rectangles inside the map, and split lines that cross exactly one room.

### dreame (facade)

- `Session::login(config) -> Session`, `Session::robots() -> Vec<Robot>`.
- `Robot` combines the cloud client, MQTT stream, profile, capabilities and map state. It exposes a single `events()` stream: property changes, map updates and errors.
- Async on `tokio`. Thin blocking wrappers for the CLI.

Implemented as `dreame::Robot` (`crates/dreame/src/robot.rs`):

- `Robot::cloud(client, device)`, generic over a `Link` (cache read, live read, send) so tests use fakes. Its futures are `Send`.
- `discover()` runs capability discovery; afterwards values decode with the robot's context and command methods return `Error::Unsupported` for missing features (do-not-disturb, wetness, station, auto-empty).
- `refresh()` reads the cloud cache, then the core services live; `status()` returns the kept snapshot.
- `events()` returns `Events`, whose `next()` yields `Event::Status` (changed ids and the updated snapshot), `Event::MapFrame` (decoded 6.1 frames, kept out of the status), `Event::RobotEvent` and `Event::Connection`.
- Command methods (`start`, `dock`, `clean_rooms`, `set_suction`, `set_mop_mode`, `set_dnd`, `reset_consumable`, `station`, …) all go through `send()`, which fails when the robot's result code is not 0.
- The map edit flow keeps its own trait, `mapedit::MapAccess`, implemented by `CloudRobot`.
- `Robot::set(&Setting)` writes one typed setting (`crates/dreame/src/settings.rs`); `settings::known(&status)` lists every setting whose value the status holds, and `settings::all_properties()` names what to read live first.
- `Robot::schedules()` reads the schedules and `Robot::set_schedules()` writes them.
- Not yet: shortcuts; `Session::robots()`.

### dreame-ffi

- UniFFI (`uniffi` crate) generates Kotlin and Swift bindings. Export the facade, typed enums, and the map model as plain records.
- Callbacks for the event stream (UniFFI callback interfaces), or polling, depending on what the apps prefer.
- Build Android libraries with `cargo-ndk` and iOS with an XCFramework. Keep `rustls` with the `ring` provider, which builds on both platforms.

Implemented as `crates/dreame-ffi` (UniFFI 0.32, proc macros, Tokio for async):

- `login(email, password, region, session_file)` and `resume(session_file)` return an `Account`; the session lives in a file the app chooses (its private folder).
- `Account.devices()`, `Account.robot(did)`; `RobotHandle.refresh()`, `start`, `pause`, `stop`, `dock`, `map_png(scale)`, `settings()`, `set_setting(value)`, `start_events(listener)` and `stop_events()`. `SettingValue` is one enum for every setting (times as `TimeOfDay`, carpet sensitivity as `CarpetLevel`); `settings()` returns `SettingEntry` records (a name and a `SettingValue`) read live, and `set_setting` writes, then reads the value back about 3 s later. Async methods become Kotlin `suspend` functions; `RobotListener` is a Kotlin callback interface.
- One error type, `DreameError` (`Auth`, `Network`, `Unsupported`, `Other`, each with a `detail` text). A field named `message` clashes with Kotlin's `Exception.message`.
- `task lab:android` cross-compiles for arm64 phones and the x86_64 emulator (`cargo ndk`, API 26), generates the Kotlin with the crate's own `uniffi-bindgen` binary, and builds `tools/dreame-lab-android`, a one-screen lab app (AGP 8.13, Kotlin 2.2, JNA 5.17). The `ring` and `rustls` builds needed no changes. Each library is about 8 MB unstripped.

### Desktop app

- Uses the same crates, with `egui` (`eframe`) for the window, the map view and the editor (`apps/dreame-desktop`).
- The editor works on the `Map` model and the `edit` diff. No command edits the grid directly. Room cells can still be moved through a backup and restore of the map archive (maps.md §12.2): the editor's paint brush sends a `GridEdit::Paint` and its Move boundary tool a `GridEdit::Border` through `dreame::archiveedit::apply` (maps.md §12.3).

## Suggested crates

| Need | Crate |
|---|---|
| Async runtime | `tokio` |
| HTTP | `reqwest` (features `rustls-tls`, `json`, `gzip`) |
| MQTT | own client in `dreame-mqtt` over `tokio-rustls` |
| TLS | `rustls`, `rustls-pemfile` |
| JSON | `serde`, `serde_json` (feature `preserve_order`, so trailers round-trip in order) |
| Hashing and crypto | `md-5`, `sha2`, `aes`, `cbc`, `ecb`, `rc4` (Mi cloud) |
| Compression | `flate2` |
| Base64 | `base64` |
| Images | `image` (PNG), `tiny-skia` (raster drawing) |
| FFI | `uniffi` |
| Errors | `thiserror` in libraries, `anyhow` in apps |
| Logging | `tracing` |

## Implementation order

Each step ends with tests and a commit. Steps 1 to 4 need only read access to the robot.

1. **Core and cloud read path.** Port the prototype client: constants, login, refresh, signing (with the unit test), device list, OTC info, cached properties, relayed `get_properties`, download URL. CLI commands: `login`, `devices`, `props`.
2. **Map codec.** Envelope, header, grids A-E, trailer structs, PNG render. Add the user's frames from `local/out/map/` as private test fixtures; do not commit them, because they contain the home layout. Also add node-dreame's public test vectors. CLI: `map`, which downloads, decodes and renders a map.
3. **MQTT push.** Connect, subscribe, decode. CLI: `watch`. Check P-frame merging with real pushes while the robot cleans.
4. **Device profile and capabilities.** Load the profile, discover capabilities on r2579a, decode every composite value, and compare with the app's screens.
5. **Writes, one at a time, with the user's approval.** Start with harmless ones: rename a room, then change and restore a setting. Then actions (start, pause, dock), then map edits. Before each edit, back up the map (action 6.3) and record the before and after frames.
6. **Map editing model and diff.** Editable `Map`, the command builders, validation, SVG export.
7. **FFI and a minimal Android app.** UniFFI bindings, token storage, a map view.
8. **Desktop editor.**

## Testing

- Unit tests for every codec, with values seen on r2579a (see l40-ultra-ae.md and device-model.md).
- Golden tests for map decoding: input envelope text, then the expected header, room histogram and PNG hash.
- A recorded-HTTP test mode: a fake `HttpTransport` replays captured JSON. Remove identifiers from recordings before committing them.
- Live tests run only with `DREAME_LIVE=1`. They read the credentials from environment variables, which `local/env` holds. They never write to the robot.

## Decisions

- **GUI toolkit**: the desktop apps use `egui` (`eframe`).
- **Mi cloud and local miIO: out of scope for now** (decided 2026-11-17). A second LAN probe on 2026-11-17 got the same result as the first (l40-ultra-ae.md): no miIO reply and no open TCP port. The r2579a is also probably not bindable to Mi Home, so no miIO token can be obtained for it. `dreame-miio` comes back only if a second robot needs it.
- **Map restore path**: tried, with the user's approval. The archive edits W9 to W33 are recorded in maps.md §12.3 and §16.2; l40-ultra-ae.md has the full record up to W24.
