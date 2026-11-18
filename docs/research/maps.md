# Dreame robot vacuum maps: codec, rendering and editing reference

This document is the reference for a Rust map codec, a map renderer, and a desktop map editor for
Dreame robot vacuums. The target robot is the Dreame L40 Ultra AE, model `dreame.vacuum.r2579a`.
Map editing flexibility is the main goal, so sections 11 to 13 carry the most weight.

## 0. Conventions

### 0.1 Verification marker

**Verified on r2579a** marks a fact observed on the user's own robot: read-only from 2026-11-15,
and through writes the user approved from 2026-11-16, dated where they appear.
Verified facts win over every other source. Everything else is cited to a source file or URL.
Verified data comes from these captures (git-ignored, in `local/out/`):

| File | Content |
|---|---|
| `map/frame-6.3.json` | Last frame named by property 6.3, decoded (header, byte histogram, full JSON trailer) |
| `map/saved-0.json` | The one saved map from the 6.8 map list, decoded (full JSON trailer) |
| `map/map-list-6.8.json` | Raw saved-map list file |
| `map/*.png` | Renders of both maps under three pixel encodings |

### 0.2 Source tags

Paths are relative to `local/refs/` in this repository.

| Tag | Meaning |
|---|---|
| `T/map.py:N` | `dreame-vacuum/custom_components/dreame_vacuum/dreame/map.py`, line N (Tasshack Home Assistant integration; the most complete reference) |
| `T/device.py:N`, `T/types.py:N`, `T/const.py:N`, `T/protocol.py:N` | Same directory |
| `T/vacuum.py:N` | `dreame-vacuum/custom_components/dreame_vacuum/vacuum.py` |
| `N/...` | `node-dreame/` (TypeScript client). Its `docs/live-map-format.md` is a good prose summary |
| `H/...` | `com.dreame.vacuum.cloud/` (Homey app) |
| `C/...` | `dreame-vacuum-map-card/` (Lovelace map card) |
| `V:` | Valetudo, `https://github.com/Hypfer/Valetudo/blob/master/backend/lib/robots/dreame/` (read 2026-11-15) |

Placeholders `<did>` (device id) and `<uid>` (user id) replace real identifiers.

### 0.3 Terms

- **MIoT**: the Xiaomi-style device model used by Dreame. A **property** is addressed as `siid.piid`
  (service id, property id). An **action** is addressed as `siid.aiid` (service id, action id).
- **Segment**: a room. Segment ids are small integers.
- **Frame**: one map message from the robot. An **I-frame** is a full snapshot. A **P-frame** is a
  difference against the previous frame. A **W-frame** is a Wi-Fi signal map.
- **Saved map**: a persistent map stored by the robot, one per floor. It is the object an editor edits.
- **Live map** (or current map): the robot's current view, sent while it works.
- **rism**: a saved map embedded as a string inside a live frame (JSON key `rism`).
- **Envelope**: the text encoding that wraps a frame (base64, optional AES, zlib).
- **Trailer**: the JSON object that follows the pixel grid inside a frame.
- **Floor plan**: the vector outline of each room (`walls_info`), separate from the pixel grid.
- **mm**: millimetres. All map coordinates are integer or float mm in the robot world frame.

---

## 1. Quick facts for r2579a

| Fact | Value | Status |
|---|---|---|
| Model | `dreame.vacuum.r2579a`, firmware `4.3.9_1664` | Verified on r2579a |
| Cloud | Dreamehome cloud (`*.iot.dreame.tech`) | Verified on r2579a |
| LAN control | None. No miIO on UDP 54321, all probed TCP ports silently dropped | Verified on r2579a |
| Map encryption | None. 6.3 has no `,key` suffix; envelopes inflate with zlib directly | Verified on r2579a |
| Live frame pixel encoding | B (frame map, `fsm == 1`) | Verified on r2579a |
| Saved map pixel encoding | E (bit 7 wall, bit 6 carpet, bits 5-0 segment) | Verified on r2579a |
| Map version (decoder mode) | 2. Capability `CLEANING_SEQUENCE_V2` present, `MAP_V2` absent, `cleanareaorder` present | Verified (key); capability set from `T/const.py:736` |
| Multi-floor | Off (6.7 = 0). One saved map, `map_id` 1 | Verified on r2579a |
| Rooms | 13 segments in the saved map, 2 of them hidden (`delsr`) | Verified on r2579a |
| Floor plan | `walls_info` with `version_flag` 3, 13 rooms, 17 door entries | Verified on r2579a |
| Grid | 50 mm per pixel; saved map 225 x 349 pixels | Verified on r2579a |
| DEVICE_INFO IV | `NRwnBj5FsNPgBNbT` listed, but unused because no key is sent | Table in `T/const.py:736` |

Map-related capabilities for r2579a, decoded from Tasshack's `DEVICE_INFO` table (`T/const.py:736`,
capability names `T/types.py:2886-3040`). Six model ids share this capability set: `r25799`,
`r2579a`, `r2579c`, `r2579h`, `r2579k`, `r500za`.

| Present | Absent (notable) |
|---|---|
| `FLOOR_MATERIAL`, `FLOOR_MATERIAL_EDIT`, `MULTI_SEGMENT_FLOOR_MATERIAL` (firmware 1623 or later), `FLOOR_DIRECTION_CLEANING`, `SEGMENT_VISIBILITY`, `AUTO_RENAME_SEGMENT`, `CLEANING_SEQUENCE_V2`, `SEGMENT_MOPPING_SETTINGS`, `WETNESS_LEVEL`, `FURNITURES_V2`, `EXTENDED_FURNITURES`, `SAVED_FURNITURES`, `PET_FURNITURE`, `RAMPS`, `CURTAINS`, `WALLS_V3`, `CARPET_ID`, `CARPET_SHAPE` (firmware 1377 or later), `CARPET_CLEANSET_V2`, `QUICK_MAP_RECOVERY_V2`, `AREA_ROTATION`, `VIRTUAL_TRACKS`, `SKIP_AREA`, `OBSTACLES`, `CAMERA_STREAMING`, `LOCAL_STORAGE` (firmware 1681 or later, so not active on 1664) | `MAP_V2`, `MAP_EDIT_WHILE_RUNNING`, `FLOOR_PLAN`, `CARPET_TYPE`, `CARPET_MATERIAL`, `CARPET_TASSEL`, `CARPET_CLEANSET_V3`, `LOW_LYING_AREAS`, `LOW_LYING_AREA_DELETE` |

A capability entry `[id, min_version]` is active when the firmware build number is at least
`min_version` (`T/types.py:3319-3335`). The r2579a build number is 1664.

---

## 2. Fetch flow

### 2.1 MIoT properties and actions

All map traffic uses MIoT service 6 (`T/types.py:1674-1687`, `T/types.py:1984-1987`).
Property ids can differ per model; Tasshack resolves them through `property_mapping`
(`T/device.py:6489`).

| Name (Tasshack) | Id | Content | r2579a cache |
|---|---|---|---|
| `MAP_DATA` | 6.1 | Inline frame envelope, usually P-frames while cleaning | Empty |
| `FRAME_INFO` | 6.2 | Input of `REQUEST_MAP`: JSON frame request | (input only) |
| `OBJECT_NAME` | 6.3 | Cloud object name of the last frame, optionally `"<obj>,<aes_key>"` | `ali_dreame/<uid>/<did>/0`, no key |
| `MAP_EXTEND_DATA` | 6.4 | Input of `UPDATE_MAP_DATA` (all edits) and `BACKUP_MAP` | (input only) |
| `ROBOT_TIME` | 6.5 | Robot time in ms, returned with `REQUEST_MAP` output | Not cached |
| `MULTI_FLOOR_MAP` | 6.7 | Bool: multi-floor enabled. Writable | `0` |
| `MAP_LIST` | 6.8 | `{"obj_name"\|"object_name": "...", "md5": "..."}` pointing at the saved-map list file | `{"obj_name":"ali_dreame/<uid>/<did>/9","md5":"..."}` |
| `RECOVERY_MAP_LIST` | 6.9 | Object name of the recovery (backup) list file | `ali_dreame/<uid>/<did>/8` |
| `MAP_RECOVERY` | 6.10 | Writable: `{"map_id":M,"map_url":"<url>"}` starts a restore | Not cached |
| `MAP_RECOVERY_STATUS` | 6.11 | 0 idle, 2 running, 3 success, 4 or 5 fail (`T/types.py:975`) | Not cached |
| `OLD_MAP_DATA` | 6.13 | Legacy: `"0,<b64>"` inline or `"1,<obj>[,<key>]"` | Empty |
| `MAP_BACKUP_STATUS` | 6.14 | 0 idle, 2 running, 3 success, 4 fail (`T/types.py:986`) | Not cached |
| `WIFI_MAP` | 6.15 | Presence means Wi-Fi map support | Not cached |
| `REQUEST_MAP` | action 6.1 | Ask the robot to publish a frame | |
| `UPDATE_MAP_DATA` | action 6.2 | All map edits | |
| `BACKUP_MAP` | action 6.3 | Back up a saved map to the cloud | |
| `WIFI_MAP` | action 6.4 | Request a Wi-Fi map | |

"Not cached" means the cloud property cache had no value. It does not prove the property is absent.
A live `get_properties` call answers that (experiment R1 in section 16).

The Homey app labels 6.2 as the object name (`H/drivers/vacuum/device.js:85`); its own comment on
line 86 agrees with Tasshack that 6.3 is the object name. Trust Tasshack.

### 2.2 Push and poll

- Dreamehome cloud relays robot `properties_changed` messages over MQTT. Tasshack routes 6.1, 6.3,
  6.5 and 6.13 to the map manager (`T/device.py:547-583`, `T/map.py:1133-1170`). It does not poll
  property history on Dreamehome (`T/map.py:207-209`).
- Mi cloud variants poll the property history instead (`T/map.py:229-248`).
- A running robot with no new frame for 60 s on Dreamehome triggers a new request
  (`T/map.py:1500-1514`).
- The cloud property cache (`iotstatus/props`) also returns the last 6.3 and 6.8 values. This is how
  a client reads maps without any action. **Verified on r2579a.** The `keys` field must be one
  comma-separated string such as `"6.3,6.8,6.9"`; a JSON array silently returns no data.
  **Verified on r2579a.**

**Verified on r2579a during a cleaning run (2026-11-15).** See protocols.md §11.1b for the full
timeline. Map-related findings:

- The run used a new `map_id` (5). The robot pushed **122 inline P-frames on 6.1, one every 3 s**,
  with `frame_id` 1 to 122. No inline I-frame was pushed. A client that has no I-frame for the
  current `map_id` must fetch one: from the 6.3 object, or with `REQUEST_MAP`.
- **6.3 flipped between `ali_dreame/<uid>/<did>/1` and `.../0`** at the start (13-15 s) and at the
  end (394-398 s). The robot re-uploads the map files and announces each upload this way.
- P-frames carry these JSON keys: `timestamp_ms`, `curid`, `mtid`, `sa`, `moptype`, `cs`, `robot`,
  `charger`, `origin`, `fsm` (= 1), `mra`, `tr`, `sneak_areas`, `ai_obstacle`, `laser_obstacle`,
  `ai_furniture_warning`, `ai_furniture`, `ai_furniture_user`, `cleanset`, `carpetcleanset`.
  - `sa` = `[[5, 1, 2, 15, 0]]`: the selected room (segment 5) and its settings.
  - `tr` holds only the new part of the path, for example `l-661,3224L6,52L5,50...`. Append it
    to the path so far.
  - `robot` and `charger` repeat the positions in mm.
- P-frame rectangles are small (for example 126 x 152, 21 x 150, 31 x 41 pixels), and many frames
  are `0 x 0`: position and path only. Pixel bytes seen in P-frames: 0, 4, 24, 220, 240. On this
  model (map version below 3) a P-frame byte is **added** to the stored byte (mod 256), not
  written over it (§6).
- At the end, 6.8 got a new md5: the saved map was re-saved after the run.
- **Cleaning history map**: event 4.e1, argument piid 9, names an object
  `ali_dreame/<yyyy>/<mm>/<dd>/<uid>/<did>_<n>.<build>.bin`. It downloads through
  `iotfile/getDownloadUrl` and is a normal envelope (no AES key). Decoded: `map_id` 5,
  `frame_id` 123, type `I`, 185 x 296 pixels, with `iscleanlog: true`, the full path in `tr`
  (starting with `S`), `cs`, `sa`, `ctyi`, `ds`, `wm`, `cleanset`, `customeClean`, and the saved
  map embedded in `rism`. That makes it a complete record of the run: a client can draw the
  history of each clean without having followed the live P-frames.

### 2.3 `REQUEST_MAP` payloads

Action 6.1 with `in = [{"piid": 2, "value": "<compact JSON>"}]` (`T/map.py:281-300`).

| Purpose | Value of piid 2 | Source |
|---|---|---|
| Plain request (robot pushes a frame) | `{"frame_type":"I"}` | `T/map.py:283-285`, `T/device.py:6470-6478` |
| Forced I-frame, synchronous reply | `{"req_type":1,"frame_type":"I","force_type":1}`, optional `"time":<s>` | `T/map.py:306-313` |
| Missing P-frame | `{"map_id":M,"frame_id":F,"frame_type":"P"}` | `T/map.py:385-391` |
| Next P-frame, synchronous reply | `{"map_id":M,"req_type":1,"frame_id":F,"frame_type":"P"}` | `T/map.py:401-408` |
| Refresh saved-map list | `{"frame_type":"T"}` | `T/map.py:438-441` |

With `req_type: 1` the action result `out` carries piid 3 (object name), piid 1 (inline map),
piid 5 (robot time) and piid 13 (legacy map). Empty strings mean "not present"
(`T/map.py:316-357`). Tasshack refuses a forced I-frame on restored, temporary or unsaved maps and
falls back to a plain request (`T/map.py:1832-1844`).

### 2.4 Object names and signed URLs

Large frames (I-frames, saved maps, history maps) travel as cloud objects. The robot uploads the
object and publishes its name.

- Name format on Dreamehome: `ali_dreame/<uid>/<did>/<n>`. **Verified on r2579a** (n = 0, 8, 9 seen).
- A comma suffix carries an AES key: `"<obj>,<key>"` (`T/map.py:551-558`). Not used on r2579a.

| Endpoint (Dreamehome) | Body | Use |
|---|---|---|
| `POST /dreame-user-iot/iotfile/getDownloadUrl` | `{"did","model","filename":<obj>,"region":<country code>}` | Live frames, map list, recovery list. **Verified on r2579a** |
| `POST /dreame-user-iot/iotfile/getOss1dDownloadUrl` | `{"did","uid","model","filename":<obj without first character>,"region"}` | Permanent files: obstacle photos, history maps, backup archives (`T/protocol.py:1487-1501`, `H/lib/DreameApi.js:393-395`) |

The response is `{"code":0,"data":"<signed URL>"}`. A GET on the URL returns the envelope as base64
text, not raw zlib. **Verified on r2579a.** Tasshack caches URLs for 30 min (`T/map.py:590-610`).

### 2.5 Saved-map list (6.8)

The file named by 6.8 is JSON (`T/map.py:1629-1763`). **Verified on r2579a** shape:

```json
{
  "mapstr": [
    {"id": 0, "name": "", "angle": "0", "map": "<envelope of the saved map, about 16 KB>"}
  ],
  "curr_id": 1
}
```

- `map` is a full envelope of the saved map. Decode it like any frame.
- `angle` is the saved display rotation as a string. `name` is the custom map name (empty here).
- `curr_id` is the selected `map_id`. The `id` field (0 here) is an index, not the `map_id`
  (the decoded header says `map_id` 1). **Verified on r2579a.**
- Other models add `mapobj`, `rismobj` and `server`. When `server == 1` and `map` is empty, fetch
  `rismobj` instead (`T/map.py:1653-1662`). r2579a sends neither key.
- The `md5` in 6.8 lets a client skip an unchanged list (`T/map.py:1610-1619`). A changed md5 is the
  best signal that an edit was applied to the saved map.

#### Several floors

With multi-floor on (6.7), `mapstr` holds one entry per floor. Tasshack keys each entry by the
`map_id` in its decoded header, not by `id` (`T/map.py:1650-1683`). The `dreame` crate does the same
(`dreame::maplist`): `read_saved_map(robot, Some(id))` reads one floor, `read_saved_maps` reads all,
and `dreame map list` prints them.

- **Per-room settings** live only in the live frame (6.3), which belongs to the current map. The
  live trailer's `curid` names that map. Another floor's map therefore has no settings.
- **Edits of another floor.** `nsr`, `msr`, `dsrid`, `delsr` and `nsm` carry `mapid`. `vw`, `vws`,
  `curtain`, `mapfurniture_v2`, `cleanareaorder` and `customeClean` do not, so the robot applies
  them to the current map (`T/device.py:6589-6606`, `7147`). The crate refuses those on a map that
  is not current (`mapedit::check_target`). Tasshack itself edits only the current or selected map,
  so whether the robot honours `mapid` for another floor is **not verified**: r2579a has
  multi-floor off and one map.

### 2.6 Recovery list (6.9)

The file named by 6.9 is a JSON array (`T/map.py:1765-1830`).

Verified on r2579a, 2026-11-15: a relayed read of 6.9 returns `{"object_name":"ali_dreame/<uid>/<did>/8"}`
(the cache holds the bare object name). The file downloads through `iotfile/getDownloadUrl` and
parses as below. It holds one map id with two entries:

| `id` | `first` | `time` | `objname` (last part) | `thb` length | `rismobj` |
|---|---|---|---|---|---|
| 1 | 1 (original) | 1776452401 (2026-04) | `100` | 6,532 chars | absent |
| 1 | 0 (edited) | 1781452777 (2026-06) | `102` | 15,752 chars | absent |

Each entry has only the keys `first`, `time`, `thb` and `objname`. There is no backup entry
(`first` 2), so no manual backup has been made yet.

```json
[{"id": 1, "info": [{"time": 1700000000, "thb": "<envelope>", "rismobj": "<obj>", "objname": "<obj>", "first": 0}]}]
```

- `id` is the `map_id`. `first` is the recovery type: 0 edited, 1 original, 2 backup
  (`T/types.py:4626-4630`).
- `thb` is an inline map preview; decode it as a saved map.
- `objname` is the restorable archive. Backup archives end in `mb.tbz2` (`T/map.py:1400-1403`); on r2579a they do not, and they download through `getDownloadUrl` like the other entries ([l40-ultra-ae.md](l40-ultra-ae.md)).

### 2.7 Frame sequencing

- Header `map_id` changes mean a new map session; reset frame state (`T/map.py:705-724`).
- P-frames must arrive in order (`frame_id == current + 1`). Queue out-of-order frames per map id
  (`T/map.py:476-549`). Queue longer than 4: request the missing P-frame. Longer than 8: request an
  I-frame (`T/map.py:644-653`, `T/map.py:767-783`).
- A P-frame before any I-frame is queued and an I-frame is requested (`T/map.py:752-757`).
- `timestamp_ms` can be robot uptime after a restart. Tasshack replaces values below
  `1577826000000` with the receive time (`T/map.py:614-619`).

### 2.8 Live-frame state flags

| Flag | JSON | Meaning | Source |
|---|---|---|---|
| `saved_map_status` | `ris` | absent: none; 0 or 1: map being built or not saved; 2: robot localised on a saved map | `T/map.py:4366-4412` |
| `restored_map` | `rpur == 1` | Saved map reloaded, no new data | same |
| `frame_map` | `fsm == 1` | Pixel grid uses encoding B | same |
| `temporary_map` | `suw` in (5, 6) and no `fsm` | New map awaiting a keep/discard decision | same; V: `DreameMapParser.js` |
| `new_map` | `risp == 0` | | same |
| `recovery_map` | `us == 1` | | same |
| `clean_log` | `iscleanlog` | Frame is a cleaning-history map | same |

**Verified on r2579a:** the cached 6.3 frame had `ris = 2`, `fsm = 1`, `iscleanlog = true`, and an
embedded `rism`. So 6.3 can name the end-of-clean history frame, not only a live frame.

### 2.9 Map version

`map_version` selects decoding rules (`T/map.py:1846-1859`): 0 vSLAM (no lidar); 1 lidar;
2 `CLEANING_SEQUENCE_V2` capability or `cleanareaorder` present (`T/map.py:4539-4540`);
3 `MAP_V2` capability. r2579a is version 2.

Do not copy the version bump in `decode_map_partial`: it sets a local `version = 3` after the value
was already stored, so it has no effect (`T/map.py:4238` against `T/map.py:4252-4262`).

---

## 3. Envelope and AES

```
text
  -> replace '_' with '/', '-' with '+'
  -> base64 decode (padding may be missing; decode leniently)
  -> [AES-256-CBC decrypt, only if a key is present]
  -> zlib inflate
  -> raw frame = 27-byte header | width*height pixel bytes | UTF-8 JSON trailer
```

Sources: `T/map.py:4198-4221`; Rust implementation in `crates/dreame-map`.

- **Verified on r2579a:** no key, plain zlib, for 6.3 frames, the 6.8 saved map and the embedded
  `rism` string (the `rism` text starts `eF7t`, the base64 form of a zlib header).
- If the text contains `,` and no key was supplied, split it as `"<b64>,<key>"` (`T/map.py:4203-4206`).
- AES, for other models: key bytes are the first 32 ASCII characters of
  `sha256(key_string).hexdigest()`; IV is a 16-character ASCII string per model from `DEVICE_INFO`
  (`T/map.py:4210-4221`, `T/types.py:3286-3291`). Do not strip PKCS#7 padding; zlib ignores
  trailing bytes. The decoder tries all 13 known IVs when zlib fails (`KNOWN_IVS` in `crates/dreame-map`).
- The per-model IV table is a tasshack construct. The official app uses a single global IV
  constant, which is the **empty string** in the r2579a plugin build, so the shipped app has no
  working map-decrypt path and depends on no key being sent (RN plugin; see
  [map-plugin.md](map-plugin.md) §4).
- Homey falls back to raw deflate when zlib fails (`H/drivers/vacuum/device.js:689-697`).
- Encoding a frame (for tests, or for any upload experiment) is the reverse: JSON trailer appended to
  header and pixels, zlib deflate, base64 with `+` to `-` and `/` to `_`.

---

## 4. Header (27 bytes)

Little-endian. All fields are signed 16-bit except `frame_type` (`T/map.py:4237-4245`,
`T/map.py:4299-4314`, `N/src/map/header.ts`).

| Offset | Type | Field | Notes |
|---|---|---|---|
| 0 | i16 | `map_id` | |
| 2 | i16 | `frame_id` | |
| 4 | u8 | `frame_type` | ASCII `I` (73), `P` (80), `W` (87) (`T/types.py:4602-4606`) |
| 5 | i16 | robot x | mm |
| 7 | i16 | robot y | mm |
| 9 | i16 | robot angle | degrees |
| 11 | i16 | charger x | mm |
| 13 | i16 | charger y | mm |
| 15 | i16 | charger angle | degrees |
| 17 | i16 | `grid_size` | mm per pixel |
| 19 | i16 | `width` | pixels |
| 21 | i16 | `height` | pixels |
| 23 | i16 | `left` | mm, world x of column 0 |
| 25 | i16 | `top` | mm, world y of row 0. Row 0 is the minimum y |

**Verified on r2579a** examples:

| Frame | map_id | frame_id | type | grid | size | left, top | robot | charger |
|---|---|---|---|---|---|---|---|---|
| 6.3 frame | 4 | 350 | I | 50 | 185 x 296 | -4100, -4500 | (-750, -570, 88) | (-750, -570, 88) |
| Saved map | 1 | 0 | I | 50 | 225 x 349 | -5950, -6900 | (0, 0, 0) | (-750, -570, 88) |

The live `map_id` (4) is a session counter. It differs from the saved `map_id` (1). Both maps share
one world frame: the charger has the same coordinates in both. **Verified on r2579a.**

Rules:

- 32767 in a position means "absent". JSON `nr` and `nc` also mark robot and charger absent
  (`T/map.py:4414-4426`).
- JSON `origin: [left, top]` overrides the header (`T/map.py:4325-4327`). On r2579a it equals the
  header values in both maps. **Verified on r2579a.**
- An empty map is `width == 2 && height == 2` with zero pixels (`T/map.py:4561-4567`).
- In P-frames, width, height, left and top describe only the changed rectangle; `0 x 0` means no
  pixel change (`N/docs/live-map-format.md:107-111`).
- Pixels: `raw[27 .. 27 + width*height]`, row-major, row 0 at `top`. Trailer: the rest.

---

## 5. Pixel encodings

### 5.1 Decoded pixel classes

Tasshack turns each byte into a pixel class (`T/types.py:4609-4623`, `T/map.py:3990-4061`). Use the
same classes as the codec's output so the renderer does not depend on the wire encoding.

| Class | Meaning |
|---|---|
| 0 | Outside (no data) |
| 1..63 | Floor of segment N |
| 101..163 | Boundary (wall) pixel of segment N-100 |
| 201..231 | Encoding A only: "wall type 3" pixel of segment N-200; not drawn |
| 2, 10..14 | Wi-Fi map: wall, unreached, poor, low, high, excellent |
| 249, 250 | Cleaned, dirty (from `decmap`) |
| 251 | Obstacle wall |
| 252 | Unknown |
| 253 | New segment (floor not yet assigned to a room) |
| 254 | Floor (no room) |
| 255 | Wall |

Carpet is a separate per-pixel boolean, not a class.

### 5.2 Encodings

Selection follows `T/map.py:4583-4686` for I-frames and `_get_pixel_type` for P-frames
(`T/map.py:3990-4061`).

| Id | When | Layout | r2579a |
|---|---|---|---|
| A | Map version 3 (`MAP_V2`) | bit 7 carpet; bits 6-5 wall (0 none, 1-2 wall, 3 type-3); bits 4-0 segment (31 special) (`T/map.py:4584-4616`) | Not used: misreads both maps |
| B | Live frame with `fsm == 1` | bits 7-2 segment (1..60 room, 61 unknown, 62 floor, 63 wall); bits 1-0 class; carpet when `(px & 3) == 3`; if segment is 0: class 1 or 3 new segment, 2 wall (`T/map.py:4617-4640`) | **Verified on r2579a**: live frames |
| C | Map being built (`ris` 0 or 1), lidar | `px & 0x3F`: 1 or 3 new segment, 2 wall; carpet when `(px & 3) == 3` (`T/map.py:4641-4655`) | Expected during a new mapping run |
| D | vSLAM unsaved, or `ris == 2` without `fsm` | `px & 0x3F`: 2 wall, other non-zero new segment (`T/map.py:4656-4670`) | Not seen |
| E | Saved map, standard lidar map, `rism` | bit 7 wall/boundary; bit 6 carpet; bits 5-0 segment (`T/map.py:4671-4686`) | **Verified on r2579a**: saved map |
| W | W-frame | `px & 0x0F` is the Wi-Fi class (`T/map.py:4573-4580`) | `whm` present, not decoded yet |
| decmap | Clean overlay | `px & 3`: 1 cleaned, 2 dirty; own header (`T/map.py:6160-6186`) | Not seen |

Valetudo reads live maps the same way as B (`px >> 2` segment, `px & 3` 0 none, 1 floor, 2 wall,
3 carpet) and calls saved maps "RISM" mode, same as E (V: `DreameMapParser.js`).

### 5.3 Byte values seen on r2579a

**Verified on r2579a**, live frame (encoding B), 185 x 296 = 54,760 pixels:

| Byte | Decoded | Count |
|---|---|---|
| `0x00` | outside | 24,083 |
| `0x04 .. 0x30` (step 4) | segment 1..12, class 0: plain room floor | rooms 1-8, 10-12 |
| `0x07`, `0x1B`, `0x1F`, `0x23`, `0x2B`, `0x2F` | segment 1, 6, 7, 8, 10, 11 with class 3: carpet | |
| `0xFC` | segment 63: wall | 3,637 |

No class 1 or 2 bytes and no segment 61 or 62 appear. Hidden segments 9 and 13 are absent from the
live grid. Furniture legs and obstacles show as `0xFC` islands inside rooms.

**Verified on r2579a**, saved map (encoding E), 225 x 349 = 78,525 pixels:

| Byte | Decoded |
|---|---|
| `0x00` | outside (41,345) |
| `0x01 .. 0x0D` | floor of segment 1..13 |
| `0x41`, `0x46`, `0x47`, `0x48`, `0x4A`, `0x4B` | carpet in segment 1, 6, 7, 8, 10, 11 |
| `0x81 .. 0x8D` | boundary pixel of segment 1..13 |

There is no `0x80` (wall without room) and no `0xC0`-range byte. Every wall pixel in the saved map
belongs to a room boundary. The saved map is a cleaned, smoothed version of the live map: it has no
obstacle islands and no stray wall pixels. Compare `saved-0-saved.png` with
`frame-6.3-frame_map.png`.

---

## 6. P-frame merge and live/saved composition

### 6.1 P-frame merge

`decode_p_map_data_from_partial` (`T/map.py:5770-6022`):

1. Allocate a buffer covering the union of the current and P-frame rectangles (`T/map.py:5873-5896`).
2. Copy the current bytes at their offset.
3. For each non-zero P byte: encoding A replaces the byte; other encodings add it with `u8`
   wrapping (`T/map.py:5929-5936`). r2579a live frames use B, so the add rule applies.
   **Verified on r2579a** (2026-11-17, experiment R4): the add rule reproduces the history map
   far better than the replace rule ([l40-ultra-ae.md](l40-ultra-ae.md)).
4. Re-classify with the current map's flags.
5. Recompute segments. With `ris == 2`, keep segments missing from the frame instead of deleting
   them (`T/map.py:5967-6002`).
6. Append the P-frame path to the current path (`T/map.py:6007-6012`).
7. Version-3 P-frames carry override objects: `cover` (replaces `tr`, `small_tr`, `moptype`,
   `laser_obstacle`, `timestamp_ms`) and `diff` (changed `ai_obstacle`, `sneak_areas`, and
   `deleteContent` ids) (`T/map.py:4335-4342`, `T/map.py:5812-5866`).

### 6.2 Live frame plus saved map

With `ris == 2` the live frame embeds the saved map as `rism`. Tasshack combines them
(`T/map.py:4807-4913`):

- Room ids and walls come from the saved map (encoding E).
- Live pixels add floor, wall and new-segment areas outside the saved rooms.
- Restrictions, carpets, furniture, cleaning order and hidden rooms come from the saved map
  (`T/map.py:4969-4995`).
- Segment names, types and colours are copied from the saved map onto the live segments
  (`T/map.py:4924-4948`).

**Verified on r2579a:** the 6.3 frame's `rism` string (16,206 characters) is almost the same size as
the 6.8 saved-map envelope (16,204 characters). After a restore the `rism` equalled the 6.8 saved
map (§12.2, experiment R3).

The editor edits the saved map. The live map is a read-only overlay (robot position, path,
obstacles).

---

## 7. JSON trailer key reference

Keys vary by firmware. Parse with `#[serde(default)]` and keep unknown keys. The `MapData` class
documents most keys with `# Data json: <key>` comments (`T/types.py:4675-4805`); parsing is in
`T/map.py:4344-5762`.

### 7.1 Keys present on r2579a

**Verified on r2579a.**

| Map | Keys |
|---|---|
| Saved map | `vw`, `vws`, `ct`, `pointinfo`, `whmp`, `whm`, `walls_info`, `delsr`, `funiture_info`, `cleanareaorder`, `carpet_polygon`, `last_complete_clean_time`, `ar_match`, `seg_inf`, `robot`, `charger`, `origin` |
| Live (6.3) frame | `cleanset`, `curid`, `mtid`, `sa`, `moptype`, `iscleanlog`, `oc`, `nar`, `cmi`, `mis`, `cmc`, `cf`, `mra`, `ris`, `wm`, `ctyi`, `robot`, `customeClean`, `mooClean`, `wt`, `smd`, `ds`, `ulmm`, `charger`, `rels`, `robot_finemop_switch`, `motion_action_statistics`, `carpet_press_times`, `carpet_level`, `backhome_fail_times`, `RobotBreakPointEnable`, `clean_finish_remain_electricity`, `RobotMultiMapEnable`, `multi_map_cnt`, `map_used_times`, `slam_map_size`, `bt`, `completeFlag`, `ai_model_inuse`, `timestamp_ms`, `carpetcleanset`, `ai_outborders_user`, `origin`, `fsm`, `ai_furniture_user`, `tr`, `sneak_areas`, `ai_obstacle`, `laser_obstacle`, `ai_furniture_warning`, `ai_furniture`, `ai_furniture_new`, `rism` |

Type quirks seen on r2579a: live `robot` and `charger` are JSON strings (`"[-750,-570]"`); saved
`robot` and `charger` are arrays. `mra` is a string (`"0"`). `cleanset` is a JSON string. All
`ai_obstacle` fields are strings. **Verified on r2579a.**

### 7.2 State and task

| Key | Meaning | Source or status |
|---|---|---|
| `timestamp_ms` | Frame time (epoch ms) | `T/map.py:4249-4250` |
| `origin` | `[left, top]` override | `T/map.py:4325` |
| `mra` | Map rotation 0, 90, 180, 270 | `T/map.py:4345-4346` |
| `oc` | Docked | `T/map.py:4366` |
| `nc`, `nr` | No charger, no robot position | `T/map.py:4414-4426` |
| `cs`, `ct` (number) | Cleaned area m², cleaning time. Not to be confused with the saved-map object `ct` (curtains) | `T/map.py:4348-4354` |
| `wm`, `cf` | Work mode, completed | `T/map.py:4356-4360`. r2579a: `wm` 18 (segment cleaning), `cf` 1 |
| `smd`, `ctyi`, `cmc` | Startup method, task end type, cleanup method | `T/map.py:4376-4393` |
| `ds`, `wt` | Dust collections, mop washes | `T/map.py:4394-4397` |
| `customeClean`, `mooClean`, `multime`, `dos`, `nopush` | Flags | `T/map.py:4398-4400`, `4365`, `5047` |
| `clean_finish_remain_electricity` | Battery at task end | `T/map.py:4362` |
| `curid` | Current saved map id as a string. r2579a: `"1"` | V; verified value |
| `mtid` | Unknown task or map id string | Unknown |
| `last_complete_clean_time` | Epoch seconds (saved map) | Inferred from value |
| `ulmm` | r2579a value `"update_cleanarea_order"`. Looks like the name of the last map modification. Unconfirmed | Open question 4 |
| `nar`, `cmi`, `mis`, `rels`, `bt`, `completeFlag`, `carpet_level`, `carpet_press_times`, `map_used_times`, `slam_map_size`, `multi_map_cnt`, `RobotMultiMapEnable`, `RobotBreakPointEnable`, `robot_finemop_switch`, `motion_action_statistics`, `backhome_fail_times`, `ai_model_inuse`, `ar_match` | Statistics and flags; not needed for drawing | Unknown; keep in `extra` |

### 7.3 Path

`tr` is a string of tokens matching `([MWSLl])(-?\d+),(-?\d+)` (`T/map.py:4431-4457`):

- `L`: relative move. The numbers are a delta from the previous point.
- `M` mop, `W` sweep and mop, `S` sweep: absolute point; sets the path type up to that point.
- `l` (P-frames only): absolute point joined to the previous frame's path.
- `small_tr`: a second path, not decoded by Tasshack.

**Verified on r2579a:** the 6.3 frame path has 642 tokens: 11 `M` and 631 `L`.

### 7.4 Rooms

| Key | Shape | Source and r2579a status |
|---|---|---|
| `seg_inf` | `{"<id>": {"type", "index", "name", "material", "direction", "nei_id", "roomID"}}` | `T/map.py:4694-4759`. **Verified on r2579a**, see below |
| `sa` | Active segments, `[[id, ...], ...]` | `T/map.py:4466-4467`. r2579a: 5 numbers per entry, e.g. `[10, 1, 2, 15, 1]`; only index 0 (id) is understood |
| `delsr` | Hidden segment ids | `T/map.py:4469-4470`. **Verified on r2579a**: `[9, 13]` |
| `cleanset` | `{"<id>": [suction, water, times, order, cleanType, mopEffect, mopTemp]}`, dict or JSON string | `T/map.py:4518-4522`, decode `T/map.py:6367-6567`. **Verified on r2579a**: JSON string, 6 numbers, e.g. `[1, 3, 1, 2, 2, 546]` |
| `cleanareaorder` | `[{"<id>": order}, ...]` | `T/map.py:4529-4540`. **Verified on r2579a**: 11 entries, hidden rooms 9 and 13 omitted; values match `cleanset[3]`; stored sorted by id. The RN plugin keeps a hidden room at the end only when the stored order already lists it, which agrees with this capture; see [map-plugin.md](map-plugin.md) §6.2 |
| `room_id_type` | Mop pad per room | `T/map.py:4542-4550`. Not on r2579a |
| `walls_info` | Floor plan | Section 8 |
| `area_clean_detail` | Blocked rooms `[[id, reason, x, y, ?]]` | `T/map.py:5534-5541`. Not seen |
| `combine*area`, `combineaddclean` | Mixed `[id]` and `[x0,y0,x1,y1]` lists | `T/map.py:4167-4193`. Not seen |

`seg_inf` on r2579a (**Verified on r2579a**):

| Field | Seen values | Meaning |
|---|---|---|
| `type` | 0, 1, 4, 5, 6, 12; absent on the two hidden rooms | Room type. 0 means "custom name". Type codes 0..15: Room, Living Room, Primary Bedroom, Study, Kitchen, Dining Hall, Bathroom, Balcony, Corridor, Utility Room, Closet, Meeting Room, Office, Fitness Area, Recreation Area, Secondary Bedroom (`T/types.py:12-29`) |
| `index` | 0 on typed rooms; 0, 3, 4, 6, 7 on custom-named rooms | Typed rooms: the number in "Bedroom 2" (`T/types.py:3896-3917`). On custom-named rooms the meaning is unknown |
| `name` | Present only when `type` is 0 | Base64 of UTF-8 text |
| `material` | 1, 2 | Floor material: 0 none, 1 wood, 2 tile, 3 wood vertical, 4 wood horizontal, 5 medium-pile carpet, 6 low-pile carpet, 7 carpet (`T/types.py:824-832`) |
| `direction` | 0, 90 | Wood grain direction. Mirrored in `walls_info.floor_type` (section 8) |
| `nei_id` | Lists of room ids | Neighbour rooms. Symmetric on r2579a. Used for colouring (section 10) |
| `roomID` | Absent on r2579a | Unique room id on other models |

Display name rule: custom name if `type` is 0; else the type name, plus ` <index+1>` when
`index > 0`; else `Room <id>` (`T/types.py:3896-3905`).

### 7.5 Restrictions and zones (saved map)

| Key | Shape | Meaning | Source; r2579a |
|---|---|---|---|
| `vw.line` | `[[x0,y0,x1,y1], ...]` | Virtual walls | `T/map.py:5264-5321`; present, empty |
| `vw.rect` | `[[x0,y0,x1,y1], ...]` | No-go zones (axis-aligned) | same; present, empty |
| `vw.mop` | same | No-mop zones | same; present, empty |
| `vw.cliff` | lines | Not in Tasshack's `vw` parser | **Verified on r2579a**: present, empty |
| `vw.addcpt` | `[[x0,y0,x1,y1,id,ellipse,type, ...]]` | User-added carpet rectangles | `T/map.py:5297-5321`; **Verified on r2579a**: 7 fields, e.g. `[375, -375, 4575, 2625, 1, 0, 7]` |
| `vw.nocpt` | `[[x0,y0,x1,y1], ...]` | "Ignore carpet here" areas | same; present, empty |
| `vws.vwsl` | lines | Virtual thresholds | `T/map.py:5337-5369`; present, empty |
| `vws.npthrsd` | lines | Impassable thresholds | same; present, empty |
| `vws.ramp` | `[[x0,y0,x1,y1,angle]]` | Ramps | same; present, empty |
| `ct.line` (or `curtain.line`) | lines | Curtains | `T/map.py:5371-5383`; **Verified on r2579a** key is `ct` |
| `pointinfo` | `{"spoint":[[x,y,flag,?]],"tpoint":[...]}` | Cruise points | `T/map.py:5482-5520`; r2579a: `[]` |
| `sneak_areas` | `[{"id","type","hide","ms","area","roi":[...]}]` | Low-lying areas | `T/map.py:5443-5480`; live frame: `[]` |
| `ai_outborders_user` | list | Unknown (possibly user-confirmed AI boundaries) | Live frame: `[]` |
| `rec_vw`, `rec_vws` | same shapes plus `type` | Robot-recommended restrictions | `T/map.py:5337-5369`; not seen |
| `da2.areas`, `sp` | rectangles, points | Active zone or spot task | `T/map.py:4494-4511`; not seen |

Rectangles are axis-aligned. Tasshack normalises corner order (`T/map.py:5264-5283`).

### 7.6 Carpets

| Key | Shape | Source; r2579a |
|---|---|---|
| `carpet_polygon` | `{"<id>": [[x0,y0,x1,y1,...], type(, hidden)]}` | `T/map.py:5385-5420`. **Verified on r2579a**: 2 entries, each `[flat vertex list, 7]` (no hidden field); 35 and 77 vertices; float mm, multiples of 50 |
| `carpet_info` | older form | `T/map.py:5421-5441`; not on r2579a |
| `carpetcleanset` | `[[object_type, id, cleaning, prefs], ...]`; object_type 0 detected, 1 added, 2 room | `T/map.py:4524-4527`, `6570-6599`. **Verified on r2579a**: `[[0,1,-1,-1],[0,2,-1,-1],[1,1,-1,-1],[1,2,-1,-1]]` |

`carpet_polygon` holds robot-detected carpets ("detected", object type 0). `vw.addcpt` holds
user-added carpet rectangles (object type 1). The two id spaces are separate. On r2579a each user
rectangle covers one detected polygon: added id 1 covers detected id 2, added id 2 covers detected
id 1. **Verified on r2579a.** Polygon edges are 50 mm staircase steps that follow the pixel grid.
Carpet type codes: 5 medium pile, 6 low pile, 7 generic, 8 floor mat (`T/device.py:6701-6708`).

### 7.7 Obstacles

`ai_obstacle` (`T/map.py:5145-5234`): `[x, y, type, confidence, ...]`. Types are in
`T/types.py:2749-2784`; ignore status values in `T/types.py:2787-2793`.

**Verified on r2579a:** one entry, 14 string fields: x, y, type (`"139"`), confidence (`"0.91"`),
an epoch-seconds timestamp, an on-robot image path, a numeric id, four bounding-box fractions, and
three small integers (`"2"`, `"1"`, `"0"`). Map the last three to picture and ignore status only
after a test. `laser_obstacle` is present and empty.

### 7.8 Furniture

`funiture_info` (spelling as on the wire) is the saved-map furniture list for furniture version 2
(`T/map.py:5050-5079`; encoder `T/map.py:2484-2505`). **Verified on r2579a**: 9 entries, 15 fields
each.

| Index | Meaning | Source or evidence |
|---|---|---|
| 0 | Furniture id | Tasshack. All 9 entries on r2579a have id 0, so ids are not unique here |
| 1 | Type. Wire codes 8 and 25 are swapped against `FurnitureType` | `T/map.py:5056-5060`; types `T/types.py:2841-2872` |
| 2 | Segment id (room) | Tasshack; consistent with rooms on r2579a |
| 3 | Width, mm | Tasshack; matches `ai_furniture_user[6]` on r2579a |
| 4 | Height (depth), mm | Tasshack; matches `ai_furniture_user[7]` |
| 5 | Always 0 | Tasshack writes 0 |
| 6 | Centre x, mm (float) | Tasshack; matches `ai_furniture_user[0]` |
| 7 | Centre y, mm (float) | same, `[1]` |
| 8 | Always 0.0 | Tasshack writes 0 |
| 9 | Angle, degrees (0, 90, 180, 270 seen) | Tasshack |
| 10, 11 | Always 0.0 | Tasshack writes 0 |
| 12 | Scale (1.0 seen) | Tasshack |
| 13 | `edit_type` (1 on 8 entries, 0 on 1) | Tasshack; meaning unknown |
| 14 | `modified` (0 seen) | Tasshack |

Types seen on r2579a: 1 single bed, 2 double bed, 5 three-seat sofa, 6 dining table, 7 nightstand,
9 toilet, 18 TV cabinet. Type 7 also appears twice in one small room, so the user may use it as a
generic small object.

The live frame also carries `ai_furniture_user` (9 fields) and `ai_furniture` (8 fields):
`[cx, cy, type, edit_type, cx, cy, width, height(, angle)]`. On r2579a fields 4 and 5 repeat the
centre, not a corner, and the angle is 0 even where `funiture_info` says 90. Treat
`funiture_info` as authoritative. **Verified on r2579a.**

### 7.9 Other embedded data

| Key | Content | Source; r2579a |
|---|---|---|
| `rism` | Saved map envelope (encoding E) | `T/map.py:4773-4779`; present in the live frame |
| `whm` | Wi-Fi heat map envelope (W-frame) | `T/map.py:5005-5021`; present in the saved map (5,936 characters) |
| `whmp` | Router position `[x, y]` | same; present |
| `decmap` | Cleaned/dirty overlay | `T/map.py:5543-5550`; not seen |

---

## 8. Floor plan (`walls_info`)

### 8.1 Structure

**Verified on r2579a** (object form; Tasshack also accepts a compact array form,
`T/map.py:5613-5735`):

```json
{
  "version_flag": 3,
  "storeys": [{
    "rooms": [{
      "room_id": 9,
      "floor_type": 3,
      "walls": [
        {"type": 1, "beg_pt_x": -3275, "beg_pt_y": 7025, "end_pt_x": -5575, "end_pt_y": 7025, "normal_x": 0.0, "normal_y": -1.0},
        {"type": 1, "beg_pt_x": -5575, "beg_pt_y": 7025, "end_pt_x": -5575, "end_pt_y": 8325, "normal_x": -1.0, "normal_y": 0.0},
        {"type": 1, "beg_pt_x": -5575, "beg_pt_y": 8325, "end_pt_x": -3275, "end_pt_y": 8325, "normal_x": 0.0, "normal_y": 1.0},
        {"type": 0, "beg_pt_x": -3275, "beg_pt_y": 8325, "end_pt_x": -3275, "end_pt_y": 7025, "normal_x": 1.0, "normal_y": 0.0}
      ]
    }],
    "doors": [
      {"door_type": 0, "door_id": 3, "nei_door_id": 2, "door_status": 0,
       "beg_pt_x": -1125, "beg_pt_y": 6625, "end_pt_x": -1125, "end_pt_y": 6825}
    ]
  }]
}
```

Facts measured on the r2579a saved map (**Verified on r2579a**):

- One storey. 13 rooms (all segments, including the two hidden ones). 108 walls. 17 doors.
- **Each room is one closed polygon.** The end point of wall i equals the start point of wall i+1,
  and the last wall closes on the first. Rooms have 4 to 18 walls.
- **All walls are axis-aligned** (no diagonal walls). Coordinates end in 25 or 75: they sit on pixel
  centres of the 50 mm grid (`left + 25 + 50k`).
- **Normals point out of the room**, as unit vectors. 20 walls have zero length (start equals end)
  and normal `(0, 0)`; all of them are type 0. Treat them as polygon vertices that mark a junction.
- **`type`**: 0 or 1. 52 of 72 type-0 walls overlap a wall of another room; 30 of 36 type-1 walls
  overlap none. So type 0 is most likely an interior wall (shared with a neighbour room or holding
  a door), and type 1 an exterior wall. This reading is an inference.
- **`floor_type`** mirrors `seg_inf` material and direction: tile (`material` 2) gives 2; wood with
  `direction` 90 gives 3; wood with `direction` 0 gives 4. This matches Tasshack's encoder for the
  old wall format, which writes 3 for wood at 90 degrees and 4 otherwise (`T/map.py:3359-3375`).
- **Doors**: all 17 have `door_type` 0 and `door_status` 0. `door_id` and `nei_door_id` are the two
  room ids the door connects (for example 2 and 1), not door ids. Every pair matches a `nei_id`
  pair in `seg_inf`. Two rooms can share several door entries (one per opening). Each door is an
  axis-aligned segment on the shared wall line. Tasshack reads `door_id` as an id and ignores
  `nei_door_id` (`T/map.py:5701-5714`).
- No edited copies (`_beg_pt_x` and so on) are present. Tasshack reads them when present
  (`T/map.py:5659-5662`, `T/map.py:5711-5714`).

### 8.2 What the floor plan implies for editing

- The floor plan is a clean vector model of the rooms. It is the best base for an editor's room
  outlines, labels and hit-testing. It is much nicer to draw than the pixel boundary.
- Tasshack offers a wall editor (`set_walls`, `T/map.py:3264-3450`; `T/device.py:6849-6884`). It
  can only move the end points of existing walls and doors. The number of walls per room must stay
  the same (`T/map.py:3273`). With `WALLS_V3` it sends only the changed walls, as pairs of original
  and new end points (section 11.2, `wallInfoV3`).
- r2579a has `WALLS_V3`, so this command is likely accepted. Its effect is not known. It may change
  only the drawn floor plan (the app's tidy map view), or it may also move room boundaries used for
  navigation. Experiment W6 in section 16 answers this.
- Walls cannot be added or deleted, and rooms cannot be created from a polygon, through any known
  command.
- After a split or merge the robot rebuilds `walls_info`. Do not cache wall indexes across
  structural edits.

---

## 9. Coordinates and transforms

- Pixel to world: `x = left + col * grid`, `y = top + row * grid` gives the pixel **centre**:
  the header origin is the centre of pixel (0, 0). The official app subtracts `grid/2` to get the
  cell corner ([map-plugin.md](map-plugin.md) §4.1), and user carpet corners on r2579a lie on that
  corner lattice. Tasshack adds `grid/2` for the centre (`T/types.py:4408-4420`,
  `T/map.py:6286-6291`), which is half a cell off.
- World to image with Y up (image row 0 at the top of the screen):
  `img_x = (x - left) / grid`, `img_y = height - 1 - (y - top) / grid` (`T/types.py:4422-4438`).
  The renderer in `crates/dreame-map` draws this way; the output is plausible.
- World axes: +x right, +y up on screen. Robot angle and charger angle are in degrees.
- Display rotation (`mra`, map-list `angle`, edit `smra`) applies to the final image only. Stored
  coordinates are never rotated (`T/map.py:9543-9548`).
- All edit commands take world mm in the same frame. The editor must invert its view transform
  (scale, rotation, Y flip) before sending.
- Valetudo coordinates, for export: `vx = round((x + 32768) / 10)`,
  `vy = 6554 - round((y + 32768) / 10)` (`T/map.py:6834-6861`; V: `DreameMapParser.js`).

Coordinate ranges seen on r2579a: saved map x from -5950 to 5300, y from -6900 to 10550.
**Verified on r2579a.**

---

## 10. Rendering guidance

Tasshack's pipeline (`DreameVacuumMapRenderer.render_map`, `T/map.py:9049`) is a good model:

1. Scale: 4 for saved maps, 3 for live maps, 2 for Wi-Fi or history maps (`T/map.py:9086-9097`).
2. Base image: one pixel per grid cell, Y flipped (`T/map.py:9338-9352`).
3. Colours: wall, floor, new segment; one colour per room by colour index; hidden rooms in a muted
   colour; boundary pixels (class 101..163) in the wall colour (`T/map.py:9244-9300`).
4. Floor material textures and carpet hatching (`T/map.py:12368`, `T/map.py:12466`).
5. Overlays in this order (`T/types.py:5402-5428`): path, no-mop, no-go, virtual walls, thresholds,
   ramps, curtains, low-lying areas, furniture, active areas and points, room labels and icons,
   charger, robot, router, obstacles, cruise points.
6. Rotate by the map rotation; counter-rotate icons and text (`T/map.py:9543-9548`).

Room colour index: greedy four-colour graph colouring over `nei_id`, largest room first
(`T/map.py:6617-6686`). Default palette "Dreame Light" (`T/types.py:5206-5254`): floor
`(221,221,221)`, wall `(159,159,159)`, new segment `(153,191,255)`, room fills `(171,199,248)`,
`(249,224,125)`, `(184,227,255)`, `(184,217,141)`. No-go fill `(177,0,0,50)` outline
`(199,0,0,200)`; no-mop `(170,47,255,50)` outline `(153,0,210,200)`.

Label point: for saved maps, the middle of the longest run of room pixels on the bounding-box centre
row, then column (`T/map.py:4063-4111`). With the floor plan available, a polygon pole of
inaccessibility over `walls_info` gives better labels.

Recommendations for the desktop renderer:

- Render in vector space, not by scaling a bitmap. Draw the pixel grid as a texture with
  nearest-neighbour sampling, and draw the floor plan, restrictions, carpets and furniture as
  vector shapes in world mm on top.
- Offer three base layers: pixel grid (truth used by the robot), floor plan polygons
  (`walls_info`), and the live overlay (`rism` plus live pixels, path, robot, obstacles).
- Draw `carpet_polygon` as polygons and `vw.addcpt` as rectangles, or as ellipses when the
  ellipse flag is 1. An ellipse entry is `[cx, cy, sx, sy]` (centre and size), not two corners
  ([map-plugin.md](map-plugin.md) §6.3). Hatch carpet pixels from the grid as a fallback.
- Draw furniture as rotated rectangles of `width x height` at the centre, rotated by the angle.
  Tasshack pastes PNG images (`T/map.py:12202-12322`); simple outlines with an icon are enough.
- Keep a single world-to-screen transform object and use it for both drawing and hit-testing.

---

## 11. Edit commands

### 11.1 Transport

Every edit is action 6.2 (`UPDATE_MAP_DATA`) with one input, piid 4 (`T/device.py:6480-6552`):

```json
{"did": "<did>", "siid": 6, "aiid": 2,
 "in": [{"piid": 4, "value": "<compact JSON string>"}]}
```

- The value is a JSON object serialised without spaces.
- Several edits can go in one action as several `in` entries (`T/device.py:6487-6493`).
- The reply `code 0` only means the robot received the command. Success is visible only when a new
  frame, or a new 6.8 md5, shows the change (open question 5).
- After each edit, request a new map (`{"frame_type":"I"}`) and re-read 6.8 (`T/device.py:6508-6518`).
- Most edits are refused while the robot is cleaning, while a temporary map exists, or without a
  saved map. r2579a lacks `MAP_EDIT_WHILE_RUNNING`.
- `mapid` names a saved map. Tasshack first selects the map when it is not the selected one
  (`set_selected_map`, for example `T/device.py:7109-7111`). Assume the robot edits the selected map
  only. On r2579a there is one map, `mapid` 1.
- Valetudo uses the same action and piid (V: `capabilities/DreameMapSegmentEditCapability.js`).

### 11.2 Command table

"r2579a" column: **Yes (key)** means the data key is present in the r2579a saved map and the
capability is present. **Likely** means the capability is present but the command is untested.
**Unlikely** means the capability is absent. No command has been sent to r2579a yet.

| Operation | Value of piid 4 | Source | r2579a | Notes |
|---|---|---|---|---|
| Select map | `{"sm":{},"mapid":M}` | `T/device.py:6886-6894` | Likely | Only one map while multi-floor is off |
| Delete map | `{"cm":{},"mapid":M}` | `T/device.py:6896-6929` | Likely | **Destructive.** With multi-floor off, deleting any map deletes all (`T/device.py:6915-6916`) |
| Rename map | `{"nrism":{"<M>":{"name":"<text>"}}}` | `T/device.py:6554-6567` | Likely | Plain text, not base64. `null` clears |
| Rotate map | `{"smra":{"<M>":{"ra":0\|90\|180\|270}}}` | `T/device.py:6569-6587` | Likely (`mra`, `angle` present) | Display only, stored on the robot |
| Temporary map: keep | `{"cw":5}` | `T/device.py:6931-6936` | Likely | After a new mapping run |
| Temporary map: discard | `{"cw":0}` | `T/device.py:6938-6943` | Likely | |
| Temporary map: replace map M | `{"cw":1,"mapid":M}` | `T/device.py:6945-6953` | Likely | |
| Merge rooms (**verified write**, 2026-11-16) | `{"msr":[A,B],"mapid":M}` | `T/device.py:7093-7116`; V sends `{"msr":[A,B]}` | **Yes** | Two neighbour rooms per call. On r2579a room A kept its id, type, order and settings; B disappeared ([l40-ultra-ae.md](l40-ultra-ae.md)) |
| Split room (**verified write**, 2026-11-16) | `{"dsrid":[x0,y0,x1,y1,id],"mapid":M}` | `T/device.py:7118-7145`; V sends `{"dsr":[x0,y0,x1,y1],"dsrid":[x0,y0,x1,y1,id]}` | **Yes** (`dsrid` alone) | Line end points in mm. Tasshack refuses at 49 rooms when not `MAP_V2`. On r2579a the room kept its id and metadata; the new half got the next free id, no name or type, the last order position and default settings |
| Rename or retype rooms | `{"nsr":{"<id>":{"name":"<base64 utf-8>","type":0,"index":0}, "<id2>":{"type":T,"index":I}},"mapid":M}` | `T/map.py:3197-3213`, `T/device.py:7687-7739` | **Yes (verified write, 2026-11-16)** | Send each custom-named room's stored `index`, not 0: r2579a stores what is sent ([l40-ultra-ae.md](l40-ultra-ae.md)). Tasshack sends every named or typed room, not only the changed one, and never sends `autonsr` (`T/device.py:7713-7722`). The official app does send `autonsr:true` with a per-room `roomID` in its auto-naming flow (RN plugin; see [map-plugin.md](map-plugin.md) §6.1) |
| Hide rooms (**verified write**, 2026-11-16; hiding drops the room from `cleanareaorder`, showing restores its position) | `{"delsr":[ids],"mapid":M}` | `T/device.py:7623-7653`, `7907-7912` | **Yes (key)** | Full list of hidden ids |
| Cleaning order v1 | `{"cleanOrder":[ids]}` | `T/device.py:7194-7198` | Unlikely | |
| Cleaning order v2 (**verified write**, 2026-11-16) | `{"cleanareaorder":[{"<id>":1},{"<id>":2}, ...]}` | `T/device.py:7194-7198`, `7741-7749` | **Yes (key)**; `ulmm` names it | Cannot be disabled |
| Per-room settings (**verified write**, 2026-11-16; result in the live frame only) | `{"customeClean":[[id,suction,water,times,cleanType,mopEffect,mopTemp], ...]}` | `T/device.py:7200-7206`, `T/map.py:3141-3180` | Likely (`cleanset` present) | Order is not sent. The last three fields are optional, by capability (RN plugin; [map-plugin.md](map-plugin.md) §6.2). See 11.4 |
| Mop pad per room | `{"RoomMopType":{"<id>":1\|2\|3}}` | `T/device.py:7655-7685` | Unlikely | No `room_id_type` key |
| Floor material | `{"nsm":{"<id>":{"material":m,"direction":0\|90}},"mapid":M}` | `T/device.py:7551-7621`, `T/map.py:3182-3195` | Likely | `MULTI_SEGMENT_FLOOR_MATERIAL` is active (firmware 1664 is at least 1623), so several rooms may go in one entry. Do not add `carpetmaterial.roomcpt` (no `CARPET_MATERIAL`) |
| Restrictions (no-go add and remove **verified write**, 2026-11-16; r2579a stores `rect` entries with a fifth field, 0) | `{"vw":{"line":[[x0,y0,x1,y1]],"rect":[[x0,y0,x1,y1]],"mop":[[x0,y0,x1,y1]]}}` | `T/device.py:6589-6606` | **Yes (key)** | Add `"temp":{}` only when not on a saved map. The official app resends every stored `vw` key (`cliff`, `addcpt`, `nocpt` and others), so do the same ([map-plugin.md](map-plugin.md) §6.3) |
| Carpet areas (add and remove of a carpet and of an ignored-carpet area **verified writes**, 2026-11-16; stored as sent, no extra fields) | `{"cpt":{"addcpt":[[x0,y0,x1,y1,id,ellipse]],"nocpt":[[x0,y0,x1,y1]]}}` | `T/device.py:6608-6647`, `T/map.py:2103-2242` | **Yes** | Replaces both lists. `CARPET_ID` present: keep existing ids, give new carpets the lowest free id. Tasshack sends 6 fields and drops the 7th, type (`T/map.py:2211-2212`); the official app resends every stored field, so do that. Ellipse entries are `[cx, cy, sx, sy]`. At most 20 carpets ([map-plugin.md](map-plugin.md) §6.3). No `carpetmaterial` or `carpettassel` follow-up (no `CARPET_TYPE`, no `CARPET_TASSEL`) |
| Carpet type | `{"carpetmaterial":{"origincpt":[[id,t]],"addcpt":[[id,t]],"roomcpt":[[id,t]]}}` | `T/device.py:6649-6730` | Unlikely | |
| Carpet cleaning | `{"carpetcleanset":[[object_type,id,setting(,prefs)]]}` | `T/device.py:7429-7549` | Likely (key present) | Setting -1..7. Prefs bits: 1 first, 2 boost, 4 intensive, 8 side brush, 16 lift chassis |
| Virtual thresholds | `{"vws":{"vwsl":[[x0,y0,x1,y1]]}}` | `T/device.py:6732-6754` | Likely | |
| Thresholds and ramps (add and remove of a threshold and of an impassable threshold **verified writes**, 2026-11-16; stored as sent; ramps untested) | `{"vws":{"vwsl":[...],"npthrsd":[...],"ramp":[[x0,y0,x1,y1,angle]]}}` | `T/device.py:6756-6778` | **Yes** (ramps: Likely) | Replaces all three lists |
| Curtains (add and remove **verified write**, 2026-11-16) | `{"curtain":{"line":[[x0,y0,x1,y1]]}}` | `T/device.py:6794-6806` | **Yes** | Stored as `ct.line`, as sent |
| Furniture (move, add and remove **verified writes**, 2026-11-16) | `{"mapfurniture_v2":[[id,type,seg,w,h,0,x,y,0,angle,0,0,scale,edit_type,modified], ...]}` | `T/map.py:2428-2505`, `T/device.py:6780-6792` | **Yes** | Full list, stored as sent. Resend every stored field. Swap types 8 and 25 only when converting from Tasshack's `FurnitureType`. r2579a accepted a new item with id 0, like its stored ones |
| Floor plan walls and doors | `{"wallInfoV3":[{"i":room,"w":[[ix0,iy0,ix1,iy1,x0,y0,x1,y1], ...]}],"doorInfoV3":[[ix0,iy0,ix1,iy1,x0,y0,x1,y1], ...]}` | `T/map.py:3326-3350`, `T/device.py:6849-6884` | Likely (`WALLS_V3`) | `ix*` are the original end points, `x*` the new ones. Only changed walls and doors. Effect unknown (section 8.2) |
| Floor plan, old format | `{"wallInfoV2":{"fixVersion":1,"version_flag":v,"storeys":[{"r":[{"i":id,"f":floor_type,"w":[...]}],"d":[...]}]}}` | `T/map.py:3352-3447` | Unlikely | For models without `WALLS_V3` |
| Cruise points | `{"spoint":[[x,y,0,1], ...],"tpoint":[]}` (at most 20) | `T/device.py:6822-6847` | Likely (`pointinfo` present) | Camera robots |
| Obstacle ignore | `{"obstacleignore":[x,y,type,0\|1]}` | `T/device.py:6402-6449` | Likely | The key-based form is for `MAP_V2` only |
| Router position | `{"wrp":[x,y]}` | `T/device.py:6451-6460` | Likely (`whmp` present) | Wi-Fi map |
| Low-lying areas | Unknown | `T/device.py:6808-6820` raises "not supported" | Unlikely | No `LOW_LYING_AREAS` |

### 11.3 Other map operations

| Operation | Command | Source | r2579a |
|---|---|---|---|
| Request a frame | Action 6.1, see section 2.3 | `T/map.py:281-300` | Likely; harmless |
| Back up map | Action 6.3, `in:[{"piid":4,"value":"<map_id>"}]`; watch 6.14 | `T/device.py:7049-7091` | Unknown. Tasshack enables it only when 6.14 exists (`T/types.py:3312`) |
| Restore map | Set 6.10 to `{"map_id":M,"map_url":"<signed URL of the recovery archive>"}`; watch 6.11 | `T/device.py:6955-7047` | Unknown (6.10 and 6.11 not in cache) |
| Multi-floor on/off | Set 6.7 (bool). Tasshack blocks it while a temporary map exists | `T/types.py:1680`, `T/types.py:2028` | Likely; current value 0 |
| New map (fast mapping) | Action 4.1, piid 1 = 21 (`FAST_MAPPING`) | `T/device.py:5332-5366` | Likely |

### 11.4 Per-room settings

`cleanset` arrives as `{"<id>": [suction, water, times, order, cleanType, mopEffect, mopTemp]}`.
`customeClean` sends `[id, suction, water, times, cleanType, mopEffect, mopTemp]` without order
(`T/map.py:3141-3180`; names from the RN plugin, [map-plugin.md](map-plugin.md) §6.2). Tasshack
calls `cleanType` "mode" (0 sweep, 1 mop, 2 both) and `mopEffect` "mopping". The app sends
`cleanType`, `mopEffect` and `mopTemp` only when the robot supports each; r2579a stores six
fields, so it gets no `mopTemp`.

- suction 0..3; times 1..3; mode 0..2 (`T/device.py:7234-7252`).
- water: wetness level 1..32 with `WETNESS_LEVEL` (present on r2579a) (`T/map.py:6451-6504`).
- mopping settings: `route | (water << 4) | (mode << 8)` (`T/map.py:6601-6615`). r2579a value 546
  (`0x222`) decodes to route 2, water 2, mode 2.

**Verified on r2579a:** each `cleanset` entry has 6 numbers, and index 3 equals the room's position
in `cleanareaorder`. `cleanset` is only in the live frame (6.3), not in the saved map. A live
`customeClean` write (2026-11-16) changed only the live frame, and the robot kept the order field
it does not carry ([l40-ultra-ae.md](l40-ultra-ae.md)).

### 11.5 Cleaning commands that use map geometry

Action 4.1 with `[{"piid":1,"value":<status>},{"piid":10,"value":"<JSON>"}]`
(`T/device.py:4596-4619`). These do not change the map but an editor will want them.

| Mode | Status | piid 10 value | Source |
|---|---|---|---|
| Rooms | 18 | `{"selects":[[id, repeat, suction, water, order], ...]}` | `T/device.py:4950-4980` |
| Zones | 19 | `{"areas":[[x0,y0,x1,y1,repeat,suction,water], ...]}` | `T/device.py:4840-4878` |
| Spots | 20 | `{"points":[[x,y,repeat,suction,water], ...]}` | `T/device.py:5020-5093` |
| Go to (camera robots) | cruising | `{"tpoint":[[x,y,0,0]]}` | `T/device.py:5120-5140` |

---

## 12. Backup, recovery and restore

Three mechanisms keep old map versions. §12.3 sums up how a restored archive edits rooms;
§12.2 is the experiment log behind it.



1. **Automatic recovery points.** The robot lists earlier versions of each saved map in the 6.9
   recovery list. `first` marks each entry as edited (0), original (1) or backup (2)
   (`T/types.py:4626-4630`). This works as an undo history kept by the robot.
2. **Manual backup.** Action 6.3 with the `map_id` uploads a backup archive (`*mb.tbz2`) and adds a
   type-2 entry to the recovery list. Status in 6.14 (`T/device.py:7049-7091`).
3. **Restore.** Setting 6.10 to `{"map_id":M,"map_url":"<url>"}` makes the robot download the
   archive from the URL and replace map M. Status in 6.11. The URL is a signed download URL for the
   entry's `objname` (`T/device.py:6955-7047`). Tasshack also exposes a service that restores from
   any URL (`restore_map_from_file`, `T/device.py:6955`; `T/vacuum.py`).

Rules from Tasshack: no restore while cleaning, while a temporary map exists, or while a restore is
running. Recovery archives come from the interim URL endpoint on Dreamehome
(`T/map.py:1400-1403`).

How the official r2579a plugin restores (read 2026-11-16, see [map-plugin.md](map-plugin.md) §8):

- The recovery page fetches a signed URL for the entry's `objname`. For a manual backup
  (`objname` contains `mb.tbz2`) it calls `getFileUrl`; for the automatic entries it calls
  `getInterimFileUrl`. These are probably `getOss1dDownloadUrl` and `getDownloadUrl`, matching
  Tasshack's split.
- It writes 6.10 as the JSON text `{"map_id":M,"map_url":"<signed url>"}`, then reads 6.11 every
  3 s. 3 is success; 4 and 5 are failure. On success it asks for a new I-frame.
- The app refuses a restore while the robot cleans or returns to the dock.
- A second, cloud-side path exists: `POST /dreame-user-iot/iotuserbind/restoreMap` with
  `{"model", "optionalModels"}` returns map archives kept by the cloud, for example from a
  previous robot. The app offers them when the robot's map is empty, and restores them through
  the same 6.10 write.

**The robot accepts any URL.** Tasshack's `vacuum_restore_map_from_file` service takes any URL,
and its documented example is a plain HTTP address on the local network
(`T/services.yaml:293-300`). Nothing in the plugin or in Tasshack signs or checks the URL host.
**Verified on r2579a** (2026-11-16, [l40-ultra-ae.md](l40-ultra-ae.md)): the robot downloaded an
archive from a plain HTTP server on this computer with one `GET` (`Wget/1.20.3 (linux-gnu)`, no
range) and restored it. So a modified archive does not need Dreame storage: the client serves it
over HTTP on the local network for the duration of the restore (`dreame map restore-file`).

r2579a status, **verified on r2579a** (2026-11-15):

- The recovery list downloads and holds an original and an edited version of map 1 (§2.6).
- 6.11 `MAP_RECOVERY_STATUS` = 0 and 6.14 `MAP_BACKUP_STATUS` = 0 on a relayed read. Both are
  missing from the cloud cache, so the cache is not a reliable presence test.
- 6.10 `MAP_RECOVERY` returns code -1 on a relayed read. It is write-only.
- `QUICK_MAP_RECOVERY_V2` is in the capability set.

So the backup and restore machinery exists on this robot.

### 12.1 Recovery archive contents (verified on r2579a, 2026-11-15)

The `objname` of each recovery entry downloads through `iotfile/getDownloadUrl` like any other
object. The entries examined were the original
(120,216 bytes) and the edited version (299,666 bytes). Both are **bzip2-compressed tar archives**
(`.tbz2`) with no signature file and no checksum list. Paths are relative to the robot's map store.
The edited archive holds 51 entries:

| Path | Format | Content |
|---|---|---|
| `1/app_map.bin` | binary, starts with four little-endian u32 (`0x7a4`, `0x78c`, `0x132`, `0xe2`) | probably the app map frame source; not decoded |
| `1/fine_large.bin` | binary, starts `aa bb 00 0a` | fine (high resolution) map; not decoded |
| `1/wifi_fine.bin` | binary | Wi-Fi heat map |
| `1/map_status.json` | JSON | `{"map_used_times":139,"map_saved_times":139,"map_replaced_times":0}` |
| `1/docker_pose_large.json` | JSON | dock pose `{x, y, theta}` in metres and radians |
| `1.dat2` | JSON, 60 keys | the saved map's master record (below) |
| `ai/1/ai_rooms_large.txt` | text, one room per line: `id type 1 <base64 name>` | AI room types and names |
| `ai/1/ai_floors_large.txt`, `ai/1/ai_furnitures_large.txt` | space-separated numbers | AI floor material and furniture detections |
| `divide/1/slam_map.png` | 8-bit grey PNG, 223 x 347 | **occupancy grid**. Values: 0, 20, 99, 255 (4 classes) |
| `divide/1/slam_map_ori.png`, `origin_img_slam.png`, `ori_slam_map.png` | grey PNG | earlier or raw versions of the grid (`ori_slam_map.png` has about 200 grey levels: a probability map) |
| `divide/1/segmented_map_large.png`, `dec_map_large.png`, `last_seg_map_large.png` | grey PNG, 223 x 347 | **room segmentation**: pixel value = room id (0, 1-13) |
| `divide/1/segmented_map_color.png` | RGB PNG | the same with colours and room numbers drawn in, for display |
| `divide/1/state_map.png`, `collision_map.png`, `contours_with_obstacle_large.png`, `segmented_map_with_obstacle_large.png` | grey PNG | navigation state, collisions, contours |
| `divide/1/map_roi_large.json` | JSON | `{x_min, x_max, y_min, y_max, width: 3600, height: 3600}`: the PNGs are a crop of a 3600 x 3600 grid |
| `divide/1/out_walls_large.json` | JSON | identical in shape to `walls_info` in the map trailer (§8) |
| `divide/1/room_neighbor_table_large.json` | JSON | `RoomIds`, and per room `room_neighbor_id`, `is_user_edited`, `rec_hide_type` |
| `divide/1/hide_rooms_large.json` | JSON | `{"HideRooms":[...]}`, matches `delsr` |
| `divide/1/all_doors_info.json`, `unified_doors_info.json`, `doors_info_large.json`, `vision_doors_info.json`, `laser_steps_info.json`, `step_infos.json` | JSON | doors and thresholds as grid segments (`beg_pt`, `end_pt`, `door_type`, `normal`) |
| `divide/1/carpet_map_large.json` | JSON | detected carpets: `id`, `father_id`, `user_edited`, `carpet_type`, `material_type`, bounding box, `polylines` in grid cells |
| `divide/1/user_set_carpet_large.json` | JSON | user carpets as `rect {tlx, tly, brx, bry}` in mm; matches `vw.addcpt` |
| `divide/1/laser_obstacle.json`, `carpet_path_large.txt`, `LDS_furniture_.txt`, `ori_instance_label.txt` | text | obstacle cells, carpet path cells, lidar furniture, segmentation scores |
| `divide/1/mirror_lines.json`, `sneak_area.json`, `sneak_path.txt`, `empty_obs.txt`, `user_confirm.txt` | small JSON or text | mirrors, low "sneak" areas, user confirmation flag |

`1.dat2` keys include: `yxmap_version` (`NEWMV001`), `yxmap_value` (base64 of a map that starts
with the ASCII tag `NEWMV001_RI_V001`), `seg_info` (same as `seg_inf`), `cleanarea_order`,
`virtual_wall_lines`, `virtual_wall_rects`, `virtual_wall_mop_rects`, `curtain_lines`,
`unpassable_step_lines`, `virtual_step_lines`, `ramp_rects`, the carpet reuse lists, `charger_pose`,
`last_app_map` (a zlib envelope), `encoded_whm`, `history_decomposed_map`, `history_trace_map`,
`history_cliff_map` (all `NEWMV001` maps), `gridmap_limit`, and `extend_msgs` (a 365 KB list of
serialised C++ objects such as `reuse_trapped_map`).

What this means for editing:

- The robot's map store is plain files: PNG grids, JSON and a few binaries. A modified archive
  could carry a repainted occupancy grid (`slam_map.png`) and a redrawn room segmentation
  (`segmented_map_large.png`). This is the only known route to the edits that no command offers.
- The same facts appear in several files (walls in `out_walls_large.json` and `1.dat2`, rooms in
  three PNGs and several JSON files, the `NEWMV001` maps in `1.dat2`). An edit would have to keep all
  of them consistent. Which file the robot treats as the source of truth on restore is unknown.
- No signature or checksum was found inside the archive. The robot may still check it elsewhere,
  for example through the md5 in 6.8 or a cloud record.
- The restore URL can be any HTTP URL the robot can reach, including one on the local network
  (§12, verified on r2579a).
- This stays research only. A test needs the user's consent, a fresh backup (action 6.3), and a
  tested restore of the unmodified archive first. Both now exist (W1, W8, W8b in §16.2).

### 12.2 How the archive files relate (read-only study, 2026-11-16)

Archives compared: the original (April), the edited entry downloaded on 2026-11-15, and the
backup made on 2026-11-16. Copies stay in `local/out/`. The comparison script read PNGs with a
small pure-Python decoder and printed counts only.

**Coordinates.** The PNG grids are crops of a 3600 x 3600 grid of 50 mm cells whose centre cell
(1800, 1800) is world (0, 0). `map_roi_large.json` gives the crop. The saved map (6.8) uses the
same cells with one extra cell on every side: on r2579a the crop starts at big-grid
(1682, 1663) and the saved map's column 0 and row 0 are big-grid 1681 and 1662. Rows are not
flipped: PNG row 0 is the saved map's row 1, the minimum y.

**Room segmentation.** `segmented_map_large.png`, `dec_map_large.png` and `last_seg_map_large.png`
are identical (pixel = room id, 0 outside). They match the saved map's segment bits (encoding E,
bits 5-0) in 75,172 of 77,381 cells. All 2,209 other cells are wall cells (bit 7) of the saved
map, which carry a room id in the saved map and 0 in the PNG. So the saved map is the
segmentation plus a wall layer.

**Occupancy.** `slam_map.png` holds four values: 0 free floor, 20 carpet (3,283 of its 3,332
cells are set in `carpet_map_bit`, below), 99 obstacle (most saved-map wall cells) and 255
unknown (outside).
`slam_map_ori.png` and `origin_img_slam.png` hold the same values in a crop 45 rows shorter, so the
robot keeps at least three copies of the grid. `ori_slam_map.png` and `ori_instance_sgm_map.png`
did not change between November 15 and 16, so they are an older baseline, not the working map.

**Binary maps.** `1/app_map.bin` is 16 bytes of header (four little-endian `i32`: 1956, 1932, 305,
226) and a 226 x 305 byte raster with only the values 0 and 255. Its 28,793 cells of 255 equal the number of free cells (0 or 20) in `slam_map.png`, so it is a free-floor mask, not a room layout. `1/fine_large.bin` and `1/wifi_fine.bin` start with the magic
`aa bb` and are not decoded. `gridmap_limit` in `1.dat2` is a big-grid box (1678, 1702, 1908,
2011), and the `extend_msgs` values `true_w_min`, `true_w_max`, `true_h_min`, `true_h_max` (1350,
2250) bound the usable big grid.

**Tiled maps (`NEWMV001`).** Decoded on r2579a with a one-off script. The firmware's
C++ type, stored next to some of them, is `shared_ptr<YXMapU8_<unsigned char, 32, 64>>`: a byte
map in 32 x 32 cell tiles on a grid 64 tiles wide. The value is URL-safe base64 of:

| Bytes | Content |
|---|---|
| 16 | ASCII `NEWMV001_RI_V001` |
| 4 x 5 | little-endian `i32`: tile count, then x max, y min, y max, x min of the used cells, in big-grid cells (an empty map has count 0 and 1800 four times) |
| 1 | `01` |
| per tile | `u16` index, then 1,024 bytes, row-major. The tile's first cell is big-grid (32 × (index / 64), 32 × (index % 64)) as (x, y) |

The maps share the PNGs' big grid with no offset. In `1.dat2`: `yxmap_value`,
`history_decomposed_map`, `history_trace_map`, `history_cliff_map`; in `extend_msgs`:
`reuse_laser_map`, `reuse_obs`, `reuse_lds_height_map`, `reuse_trapped_map`, `carpet_map`,
`carpet_map_bit` and `obstacle_map`. What they hold, compared with the PNGs:

| Map | Values | Reading |
|---|---|---|
| `history_decomposed_map` | room ids 1..12 | Room ids equal the PNG's in 27,371 cells and differ in 461, but 7,153 room cells are empty and the hidden rooms 9 and 13 are missing. A per-room cleaning history, not the full room layout |
| `yxmap_value` | 0, 16 (and a few 1, 2) | 16 on part of the free floor: probably cleaned area |
| `history_trace_map` | mostly 0, 1 | 1 along cleaned floor: a trace |
| `carpet_map_bit` | 0, 1 | 1 on carpet (slam value 20) |
| `carpet_map` | 0, 1, 100, 255 and others | Carpet detection levels |
| `reuse_lds_height_map` | 0, 1, 3 | 1 on almost all mapped floor |
| `reuse_laser_map`, `reuse_obs`, `obstacle_map` | sparse | Laser hits and obstacles |
| `history_cliff_map`, `reuse_trapped_map` | empty | |

None of the tiled maps holds the complete room layout. The other `extend_msgs` entries are
small: `door_reuse_info` and `retry_area_list` (binary strings), `save_bridge` (text
`x_y_x_y_n;` segments), and flags such as `new_seg_map` and `sgm_confirm_flag`.

**`last_app_map` is the last live frame**, not the saved map: map session 3, frame 99, 185 x 296,
with `cleanset`, `ris` and `rism`. After a restore the robot did not take the per-room settings
from it (§12, [l40-ultra-ae.md](l40-ultra-ae.md)).

**What changes when the robot re-maps.** Between the November 15 and 16 archives the user's edits
were all undone, but the robot cleaned twice. The PNG grids, walls, doors, carpets detected by the
robot, `1.dat2` and all three binaries changed. Room names, AI rooms and furniture, hidden rooms
and user carpets did not.

**No checksums.** No MD5, SHA-1, SHA-256 or CRC-32 of any member appears anywhere in the archive.
The tar is GNU format, owned by root (0/0), members `0644`, directories `0750`, every mtime set to
the backup time.

**The live frame after a restore does not show a rebuilt map.** After a restore the robot uploads
a live frame whose `rism` equals the 6.8 saved map. After W9b that `rism` was byte for byte the
`rism` inside the archive's own `last_app_map` (§12.1). The rest of that live frame differs from
`last_app_map` only in volatile keys (`cleanset`, `cs`, `mra`, `mtid`, `robot_mode`,
`timestamp_ms`, `ulmm` and two AI keys). So the robot most likely copies `last_app_map` back, and
the `rism` says nothing about how it read the other members. W9a and W9b used this `rism` as their
only check, so both are **inconclusive** (§16.2). A check that works must make the robot save
the map again, so that 6.8 is rebuilt from its own state.

**Which member holds the room layout is open.** W9a (segmentation PNGs) and W9b (one door line)
showed nothing for the reason above. W9c then made the robot save the map again after W9b: the
new 6.8 still had the old border, although the robot kept our shifted door line in its own next
archive. So the door lines do not drive the saved map. The decoded tiled maps hold no complete
room layout. W9d edited only the saved map inside `last_app_map` (its `rism`) and forced a
re-save: 6.8 again had the old layout. So that copy does not drive it either. The candidates left
are the segmentation PNGs (W9a was never re-saved) and the binaries (`fine_large.bin`,
`app_map.bin`). It is also possible that a re-save after a restore only re-uploads a map the
robot keeps elsewhere, so that no archive member changes the room layout on a restore.

**Testing the third possibility (W10).** The robot keeps its working map either in the archive
members or somewhere it does not upload. Facts so far:

- After every restore the robot's next archive held the members we sent (W9c: our shifted door
  line, untouched PNGs, `1.dat2` changed only in names, `last_app_map` and a counter). So the
  archive's files do land in the robot's map store.
- Yet after W9b and W9d a forced re-save produced a 6.8 identical to the map before, in every
  cell and trailer key.
- The members that could hold a room layout are now few: the segmentation PNGs (W9a, never
  re-saved) and `fine_large.bin` (not decoded). `app_map.bin` is a floor mask, and the tiled maps,
  door lines and embedded `rism` were ruled out.

A decisive test restores an archive that differs as a whole, not in 9 cells. The April
"original" entry does: its map is 211 x 307 cells (225 x 349 now), it has 12 rooms instead of 13
and no hidden rooms, and 7,708 of today's room cells are outside its rooms (the area of today's
room 9 belongs to room 10 there). W10 restores it, forces a re-save and reads 6.8. A 6.8 with
the April size and rooms means the archive drives the map, and W9a should be repeated with a
re-save. A 6.8 that still shows today's map means the robot keeps its working map outside the
archive and a restore cannot reshape it.

**Result (W10, verified on r2579a): a restore does replace the map.** 6.8 was republished at once,
without a forced re-save, and held the April map: 211 x 307 cells, 12 rooms, room 9 visible, the
April names, types, order, furniture, and no user carpets. It equals the `rism` inside the
original archive's `last_app_map` byte for byte (pixels and trailer) and matches the original's
segmentation PNG in every cell that is not a wall. So:

- The robot loads its working map from the archive. The third possibility is ruled out.
- The robot republishes 6.8 after a restore only when the map differs; after W8, W9b and W9d it did
  not.
- W9d showed the embedded `rism` is not the input: an edited `rism` did not survive a forced
  re-save. So the robot rebuilds the map from other members, and for a consistent archive the
  result equals that `rism`.
- The candidates left are the segmentation PNGs (W9a was never re-saved) and `fine_large.bin`.
  W9e repeats W9a with a forced re-save.

**W9e (verified on r2579a): the PNG edit does not survive a re-save either.** The restore took
place (live map session 12 to 13), and after a forced re-save 6.8 equalled the map before in
every cell and trailer key; the 9 edited cells were still room 12. So the door lines, the
embedded `rism` and the segmentation PNGs each fail alone.

**A pattern fits every result.** Each edited archive (W9a, W9b, W9d, W9e) was built from the W1
backup, which is the map the robot already held, and none changed 6.8. The April original, a
different map, replaced it at once. So the robot may keep its working map when an archive is
the map it already has, perhaps by an identity in `1.dat2` (for example `map_ids`, `version` or
`last_complete_clean_time`) or in `1/map_status.json`, and load the members only for a new map.
`fine_large.bin` is the one member never edited. W11 tests the identity idea: restore the April
original first, so that the robot holds a different map, then restore the W9a archive (today's
map with the PNG edit) and read 6.8.

**W11 (verified on r2579a): the PNGs do not drive the layout, even on a real load.** With the April
map loaded, restoring the W9a archive made the robot switch back: 6.8 changed at once to today's
size and 13 rooms. It equalled today's unedited map in every cell and trailer key; the 9 edited
cells were still room 12, and a forced re-save did not change that. So the segmentation PNGs are
ruled out. The 6.8 published after the load again equalled the unedited `rism` inside the
archive's `last_app_map`. W9d edited that `rism` only while the robot held the same map, when it
may have skipped the load, so the embedded `rism` is a candidate again. W12 repeats W9d after the
April original, the way W11 repeated W9a.

**W12 (verified on r2579a): the embedded `rism` is not the source either.** With the April map
loaded, restoring the W9d archive (9 cells moved only in `last_app_map`'s `rism`) made the robot
switch back to today's map at once. The new 6.8 equalled today's unedited map in every cell and
trailer key, and did not equal the edited `rism` that the archive carried; a forced re-save
changed nothing. So the robot builds the saved map itself, from members none of the experiments
edited, and the result happens to equal the `rism` of an unedited archive.

**Where this leaves archive editing.** Ruled out as the source of the room layout, each on a real
load: the segmentation PNGs (W11) and the embedded `rism` (W12); on a same-map restore with a
re-save: the door lines (W9c). Never edited: `fine_large.bin`, `app_map.bin` (a floor mask), the
`NEWMV001` maps, `out_walls_large.json`, `room_neighbor_table_large.json`, `all_doors_info.json`,
`unified_doors_info.json` and `seg_info`. The likeliest source is `fine_large.bin`, the one large
member not decoded at that point; it is decoded below and holds no room ids.

**`fine_large.bin`, decoded** (read-only, 2026-11-16; the April and November archives).
All fields are big-endian:

| Offset | Bytes | Content |
|---|---|---|
| 0 | 2 | magic `aa bb` |
| 2 | 4 | length of the rest of the file (file size − 8) |
| 6 | 10 | `01 06 00 19 20 00 20 00 00 05`, the same in both archives: `0x0019` = 25, the cell size in mm; `0x2000` twice = 8,192, probably the grid size (8,192 × 8,192 cells of 25 mm, centre 4,096 = world 0); the last `0x0005` is the number of bytes per cell |
| 16 | 8 | four `u16`: x min, y min, x max, y max in 25 mm cells. November: 3864, 3758, 4302, 4513 (439 × 756 cells), which matches the saved map's extent at twice its resolution; April: 3891, 3909, 4301, 4512 |
| 24 | 1 + 4 | section type `01` and the section length |
| 29 | n | the section data |
| end | 2 | `ee ff` |

**The section data is decoded** (with a one-off script; exact for the April and the
November archives). It is run-length encoded: a byte with the high bit set means "repeat the next
byte (byte & 0x7f) times", and any other byte is one value. The output is exactly
5 x width x height bytes (`00 05` in the header): five bytes per cell, interleaved, row by row from
the box's x min, y min. Cell (fx, fy) at 25 mm lines up with big-grid cell
((fx − 4096) / 2 + 1800, (fy − 4096) / 2 + 1800). Compared with `slam_map.png` (November):

| Byte | Content |
|---|---|
| 0 | 127 on many obstacle cells, else mostly 0; probably obstacle confidence |
| 1 | Occupancy: 1 on free floor and carpet, 255 on obstacles, 0 unknown |
| 2 | Almost always 0 |
| 3 | Bit flags on free floor (4, 8, 12, 28 most often); probably cleaning or carpet marks |
| 4 | Always 0 |

No byte holds room ids, so `fine_large.bin` is a fine occupancy map, not the room layout.

**The original segmentation.** `divide/1/ori_instance_sgm_map.png` (205 x 301, crop
`ori_slam_roi.json`) is identical in the April and November archives. Its labels (1..14, 255)
are not today's room ids, but after mapping each label to the room it mostly covers, it agrees
with today's segmentation in 29,886 of 31,010 shared room cells (96%). So the robot keeps the
first segmentation of the map and applies the user's room edits on top.

**Where the user's edits may live.** `doors_info_large.json` has 9 lines with `user_line_` true in
November and none in April. `all_doors_info.json` (69 lines in November, 7 in April) has none and
shares no line with `doors_info_large.json`. W13 and W14 moved door lines that are not user
lines. If the rooms are the original segmentation cut by the user lines, the robot would ignore
those edits, as it did. W15 moves a user line.

**W15 (verified on r2579a): moving a user line does not change the rooms either.** User line 7
of `doors_info_large.json` (the border of rooms 8 and 5, y 1859) moved 3 cells to y 1862. After
the April original and then this archive, 6.8 equalled today's unedited map in every cell and
trailer key, also after a forced re-save; rooms 5 and 8 kept 1,620 and 6,638 cells.

**What is left.** `ori_instance_sgm_map.png` is the same in the April and November archives, yet
the two archives load as different room layouts, so it cannot be the only input. The members that
differ between the two archives and that no experiment edited are: the occupancy data
(`fine_large.bin`, `slam_map.png` and its copies, `app_map.bin`), `all_doors_info.json`, the
`NEWMV001` maps, `room_neighbor_table_large.json`, `ai/1/*`, and `1.dat2` entries such as
`extend_msgs` (`door_reuse_info`, `retry_area_list`, `save_bridge`) and `seg_info`. The likeliest
explanation now is that the robot runs its segmentation again when it loads an archive, from the
occupancy data and stored line or bridge records, rather than reading a stored room grid. If so,
reshaping rooms through an archive would mean editing the occupancy itself, which is the original
W9 goal and a larger, riskier experiment.

**W16 (verified on r2579a): erasing occupancy is ignored too.** A stray 6-cell obstacle cluster
outside the rooms (big-grid x 1868..1870, y 1744..1747), shown as wall cells (room 11, wall bit)
in 6.8, became unknown (255) in `slam_map.png` and its two copies `slam_map_ori.png` and
`origin_img_slam.png` (rows 43 onward of `slam_map.png`). `fine_large.bin` already held "unknown"
there, and no other layer marked the cells. After the April original and then this archive, 6.8
equalled today's unedited map in every cell and trailer key, and the 6 cells were still walls,
also after a forced re-save.

**Where this leaves the question.** Edits to every member that holds a room grid, room borders,
the embedded saved map or the occupancy PNGs have been ignored on a real load, while whole
archives of a different map (April) load as that map. Two explanations fit all results:

- The robot rebuilds the saved map from a member none of the edits touched, for example the
  `NEWMV001` maps in `1.dat2`, the `ai/` files, or data in `extend_msgs` such as
  `door_reuse_info` or `save_bridge`.
- The robot keeps maps it has seen (on the robot or in the cloud) and, when an archive's identity
  matches one, uses the stored copy instead of rebuilding. The identity could be a field in
  `1.dat2` or `1/map_status.json`. The April map would then load from its own stored copy too.

A test for the second: change an identity candidate (for example `map_status.json` counters or
`1.dat2` timestamps) together with a content edit, and see whether the robot then rebuilds.

**Found: the room grid is `divide/1/segmented_map_with_obstacle_large.png`** (verified on r2579a,
2026-11-17). Swapping whole parts between the April and November archives (W17) and then editing
that one image (W18) settled the question:

- **W17a** (today's archive with April's `1.dat2`): the grid stayed today's, pixel for pixel, but
  the names, types, cleaning order, no-go zones and user carpets (`vw`), charger, Wi-Fi map
  (`whm`, `whmp`) and `last_complete_clean_time` became April's. So `1.dat2` carries the room
  metadata, not the grid.
- **W17c** (today's archive with April's `divide/`): the grid became April's exactly (211 x 307,
  every cell), with today's metadata. The 6.10 write itself returned 80001 (no acknowledgement)
  and the live frame afterwards showed map session 3 with 0 rooms, which suggests the robot
  restarted while it processed the mixed archive; a restore of the W1 backup brought it back to
  normal.
- **The grid image.** `segmented_map_with_obstacle_large.png` equals the saved map's pixels in
  every cell, for both the April and the November archive: it uses encoding E (bits 5-0 room id,
  bit 6 carpet, bit 7 wall), cropped to `map_roi_large.json`; the saved map adds a one-cell empty
  border. It was the only segmentation image that W9a, W11 and W14 left untouched; the three plain
  segmentation PNGs they edited lack the wall and carpet bits.
- **W18**: moving a 3 x 3 block from room 12 to room 10 in this image only, after loading the April
  map, gave a 6.8 that differed from today's map in exactly those 9 cells (room 10: 3,181 to
  3,190 cells; room 12: 1,497 to 1,488) and in nothing else, trailer included. A forced re-save
  kept the change.

So a restored archive can reshape rooms: edit `segmented_map_with_obstacle_large.png` and restore
it while the robot holds a different map (or use another way to force a load). Not tested yet:
whether the robot cleans and navigates by the edited rooms, and whether edits beyond room
borders (walls, new rooms, erased noise via the wall bit) are accepted.

**W19 (verified on r2579a): the robot cleans by the edited rooms, but forgets the edit after the
run.** The border between room 12 (above) and room 10 (below), moved 10 cells
(0.5 m) into room 12 in `segmented_map_with_obstacle_large.png` only (290 cells, 17 of them wall
cells, set by a one-off script). After the April original and then this archive, 6.8
held exactly that change. A clean of room 10 alone then:

- **Covered the strip.** 48 of the run's 390 path points lie in it, down to y −3444 mm (its far
  edge is −3450), and the track spans x −3967..−3271 mm, which with the robot's width covers the
  strip's whole free floor (x −4050..−3100). No point entered the rest of room 12.
- **Was undone by the robot.** The map the robot saved after the run had the old border again: the
  290 cells were room 12. The only other changes were the run's own (17 carpet-flag cells in room
  7, one cell from room 5 to room 3). The plain segmentation PNGs in the archive still had the old
  border, so the robot most likely rebuilds the grid image from them (adding obstacle and carpet
  bits) when it saves after a run.

So the robot uses the loaded grid for cleaning, but an edit that should last has to be made in the
plain segmentation PNGs too. W20 repeats W19 with the same change in all four segmentation
images.

**W20 (verified on r2579a): with all four images edited, the border survives the run.** The grid
image took the W19 change (290 cells) and the three plain images the same floor cells (273; their
walls are 0). The build script took the plain-image cells from the grid image,
because in 3 edge columns the cell below the border is a wall, which the plain images mark as 0.
After the April original and this archive, 6.8 had room 10 at 3,454 and room 12 at 1,224 cells, as
in W19. A clean of room 10 alone then:

- covered the strip again (44 of 429 path points, to y −3443 mm, x −3960..−3270 mm);
- left 261 of the 290 edited cells as room 10 in the map saved after the run (246 floor, 15 wall);
  28 floor cells and 1 wall cell returned to room 12;
- changed about 4,600 other cells: 1,559 carpet flags, 1,390 wall bits, and about 1,600 cells at
  room edges that moved between a room and the outside. After W19's run only 18 cells had changed.
  The saved map also grew by one row (225 x 350). The likeliest reason is that the archive carried
  the occupancy of the day before, which the robot reconciled with what it saw.

So an archive edit to all four segmentation images reshapes rooms durably, apart from a small
margin that the robot re-segments along walls.

**The library version.** `dreame_map::archive` decodes and encodes recovery archives. It keeps
every tar header as raw bytes and rewrites only the size and checksum of a replaced member, so
the W1 backup round-trips byte for byte. `dreame_map::segmap` reads the four segmentation images
and applies the W20 rule: in the grid image only the room bits change, and in each plain image a
cell changes only where it holds the old room id. `Segmentation::grow` moves a shared border in
one of four directions; on the W1 backup it reproduces the W20 archive cell for cell (test
`crates/dreame-map/tests/real_archive.rs`). `dreame::archiveedit` holds the robot steps, and
`dreame map edit-archive grow ROOM FROM DIRECTION CELLS --apply` runs them in this order:

1. back up the map and keep the downloaded backup as a revert file, so the edit starts from
   today's occupancy (the likely cause of W20's 4,600 re-mapped cells);
2. edit the archive's segmentation images;
3. restore the oldest original entry, because the robot skips an archive of the map it holds;
4. restore the edited archive from this machine;
5. put back the per-room settings.

Without `--apply` the command only edits an archive (`--archive`, or the newest backup in the
recovery list) and writes it to a file.

The steps live in `dreame::archiveedit::apply`, which runs one `GridEdit` and reports each step
through a callback. Besides `grow`, `GridEdit::Paint` moves any cells, given as world mm, to an
existing room; cells outside every room stay outside. The CLI offers it as `edit-archive paint
ROOM X0 Y0 X1 Y1` (a rectangle), and the desktop editor as a brush: select a room, press "Paint
cells into this room" and drag over cells of neighbour rooms. A test on real captures checks that
every room cell of a saved map, taken by its centre in world mm, lands on the same room in the
archive grid of a backup taken at the same moment.

**W25 (verified on r2579a): the first paint from the editor loaded, after a late firewall answer.**
The user painted 111 cells of room 10 into room 12 along their border (big grid y 1731..1734,
x 1717..1744) and applied. The edited archive held exactly that change. The editor's build was a
new program file, so the computer's firewall asked whether to allow it on the network just as the robot
downloaded; until the user answered, the download failed and 6.11 went to 5 (failure). The flow
stopped there and did not put back the room settings. The robot loaded the archive anyway after the
answer: 6.8 differed from the map before the edit in exactly the 111 cells. `restore-settings` and
`cleaning-order` were then sent by hand (code 0 each). Two changes followed: `apply` now starts
the file server before its first write to the robot, so a firewall prompt shows while the backup
runs, and after the edited restore a changed 6.8 counts as loaded whatever 6.11 says.

**The cleaning-order positions after a restore.** After every archive restore the live frame's
`cleanset` had 0 as each room's fourth value, the cleaning-order position, while 6.8's
`cleanareaorder` kept the order. Neither a forced I-frame request (action 6.1, the robot returned
the same frame) nor a `cleanareaorder` write, even one with two rooms swapped, refreshed the
positions; the swapped order did reach 6.8. The next cleaning run did (W23, W26). So the saved
order is never lost, and only the live frame's copy lags until a run. `archiveedit::apply` says so
in its last progress line.

**W26 (verified on r2579a): the editor paint survives a cleaning run.** After W25,
`dreame control clean-rooms 12 --apply` cleaned room 12 alone (about 4 minutes); all 160 path
points lie in room 12 as painted, down to y = −3,307 mm inside the painted strip. The map saved
after the run kept all 111 painted cells in room 12. 13 other cells changed: 8 carpet flags and
5 edge cells that also moved from room 10 to room 12. The live frame had the cleaning-order
positions back.

**W27 (verified on r2579a): a border move by line from the editor.** After W26 the user drew a
new border between rooms 10 and 12 with the editor's Move boundary tool and applied it. The
edited archive moved 357 cells of room 10 to room 12 (big grid y 1735..1746, x 1716..1746), the
cells the editor had previewed. 6.8 then differed from the map before in exactly those cells
(room 10: 3,598 to 3,241; room 12: 1,483 to 1,840), and every trailer key was unchanged: room ids,
names, types, settings and the saved cleaning order. The settings write was sent as usual, and
the live frame showed the usual 0 cleaning-order positions until the next run. Unlike the merge
and split move (§13.3), nothing had to be restored afterwards.

**W28 (verified on r2579a): the W27 border survives a cleaning run.** `dreame control
clean-rooms 12 --apply` cleaned room 12 alone. All 214 path points lie in room 12, down into the
moved strip (to y = −2,706 mm; the strip spans −3,250 to −2,650). The map saved after the run kept
all 357 moved cells in room 12, and the room sizes were unchanged. The only other changes were
13 carpet flags and the charger pose. The live frame had the cleaning-order positions back.

**W29 (verified on r2579a): a new room registered in the archive survives a cleaning run.** W24
again, with the room lists filled in. `dreame map edit-archive new-room 7 14 x 2600 --apply`
gave the 3,551 cells of room 7 east of x = 2,600 mm the id 14 in the four grid images, and
registered room 14 (`roomreg::add_room`):

- in `seg_info` with room 7's type (1) and floor, the next free index (1), no name, and its grid
  neighbours 6, 7 and 11, which also got 14 in their `nei_id`;
- last in `cleanarea_order` (place 12);
- in `room_neighbor_table_large.json` (`RoomIds` and a row) and `ai/1/ai_rooms_large.txt` (room
  7's AI type).

The edited restore reported 6.11 = 5 (failure) after one download, but 6.8 changed, as in W25.
6.8 then had room 14 with that metadata and place 12 in `cleanareaorder`; the settings write gave
room 14 room 7's settings (code 0). `dreame control clean-rooms 14 --apply` cleaned it: all 657
path points lie in room 14. The map saved after the run:

- kept all 3,551 cells in room 14; no cell moved between rooms (W24: room 7 took room 14 back
  and 1,671 cells of room 11);
- changed 164 carpet flags and 4 wall bits;
- kept room 14 in `seg_inf` and `cleanareaorder`. The robot rewrote two `seg_inf` fields: room
  7's `nei_id` became `[8, 14]` (it no longer touches 6 and 11), and room 14's `material` became 0
  (none).

The live frame had the cleaning-order positions back, room 14 included. So a new room needs its
`seg_info` entry (and probably the other lists) to last; with them, the archive can create rooms.
`out_walls_large.json` was not changed and was not needed. Reverted afterwards with the fresh
backup, `restore-settings` and `cleaning-order`; 6.8 then equalled the map before W29 in every
cell and trailer key.

**W30 (verified on r2579a): a new room painted in the editor.** The user selected room 8, pressed
"Paint a new room out of this room" and painted 658 of its cells (big grid x 1781..1798,
y 1813..1853), an island inside room 8. The editor sent `GridEdit::PaintNewRoom` with the unused
id 14. 6.8 then differed from the map before only in those 658 cells (room 8: 6,832 to 6,174;
room 14: 658) and in `seg_inf` and `cleanareaorder`: room 14 had room 8's type (5) with index 1,
room 8's floor, neighbour 8, and place 12. The live frame had a `cleanset` entry for room 14.

**W31 (verified on r2579a): removing a room through the archive.** `dreame map edit-archive
remove-room 14 8 --apply` moved the 658 cells back to room 8 and took room 14 out of `seg_info`,
`cleanarea_order`, the neighbour table and the AI room list (`roomreg::remove_room`). Both
restores reported success. 6.8 then equalled the map before W30 in every cell and trailer key,
so the removal is also the revert of W30.

**W32 (verified on r2579a): a new room keeps its name and floor through a run.** In W29 the robot
reset room 14's floor to 0 after the run. The archive's AI floor list (`ai/1/ai_floors_large.txt`,
per room `wood` with a direction or `ceramic`) had no entry for it, so `add_room` now adds one: the
parent's, or the floor chosen for the new room. The user painted 381 cells of room 8 into a new
room 14 in the editor, with a custom name and wood at 90°. 6.8 then had room 14 with type 0, the
name, material 1 and direction 90. After `clean-rooms 14` (81 of 96 path points in room 14; the
rest in room 8 around it) the map saved after the run kept all 381 cells and room 14's `seg_inf`
entry unchanged, name and floor included.

**W33 (verified on r2579a): a removal survives a run.** `remove-room 14 8 --apply` gave the 381
cells back to room 8 and took room 14 out of the room lists, the AI floor list included. 6.8 then
matched the map before W32 apart from the W32 run's own changes. `clean-rooms 8` ran about 30
minutes and stopped once: the robot got stuck, reported state 4 (error) and then paused with
error code 0, and resumed on action 2.1. The map saved after the run had 13 rooms, the 381 cells
in room 8 and no trace of room 14. The run re-mapped more than usual: 88 cells at room edges
(room 8 +57, room 7 +24, room 5 +7), 135 wall bits and 130 carpet flags.

**W21 (verified on r2579a): the command works end to end.** `dreame map edit-archive grow 10 12
up 10 --apply` made a new backup (object 103). It replaced the W1 backup (object 102): the list
holds one backup entry per map, so the downloaded copies in `local/out/recovery/` are the only
older backups. The command then
moved the room 12/10 border 10 cells up in its four images, restored the April original, then
the edited archive (one `Wget` request from the robot, 323,525 bytes), and put back the per-room
settings (code 0). 6.8 then had room 10 at 3,454 and room 12 at 1,224 cells, as in W19 and W20,
and no other difference in rooms, names, settings or zones. While the April original loaded,
6.11 showed only the old success, so the wait ran to its 180 s timeout before the next step. The
command now also ends a restore wait as soon as 6.8 changes.

**W22 (verified on r2579a): an edit from a fresh backup survives a cleaning run almost intact.**
After W21, `dreame control clean-rooms 10 --apply` cleaned room 10 alone (about 10 minutes). The
map the robot saved after the run kept all 290 edited cells in room 10. Only 13 other cells
changed: 12 carpet flags and 1 cell moved between rooms 3 and 5. After W20, whose archive was a
day old, the run had returned 29 edited cells to room 12 and changed about 4,600 others. So
building the edit from a fresh backup avoids the re-mapping, which confirms the W20 explanation.

**W23 (verified on r2579a): wall bits in the room grid neither block the robot nor last.**
Room 1 connects to the rest of the map only through door 2 (big-grid row 1957, x 1777..1805,
1.45 m). `dreame map edit-archive wall -1125 7875 275 7875 --apply` set the wall bit on the 22
cells of that row that were not walls yet (`Segmentation::set_wall`; the plain images took 0
there). After the restore, 6.8 had all 22 as walls, and the restore waits ended as soon as 6.8
changed. Then `dreame control clean-rooms 1 --apply`:

- The robot cleaned room 1 without an error: 132 of the run's 133 path points lie in room 1. So
  it plans its route on its own occupancy data, not on the room grid's wall bits.
- The map saved after the run had cleared all 22 wall bits; the row was back to its walls before
  the edit. 32 carpet flags and 5 other wall bits changed too. So the robot recomputes wall bits
  from what it senses.
- Right after the restore the live frame's `cleanset` had 0 as every room's cleaning-order
  position, while 6.8's `cleanareaorder` kept the order. `cleaning-order` with the old order was
  sent (code 0); after the run the live frame had the order again.

So a door is blocked with a virtual wall (`vw`, §11), not through the archive. Erasing noise
through the wall bit (the counterpart of W16) is unlikely to last for the same reason, unless the
robot no longer senses the obstacle.

**W24 (verified on r2579a): a new room id works until the next run, then the robot re-segments.**
`dreame map edit-archive split 7 14 x 2600 --apply` gave the 3,551 cells of room 7 east of
x = 2,600 mm the unused id 14, in the four grid images only; `seg_info` got no entry. After the
restore:

- 6.8's grid had room 14 (3,551 cells) and room 7 (3,125). `seg_inf` and `cleanareaorder` had no
  entry for 14; the live frame's `cleanset` had none either. The renderer showed 14 rooms.
- As after W23, the live frame briefly had 0 as every cleaning-order position.
- `dreame control clean-rooms 14 --apply` was accepted (code 0). The robot cleaned for about
  13 minutes without an error, and all 633 path points lie in room 14. So the robot cleans a room
  by its grid cells and needs no metadata for it.
- The map saved after the run had no room 14. Room 7 took all 3,551 cells back and also 1,671
  cells of room 11 (2,243 to 572), the part next to room 14. 182 carpet flags and 10 wall bits
  changed too. The metadata of rooms 7 and 11 did not change.

So the robot keeps rooms it has metadata for and re-segments the rest after a run, and the
re-segmentation can move cells between existing rooms. A new room needs a `seg_info` entry (and
probably `cleanareaorder` and `cleanset` entries) in the same archive; the native split command
`dsrid` (§11) does this already.

Reverted: `restore-file` of the W24 fresh backup (loaded without a restore of the April original
first, because the map then differed), then `restore-settings` and `cleaning-order` (code 0
each). 6.8 equalled the map before W24 in every cell and trailer key. The live frame's `cleanset`
again had 0 as every cleaning-order position, with every other setting back; after W23 the
positions returned with the next cleaning run.

**One gap in the door test.** Like W9a and W9d, W9c ran while the robot held the same map, when
it may skip the load. The door lines are therefore not ruled out on a real load. W13 repeats W9b
after the April original, as W11 and W12 did for the PNGs and the `rism`. The other never-edited
candidates for the room layout are `seg_info` and `out_walls_large.json` (per-room walls).

**W13 (verified on r2579a): the door lines do not drive the layout on a real load either.** With
the April map loaded, restoring the W9b archive (door 13 moved 4 cells) made the robot switch back
to today's map; 6.8 equalled today's unedited map in every cell and trailer key, also after a
forced re-save. Room 10 and room 11 kept their cell counts.

**Conclusion so far** (superseded by W17 and later: the room grid is
`segmented_map_with_obstacle_large.png`, see §12.3). On a real load the robot ignores edits to the segmentation PNGs (W11), the
embedded `rism` (W12) and the door lines (W13). It rebuilds the same room layout from members not
edited yet: `fine_large.bin` (header only decoded), `seg_info`, `out_walls_large.json`, the
`NEWMV001` maps and the other door and neighbour files. Until one of those is decoded and shown to
carry the layout, editing an archive cannot reshape rooms. The commands in §11 (split, merge,
move boundary) remain the way to change rooms.

**`seg_info` is room metadata.** It is a JSON object per room id: the base64 name, type, index,
floor material, direction and neighbour ids, the same fields as the trailer's `seg_inf`. It holds
no room shape.

**W14 (verified on r2579a): one consistent edit of every editable member.** The border between
rooms 12 and 10 (big-grid y 1740, x 1717..1743) moved 3 cells (150 mm) into room 12 in the three
segmentation PNGs, the `rism` in `last_app_map` (81 cells each), door 14 of
`doors_info_large.json`, door 13 of `unified_doors_info.json`, and door 16 plus 8 wall end points
of rooms 10 and 12 in `out_walls_large.json` (y −2975 to −3125 mm). `history_decomposed_map` was
left alone: its room 12 runs past the border, so it is not the current layout. After the April
original and then this archive:

- **The room grid did not change.** 6.8 had today's pixels exactly; rooms 10 and 12 kept 3,181
  and 1,497 cells (the edit would give 3,262 and 1,416).
- **The floor plan did.** 6.8's `walls_info` carried every edited value of
  `out_walls_large.json` (door 16 and the 8 wall end points at −3125), and kept them after a forced
  re-save. So `walls_info` is copied from `out_walls_large.json`.
- Reverted with the W1 backup: 6.8 equalled today's map again in every cell and trailer key.

So the robot takes the room grid from a member none of the experiments could edit, most likely
`fine_large.bin` (or a grid it derives from it), and takes the vector floor plan from
`out_walls_large.json`. An archive can change the floor plan, but the map edit command
`wallInfoV3` already does that (§11).

**The rooms are the floor cut along the doors** (read-only test with a one-off script).

- `divide/1/doors_info_large.json` holds 18 door lines. Each has `start_point` and `end_point`
  and `lines_`, the list of every cell on the line (x, y pairs), all in big-grid cells. Flags
  `user_line_` (9 doors; probably user splits) and `new_line_` (1 door).
- Taking the free floor of `slam_map.png` (values 0 and 20), removing the door cells and labelling
  the connected pieces (4-neighbour) gives one large piece per room for 12 of the 13 rooms, each
  with a single room id. Only rooms 10 and 11 stay joined. Door 13 lies on 23 of their 25 border
  cells, and the pieces join through the 2-cell gap at its end, so a fill that closes line ends
  separates them too.
- The room cells of the PNG, cut the same way, stay in 3 pieces: the door lines end at walls and
  obstacles, so they only separate rooms on the free floor.
- The other door files do not explain the rooms better. `all_doors_info.json` (69 lines, 3,485
  cells) cuts rooms into many pieces: candidate lines. `unified_doors_info.json` (17 doors,
  `beg_pt`, `end_pt`) gives the same result as `doors_info_large.json` without its `lines_`.
  `out_walls_large.json` repeats 18 doors in world mm with `nei_door_id` pairs, and per room the
  floor-plan walls (§8).
- Every pair of rooms that touch in the PNG also appears in `room_neighbor_table_large.json`.

So the room layout can be recomputed from the occupancy grid and the door lines. Whether the robot
does so when it restores an archive is not shown yet (W9b above). Which file assigns the room ids
to the pieces is not known (`seg_info`, `out_walls_large.json` or the PNG).

**What the app shows** (read-only search of the Android app, the Flutter decompile and the r2579a
plugin, 2026-11-16):

- No archive member name (`NEWMV001`, `yxmap`, `slam_map`, `segmented_map`, `fine_large`,
  `app_map.bin`) appears anywhere in the app. The app never opens, builds, re-packs or uploads an
  archive; the robot firmware does all of that.
- The 6.10 and 6.16 writes carry only `map_id` and `map_url`, with no hash or size, although the
  plugin sends an md5 for voice packs and OTA files. The app checks neither the URL host nor its
  scheme. The 6.8 md5 is compared only with the app's own map cache.
- `POST /dreame-user-iot/iotuserbind/restoreMap` returns `data.items[]` with `model`,
  `displayName`, `customName` and a JSON string `data`: an array of
  `{id, info:[{objname, objnameUrl, thb, time}]}`. The app writes the cloud's `objnameUrl` into
  6.10 or 6.16 unchanged. It keeps the newest entries, at most four per model.
- No app code resends `cleanset` or `customeClean` after a restore; the app re-reads them from
  the robot. That matches what r2579a does: the settings are gone until a client sends them.

Editor policy: before any structural edit (split, merge, wall move, carpet or restriction batch),
trigger a backup and wait for 6.14 = 3. Keep a local copy of the decoded saved map and the full
6.8 file for every editing session.

### 12.3 Editing rooms through the archive: what works (summary, 2026-11-17)

This section sums up experiments W9 to W33 (§12.2, §16.2). Read it instead of the log unless you
need the evidence.

**What can be changed.** Room cells can move between rooms that already exist. The change lasts
through cleaning runs when the archive comes from a fresh backup (W21, W22, W25 to W28). Rooms can
also be added and removed when the room lists are edited too (W29 to W33). Nothing else in the
archive has been shown to change the map:

| Edit | Result | Experiments |
|---|---|---|
| Move cells between existing rooms in the four segmentation images | Loads, cleans by the new rooms, survives runs | W18 to W22, W25, W26 |
| The same in the grid image only | Loads, but the robot undoes it after the next run | W19 |
| The three plain segmentation images, `rism`, door lines, user lines, wall end points | Ignored on load | W9a to W16 |
| Occupancy (`slam_map.png` and copies) | Ignored on load | W16 |
| Wall bits in the grid image | Loads; the robot drives through and clears them after a run | W23 |
| A new room id without a `seg_info` entry | Loads and can be cleaned; re-segmented away after a run, and can move cells of a neighbour | W24 |
| A new room id registered in `seg_info`, `cleanarea_order`, the neighbour table and the AI room list | Loads, cleans, survives runs | W29, W30 |
| The same with a custom name and a floor, and an entry in the AI floor list | Name and floor survive a run | W32 |
| A room's cells given to a neighbour and the room taken out of the room lists | Loads; the map is exactly as if the room had never been added; lasts through a run | W31, W33 |
| Members from another archive (`1.dat2`, `divide/`) | Swaps metadata or the whole grid; a mixed `divide/` made the robot restart | W17a, W17c |

So the archive is a way to reshape rooms cell by cell, to add a room when its id is also
registered in the room lists (W29, W30), and to remove one (W31). Use the map commands (§11) for everything else: merges
(`msr`), walls and doors (`vw`, `vws`), names and floors (`nsr`, `nsm`).

**The images and the edit rule.** The robot loads its room grid from
`divide/1/segmented_map_with_obstacle_large.png` (encoding E: bits 5 to 0 room id, bit 6
carpet, bit 7 wall), a crop of the 3600 x 3600 big grid given by `map_roi_large.json`. After a
run it rebuilds that image from three plain copies (`segmented_map_large.png`,
`dec_map_large.png`, `last_seg_map_large.png`; bare room ids, walls as 0). An edit therefore
changes all four: in the grid image only the room bits, and in each plain image only the cells
that hold the old room id. Big-grid cell `g` spans `(g - 1800) * 50` to `(g - 1799) * 50` mm.

**The method.** `dreame::archiveedit::apply` runs these steps for one `GridEdit`:

1. Check that the robot is idle and docked.
2. Save the saved map and the live frame as snapshots.
3. Start the HTTP file server, so a firewall prompt shows before the robot is changed.
4. Back up the map (action 6.3) and download the new backup. It is kept as the revert file.
5. Edit the four images in the backup, and keep the edited archive as a file.
6. Restore the oldest original entry from the recovery list. The robot skips an archive of the
   map it already holds, so this forces a real load.
7. Restore the edited archive from the file server (6.10). Success is a changed 6.8 pointer.
8. Put back the per-room settings (`customeClean`), which a restore drops.

It takes about two minutes. The live frame shows 0 as every room's cleaning-order position until
the next cleaning run; the saved order in 6.8 stays intact.

**Requirements and risks.**

- The robot must reach this computer over HTTP. The computer's firewall may ask about each new
  program file; until the user answers, the robot's download fails (W25).
- The recovery list holds one backup entry per map, and a new backup replaces it. The downloaded
  files in `local/out/recovery/` are the only older backups.
- To revert, restore the kept backup file (`dreame map restore-file`), then `restore-settings`
  and `cleaning-order`.
- The robot re-segments a small margin along walls after a run, most when the archive is old
  (W20: about 4,600 cells with a day-old archive; W22: 13 with a fresh one).

**The code.**

- `dreame_map::archive` decodes and re-encodes archives. It keeps the tar headers byte for byte.
- `dreame_map::segmap` reads the four images and edits them:
  - `grow` moves a shared border in one direction;
  - `paint` moves given cells to a room;
  - `move_border` moves a shared border to a drawn line, straight or bent (a polyline);
  - `split` and `set_wall` are the edits of W24 and W23;
- `dreame_map::roomreg::add_room` registers a new grid room in `seg_info`, `cleanarea_order`, the
  neighbour table and the AI room and floor lists, with an optional name and floor (W29, W32);
  `remove_room` takes a room out of them (W31, W33).
  `GridEdit::NewRoom` (split by an axis) and `PaintNewRoom` (painted cells) add a room with its
  parent's type, floor and settings; `RemoveRoom` gives a room's cells to a neighbour and
  removes it. Adding and then removing a room gives back the original archive (fixture test).
- `Segmentation::erase` makes floor cells outside (0) in all four images; `GridEdit::Erase` also
  takes a room left without cells out of the room lists (`roomreg::forget_room`). Untested on the
  robot (W34).
- `dreame::archiveedit` holds the robot steps.
- The CLI runs them with `dreame map edit-archive <grow|paint|border|split|new-room|paint-new-room|remove-room|erase|wall> ... --apply`.
  Without `--apply` it only edits an archive file.
- The desktop editor offers three tools: the paint brush (round, or a dragged box), "Paint a new
  room out of this room" (the same brush into an unused id, with a name and a floor), and Move
  boundary. Move boundary takes
  a dragged straight line, or clicked points of a bent one (double-click ends it). Both preview
  the cells in their new room's colour before sending.

`edit::border_cells` and `Segmentation::move_border` cut the two rooms along the line in the same
way (`segmap::border_cells`), on the saved map and on the archive grid. A test on real captures
checks that both pick the same cells. W27 ran it from the editor: the robot took exactly the
previewed cells and nothing else changed, and the border survived the next cleaning run (W28).
A bent line has not been run on the robot; it uses the same cut and the same archive edit.

---

## 13. Feasibility of a desktop map editor

### 13.1 Summary table

| Capability | Class | How | r2579a |
|---|---|---|---|
| Rename, retype, hide rooms | Persisted on robot | `nsr`, `delsr` | Likely; `delsr` key present |
| Cleaning order | Persisted | `cleanareaorder` | Yes (key, last edit seen in `ulmm`) |
| Per-room suction, water, passes, mopping | Persisted | `customeClean` | Likely |
| Floor material and wood direction | Persisted | `nsm` | Likely |
| Split a room along a line | Persisted | `dsrid` | Yes (verified write) |
| Merge two neighbour rooms | Persisted | `msr` | Yes (verified write) |
| No-go zones, no-mop zones, virtual walls | Persisted | `vw` (full replacement) | Yes (key) |
| Thresholds, impassable thresholds, ramps | Persisted | `vws` | Yes (verified write; ramps likely) |
| Curtains | Persisted | `curtain` | Yes (verified write) |
| Carpet rectangles and ellipses; "ignore carpet" areas | Persisted | `cpt` | Yes (verified write, rectangles) |
| Carpet cleaning preferences | Persisted | `carpetcleanset` | Likely |
| Furniture placement, size, angle | Persisted | `mapfurniture_v2` | Yes (verified write) |
| Move floor-plan wall and door end points | Persisted (effect on navigation unknown) | `wallInfoV3`, `doorInfoV3` | Likely |
| Map name, display rotation | Persisted | `nrism`, `smra` | Likely |
| Cruise points, router position, obstacle ignore | Persisted | `spoint`, `wrp`, `obstacleignore` | Likely |
| Backup and restore from the robot's own recovery list | Persisted | Action 6.3, property 6.10 | Yes (verified backup and restore of an unmodified backup, 2026-11-16) |
| Move a boundary between two rooms | Composite (persisted) | Merge, then split along the new line, then re-apply names, floors, settings and order | Yes (verified write, 2026-11-16) |
| Batch edits (many zones, many rooms) | Composite | One `vw` or `cpt` replacement; chained `msr` with a re-read between calls | Likely |
| Room colours, label positions, icons, themes | Cosmetic (local only) | Editor state | n/a |
| Hiding noise or stray pixels in the view | Cosmetic | Editor mask | n/a |
| Polygon or rotated no-go zones | Cosmetic shape, approximated | Cover the polygon with several axis-aligned rectangles | Partly |
| Furniture images, 3D view | Cosmetic | Editor rendering | n/a |
| Move room cells between existing rooms (paint, move a border by line) | Persisted, through the archive | Edit the four segmentation images of a fresh backup, restore it (§12.3) | Yes (verified, survives cleaning runs; W21, W22, W25 to W28) |
| Add a room by giving cells a new id; remove a room | Persisted, through the archive | Edit the grid images and the room lists (§12.3) | Yes (verified; new and removed rooms survive a cleaning run; W29 to W33) |
| Painting walls or floor pixels; erasing noise in the grid | Impossible by command; archive edits do not last | Wall bits and occupancy are recomputed or ignored (W16, W23) | No |
| Adding or deleting floor-plan walls; drawing a room as a polygon | Impossible by command | None known | No |
| Uploading a whole edited map | Research only | Modified recovery archive restored through 6.10 | Only the room grid is known to load (§12.3) |
| Low-lying area editing | Impossible on r2579a | No capability, no payload | No |

### 13.2 Persisted edits

Everything in section 11.2 is stored in the robot's saved map and survives an app reinstall. A
desktop editor can do these better than the phone app, because the payloads are simple geometry in
mm:

- Exact numeric entry and snapping (to the 50 mm grid, to floor-plan walls, to other shapes).
- Batch editing. Draw many no-go zones, then send one `vw` command. Draw many carpets, then send one
  `cpt` command.
- Preview before sending. Render the planned change over the current map, then send, then compare
  with the next frame.
- Validation before sending: rectangles axis-aligned with ordered corners; split lines that cross
  exactly one room; merges only between `nei_id` neighbours; at most 49 rooms.
- Diff and history. Store every sent payload with the 6.8 md5 before and after.

### 13.3 Composite operations

Structural edits change room ids and wipe metadata. The editor should wrap them:

1. Save the current metadata of the affected rooms: name, type, index, material, direction, order,
   `cleanset`, carpet settings, hidden state, furniture in the room.
2. Send the structural command (`msr` or `dsrid`).
3. Wait for the new saved map (new 6.8 md5) and decode it.
4. Match new rooms to old rooms by overlap of pixel areas.
5. Re-apply metadata with `nsr`, `nsm`, `cleanareaorder`, `customeClean`, `delsr`.

"Move a boundary" is merge followed by split along the desired line. Polyline boundaries need more
than one split. Each step needs a fresh map, so the operation takes several round trips.

On r2579a this flow is implemented (`dreame::mapedit`, `Edit::MoveBoundary`) and verified with a live
move ([l40-ultra-ae.md](l40-ultra-ae.md)). Observed: a structural edit does not wipe the metadata
of the room that keeps its id; only the new half starts with no name, no type and default settings.
After `msr`, the next split reuses the merged-away id. Carpet settings, hidden state and furniture
are not restored yet; the editor refuses a move with a hidden room.

### 13.4 Cosmetic only

Room colours (computed from `nei_id`), label positions, icons, colour schemes, Y flip and scale,
furniture images, 3D rendering and any retouching of the pixel grid exist only in the client.
Display rotation is the exception: `smra` stores it on the robot.

### 13.5 Not possible through known commands

- Editing the occupancy grid: drawing or erasing walls, removing reflections or noise, adding floor.
- Moving room boundaries other than by split and merge (and possibly by floor-plan wall moves; see
  section 8.2).
- Creating a room from a polygon, or adding and deleting floor-plan walls.
- Uploading an arbitrary map file through a documented command.

Practical substitutes: block unwanted areas with no-go zones or virtual walls; hide unwanted rooms
with `delsr`; hide noise in the editor view only.

### 13.6 Research-only path: restoring an edited archive

*Status, 2026-11-17: room-cell edits through the archive work and last (§12.3). The text below
is the plan from before the experiments.*

`MAP_RECOVERY` (6.10) makes the robot download a map archive from a URL it is given
(`T/device.py:6990-6995`). In principle a modified backup archive could carry an edited pixel grid
or floor plan. This is the only known route to "upload a whole map". It is research, not a feature.

Unknowns:

- The archive format is now known (§12.1: tar plus bzip2 with PNG, JSON and binary files). The
  binary files (`app_map.bin`, `fine_large.bin`, the `NEWMV001` maps) are not decoded yet.
- Integrity checks: none found inside the archive; checks elsewhere are unknown.
- ~~Whether r2579a accepts URLs outside Dreame's object storage.~~ It does: a restore from a
  plain HTTP server on the local network worked (§12).
- Whether the robot checks an archive's content before it replaces the map.
- What a corrupt or inconsistent map does to the robot (failed localisation, map loss, a confused
  robot that needs a factory reset).

Required order of work: download and study archives read-only first (experiment R2). Write nothing
to the robot on this path without the user's explicit consent, a fresh backup, and a tested way
back (restoring the unmodified archive).

### 13.7 Constraints for the editor

- Cloud is required. All map files live in Dreame cloud storage, and r2579a has no local control
  surface. **Verified on r2579a.**
- Edits apply to the selected saved map. With multi-floor on, select (`sm`) first.
- The robot must be idle and docked, with no temporary map.
- After each structural edit, wait for a new 6.8 md5 before the next one.
- Replacement commands (`vw`, `cpt`, `vws`, `curtain`, `mapfurniture_v2`, `delsr`,
  `cleanareaorder`) need the full current list. Always rebuild them from the freshest saved map, or
  the robot loses items.

---

## 14. Suggested Rust module structure

```
dreame-map/
  envelope.rs    URL-safe base64 <-> bytes; optional AES-256-CBC; zlib inflate and deflate.
                 decode(text, key) and encode(frame) for test fixtures.
  header.rs      27-byte Header struct; read and write; "absent" (32767) handling.
  grid.rs        Grid { width, height, left, top, grid_size, bytes }.
                 enum Encoding { V3, FrameMap, Building, VslamRestored, Saved, Wifi, CleanMap }.
                 fn select_encoding(map_version, &Trailer) -> Encoding.
                 fn classify(byte, Encoding) -> Pixel { class: u8, segment: u8, carpet: bool }.
  trailer.rs     serde structs with #[serde(default)] and a flatten catch-all
                 (extra: serde_json::Map). Lenient types: numbers or strings, arrays or
                 JSON-in-string (cleanset, robot, charger, mra, ai_obstacle).
  floorplan.rs   WallsInfo, Room polygon, Wall { type, beg, end, normal }, Door { a, b, beg, end }.
                 Polygon helpers: closed-ring check, area, point-in-room, label point.
  model.rs       SavedMap: header + grid + typed trailer + floor plan; LiveMap: frame + rism.
  merge.rs       P-frame union and add/replace; live plus saved composition for ris == 2.
  geometry.rs    World <-> pixel <-> screen transforms, rotation, bounding boxes,
                 nei_id colouring, rectangle normalisation.
  commands.rs    One typed builder per row of table 11.2. Each returns the piid-4 JSON
                 string. Builders take the current SavedMap for full-replacement commands.
  composite.rs   Merge-then-split and metadata re-application (section 13.3).
  render.rs      Raster base layer plus vector overlays; theme table.
  fetch.rs       (in the app crate) cache read, signed URL, download, REQUEST_MAP, md5 tracking.
```

The crate as built has these modules: `archive`, `edit`, `envelope`, `frame`, `grid`, `header`,
`merge`, `model`, `obstacle`, `path`, `render`, `roomreg`, `segmap`, `trailer` and `view`.

Fixtures: the r2579a outputs in `local/out/map/` (keep them out of version control, since they hold
a real floor plan), plus `N/test/map.decoder.test.ts`. Test that every decoded map re-encodes to the
same bytes.

---

## 15. Open questions

1. **Floor-plan wall moves.** Does `wallInfoV3` change navigation boundaries or only the drawn
   floor plan? (Experiment W6.)
2. **Wall `type` meaning.** Type 0 looks interior and type 1 exterior (section 8.1). Confirm with a
   second map or after a split.
3. **`dsr` and `dsrid`.** Valetudo sends both; Tasshack sends `dsrid` and `mapid`. Which does
   r2579a need? Answered: `dsrid` with `mapid` alone works on r2579a
   ([l40-ultra-ae.md](l40-ultra-ae.md)).
4. **Edit acknowledgement.** `UPDATE_MAP_DATA` replies before the change is applied. Is `ulmm` the
   name of the last applied edit? Does 6.8's md5 always change? Partly answered: for `nsr` on
   r2579a the reply carries result code 0 and 6.8 changes within 60 s. The other edits recorded
   in [l40-ultra-ae.md](l40-ultra-ae.md) (2026-11-16) were each checked in the next saved map.
   The meaning of `ulmm` is still open.
5. ~~**Backup and restore support.** Do 6.10, 6.11 and 6.14 exist on r2579a?~~ Answered: all
   three exist. Backup (action 6.3) and restore (6.10) work, with 6.14 and 6.11 as their status
   ([l40-ultra-ae.md](l40-ultra-ae.md), 2026-11-16).
6. **Recovery archive format** and whether 6.10 accepts URLs outside Dreame storage. Format
   settled (§12.1); a local-network HTTP URL works on r2579a (§12).
7. ~~**`vw` replacement scope.**~~ Avoided: the RN plugin resends every stored `vw` key, and the
   library now does the same ([map-plugin.md](map-plugin.md) §6.3).
8. **Unknown fields.** `funiture_info[13]` (edit type) and repeated id 0; `seg_inf.index` on
   custom-named rooms (stored and overwritten by `nsr`; meaning unknown); `sa` fields 1..4; `ai_obstacle` fields 4..13; `carpet_polygon` type for
   detected carpets; `vw.cliff`; `ai_outborders_user`; `ar_match`.
9. ~~**P-frame merge rule for encoding B** (add or replace) on r2579a.~~ Resolved: the RN plugin
   merges encoding-B P-frames additively (byte + value, mod 256); see [map-plugin.md](map-plugin.md)
   §4.2.
10. **App-only features with no known payload**: 3D map, AI furniture recognition, polygon no-go
    zones, `AREA_ROTATION`, `VIRTUAL_TRACKS`, `SKIP_AREA`. The React Native map plugin has since been
    captured and read, which recovers the app's own edit payloads for many of these (`mapwall`,
    `borderflag`, `addclean`, `skipclean`, map rotation `smra`, and more); see
    [map-plugin.md](map-plugin.md) §6. Decompiling `libapp.so` does not help: the map codec and edit
    payloads are in that JavaScript plugin, not the Flutter binary ([decompile.md](decompile.md) §4).

---

## 16. Next experiments

Read-only experiments change nothing on the robot. Write experiments change the saved map; each
needs the user's go-ahead, an idle docked robot, and a backup first (W1).

### 16.1 Read-only

| Id | Experiment | Writes to robot? | Status |
|---|---|---|---|
| R1 | `get_properties` for 6.1, 6.2, 6.5, 6.9, 6.10, 6.11, 6.14, 6.15 through `sendCommand`, to learn which map properties exist | No | **Done 2026-11-15** ([device-model.md](device-model.md) §2.17) |
| R2 | Download the 6.9 recovery list. Decode each `thb`. Fetch each `objname` archive (try both URL endpoints), list its contents, and look for checksums or signatures | No | **Done 2026-11-15** (§12.1) |
| R3 | Decode `rism` from the 6.3 frame and compare it byte for byte with the 6.8 saved map | No | **Done 2026-11-16**: after a restore the `rism` equalled the 6.8 saved map (§12.2) |
| R4 | Listen to MQTT pushes during a normal cleaning run started by the user. Capture P-frames and check the merge rule for encoding B | No | **Done**: captured 2026-11-15; the add rule checked 2026-11-17 ([l40-ultra-ae.md](l40-ultra-ae.md)) |
| R5 | Decode `whm` (W-frame) with encoding W | No | Open |
| R6 | Send `REQUEST_MAP` `{"frame_type":"I"}` (action 6.1) and watch for a new 6.3 or 6.1 value | Sends an action; does not change the map | **Done 2026-11-16**: no new frame from a sleeping robot ([l40-ultra-ae.md](l40-ultra-ae.md)) |
| R7 | Round-trip test: re-encode the decoded saved map and compare with the original bytes | No (local) | **Partly done**: the real-data tests re-encode each frame and decode it to an equal frame; byte equality with the original is not checked |

### 16.2 Writes (need the user's consent)

Ordered from least to most risk. Each one: read the map, send the command, wait for a new 6.8 md5,
decode, compare, then revert.

| Id | Experiment | Revert |
|---|---|---|
| W1 | Back up the map (action 6.3, `"1"`), watch 6.14, then re-read 6.9 | Nothing to revert. Creates a backup entry. **Done 2026-11-16**: worked |
| W2 | Rename the map with `nrism`, then clear the name | `nrism` with the old name |
| W3 | Add one small no-go rectangle with `vw` in an open area, confirm `cliff`, `addcpt` and `nocpt` survive, then remove it | `vw` with the original lists |
| W4 | Swap two entries of `cleanareaorder`, check `ulmm`, then swap back | Original order |
| W5 | Rename one room with `nsr` (base64 name), then restore its name | Original `nsr` |
| W6 | Move one floor-plan wall by 50 mm with `wallInfoV3`; check whether the pixel grid or only `walls_info` changes | Move it back, or restore W1 backup |
| W7 | Split a room, then merge it back with `msr`; test `dsrid` alone first, then with `dsr`. **Done 2026-11-16**: `dsrid` alone worked, and the merge restored the map exactly | Merge; re-apply names and settings; or restore W1 backup |
| W8 | Restore the W1 backup through 6.10 with Dreame's own signed URL | This is the revert path itself. **Done 2026-11-16**: 6.11 reached 3 and the saved map was unchanged; per-room settings were dropped and had to be re-applied with `customeClean` ([l40-ultra-ae.md](l40-ultra-ae.md)) |
| W8b | Restore the same unmodified archive from an HTTP server on the local network | Restore it again from Dreame's URL. **Done 2026-11-16**: the robot fetched it with one `GET` and restored it; the saved map was unchanged; per-room settings were re-applied |
| W9 | Research only: restore a modified archive. Do not run without explicit consent, a tested W8, and a clear way back | Restore the unmodified archive |
| W9a | Move a 3 x 3 cell block on the border of two neighbour rooms from one room id to the other in the three identical segmentation PNGs only; keep every other member and the tar metadata byte for byte. Restore it with `restore-file`, then compare the `rism` of the next live frame with the original. A moved block means the PNGs are the source of truth; none means the robot rebuilds from `1.dat2` or the binaries | Restore the unmodified archive with `restore-file`, then `restore-settings`. **Done 2026-11-16**: the robot restored the archive, but its rebuilt map did not change, then reverted. **Inconclusive**: the `rism` checked is a copy of the archive's `last_app_map` (§12.2) |
| W9b | Move door 13 (the border of rooms 10 and 11) by a few cells in `doors_info_large.json` only, keeping its length; keep every other member byte for byte. Restore it with `restore-file` and compare the `rism` of the next live frame with the original. A moved border means the robot recomputes rooms from the door lines | Restore the unmodified archive with `restore-file`, then `restore-settings`. **Run 2026-11-16**: restored, no change in `rism`; **inconclusive** for the same reason as W9a |
| W9c | Repeat W9b, then make the robot save the map again and compare the new 6.8 with the map before. First try a write that changes nothing (resend the current room names with `restore-names`); if 6.8 does not change, rename one room and rename it back | Restore the unmodified archive, then `restore-settings`. **Done 2026-11-16**: the names write re-saved 6.8, which still had the old border; the robot kept the shifted door line in its next archive. Reverted through Dreame's URL (`restore-file` failed: no request reached this computer) |
| W9d | Change the room layout only in the `rism` inside `1.dat2`'s `last_app_map` (move a 3 x 3 block to the neighbour room, re-encode the envelope), restore it, force a re-save with `restore-names`, and compare 6.8 with the map before | Restore the W1 backup through Dreame's URL (`dreame map restore`), then `restore-settings`. **Done 2026-11-16** (second try; the first was blocked by the computer's firewall): the re-saved 6.8 had the old layout, so the embedded `rism` does not drive it; reverted |
| W10 | Restore the April "original" recovery entry (different size, room count and hidden rooms) through Dreame's URL, force a re-save with `restore-names` from a snapshot taken right after the restore, and compare 6.8 with today's map and with the original's `rism` | Restore the W1 backup (object 102) through Dreame's URL, then `restore-settings` (the backup carries today's names). **Done 2026-11-16**: 6.8 became the April map at once, equal to the original's embedded `rism`; reverted |
| W9e | Repeat W9a (move a 3 x 3 block between rooms in the three segmentation PNGs only), restore it from this computer, force a re-save with `restore-names`, and compare 6.8 with the map before | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the restore took place, but the re-saved 6.8 was unchanged; reverted |
| W11 | Restore the April original (Dreame's URL), then the W9a archive (today's map with 9 cells moved in the segmentation PNGs) from this computer, and read 6.8 at once and after a forced re-save. Today's layout with the block moved means the robot loads a same-map archive only when it holds a different map, and that the PNGs drive the layout | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the robot loaded today's map from the archive but without the PNG edit; reverted |
| W12 | Restore the April original (Dreame's URL), then the W9d archive (today's map with 9 cells moved only in the `rism` inside `last_app_map`) from this computer, and read 6.8 at once and after a forced re-save. The block moved means the robot takes the saved map from that embedded copy | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the robot loaded today's map without the edit; the embedded `rism` is not the source; reverted |
| W13 | Restore the April original (Dreame's URL), then the W9b archive (today's map with door 13 moved 4 cells) from this computer, and read 6.8 at once and after a forced re-save. A moved border between rooms 10 and 11 means the robot builds rooms from the door lines | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the robot loaded today's map without the door shift; reverted |
| W14 | Move the border between rooms 12 and 10 by 3 cells consistently in every editable member (PNGs, embedded `rism`, door records in three files, wall end points in `out_walls_large.json`), after loading the April original; read 6.8 at once and after a forced re-save | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the room grid did not change; `walls_info` took the edited `out_walls_large.json`; reverted |
| W15 | After the April original, restore today's map with one `user_line_` door line of `doors_info_large.json` moved a few cells, and read 6.8 at once and after a forced re-save. A moved border means the rooms are the original segmentation cut by the user lines | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-16**: the robot loaded today's map without the shift; reverted |
| W16 | Erase a stray 6-cell obstacle cluster outside the rooms (slam value 99 to 255) in `slam_map.png` and its two copies; after the April original, restore it and read 6.8 at once and after a forced re-save | Restore the W1 backup through Dreame's URL, then `restore-settings`. **Done 2026-11-17**: the cells stayed walls; reverted |
| W17a | Today's archive with April's `1.dat2`, loaded while the robot holds today's map | Restore the W1 backup, then `restore-settings`. **Done 2026-11-17**: the grid stayed today's; names, order, `vw`, charger and Wi-Fi map became April's |
| W17c | Today's archive with April's `divide/` folder | Same. **Done 2026-11-17**: the grid became April's; the 6.10 write returned 80001 and the robot seemed to restart; the revert worked |
| W18 | After the April original, today's archive with a 3 x 3 block moved from room 12 to room 10 in `segmented_map_with_obstacle_large.png` only | Same. **Done 2026-11-17**: 6.8 changed in exactly those 9 cells and kept them after a re-save; reverted |
| W19 | After the April original, today's archive with the room 12/10 border moved 10 cells into room 12 in `segmented_map_with_obstacle_large.png` only; then clean room 10 alone and read its track | Restore the W1 backup, then `restore-settings`. **Done 2026-11-17**: the robot cleaned the whole strip, but its post-run save restored the old border; reverted |
| W20 | W19 with the same border move in all four segmentation images (`segmented_map_with_obstacle_large.png`, `segmented_map_large.png`, `dec_map_large.png`, `last_seg_map_large.png`); clean room 10 and check whether the border survives the post-run save | Same**Done 2026-11-17**: 261 of 290 cells kept the new room after the run; the robot also re-mapped about 4,600 other cells |
| W21 | `map edit-archive grow 10 12 up 10 --apply`: fresh backup, edit, April original, edited archive, settings, all in one command | `map restore-file` of the fresh backup, then `restore-settings`. **Done 2026-11-17**: rooms 10 and 12 at 3,454 and 1,224 cells, nothing else changed |
| W22 | After W21, clean room 10 alone and compare the map saved after the run with the W21 map | Nothing to revert; the user keeps the edited map. **Done 2026-11-17**: all 290 edited cells kept; 13 other cells changed |
| W23 | `map edit-archive wall` across door 2, the only way into room 1; then clean room 1 alone | Nothing to revert: the run cleared the wall bits. **Done 2026-11-17**: the robot cleaned room 1 through the wall; the saved map lost the 22 wall bits |
| W24 | `map edit-archive split 7 14 x 2600`: a new room id in the grid images only; then clean room 14 alone | Restore the W24 fresh backup, then `restore-settings` and `cleaning-order`. **Done 2026-11-17**: room 14 cleaned; after the run room 7 took back room 14 and 1,671 cells of room 11 |
| W25 | Editor paint brush: 111 cells of room 10 into room 12 | Nothing to revert: the user keeps it (the fresh backup `1794933316-backup.tbz2` would undo it). **Done 2026-11-17**: loaded after a late firewall answer; settings put back by hand |
| W26 | After W25, clean room 12 alone; compare the map saved after the run, and the live cleaning-order positions | Nothing to revert. **Done 2026-11-17**: all 111 painted cells kept; positions back after the run |
| W27 | Editor Move boundary through the archive (`GridEdit::Border`): draw a line across two neighbour rooms and apply | Nothing to revert: the user keeps it (the fresh backup `1794937671-backup.tbz2` would undo it). **Done 2026-11-17**: 357 cells of room 10 moved to room 12; nothing else in the saved map changed |
| W28 | After W27, clean room 12 alone; compare the map saved after the run | Nothing to revert. **Done 2026-11-17**: all 357 moved cells kept; 13 carpet flags and the charger pose changed |
| W29 | `map edit-archive new-room 7 14 x 2600 --apply`: W24 with room 14 registered in the room lists; then clean room 14 alone | Reverted: `restore-file` of the fresh backup `1794939672-backup.tbz2`, then `restore-settings` and `cleaning-order` (code 0 each); 6.8 then equalled the map before W29 in every cell and trailer key. **Done 2026-11-17**: room 14 kept all 3,551 cells and its metadata through the run; nothing else moved |
| W30 | Editor: paint a new room out of room 8 | W31 removes it. **Done 2026-11-17**: room 14 (658 cells) loaded with room 8's type, floor and settings, place 12 |
| W31 | `map edit-archive remove-room 14 8 --apply` | Nothing to revert. **Done 2026-11-17**: 6.8 equalled the map before W30 in every cell and trailer key |
| W32 | Editor: a new room out of room 8 with a name and a wood floor; then clean it | W33 removes it. **Done 2026-11-17**: name and floor kept through the run |
| W33 | `remove-room 14 8 --apply`, then clean room 8 | Nothing to revert. **Done 2026-11-17**: room 14 stayed gone after the run; the robot got stuck once and resumed on action 2.1 |
| W34 | Editor "Erase floor" (or `map edit-archive erase`) over floor that does not exist; read 6.8, then clean a neighbour room and read it again | Restore the kept backup with `restore-file`, then `restore-settings`. **Not run yet**: the occupancy still marks the area as free floor, and occupancy edits are ignored (W16), so the robot may rebuild the floor |
