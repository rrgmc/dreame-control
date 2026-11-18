# Remote control and camera pictures

This document describes how the official app drives the robot by hand and how it gets pictures from the robot camera. It comes from reading the r2579a React Native plugin (see [map-plugin.md](map-plugin.md)).

**Verified on r2579a (2026-11-17):** the live map (6.3) listed 9 `ai_obstacle` entries, all with upload status 2. `getOss1dDownloadUrl` with `(file + "-" + key)` minus its first character, then AES-128-ECB with key MD5(pwd), gave 9 valid JPEGs (95 to 131 KB). No write was needed. `dreame obstacles --save <dir>` does this. Reading 4.15 returns code -1 (write-only).

**Remote control on r2579a (2026-11-17), robot docked and idle:** one step `{"spdv":0,"spdw":120,"audio":"true",...}`, then one second later a zero step. The cloud returned code 0 for both. Right after the stop, 4.1 read remote control; 10 s later it read standby, so the robot leaves the mode by itself within 10 s. The robot drove off the dock, then went quiet and stopped. It did not turn. So from the dock, the first step makes the robot leave the dock and the step itself is not carried out. It stays off the dock in standby afterwards. An app should warn that driving a docked robot undocks it, and should wait for the undock before sending turns.

Off the dock, two steps of `spdw` 120 one second apart, then a stop, turned the robot left. The user estimated a quarter turn in the first test and a half turn in the second. At 120 deg/s for 2 s it would turn 240 degrees, so a step runs for roughly 0.4 to 0.75 s, or `spdw` is not deg/s. Positive `spdw` turns left, as the plugin's `remote_left` button (B:259967, B:260006). State and work mode read remote control after the stop and idle/standby 10 s later. During remote control the robot pushes `P` map frames on 6.1 with its pose; in the second test they came 2 and 5 s after the stop, with heading 295 and 297.

## Summary

- **Remote control works over the cloud with one property.** Write siid 4 piid 15 with a small JSON value about once a second while the user holds a direction, and write zero speeds on release. It needs no video, session or access code.
- **The official "take photo" is not a robot feature.** The app grabs a frame from the live Tencent video on the phone and uploads it. We cannot do that without reimplementing Tencent's peer-to-peer video.
- **The robot's own pictures are obstacle and pet photos.** The map lists them. The cloud serves them encrypted, and the key is in the map entry. This is the practical way to get pictures.
- **An unused robot-side photo command exists** (siid 10001 action 1, piid 5 `takephoto`). Whether this robot supports it is unknown, and testing it is a write.

## Sources and shorthand

- **B:N** = line N of `local/apk/rn_plugins/rn_plugins/dreame.vacuum.r2579a/a/dreame.vacuum.r2579a/index.android.bundle`. The bundle is already pretty-printed (one statement per line), so plain line reads work. Module names are on the closing `},"projects_..."` line of each module.
- **SDK:N** = `local/apk/rn_plugins/rn_plugins/sdk/a/rn_sdk_plugin/sdk.bundle` (the shared miot-sdk shim).
- **T/device:N**, **T/types:N** = `local/refs/dreame-vacuum/custom_components/dreame_vacuum/dreame/{device,types}.py`.
- Status marks: **confirmed** (read in code), **likely** (strong inference), **unknown**.

Relevant modules in the bundle:

| Module | Lines (end) | Role |
|---|---|---|
| `main_models_Monitor` | B:173862-174207 | siid 10001 model: all camera actions |
| `src_widget_RemoteView` | B:259797-260092 | joystick on the "Remote control" settings page |
| `src_page_RemoteControlPage_index` | B:259535-259796 | settings page hosting RemoteView |
| `src_page_RemoteTestPage_index` | B:287843-288012 | hidden scripted test page ("v,w,duration/...") |
| `src_widget_VideoRemoteView` | B:373036-373373 | joystick overlaid on the live-video page |
| `src_page_video_LaunchPage_index`, `StartUpPage_index`, `main_util_VendorCheckUtil` | B:355001-356678 | video entry: access code, vendor check |
| `src_page_video_BaseMainPage` | B:368660-371618 | shared live-video page (keep-alive, photo, record) |
| `src_page_video_TxMainPage_index` | B:374995-375371 | Tencent ("tx") player variant |
| `main_util_PhotoUploadUtil` | B:373716-373858 | phone-side photo encrypt + upload |
| `main_util_ObsDownloadManager`, `PhotoCacheUtil`, `VacuumFileUtil` | B:267980-268203, 233666-233715, 228174-228240 | obstacle/pet photo fetch |

---

## Remote control

### a. Property, payload, ranges, rate, stop

**Property: set_properties siid 4 piid 15 (`remoteState`), value = a JSON string.** Confirmed.
- `PropRemoteState: 'remoteState'` (B:5594); siid-4 model (`_this.SIID = 4`, B:171532) maps `PIID[PropRemoteState] = 15` (B:171558), default `''` (B:171613).
- Request built by the generic model helper `createSetPropReq` → `{did, siid, piid, value}` (B:170281-170294) and sent with `updateProps` (cloud set_properties).
- Matches HA: `REMOTE_CONTROL: {siid: 4, piid: 15}` (T/types:1616), `remote_control_move_step` (T/device:5942-5968).

**Payload** (VideoRemoteView, B:373242-373272; RemoteView, B:259961-259991), confirmed:

```json
{"spdv": <int>, "spdw": <int>, "audio": "true"|"false", "random": <0..999>, "timestamp": <ms epoch>}
```

`audio` is a **string**, not a boolean. `timestamp` is new compared with HA (HA omits it; the RemoteTestPage at B:287880-287923 also omits it, so it is optional, likely).

**Values the app sends** (confirmed):

| Button (touchId) | Video joystick (B:373245-373259) | Settings joystick (B:259964-259978) |
|---|---|---|
| 1 forward | spdv 200, spdw 0 | spdv 300, spdw 0 |
| 2 left | spdv 0, spdw +45 | spdv 0, spdw +120 |
| 3 right | spdv 0, spdw -45 | spdv 0, spdw -120 |
| 4 back / turn around | spdv 0, spdw 180, sent **once** | same |
| 0 release | spdv 0, spdw 0 | same |

- There is no reverse: the down arrow is a single "turn around" command (`TYPE.TRUN_AROUND` burying point, B:259857; `BACKWARD`, B:373135). Confirmed.
- No combined drive-and-turn from the official UI; only one axis is non-zero. The RemoteTestPage accepts arbitrary `v,w,duration` triples (B:287895-287932, placeholder B:287976), so the robot accepts free values. Confirmed that the app sends them; ranges the firmware accepts are **unknown**.
- Units: spdv is **likely mm/s** (200-300 is a normal robot-vacuum speed). spdw is **likely deg/s**, positive = counter-clockwise/left. Evidence: a single `spdw: 180` produces a "turn around", and the release handler for touchId 4 does not send a stop (B:259882-259887, B:373169-373174), which fits "each command runs about one second, then the robot stops by itself".

**Send rate** (confirmed): first command immediately on touch, then repeat with `setInterval`:
- video joystick: every **1000 ms** (B:373223-373233);
- settings joystick: every **1300 ms** (B:259942-259952);
- RemoteTestPage splits long moves into chunks of at most **1200 ms** per command (`maxDuration = 1200`, B:287899-287916).

This strongly suggests a firmware watchdog of roughly 1.2-1.5 s per command (likely; exact value unknown).

**Stop** (confirmed): on release, clear the interval and send one `{"spdv":0,"spdw":0,...}` (`stopRemote`, B:373234-373241; B:259953-259960). The RemoteTestPage also ends with an explicit zero command (B:287878-287893). There is **no "end remote control" action** anywhere in the plugin; the app never sends anything else to leave the mode (confirmed by absence: the only writers of `PropRemoteState` are the three components above, B:259991, 287886, 287924, 373272).

**`audio`** (confirmed mechanics, likely meaning): the component starts with `this.audio = 'true'` (B:260023, B:373305) and sets it to `'false'` after the first send (B:259990, B:373271). So only the first command after the joystick view is created carries `"true"`. HA does the same: `"true"` unless already in remote control or sleeping (T/device:5955-5963). Meaning: **likely** "play the entering-remote-control voice prompt".

**`random`** (confirmed value, likely purpose): `Math.floor(Math.random() * 1000)` (B:259982, B:373263). Likely there so that two identical consecutive commands are still different property values (the cloud/robot may drop a set that equals the cached value). HA uses `randrange(65535)` (T/device:5964), so any small int works (likely).

**Transport** (confirmed): `isLocalNet` is initialised `false` and never set `true` (only assignments: B:259998, 260024, 373280, 373306), so the official app always goes through the cloud `updateProps` path. The local `callMethodFromLocal('set_properties', ...)` branch is dead code. Good news for us: the cloud relay is the production path.

### b. Preconditions, entering/leaving, timeouts, status

**App-side blocks** (confirmed). The joystick refuses the touch and shows a toast when:
- Settings joystick (B:259839): washing mop or wash paused, fast mapping, base self-test, water self-check, water draining, dust collecting, battery low, station cleaning; and, only on models with `supportLDSUp`, a running clean task while the lidar is down. Messages: B:259733-259752.
- Video joystick (B:373078-373120): overflow alert, base self-test, water self-check, water draining, station cleaning, washing mop or wash paused, person-follow task.
- Battery low = `batteryLevel < minOperableBattery` (B:169076-169081), `minOperableBattery: 15` (B:1311).

**Not blocked by the app** (confirmed by absence): being docked/charging, being idle, or (on non-LDS-up models) a running clean. The robot itself decides what to do; when the user taps during a task the settings page offers "stop task" (`showStopDialog` → `VacuumExtend.stopClean`, B:259599-259612). The robot does leave the dock on the first command (no dock check exists in the app; **verified on r2579a**, 2026-11-17, see the top of this document).

**Entering the mode** (confirmed): there is no separate "start remote control" action. The first `remoteState` write switches the robot into remote control. The local-net branch even fakes it by setting local `workMode = RemoteCtrlMode` before sending (B:373276).

**Leaving the mode**: no app command (confirmed). The robot drops out on its own after commands stop. **Verified on r2579a** (2026-11-17): it read standby within 10 s of the zero step. HA tracks the same: it clears its `_remote_control` flag once status is no longer REMOTE_CONTROL (T/device:1421-1426). Ways to force it out: start any other task, `stopClean`, or go-charge (the video page offers "go charging" when battery < 15 % in remote mode, B:370564-370566).

**Status values** (confirmed):
- siid 4 piid 1 (`PropWorkMode`, B:171547) = **13** `RemoteCtrlMode` (B:5298). `isRemoteCtrlCleaning()` tests exactly this (B:169106-169110). Task text "taskRemote" shown when workMode 13 (B:369773-369774).
- siid 2 piid 1 device status = **23** `kRemoteClean` (B:5396); an older status table uses 19 (B:5441). HA: `DreameVacuumStatus.REMOTE_CONTROL = 23` (T/types:404), state 13 (T/types:631).
- Our docs already list these: docs/research/device-model.md:518 (status 23) and :572 (state 13).

**Where the UI lives** (confirmed / unknown):
- Settings → Auxiliary functions → Remote control (`remoteControl` entry, B:252324-252329) is hidden unless `Const.showRemoteControl` (B:251179); the default is `false` (B:806). Model flags come from a per-model resource pack loaded at start-up (`loadResourcePack(...)` then `extends(Const, config)`, B:2029-2033), which we have **not** captured (`raw/..._res_config.json` is `{}`). So whether the L40 Ultra AE shows the settings joystick is **unknown**.
- The live-video page always renders a 124×124 `VideoRemoteView` (B:371047-371081), so on this camera robot remote control is reachable from the video screen (confirmed for the code; visibility before the stream joins is likely).

### c. Is the video stream required?

**No** (confirmed at protocol level). `sendRemoteCtrl` carries no `session` and does not touch siid 10001 (B:373242-373284). The settings `RemoteView` page has no video at all and drives blind (B:259535-259796). The video page only places the same joystick over the picture. The RemoteControlPage runs a `localPing` on focus, but a failure only hides the loader (B:259695-259709); `showMessage` is never rendered, and the send path is cloud anyway.

---

## Photos

### d. Sequence to take a photo

**The official app does not ask the robot for a photo.** Confirmed.

1. The page method `takePhoto()` (BaseMainPage, B:370846-370882) checks cloud storage (`checkDevOssStorage`, refuses below 0.3 MB free, B:370858-370866), then calls `this.snapShot()`.
2. `snapShot()` in TxMainPage (B:375224-375241) calls `TxPlayerEngine.snapShot()`: a **frame grab from the decoded Tencent video on the phone**, returning a local file path. Ali (B:368476-368494) and Agora (`_engine.takeSnapshot(...)`, B:375667-375672) variants do the same with their SDKs.
3. The only callers of `takePhoto` are the page buttons (B:371223, B:371232).

`Monitor.takePhoto` exists (action siid 10001 aiid 1 `CAMERA_OPERATE`, in-piid 5 `TAKE_PHOTO`, value `{"operType":"takephoto","operation":"start","session":<s>}`, B:173994-174000) but **is never called** (no `Monitor.takePhoto(` anywhere; the `.takePhoto(` hits are the page method). Whether the r2579a firmware implements it, and where its output would go, is **unknown**. HA lists `TAKE_PHOTO` 10001.5 (T/types:1950) but has no caller either (T/device:437 is only a property list).

So a photo requires a live Tencent session. The session sequence (all confirmed, all through `Monitor.sendAction`, B:173947-173970, which wraps every value as a JSON string and always merges `session`):

| Step | Call | Wire form |
|---|---|---|
| 0 | `updateSession()` (B:173976-173979, called at LaunchPage, B:355101, and B:166971) | `session = MD5(accountId + "_" + nowMs)` hex |
| 1 | privacy flag | cloud `batchGetDeviceDatas` `prop.s_video_config` → `{"privacyAuthed":true}` (B:356227-356267) |
| 2 | access-code state | action 10001 aiid 4, piid 1100, value `{"open":true,"session":s}` (B:174091-174096) → `out[0].value` 0/1/2/3, see g |
| 3 | vendor | `get_properties` 10001.11 with value `{"session":s}` (B:174172-174175, B:174177-174195) → `{"initStatus":1,"vendor":"tx"}`; if not initialised: action 10001 aiid 7 piid 11 (B:174167-174170) |
| 4 | Tencent identity | host `TxPlayerEngine.getTxRtcInfo(did)` → `{deviceId, productId, deviceName}` (B:375140-375155) |
| 5 | P2P info | host `TxPlayerEngine.getP2PInfo(did)` (B:375190), which the Flutter host serves from `POST /dreame-third-video/tx/dev/getP2PInfo` (blutter `tx_api_client.dart:94`) |
| 6 | start stream | action 10001 aiid 1, piid 1, value `{"token":"tx","channelId":<deviceId>,"operType":"monitor","operation":"start","session":s}` (B:375192-375196, B:174031-174037) |
| 7 | play | `TxPlayerEngine.startPlay(p2pInfo)` after 1 s (B:375203-375206); 30 s join timeout (B:375200-375202) |
| 8 | keep-alive | every 10 s (TxMainPage sets `useAliSDK = true`, B:375042, so the 10 000 ms branch applies, B:369027): action 10001 aiid 1 piid 6 `{"operType":"keep_alive","videoStatus":"opened"|"preparing","session":s}` (B:173986-173992, B:369002-369027). Reply `out[0].value` should be `"ok"`; three misses → "camera fault" |
| 9 | stop | action 10001 aiid 1 piid 1 `{"operType":"monitor","operation":"end","session":s}` (B:174039-174045, used at B:355368-355371) |

The robot reports stream state in 10001.1 (`{operType, operation, session, result, status}`), which the page watches (B:370535-370553); docs/research/device-model.md:470 already has a captured value.

### e. Where the image goes

Confirmed: the image never comes from the robot. After the phone grabs a frame:

- **Default (`isMachineStorageResources` false)**: `PhotoUploadUtil.upload(path)` (B:373735-373826):
  1. `secretKey = MD5(path + "_" + accountId + "_" + nowMs)[0..16]`.
  2. Encrypt the JPEG with AES-CBC-PKCS7, key = UTF-8 of `MD5(secretKey)` hex (32 bytes → AES-256), IV = UTF-8 `"aebf8c52e26a4767"`.
  3. `POST /dreame-user-iot/iotoss/addOssNew` `{category:0, did, fileSize, filename:"<ms>.jpg", model, source:2, type:"jpg", uid:<owner>, key:secretKey}` → `{ossId, url, expiresTime}`.
  4. `PUT url` with the ciphertext.
  5. `POST /dreame-user-iot/iotoss/ossUploaded` with the same fields plus `ossId`.
- **Firmware with `isMachineStorageResources`** (version-gated, B:1865-1866; threshold in the unknown resource pack): saved only on the phone (`AlbumNewManage.savePhotoData`, B:227642-227666).

The album later lists them with `POST /dreame-user-iot/iotoss/userDidOssList?current=&size=` body `{did, type:"jpg"|"gif"|"thumb"}` (SDK:37480-37497, used at B:359099-359138). Each record has `filepath` (a URL) and `key`; decrypt with AES-CBC, IV `aebf8c52e26a4767`, key derived from `key` (B:359126-359133). Endpoints `checkDevOssStorage` and `deleteDidOss`: SDK:37499-37528.

Not used for live photos: `getOss1dDownloadUrl` and `/dreame-third-video` (the latter only carries Tencent pairing/P2P/identity endpoints: `tx/dev/pair`, `tx/dev/getP2PInfo`, `tx/user/accesstoken`, `tx/mgr/dev/getIdentity`, `tx/dev/isDevUser`, `tx/dev/setShare`, `tx/mgr/family/getFamilyId` in `local/apk/libapp.strings`).

### f. Stills the robot produces without live video

**Obstacle and pet photos (AI obstacle avoidance).** Confirmed; this is the practical source of robot pictures.

- Each map `ai_obstacle` entry is an array `[x, y, type, confidence, key, filename, pwd, ...]` (`initPhoto`, B:268103-268135). Type `158` is a pet (pet-snap flag, B:268158-268160).
- If the photo is not yet in the cloud, the app asks the robot to upload it: action **siid 4 aiid 6** (`getPhotoInfo`, B:171542, B:171669-171685), `in: [{piid: 10, value: JSON{"name": filename, "pwd": pwd, "object": filename + "-" + key}}]` (B:161897-161916, also B:233690-233697). HA calls this `GET_PHOTO_INFO {siid 4, aiid 6}` (T/types:1981). This is a write-class action (approval needed).
- Download: `POST /dreame-user-iot/iotfile/getOss1dDownloadUrl` `{filename: (filename + "-" + key).slice(1), model, region?}` (`region:"ru"` only for RU, B:268007-268014) → `data` is a URL; GET it (B:268026-268029).
- Decrypt: AES-**ECB**-PKCS7, key = the 16 raw bytes of `MD5(pwd)` (`strTo16Array` turns the hex into 4 words, B:268049-268054, B:268178-268192). Result is a JPEG.
- Retry every 3 s up to 5 times on failure (B:268063-268074). Cached on the phone for 24 h keyed by `pwd` (B:233699-233703).
- Already documented for the cloud side in docs/research/maps.md:189.

**Robot-local storage variant** (`isMachineStorageResources`): obstacle, pet and cruise photos are pulled from the robot through `POST /file-bridge/user/getDeiviceFile` with `fileinfo = JSON{"filename", "type": "obstacle"|"pet"|"cruise"|"list"}` (B:228192-228233). The reply `byte` is AES-CBC with key UTF-8 of `MD5(decryptAESKey)` hex and IV `aebf8c52e26a4767`. `decryptAESKey` comes from an RSA exchange: action siid 14 aiid 1 in-piid 5 with the phone's RSA public key; the robot returns the AES key RSA-encrypted (B:162802-162824, B:178167-178210). Whether r2579a uses this path is **unknown** (version gate in the missing resource pack).

**Cruise/patrol and pet "gif" photos** in the album (`type:"gif"`) are robot-produced, listed through `userDidOssList` (B:359087-359118). How and when the robot uploads them (during cruise tasks / pet detection) is **likely** robot-initiated; not traced further.

**Server-side obstacle library**: `POST https://aiphotos.dreame.tech//getPhotosInApp` `{ver, model, pageSize, pageIndex}` (B:292367-292376), the "obstacle picture management" page. Probably generic sample images, not this robot's photos (likely).

### g. Security step before the camera

Confirmed: an optional **video access code** (PIN) gate, all on action siid 10001 aiid 4 (`ACCESS_CODE_OPERATE`):

| Call | in-piid | value (plus `session`) | Lines |
|---|---|---|---|
| query on launch | 1100 | `{"open":true}` | B:174091-174096, B:355046-355077 |
| query | 1100 | `{}` | B:174086-174089 |
| set code | 1101 | `{"newcode": SHA256(code)}` | B:174098-174106 |
| change code | 1101 | `{"oldcode": SHA256(old), "newcode": SHA256(new)}` | B:174108-174120 |
| verify and start | 1102 | `{"oldcode": SHA256(code), "lazymode": 0|1}` | B:174122-174131, B:356339-356403 |
| verify | 1102 | `{"oldcode": SHA256(code)}` | B:174133-174141 |
| reset | 1103 | `{}` | B:174143-174146 |

Launch result `out[0].value` (B:355052-355074, B:356108-356117): `2` = no code needed or already remembered → go straight to vendor check and video; `0` with a `FEATURE` field = no code set yet → "set a code" page; `3` = locked out; else (1) = ask for the code. Verify result `JSON(out[0].value).code`: 2 = ok, 1 = wrong (`5 - fail` tries left), 3 = locked, 0 = no code set (B:356363-356399). `lazymode` = "remember code" checkbox.

Plus the one-time privacy consent stored in cloud device data `prop.s_video_config = {"privacyAuthed":true}` (B:356227-356267); this is app-side only (likely the robot does not check it).

Remote control has **no** security step (confirmed: no code, no session).

---

## Feasibility verdicts

**Remote control over the cloud: feasible now.** Everything needed is one cloud `set_properties` of 4.15 with the JSON string above, repeated every ~1 s while held, and a zero command on release. The official app itself uses only this cloud path. No Tencent SDK, no session, no access code. The open questions are robot behaviour, not protocol. Dock exit and the idle timeout were settled on 2026-11-17 (see the top of this document); the accepted speed ranges are still open.

**Photos over the cloud without the Tencent SDK: not feasible the way the app does it.** The official "take photo" is a client-side frame grab from the Tencent P2P stream; the robot never produces a still for it. Options:
1. **Obstacle and pet photos (feasible now)**: parse `ai_obstacle` from the map, call action 4.6 if needed (write, approval), then `getOss1dDownloadUrl` + AES-ECB(MD5(pwd)). No video involved.
2. **Robot-side `takephoto` (10001 aiid 1 piid 5)**: present in the plugin model but unused; may or may not work on r2579a and may need an active monitor session. Needs a write probe.
3. **Full live video**: does not need a reimplementation. Tencent publishes the XP2P client as binaries for Windows, Linux x86_64 and Android, and the app can link it. See [video-streaming.md](video-streaming.md).

---

## Read-only probes to settle the unknowns

All are reads (cloud cache or relayed `get_properties`); none move the robot.

1. Read 4.15 (cloud cache and relayed) to see whether the robot echoes the last remote command (docs say cache-absent). **Done 2026-11-17**: the relayed read returns code -1 (write-only).
2. Read 2.1, 4.1 and 4.7 now (expect not 23/13), to have a baseline for when the user drives once from the official app.
3. Read 10001.1, 10001.2, 10001.4, 10001.5, 10001.6, 10001.7, 10001.1003, 10001.2003, 10001.103 (cache and relayed). 10001.5 and 10001.1003 show whether the robot ever reported a robot-side photo / upload status.
4. Relayed `get_properties` 10001.11 with value `{"session":"<any hex>"}`: vendor and init status (already known `tx`, confirms the value-carrying read works).
5. Relayed `get_properties` 10001.99 with value `{"type":"checkAlive","session":...}` (the app's `getProp('checkAlive')`, B:174017-174022, B:370540): shows whether a stream is alive; harmless.
6. Cloud `POST /dreame-user-iot/iotoss/checkDevOssStorage {did}` and `userDidOssList {did, type:"jpg"|"gif"}`: shows whether the robot or app has put any photos in OSS.
7. For an existing `ai_obstacle` entry in the current map: `getOss1dDownloadUrl` with `(filename + "-" + key).slice(1)` and decrypt; succeeds without any write if the robot already uploaded it (upload status field `data[11]`, B:268152-268154). **Done 2026-11-17**: 9 valid JPEGs, no write needed.
8. Read `device info`/firmware version and compare against later-captured resource-pack thresholds (`isMachineStorageResourcesVersion`, `showRemoteControl`) once the per-model resource pack is captured from the phone.

Writes that need the user's approval, in order of usefulness:
- ~~one `4.15 = {"spdv":0,"spdw":0,"audio":"false","random":n}` (no motion; tests acceptance and whether status flips to 23/13 and how long it stays), then a short `spdw: 45` burst;~~ **Done 2026-11-17** with `spdw` 120: the write is accepted, the robot enters remote control, leaves the dock, and drops out within 10 s (see the top of this document);
- action 4.6 for one obstacle photo;
- action 10001 aiid 4 piid 1100 `{"open":true,"session":s}` (access-code state; read-like but it is an action);
- action 10001 aiid 1 piid 5 `takephoto` with and without a running `monitor start`, then watch 10001.5/10001.1003 and `userDidOssList`.
