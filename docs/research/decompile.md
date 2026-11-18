# Dreamehome app decompilation: what libapp.so does and does not contain

Status: research reference, 2026-11-15.
Audience: the agent that implements the transport and protocol layers, and anyone extending the
APK analysis.
Scope: a full decompile of the Flutter application code (`libapp.so`) of Dreamehome 2.6.7.0, what
it holds, and the one structural finding that matters for this project: the vacuum map code is not
in it.

## 1. Summary

An earlier pass recovered only the `strings` of `libapp.so` (see `local/apk/libapp.strings`,
cited as `apk:<line>` in the other documents). That was thought to be the limit, because the app
is Flutter and its Dart code is compiled ahead of time into a stripped snapshot.

That limit is gone. [Blutter](https://github.com/worawit/blutter) compiles a Dart runtime that
matches the snapshot and reconstructs the Dart classes, methods and object pool. The full
reconstructed source is now in `local/apk/blutter/out/` (git-ignored, like the rest of
`local/apk/`).

Two things came out of reading it:

1. **The request authentication layer is confirmed and slightly extended.** Headers, the `Dreame-RLC`
   cipher and the request signing all match [protocols.md](protocols.md) §2 and §3. The one new
   detail is that the signature hash is SHA-384 *or* MD5, chosen at runtime, not MD5 only (§5.3).
2. **The vacuum map code is not in `libapp.so` at all.** The map codec, the renderer and every map
   edit command live in a React Native plugin that the app downloads per robot model. That plugin is
   JavaScript, so Blutter cannot decompile it, and none of it is in the Dart binary. See §4. This
   corrects the earlier note that the app's map rendering and editing are native Flutter code.

## 2. How it was produced (reproducible)

The build is self-contained in Docker, so it needs no toolchain on the host beyond Docker.

- Dart version, detected from `libflutter.so`: **3.9.2**. Snapshot hash
  `97ff04a728735e6b6b098bdf983faaba`. Flags: `product arm64 android compressed-pointers`.
- Image: built from `local/apk/blutter/Dockerfile` (Ubuntu 24.04 with Blutter's documented
  dependencies; it clones Blutter inside the image). Tagged `blutter-dreame`.
- The Dart runtime that Blutter compiles for this version is cached in a Docker volume
  (`blutter_dreame_work`), so a second run is quick.
- Input: `libapp.so` and `libflutter.so` from `native/lib/arm64-v8a/`, staged in
  `local/apk/blutter/in/`.

Run:

```bash
docker run --name blutter_run \
  -v "$PWD/local/apk/blutter/in":/in \
  -v "$PWD/local/apk/blutter/out":/out \
  -v blutter_dreame_work:/opt/blutter \
  blutter-dreame python3 -u /opt/blutter/blutter.py /in /out
```

One function (`FunctionAnalyzer::handleOptionalNamedParameters`) raised a non-fatal analysis error
and was skipped. Everything else was produced.

## 3. What the output contains

Under `local/apk/blutter/out/`:

| File or dir | Content |
|---|---|
| `asm/<package>/.../<file>.dart` | Reconstructed Dart, 190 packages, about 3000 files. Each file has a `// lib:` header, class and field declarations, and method bodies shown as annotated arm64 disassembly. Every string, object and called method is named in a trailing comment, for example `; [pp+0x..] "sign"` or `; bl #0x.. [package:crypto] Hash::convert`. You read the comments, not the raw assembly. |
| `pp.txt` | Every constant in the object pool, 5.3 MB. Grep this for string literals. |
| `objs.txt` | Nested dump of the same objects. |
| `blutter_frida.js` | A Frida hook template for this build. |
| `ida_script/` | Scripts to import the symbols into IDA. |

### Citation convention

Cite a decompiled fact as `app:<package>/<path>:<line>`, for example
`app:dreame_flutter_base_network/src/interceptor/params_sign_interceptor.dart:462`. The line
numbers are specific to this build, so when a line may drift, name the string or symbol as well
(`"EETjszu*XI5znHsI"`, `NetworkEncryptUtil::rlcHeader`); those are the stable anchor. This keeps
`apk:<line>` for the older strings dump and `app:` for the decompile.

## 4. The structural finding: libapp.so is the host shell, not the device UI

The decompiled Dart is the host application: sign-in, the home and room list, the device list, the
MQTT relay, networking, and the download and launch of device plugins. The per-robot control
screens, including the vacuum map, are **React Native plugins** downloaded at runtime as JavaScript
bundles. They are not in the APK and not in `libapp.so`.

Evidence:

- **No map code in Dart.** `pp.txt` and `libapp.strings` have zero hits for every map token from
  [maps.md](maps.md): `UPDATE_MAP_DATA`, `customeClean`, `wallInfoV3`, `carpetcleanset`, `nrism`,
  `virtual_wall`, `walls_info`, `decode_map`, `robotMap`. No `CustomPainter` or `Canvas` grid
  renderer, no zlib map decode, no MIoT map action literal.
- **A plugin subsystem that downloads bundles.** The app package is `package:flutter_plugin`
  (1062 files under `asm/flutter_plugin/`). It has
  `asm/flutter_plugin/ui/page/home/plugin/plugin_provider.dart` and siblings, with strings
  `common_plugin_mall`, `index.android.bundle`, `sdk.bundle`, `SDK mismatch`. Device UIs are
  versioned bundles fetched from the cloud.
- **A React Native bridge.** `asm/dreame_flutter_rn_bridge/flutter_rn_bridge.dart`
  (`_initReactChannel`, `reactNativeCallback`).
- **React Native engines ship in the APK:** `libreactnativejni.so`, `libjscexecutor.so`,
  `libv8android.so`, `libv8executor.so`, `libreactnativeblob.so`, plus a QuickJS runtime
  (`libfastdev_quickjs_runtime.so`), under `local/apk/native/lib/arm64-v8a/`.
- **Only a reduced map summary reaches Dart.**
  `asm/flutter_plugin/model/home/vacuum_room_sync.dart` syncs a geometry-free view of the robot's
  rooms into home automation: `mapId`, `mapName`, `mapList`, `roomId`, `roomName`, `roomList`,
  `sortOrder`, `did`. No pixels, walls or zones.

The plugin talks to the robot through generic host bridges, not a map-specific API:
`asm/flutter_plugin/common/bridge/request_module.dart` (generic HTTP `request` and `upload`) and
`message_channel.dart` (MQTT relay: `mqttMsgArrived`, `topic`, `payload`). So the MIoT
`siid.piid`/`aiid` encoding for a map edit is built inside the plugin's JavaScript and sent over
these bridges. The Dart layer carries it without understanding it.

**Consequence for map research.** Decompiling `libapp.so` does not yield the app's map wire format.
To read the official app's own codec, renderer and edit payloads you need the downloaded React
Native bundle for the vacuum (the `common_plugin_mall` / `index.android.bundle` the app fetches per
model), not this decompile. That bundle for r2579a has since been captured from a rooted emulator and
read; the resulting wire-format reference is [map-plugin.md](map-plugin.md). It is stored at
`local/apk/rn_plugins/.../dreame.vacuum.r2579a/.../index.android.bundle` (git-ignored). [maps.md](maps.md)
stays primarily sourced from tasshack, Valetudo and node-dreame, with the plugin as the authoritative
cross-check.

## 5. What the decompile does confirm: request authentication

All of this is in `package:dreame_flutter_base_network`. It confirms
[protocols.md](protocols.md) §2 and §3, which were reconstructed from other clients.

### 5.1 Headers

Set by `src/interceptor/header_interceptor.dart`, `token_interceptor.dart` and
`app_info_interceptor.dart`. The header name constants in the code are title-case; HTTP header
names are case-insensitive, so this does not change the wire.

| Header | Source in code | Meaning |
|---|---|---|
| `Dreame-Auth` | token_interceptor, header_interceptor | Access token, refreshed with `grant_type=refresh_token`. |
| `Dreame-RLC` | header_interceptor -> `NetworkEncryptUtil::rlcHeader` | Encrypted anti-tamper value, see §5.2. |
| `Tenant-Id` | header_interceptor | Tenant id, `000000` for Dreame. |
| `dreame-meta` | app_info_interceptor | App and client metadata. |

### 5.2 The Dreame-RLC header

`NetworkEncryptUtil::rlcHeader` (`src/utils/network_encrypt_util.dart`) uses `package:encrypt`:

- Key: `"EETjszu*XI5znHsI"` (16 bytes, so AES-128).
- Mode: ECB. The `AESMode` constant it loads is `AESMode@12b4031`, which `pp.txt` maps to `"ECB"`.
- Output: Base64.
- Plaintext: fields of a server-provided `RlcHeaderConfig`, joined with `"|"`. This matches
  protocols.md's `"<region>|<lang>|<country>"`.

So `Dreame-RLC = base64(AES-128-ECB(key "EETjszu*XI5znHsI", "<field>|<field>|..."))`. Confirms
protocols.md §2.

### 5.3 Request signing

`ParamsSignInterceptor` (`src/interceptor/params_sign_interceptor.dart`):

1. `splicingParams` builds a query string from the request parameters: `key=value` joined with `&`.
   Array values are bracketed as `key=[...]&`. (Sorted, per protocols.md §3.)
2. It appends the salt `"EETjszu*XI5znHsI"` (the same string as the AES key).
3. It UTF-8 encodes the result and hashes it with `package:crypto` (`Hash::convert` then
   `Digest::toString`, lowercase hex).
4. The hex digest goes in the `sign` field; the current time in milliseconds goes in `timestamp`.

**New detail.** The hash is chosen at runtime by a boolean: one branch loads `_Sha384`, the other
loads `_MD5`. protocols.md §3 documents MD5 only, which was verified to work on r2579a. SHA-384 is
a second, code-confirmed option. If an MD5 signature is ever rejected, try SHA-384 of the same
base string before assuming the scheme changed.

The salt `"EETjszu*XI5znHsI"` is the `cid` in protocols.md §2's secret table, used both as the
signing salt and as the `Dreame-RLC` AES key.

## 6. Where to read more

- Request auth: `asm/dreame_flutter_base_network/src/interceptor/` and `src/utils/`.
- Plugin download and launch: `asm/flutter_plugin/ui/page/home/plugin/`.
- React Native bridge: `asm/dreame_flutter_rn_bridge/`, and
  `asm/flutter_plugin/common/bridge/` (`request_module.dart`, `message_channel.dart`,
  `plugin_sdk_module.dart`).
- MQTT: `asm/dreame_flutter_base_mqtt/`.
- Everything else: grep `pp.txt` for a literal, then follow the class to its file under `asm/`.
