# Dreame robot vacuum research

This folder holds the research for a replacement for the Dreamehome app. The goal is a generic Rust library for Dreame robot vacuums. Desktop and Android apps are built on it, and an iOS app may follow. A desktop map editor is a main goal.

The first target robot is the user's Dreame L40 Ultra AE, model id `dreame.vacuum.r2579a`.

## Documents

Read them in this order:

| Document | What it gives the implementer |
|---|---|
| [l40-ultra-ae.md](l40-ultra-ae.md) | What the real robot showed: identity, connectivity, API quirks, properties, map formats. Start here. |
| [protocols.md](protocols.md) | How to talk to the robot: cloud hosts and regions, login, request signing, commands, MQTT push, file downloads, the APK endpoint list, Mi cloud and local miIO. |
| [device-model.md](device-model.md) | What to say: every MIoT property, action and event, enums, codecs for packed and JSON values, and capability detection per model. |
| [maps.md](maps.md) | Map fetch, binary format, rendering, every edit command, and what a desktop editor can and cannot do. |
| [remote-and-camera.md](remote-and-camera.md) | Driving the robot by hand, the camera stream, the access code, and how to get obstacle and pet photos. |
| [video-streaming.md](video-streaming.md) | Live camera video: the Tencent XP2P library, the Dreame backend calls, and a go/no-go per platform. |
| [rust-library-design.md](rust-library-design.md) | Proposed crate layout, crate choices, implementation order and tests. |
| [sources.md](sources.md) | Every source, its licence, and how far to trust it. |

The first checks ran with throwaway prototypes (a LAN probe, a cloud client and a map decoder). The library crates and the `dreame` CLI replaced them, and they are not kept in this repository.

## Main findings

1. **The cloud is the only way in.** The L40 Ultra AE answers ping but has no open local port and no miIO. Every command goes through the Dreamehome cloud (`https://<region>.iot.dreame.tech:13267`). Live updates come from the cloud's MQTT broker. An account registered outside the US can live in region `us`.
2. **The cloud API is well understood and works from Rust.** Login, signing, the device list, the property cache, relayed `get_properties` and map downloads all work from Rust. It uses plain `rustls`, and the server did not block it. The username must be the account email.
3. **The device model is MIoT.** Properties are addressed as `siid.piid` (service and property ids) and actions as `siid.aiid`. tasshack's tables cover about 766 models. They map r2579a to capability set 152. Many values are packed integers or JSON inside strings, and meanings vary by firmware, so the library must keep raw values.
4. **Maps decode fully.** On r2579a the map files are not encrypted. The live map uses one pixel encoding (B) and the saved map another (E). The saved map also carries a vector floor plan (`walls_info`), carpet polygons, furniture, and room metadata with neighbours.
5. **Map editing: much is possible, some is not.** The robot's map backups turned out to be plain archives of PNG grids and JSON (maps.md §12.1), which opens a research path for deeper edits.
   - Possible: rename, retype, hide, split, merge and reorder rooms; per-room settings; no-go and no-mop zones; virtual walls; thresholds; carpets; curtains; furniture; floor-plan wall end points. All are stored on the robot. A desktop editor can add precise geometry, snapping, batch edits, previews and validation. Moving a boundary between two rooms works as merge, then split.
   - Through the archive: room cells can be moved between existing rooms by editing a fresh backup and restoring it. Rooms can also be added and removed when the room lists in the archive are edited too. The changes last through cleaning runs (maps.md §12.3).
   - Not possible: no known command or archive edit changes the occupancy grid or walls for good. You cannot paint or erase walls or remove noise. A new room can only be cut from the floor of an existing room (archive edits W29 and W30), not drawn freely.
6. **The official app is Flutter and now fully decompiled** (Blutter; see [decompile.md](decompile.md)). The strings confirmed the known endpoints and list about 150 in total; the full decompile confirms the request auth and signing (and adds a SHA-384 signing option alongside MD5). But the decompiled `libapp.so` is only the host shell. The per-robot control screens, including map viewing and editing, are React Native plugins the app downloads per model as JavaScript bundles, so the app's own map codec and edit payloads are not in it. That plugin for r2579a has since been captured from a rooted emulator and read; it confirms most of maps.md and corrects a few details (see [map-plugin.md](map-plugin.md)).

## Status and next steps (2026-11-17)

### Done

1. **The read-only follow-ups**, including an MQTT capture of a cleaning run (see [l40-ultra-ae.md](l40-ultra-ae.md)).
2. **The library core**, built in the order given in [rust-library-design.md](rust-library-design.md). The crates are `dreame-core`, `dreame-cloud`, `dreame-mqtt`, `dreame-model`, `dreame-map`, `dreame-i18n`, the `dreame` facade and the `dreame-ffi` bindings.
3. **Writes to the robot**, each approved by the user: map edits, control commands, settings, backup and restore, and the archive edits W9 to W33. [l40-ultra-ae.md](l40-ultra-ae.md) records each write up to W24, [maps.md](maps.md) §12.3 and §16.2 record W25 to W33, and §16 marks the experiments that ran.
4. **The decisions.**
   - The desktop apps use `egui` (`eframe`). `dreame-lab-editor` is a lab for trying features, and `dreame-desktop` is the end-user app.
   - Mi cloud and local miIO are out of scope for now. The r2579a has no local port, so only a second robot that needs them would bring them back.
5. **The apps.** They are the `dreame` CLI, `dreame-lab-editor`, `dreame-desktop`, the `dreame-lab-android` lab app and the `dreame-android` end-user app. A release workflow builds the desktop app and the signed Android APK. A CI workflow runs clippy and the tests on every push and pull request.

### Open

1. **Map editing in `dreame-android`.** The facade's edit flow (snapshot, apply, read back) is used by the desktop apps but not yet by the Android app.
2. **An iOS app.** `dreame-ffi` makes Swift bindings, but no iOS app uses them.
3. **Camera streaming.** The r2579a reports `CAMERA_STREAMING`. The library detects it, but there is no stream client.
4. **Untested writes.** The "Not yet tested" list in [l40-ultra-ae.md](l40-ultra-ae.md) covers the untested actions, settings and map edit commands, and the refresh token lifetime. P-frame merging, unsigned request bodies, old refresh tokens and the stop action were checked on 2026-11-17. The writes need the user's approval, one at a time.

## Notes

- Personal data stays out of git. Credentials live in `local/env`; reference clones, the APK and its decompilation live in `local/`; captures and outputs live in `local/out/`. All of `local/` is git-ignored.
- Robot access was read-only until the user approved the first write on 2026-11-16. Since then, each write needs the user's approval.
