# Live video streaming

Status: feasibility research, 2026-11-17. No robot or cloud call was made for it.

This document asks whether our apps can show the live camera of the L40 Ultra AE (`dreame.vacuum.r2579a`). It extends [remote-and-camera.md](remote-and-camera.md), which covers the robot-side session (siid 10001) and found that the stream is Tencent peer-to-peer video.

## 1. Summary

- **Live video is feasible on Windows and Android without reimplementing any protocol.** Tencent publishes its XP2P client library as binaries for Android, iOS, Linux x86_64 and Windows. We link that library, as the official app does, and it hands us a plain HTTP-FLV stream on localhost.
- **Everything else comes from the Dreame cloud and the robot.** The Dreame backend gives the Tencent device identity and the `p2pInfo` string. The robot needs a camera session, the access-code check, `monitor start` and a keep-alive. All of this is already documented in [remote-and-camera.md](remote-and-camera.md) or below.
- **A Home Assistant add-on already does this for Dreame robots** on Linux x86_64 (§4.4). It is the best evidence that the plan works.
- **The costs are licensing and native packaging, not protocol work.** The library is closed source under a "Tencent Binary License" whose text we could not read. We should fetch it at build time and not commit it. Linux on ARM and macOS have no build.
- **Testing needs approved robot writes.** `monitor start`, the access-code actions and the keep-alive are actions on siid 10001. Possibly also `dev/pair` on the Dreame cloud, if this account is not yet bound.

## 2. Sources and shorthand

- **B:N**: line N of the r2579a React Native bundle (see [remote-and-camera.md](remote-and-camera.md)).
- **TxApi**: `local/apk/blutter/out/asm/dreame_flutter_plugin_tx_video/src/api/tx_api_client.dart`, the Blutter output for the app's Flutter plugin `dreame_flutter_plugin_tx_video` (see [decompile.md](decompile.md)).
- **TxSdk**: `.../dreame_flutter_plugin_tx_video/src/tx_video_sdk.dart` in the same tree.
- **so:NAME**: `strings` of `local/apk/native/lib/arm64-v8a/NAME`.
- The Java side of the APK is packed. `base.apk` has one 106 KB `classes.dex`, and a full jadx run yields 32 classes, none of them video code. So the Java glue between Flutter and the native libraries cannot be read; the native libraries and the Dart plugin are the evidence.

## 3. What the APK contains

### 3.1 Native libraries

| Library | Size | What it is | Evidence |
|---|---|---|---|
| `libxnet-android.so` | 51 KB | JNI wrapper for `com.tencent.xnet.XP2P` | 26 `Java_com_tencent_xnet_XP2P_*` exports (so:libxnet-android.so) |
| `libiot_video_demo.so` | 4.2 MB | The XP2P core: `AppXp2pClient`, `Xp2pRequest`, STUN, TLS, curl | C++ symbols, "Tencent Cloud, XP2P Team" (so:libiot_video_demo.so) |
| `libtxffmpeg.so`, `libtxsoundtouch.so` | 2.6 MB, 0.2 MB | Tencent player decode and audio | names |
| `libdreame_iot_video_native.so` | 8 KB | Dreame's own `FLVPacker` for a WebRTC path | `Java_com_dreameiotvideo_core_webrtc_flv_FLVPacker_*` |
| `libagora-*.so` | many | Agora RTC, the other video vendor | names; B:375667 |

The robot reports vendor `tx` (remote-and-camera.md §d step 3), so the Tencent path is the one that matters for r2579a.

### 3.2 The XP2P client API

The JNI methods are: `setQcloudApiCred`, `setDeviceXp2pInfo`, `setStunServerToXp2p`, `setCrossStunTurn`, `startServiceNative`, `startLanServiceNative`, `delegateHttpFlv`, `getLanUrl`, `getLanProxyPort`, `startAvRecvService`, `stopAvRecvService`, `postCommandRequestSync`, `runSendService`, `dataSend`, `stopSendService`, `stopServiceNative`, `setCallbackNative`, `recordstream`, `recordstreamPath`, `getStreamLinkMode`, `getStreamBufSize`, `getAvgMaxMin`, `resetAvg`, `setContentDetail`, `setLogEnable`, `getVersion`.

Strings in the core show how it works (all so:libiot_video_demo.so):

- It runs a local HTTP proxy. Video is `http://127.0.0.1:<port>/app.xnet/ipc.p2p.com/ipc.flv?...`, so the player reads plain HTTP-FLV. Commands go to `.../command?`, talk-back audio to `.../voice?`, and a keep-alive to `.../keepalive?`.
- A LAN mode exists (`/ipc.p2p.com/ipc.flv?_protocol=tcp&`, `startLanServiceNative`, `getLanUrl`).
- STUN servers: `stun.iotcloud.tencentiotcloud.com` and `stun.iotcloud.tencentdevices.com:20002`.
- It talks to `https://iotexplorer.tencentcloudapi.com` (the Tencent IoT Explorer API), which is what `setQcloudApiCred` is for, and logs to `log.qvb.qcloud.com`.
- It checks the robot's XP2P version: "The xp2p_device_sdk is low, Please upgrade the device version to at least %s".

### 3.3 The Dreame backend calls

The Flutter plugin wraps five Dreame cloud endpoints under `/dreame-third-video/tx/` (TxApi). All are `POST` on the normal Dreame cloud host with the normal signed headers.

| Endpoint | Body | Reply `data` | Kind |
|---|---|---|---|
| `user/accesstoken` | none seen | `{token, userId, expireAt}` (`TxAuthModel`) | read |
| `mgr/dev/getIdentity` | `{did, os}` | `{deviceId, deviceName, productId, secretId, secretKey}` (`TxRtcInfoModel`) | read |
| `dev/isDevUser` | `{accesstoken, did}` | `{isDevUser}` | read |
| `mgr/family/getFamilyId` | `{accesstoken}` | `{familyId}` | read, may create a family (unknown) |
| `dev/pair` | `{accesstoken, familyId, did, bindType}` | `{status}` | **write**: binds the robot to the user's Tencent account |
| `dev/getP2PInfo` | `{accesstoken, did}` | `{p2pInfo}` (a string) | read |
| `dev/setShare` | `{accesstoken, userId, did, uid}` | | **write**: sharing |

Replies are wrapped in `{code, data, message, requestId, error}` (`TxDataModel`).

`getIdentity` returns Tencent API credentials (`secretId`, `secretKey`) next to the Tencent product id and device name. These feed `setQcloudApiCred` and the device identity. Whether they are temporary is unknown.

The plugin also initialises a Tencent licence: a `trtcube-license.cn` URL and a licence key are hard-coded in TxSdk. That licence is for Tencent's player SDK (TRTC "cube"), not for XP2P itself (likely). We would not use it.

### 3.4 Full sequence, as far as it is known

1. Robot side (remote-and-camera.md §d): new session id, access-code check (action 10001.4 piid 1100), vendor check (10001.11).
2. `user/accesstoken`, then `isDevUser`; if not bound, `getFamilyId` and `pair` (write).
3. `getIdentity` for the Tencent product id, device name and API credentials.
4. `getP2PInfo` for the device's current `p2pInfo` string.
5. Robot action 10001 aiid 1 piid 1 `monitor start` with `channelId = deviceId` (write).
6. XP2P: `setQcloudApiCred`, `setDeviceXp2pInfo(p2pInfo)`, `startServiceNative`, then `delegateHttpFlv` gives the local FLV URL.
7. Play the FLV stream. Every 10 s send the robot keep-alive (10001.1 piid 6) and stop with `monitor end`.

## 4. The public XP2P library

Web research, 2026-11-17. **Confirmed** means the source was read; **likely** means inferred.

### 4.1 Availability and licence

- Android: Maven `com.tencent.iot.thirdparty.android:xp2p-sdk` (2.4.54 on Maven Central; Tencent's own `video-link-android` pins 2.4.72). The AAR holds `libxnet-android.so` for arm64-v8a and armeabi-v7a plus a C header `appWrapper.h`. The POM licence is "Tencent Binary License". Confirmed: [POM](https://repo1.maven.org/maven2/com/tencent/iot/thirdparty/android/xp2p-sdk/2.4.54/xp2p-sdk-2.4.54.pom), [build.gradle](https://github.com/tencentyun/iot-link-android/blob/master/sdk/video-link-android/build.gradle).
- Desktop: [tencentyun/iot-p2p-build release v2.4.72](https://github.com/tencentyun/iot-p2p-build/releases/tag/v2.4.72) (2026-08-24) ships `xp2p_linux.zip` (28 MB) and `xp2p_windows.zip` (134 MB). Confirmed through the GitHub API. The Linux build is static x86_64 only. The Windows zip likely has 32- and 64-bit builds.
- iOS: CocoaPods `TIoTLinkKit_XP2P`, header in [iot-thirdparty-ios](https://github.com/tencentyun/iot-thirdparty-ios/blob/master/Source/XP2P-iOS/Classes/AppWrapper.h). Confirmed.
- No macOS build was found. No source is public: the build CI clones a private repository.

The licence text itself was not found (the linked URL returns 404). Until it is read, treat the binaries as not redistributable: download them in the build, do not commit them.

### 4.2 C API

`AppWrapper.h` declares (confirmed): `setUserCallbackToXp2p`, `startService` (older name `startServiceWithXp2pInfo(id, product_id, device_name, xp2p_info)`), `setDeviceXp2pInfo`, `delegateHttpFlv`, `startAvRecvService`, `postCommandRequestSync`, `runSendService` / `dataSend`, `startLanService` / `getLanUrl`, `setQcloudApiCred`, `stopService`. On Windows the functions are `__declspec` exports, so a plain DLL import works from Rust.

Use, from Tencent's [iOS guide](https://github.com/tencentyun/iot-link-ios/blob/master/Source/SDK/LinkVideo/doc/iOS%20Video接入指引文档.md) and [device–app guide](https://github.com/tencentyun/iot-link-android/blob/master/sdk/video-link-android/doc/设备与APP交互指引.md) (confirmed):

1. `startService(id, product_id, device_name)` with `setDeviceXp2pInfo(id, xp2p_info)`.
2. Wait for callback type 1004 (`XP2PTypeDetectReady`).
3. `delegateHttpFlv(id)` returns a local base URL. Append `ipc.flv?action=live&channel=0&quality=standard|high|super`. Any FLV reader (ffmpeg, ijkplayer) plays it.
4. Or `startAvRecvService(id, "action=live", crypto)` delivers the raw FLV bytes to a callback. This avoids an HTTP client but gives the same container.
5. `postCommandRequestSync` sends signalling such as `action=inner_define&channel=0&cmd=get_device_st&type=live`. `runSendService` + `dataSend` is talk-back.

The video codec is H.264 or H.265 inside FLV (likely; Tencent's device SDK mentions H.265). The audio codec is unknown.

### 4.3 Credentials

With `xp2p_info` supplied by the caller, no Tencent `SecretId`/`SecretKey` is needed (confirmed): `setQcloudApiCred` is only for letting the library fetch `xp2p_info` itself, and Tencent advises against putting those keys in apps. So we can ignore the `secretId`/`secretKey` that `getIdentity` returns and use only `productId` and `deviceName`.

### 4.4 Prior art: dreame-vacuum-unlocked

[Web3Dave/dreame-vacuum-unlocked](https://github.com/Web3Dave/dreame-vacuum-unlocked) is a Home Assistant add-on that streams Dreame robot cameras. Confirmed from its README:

- It links Tencent's Linux x86_64 static libraries with a small C wrapper, not a reimplementation.
- It gets `product_id` and `device_name` from Dreame's `getIdentity`, and a fresh `xp2p_info` from the Dreame backend.
- Before each stream it opens a camera session, verifies the 4-digit PIN, and starts the encoder (our §3.4 step 1 and 5).
- It sends the robot keep-alive (10001 aiid 1 piid 6, with the session) every 20 s. Without it the picture freezes after about 60 to 73 s.
- It republishes the FLV feed as RTSP through MediaMTX.
- No licence is stated, so we read it only as evidence and copy no code.

No open protocol write-up or reimplementation of XP2P exists. go2rtc has no XP2P source, but it can read the local FLV URL.

## 5. Verdict

**Go, by linking Tencent's library.** A clean reimplementation of XP2P is not worth it: the protocol is closed, undocumented and includes NAT traversal.

| Platform | Library | Verdict |
|---|---|---|
| Windows (`dreame-desktop`) | `xp2p_windows.zip` DLL | feasible |
| Android (`dreame-android`) | `xp2p-sdk` AAR, `libxnet-android.so` | feasible; the JNI class must be `com.tencent.xnet.XP2P` or we call the C API directly |
| Linux x86_64 | static libraries | feasible |
| Linux ARM, macOS | none | not possible |
| iOS | CocoaPods | feasible, not planned |

Proposed shape:

- A new crate `dreame-video` behind a Cargo feature. It loads the XP2P library at run time (`libloading`), so builds without it still work and the binary stays out of the repository.
- The `dreame` facade gets a video session: Dreame cloud calls (§3.3), robot camera session, PIN check, `monitor start`, keep-alive every 10 to 20 s, `monitor end`.
- Output is the raw FLV stream. Decoding is the app's job: on desktop through ffmpeg (a bundled binary or the `ffmpeg-next` crate), on Android through `MediaCodec` after a small FLV demuxer, or through ExoPlayer reading the local URL.

Risks:

- The licence may forbid redistribution. Downloading at build time or on first use avoids shipping it, but this must be read first.
- `p2pInfo` may require the Tencent binding (`dev/pair`). If the official app has streamed video on this account, it is probably bound already. `isDevUser` answers that as a read.
- H.265 decoding on desktop needs ffmpeg built with HEVC.
- The XP2P version check on the robot side (§3.2) could reject an old client. The newest release avoids it.

## 6. Next steps

Reads first, then writes, each write approved by the user:

1. **Reads on the Dreame cloud:** `user/accesstoken`, `isDevUser`, `getIdentity`, `getP2PInfo`. They show whether the account is bound and what `p2pInfo` looks like. Do not log `secretId`, `secretKey`, `p2pInfo` or the Tencent device name.
2. **Read the XP2P licence text** (from the AAR or the release zip).
3. **Prototype**: load `xp2p_windows.zip` from Rust, start the service with the values from step 1, and save 30 s of FLV.
4. **Robot writes for the prototype** (approval needed): access-code query (10001.4 piid 1100), PIN verify (piid 1102), `monitor start` (10001.1 piid 1), keep-alive (piid 6), `monitor end`.
5. Only if `isDevUser` is false: `dev/pair` (a cloud write, approval needed).
