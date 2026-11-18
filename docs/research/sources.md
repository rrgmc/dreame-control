# Sources

This page lists every source used for the research in this folder. It says what each source covers and how far to trust it.

Short names in the citations of the other documents:

| Short name | Meaning |
|---|---|
| `T/` or `proto:` | tasshack `dreame-vacuum`, path under `custom_components/dreame_vacuum/` (`proto:` = `dreame/protocol.py`) |
| `N/` | `node-dreame` |
| `H/` | `com.dreame.vacuum.cloud` (Homey app) |
| `C/` | `dreame-vacuum-map-card` |
| `V:` | Valetudo |
| `APK` | Dreamehome 2.6.7.0, decompiled locally. `apk:<line>` cites `libapp.strings`; `app:<path>` cites the Blutter decompile, see [decompile.md](decompile.md) |
| "Verified on r2579a" | Observed on the user's own L40 Ultra AE, first with prototype tools, later with the `dreame` CLI and apps |

## Reference repositories

The reference repositories are cloned read-only into `local/refs/`, which git ignores. Clone them again with `git clone --depth 1 <url>` to check a citation.

| Repository | Licence | Commit read | Language | What it covers | Trust |
|---|---|---|---|---|---|
| [Tasshack/dreame-vacuum](https://github.com/tasshack/dreame-vacuum) | MIT | `fec086c` (2026-11-03) | Python | Home Assistant integration. It is the most complete source for every topic: Dreame cloud and Mi cloud login, request signing, TLS fingerprinting, MQTT, local miIO, the full MIoT property mapping, capability tables for about 766 models, the map codec and renderer, and all map edit commands. Some strings in `protocol.py` are obfuscated in a string table. | High. It is used daily by many users. |
| [malard/node-dreame](https://github.com/malard/node-dreame) | MIT | `0345f0f` (2026-06-24) | TypeScript | Dreame cloud client with docs: `docs/auth-flow.md` and `docs/live-map-format.md`. It was checked on r2532a (X50 Ultra) and r2449a (X40 Ultra). It has clear notes on `sendCommand` quirks (`80001`, action params must be an object). | High for the models it tested. Some of its line references point at tasshack's `dev` branch. |
| [pgrootkop-cmyk/com.dreame.vacuum.cloud](https://github.com/pgrootkop-cmyk/com.dreame.vacuum.cloud) | MIT | `61c4fe8` (2026-07-17) | JavaScript | Homey app: cloud API, MQTT and map download. Uses the older iOS header dialect. | Medium. It mislabels property 6.2 as the object name; its own comment agrees it is 6.3. |
| [noambergauz/dreame-vacuum-map-card](https://github.com/noambergauz/dreame-vacuum-map-card) | MIT | `4ab0720` (2026-10-15) | TypeScript | Home Assistant Lovelace card. It renders tasshack's map entity and calls tasshack services for room and zone cleaning. It has no map editing of its own. | Useful for UI ideas only. |

### The paid card in tasshack v2.0.0

Version 2.0.0 of tasshack's integration adds the "Dreame Vacuum Card", which costs USD 25 for each robot. The card adds map editing: creating, splitting, merging and renaming rooms, and placing virtual walls. Users in a [Reddit thread](https://www.reddit.com/r/homeassistant/comments/1wutjhk/dreame_vacuum_integration_v200_finally_released/) describe how it works. We have not checked these reports ourselves.

- `frontend.py` holds the card's loader as a JavaScript bundle. The bundle is base64-encoded gzip and obfuscated.
- Without a licence key, the card renders empty. With a key, the integration posts the key, a token, a hash of its source and the robot model to `api.tasshack.com`.
- The reply is encrypted (AES-GCM) and signed (Ed25519). The integration caches it and passes it to the browser encrypted with ChaCha20. The card decrypts the reply and runs it with `eval`.
- So the card's editing code is downloaded after payment and is not in the repository.

Rules for this project:

- Do not cite or copy `frontend.py` or any code the licence server returns. The MIT licence covers only the code in the repository, and the downloaded code is a paid product.
- Our citations come from `map.py`, `device.py`, `types.py`, `const.py` and `dreame/protocol.py`. These files are readable apart from the string table noted above.

## Websites and other projects

| Source | What it covers |
|---|---|
| [dvc.tasshack.com](https://dvc.tasshack.com/) and [dv.tasshack.com supported devices](https://dv.tasshack.com/guide/more/supported-devices) | tasshack's user documentation and supported model list. The list gives four codes for the L40 Ultra AE: `r2579a`, `r2579h`, `r500za`, `r500zh`. |
| [TA2k/ioBroker.dreame](https://github.com/TA2k/ioBroker.dreame) | ioBroker adapter: cloud API, MQTT fallback hosts, user-data endpoints. |
| [rytilahti/python-miio](https://github.com/rytilahti/python-miio) | The local miIO packet format and its encryption. |
| [Hypfer/Valetudo](https://github.com/Hypfer/Valetudo/tree/master/backend/lib/robots/dreame) | Cloud-free firmware for rooted robots. Its Dreame map parser and segment-edit code confirm the pixel encodings and room-edit payloads. Its supported list includes the L40 Ultra (r2492) but not the AE. |
| [home.miot-spec.com](https://home.miot-spec.com/) | Public MIoT specifications. The newest public Dreame vacuum spec found is r2320. None exist for r24xx, r25xx or r2579a. |
| [Home Assistant community thread](https://community.home-assistant.io/t/custom-component-dreame-vacuum/473026) | User reports: wrong-region errors, L40 Ultra model ids. |
| [leo.leung.xyz: Dreame L40 Ultra](https://leo.leung.xyz/wiki/Dreame_L40_Ultra) | L40 Ultra (r2492) teardown and rooting notes. |
| Retail pages ([Notebookcheck](https://www.notebookcheck.net/Dreame-releases-cheaper-robot-vacuum-with-upgraded-suction-power.1026644.0.html), [Euronics](https://euronics.ie/products/dreame-l40-ultra-ae-robot-vacuum-mop-combo-white-rll77se_wh)) | L40 Ultra AE hardware and its retail SKU `RLL77SE`. |
| [Google Play: Dreamehome](https://play.google.com/store/apps/details?id=com.dreame.smartlife) | The official app, package `com.dreame.smartlife`. |

## The Dreamehome APK

- File: Dreamehome 2.6.7.0 (`versioncode` 102060700), an APKMirror bundle (`.apkm`), 259.46 MiB, uploaded 2026-11-06.
- Source: [APKMirror release page](https://www.apkmirror.com/apk/dreame-innovation-technology-suzhou-co-ltd/dreamehome/dreamehome-2-6-7-0-release/).
- SHA-256 of the bundle: `c32f9fabb3a327b724382bdb5ed6d499c05e9de96d05b46ce79efc7aa6979cd1`. This matches the hash APKMirror publishes.
- Analysis was static only. The app was never installed or run. jadx 1.5.6 decompiled the Java side. The Flutter app code is AOT-compiled into `libapp.so`. An early pass recovered only its strings (`libapp.strings`, cited `apk:<line>`). It has since been fully decompiled with Blutter; see [decompile.md](decompile.md).
- Local working files are in `local/apk/`: `splits/`, `jadx-out/`, `libapp.strings`, `native/`, `extracted/`, and `blutter/` (the Blutter decompile under `blutter/out/`). Git ignores this folder.
- The app is Flutter, but it is only the host shell: sign-in, home, rooms, the device list, MQTT relay and networking. The per-robot control screens, including the vacuum map view and editor, are React Native plugins downloaded per model as JavaScript bundles (run on V8/JSC). They are not in the APK or in `libapp.so`. See [decompile.md](decompile.md) §4.
- The React Native map plugin for r2579a was captured from a rooted emulator (`local/apk/rn_plugins/`, git-ignored) and read. It is the official app's own map codec and edit payloads; see [map-plugin.md](map-plugin.md).

## The user's robot

- Model `dreame.vacuum.r2579a`, Dreame L40 Ultra AE, firmware `4.3.9_1664`, on a Dreamehome account registered outside the US (cloud region `us`).
- Accessed read-only on 2026-11-15 with prototype tools. See [l40-ultra-ae.md](l40-ultra-ae.md).
