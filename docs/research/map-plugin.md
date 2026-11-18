# Dreamehome map plugin: the official app's own map wire format

Status: research reference, 2026-11-15.
Audience: the agent that implements the map codec and map edits.
Scope: how the official Dreamehome app decodes the vacuum map and encodes every map edit, read
from the app's own React Native plugin for `dreame.vacuum.r2579a` (Dreame L40 Ultra AE). This is the
authoritative cross-check for [maps.md](maps.md), whose model is otherwise sourced from tasshack,
Valetudo and node-dreame.

## 1. Source and how it was obtained

The app's per-robot screens, including the map, are a React Native plugin downloaded at runtime, not
part of `libapp.so` (see [decompile.md](decompile.md) §4). The plugin for this robot was captured
from a rooted emulator and lives at
`local/apk/rn_plugins/rn_plugins/dreame.vacuum.r2579a/a/dreame.vacuum.r2579a/index.android.bundle`
(git-ignored). It is a 37.5 MB Metro JavaScript bundle of 2495 modules, plain ASCII, readable (not
Hermes bytecode). Variable names are mangled, but object keys and string literals, which carry the
wire format, are intact.

Citation convention: `plugin:mod_<n>` is the Nth `__d(...)` module in that bundle (re-split into modules
with a one-off script, not kept). Module numbers are stable for this build; the function or key name
is the durable anchor. Findings here were cross-checked against the JavaScript and against maps.md.

## 2. MIoT map service id map (resolved from the plugin source)

The map service is **SIID 6** (`plugin:mod_1695` `CleanMap`, `this.SIID = 6`). The plugin defines the
full property and action table (`plugin:mod_1689` base `Model`, `plugin:mod_1695` `initProps`), which
fills in and confirms maps.md §2:

Properties (`siid.piid`):

| Name | piid | Name | piid |
|---|---|---|---|
| mapdata | 6.1 | backupMapInfo | 6.9 |
| frameInfo | 6.2 | restoreMap | 6.10 |
| objectName | 6.3 | restoreMapStatus | 6.11 |
| mapExtendData | 6.4 | oldIMap | 6.13 |
| robotTime | 6.5 | backupMap | 6.14 |
| resultCode | 6.6 | wifiMap | 6.15 |
| multMapState | 6.7 | restoreMapByArr | 6.16 |
| multMapInfo | 6.8 | | |

Actions (`siid.aiid`): `mapReq` 6.1, **`updateMapInfo` 6.2**, `backupMap` 6.3, `wifiMap` 6.4.

## 3. Transport: every edit is action 6.2 with one JSON input

`plugin:mod_1695` `updateMapInfo(mapInfo, cb)` is the single path for every map edit. Verified
verbatim:

```js
updateMapInfo = function (mapInfo, cb) {
  normalCallMethod('action', {
    did: Device.deviceID,
    siid: 6,                               // SIID
    aiid: 2,                               // AIID.updateMapInfo
    in: [{ piid: 4,                        // PIID.mapExtendData
           value: JSON.stringify(mapInfo) }]
  })...
```

So **every edit = action `6.2`, one input `piid 4`, value = a compact JSON string**. This confirms
maps.md §11.1 exactly. Success is read back from the action output where `piid == 6` (resultCode)
and `value == 0`; a non-zero result is a failure (room split reports codes 5 and 6).

Related getters: frame request is action `6.1` with `piid 2` (frameInfo), value
`{"frame_type":"I"|"T"}`; backup is action `6.3` with `piid 4`; wifi map is action `6.4`.

Every edit in §6 is the JSON object passed to `updateMapInfo`. Geometry is **world millimetres**. The
editor converts on-screen coordinates to map coordinates with `MapDataUtil.rotateViewXxx2Map(...)`
immediately before each commit, which confirms that all edit commands share one world frame.

## 4. Decode pipeline (how the app parses a map)

From `plugin:mod_1865` (the map model and codec hub; functions `addMapData`, `decodeSaveMapData`,
`decodeMultiMap`). The pipeline:

```
payload text (base64, URL-safe)
  -> if it contains ',': split "<b64>,<keyString>"; aesKey = sha256(keyString).hex[:32]
  -> replace '-' with '+', '_' with '/'
  -> base64 decode
  -> [only if a key was present] AES-256-CBC / PKCS7 decrypt, IV = Const.AES_IV
  -> pako.inflate (zlib)
  -> raw = [27-byte header][width*height pixel bytes][UTF-8 JSON trailer]
```

This matches maps.md §3, with one correction about the IV:

- **AES IV correction.** The app uses a single global constant `Const.AES_IV`
  (`plugin:mod_11`), not a per-model IV table. In this build that constant is the **empty string**
  (`AES_IV: ''`). A separate `RESOURCES_AES_IV: 'aebf8c52e26a4767'` exists but is for downloaded
  resources, not maps. So the shipped app has no working map-decrypt path and relies on no key ever
  being sent. For r2579a no key is sent, so this never bites. maps.md's per-model `DEVICE_INFO` IV is
  a tasshack construct; the official app does not use it.

### 4.1 Header (27 bytes, little-endian) — confirms maps.md §4

| Offset | Field | Offset | Field |
|---|---|---|---|
| 0 | map_id (i16) | 15 | charger angle (i16) |
| 2 | frame_id (i16) | 17 | grid_size mm/pixel (i16) |
| 4 | frame_type (i8): 73=I, 80=P, 87=W | 19 | width px (i16) |
| 5 | robot x mm (i16) | 21 | height px (i16) |
| 7 | robot y mm (i16) | 23 | left / min-x origin mm (i16) |
| 9 | robot angle deg (i16) | 25 | top / min-y origin mm (i16) |
| 11 | charger x (i16) | | |
| 13 | charger y (i16) | | |

- **Origin is pixel-centre.** Right after reading the header the app shifts the origin by half a
  cell: `_x = left - gridWidth/2; _y = top - gridWidth/2` (`plugin:mod_1865`, verified at the decode
  site and repeated in other decode paths). So the stored `left`/`top` are the world coordinates of
  the **centre** of pixel (0,0); the app's internal `_x`/`_y` are the cell corner. The pixel centre
  is then `_x + col*grid + grid/2 = left + col*grid`. maps.md §9's `x = left + col*grid` is therefore
  the pixel **centre**, and `+ grid/2` would double-count. Treat maps.md's `left` as the app's `_x`
  (corner) if you want corner coordinates, else there is a half-cell (grid/2, about 25 mm) offset.
  The r2579a capture confirms this: user carpet corners, which the app snaps to cell corners
  (`adjustValue` against `_x`/`_y`), all lie on `left - grid/2 + n*grid`. `dreame-map` applies
  the shift in `Grid::transform`.
- Charger is absent when its angle is 32767. An empty map is `width == 2 && height == 2` with all
  pixels zero. The JSON trailer's `expands.origin`, `expands.robot` and `expands.charger`, when
  present, override the header values.

### 4.2 Pixel encodings — confirms maps.md §5 and §6

- **Live encoding B** (`fsm == 1`): segment id = `byte >> 2`; class = `byte & 3`, carpet when
  `(byte & 3) == 3`. After the shift, `63` renders as wall (255), `62` as floor/unreached (254), `0`
  is background, `1..60` are rooms.
- **Saved encoding E**: segment id = `byte & 0x3F`; wall when bit 7 set; carpet when bit 6 set;
  `byte == 63` (no high bits) is a new segment.
- **Live plus saved** (`ris == 2`, `fsm != 1`): the saved map supplies segment ids, walls and
  carpet; the live grid overlays only the `& 3` class bits.
- **P-frame (type 80) is additive.** The P rectangle is merged as
  `nMapInfo[idx] = nMapInfo[idx] + value` with Uint8 wrap (mod 256), not replace. This **resolves
  maps.md open question 9** for r2579a: encoding-B P-frames add. maps.md's MAP_V2 encoding A does not
  appear in this live path, consistent with r2579a lacking MAP_V2.

### 4.3 Decoder details new since maps.md

- `funiture_info[15] = inHideArea` (a 16th field, read only when the entry has at least 16 elements).
  maps.md lists fields through 14.
- `ai_obstacle` render filter: the app drops entries whose status is `'2'` or `'4'`, and keeps only
  those with confidence `item[3] >= 0.5`.
- Room label point: longest horizontal run of the room id on the bounding-box centre row, tolerating
  gaps of up to 4 zero pixels, then the longest vertical run at that column. maps.md §10 describes the
  method; the 4-pixel gap tolerance is the new detail.

## 5. Map data model

The trailer keys the plugin reads (`seg_inf`, `vw`, `vws`, `ct`, `carpet_info`, `carpet_polygon`,
`walls_info`, `walls_info_new`, `funiture_info`, `delsr`, `ai_*`, and the live-only keys) all match
maps.md §7 by name. `seg_inf` name is base64 UTF-8 when `type == 0`; room colour is a greedy
four-colour over `nei_id`; floor plan walls carry original (`beg_pt_*`) and edited (`_beg_pt_*`)
endpoints. These confirm maps.md §7, §8 and §10.

## 6. Edit-command wire format

Each row is the JSON object passed to `updateMapInfo` (action 6.2, piid 4). Coordinates are world mm,
flat arrays `[x0,y0,x1,y1,...]`. Verdict is versus maps.md §11.

### 6.1 Rooms

| Edit | Payload | Verdict |
|---|---|---|
| Merge | `{"msr":[ids],"mapid":M}` | Confirms. No two-room cap in the app |
| Split | `{"dsrid":[x0,y0,x1,y1,areaId],"mapid":M}` | Confirms. App sends only `dsrid`, not `dsr`. It trims the drawn line to the room's edge first (`calculateCrossAreaLine`, `lookForValidPos`): each end must lie outside the room, then moves toward the other in 25 mm steps along the main axis and stops just before the room's floor; ends are rounded down. Ends in another room are allowed. Result 5 means an end is not on the room's boundary; 6 means a part would be under 1 m² |
| Rename / retype | `{"nsr":{"<id>":{"name":"<base64>","type":T,"index":I,"roomID":R}},"mapid":M,"autonsr":true}` | See correction below |
| Hide rooms | `{"delsr":[ids],"mapid":M}` | Confirms. Full hidden-id list. Sibling `{"delsr_rec":...}` confirms a hide recommendation |

- **`autonsr` correction.** maps.md says `autonsr` is never sent on Dreamehome cloud. The app **does**
  send `autonsr: true` in its auto-room-naming flow, which links Mi-Home room ids through the per-room
  `roomID` field (`plugin:mod_1885` `renameArea`, `plugin:mod_1912`). tasshack omits it; the app sends
  it.
- **Manual rename** (`commitRename`, the room-edit screen) sends every typed room and only the
  custom-named rooms being renamed, each with its stored index (`index: _item.num`) and, for custom
  names, the base64 `name`. So `nsr` merges into the stored names. A live write on r2579a
  confirmed that the robot stores the index it is sent: tasshack's fixed `index: 0` reset four
  rooms' index ([l40-ultra-ae.md](l40-ultra-ae.md)).

### 6.2 Per-room cleaning settings

| Edit | Payload | Verdict |
|---|---|---|
| Cleaning order v2 | `{"cleanareaorder":[{"<id>":1},{"<id>":2}, ...]}` | Confirms, with a nuance below |
| Custom clean | `{"customeClean":[[id,suction,water,times,cleanType,mopEffect,mopTemp], ...]}` | See fields below; the last three are optional |
| Carpet clean setting | `{"carpetcleanset":[[objectType,id,setting]]}` | Confirms |

- **`customeClean` fields.** The per-room in-memory array is
  `[suction, water, times, order, cleanType, mopEffect, mopTemp]`. The sent tuple is
  `[id, suction, water, times, cleanType, mopEffect, mopTemp]`: index 3 (order) is not sent.
  maps.md's field 5 "mode" is **cleanType** (0 sweep, 1 mop, 2 both) and "mopping" is **mopEffect**;
  the **7th field `mopTemp`** is new.
- **The tuple length depends on capabilities** (`commitCleanSet`). The app always sends
  `[id, suction, water, times]` and appends `cleanType` only when `enableCleanTypeModfiy`,
  `mopEffect` only with `showCustomMopEffect`, `showCustomMopPrefer` or `supportFastClean`, and
  `mopTemp` only with `supportMopTemperature`. It fills `water` 0 with 3 (without 32 levels) and
  `times` 0 with 1, and sends every room of the clean set. r2579a stores six fields, so it has no
  `mopTemp`. The app reads the clean set from the live frame; the saved map has none. A live
  six-field write on r2579a worked ([l40-ultra-ae.md](l40-ultra-ae.md)).
  `mopEffect` is `preference | (water << 4) | (mopEffect << 8)`, confirming maps.md's encoding.
  `water` is raw 1..32 when the robot uses 32 wetness levels, else 1..3.
- **`cleanareaorder` nuance.** The app (`commitCleanOrder`) appends hidden rooms at the end with
  `order = visible.length + index + 1`, but only hidden rooms that the stored `cleanareaorder`
  already lists (it builds that set from the stored order, filtered by `delsr`). On r2579a the
  stored order does not list hidden rooms 9 and 13, so the app would omit them too: the capture
  and the app agree. The robot stores the list sorted by id; the app sends it in cleaning order.

### 6.3 Geometry: restrictions, thresholds, carpets, curtains

| Edit | Payload | Verdict |
|---|---|---|
| Virtual walls / no-go / no-mop | `{"vw":{"line":[...],"rect":[...],"mop":[...],...}}`, add `"temp":{}` when not on a saved map | Confirms; the app resends every stored `vw` key (below) |
| Carpet areas | `{"cpt":{"addcpt":[...],"nocpt":[...]}}` | Confirms maps.md §11.2. The robot stores both lists under `vw` |
| Thresholds / ramps | `{"vws":{"vwsl":[...],"npthrsd":[...],"ramp":[[x0,y0,x1,y1,angle]]}}`, `ramp` padded to 5 (angle 0), `"temp":{}` when not saved | Confirms |
| Carpet type | `{"addcpt":[[id,type]],"origincpt":[[id,type]],"roomcpt":[[id,material]]}` (default type 7, material 0) | Confirms the field shapes |
| Curtain | `{"curtain":ct}` (stored as `ct`) | Confirms |

- `vw.line` entries are `[x0,y0,x1,y1,flag]` (flag at index 4 marks delete); `vw.rect` (no-go) and
  `vw.mop` (no-mop) are `[x0,y0,x1,y1,?,flag]` with the flag at index 5.
- **`vw` commit resends everything.** `commitVirtualWall` sends `getCommitVMData()`, which copies
  every key of the stored `vw` (`line`, `rect`, `mop`, `cliff`, `addcpt`, `nocpt` and any other),
  each entry with all its fields. This settles maps.md open question 7 in practice: send the
  whole object.
- **Carpet commit is a standalone `cpt`.** `didCommitCpt` sends `{"cpt": {"addcpt": [...],
  "nocpt": [...]}}`, built by `getCommitCarpetData` from the stored `vw.addcpt` and `vw.nocpt`.
  Each `addcpt` entry is resent with all its fields (type included), corners snapped to the cell
  lattice (`adjustPoint`). New carpets get the lowest free id, or `-1` (the robot assigns one) when
  `supportFullCarpet`. The editor refuses a 21st carpet (`isAddCptLimit`, 20).
- **Ellipse carpets are `[cx, cy, sx, sy]`.** When field 5 is 1, fields 0 and 1 are the centre
  (snapped against the origin) and fields 2 and 3 are sizes (snapped against 0); a 90 or 270 degree
  rotation moves the centre and swaps the sizes. Treating them as two corners reorders them and
  corrupts the carpet.

### 6.4 Floor plan, furniture, obstacles

| Edit | Payload | Verdict |
|---|---|---|
| Floor-plan walls / doors | `{"wallInfoV3":[{"i":roomId,"w":[[ox0,oy0,ox1,oy1,nx0,ny0,nx1,ny1], ...]}],"doorInfoV3":[[ox0,oy0,ox1,oy1,nx0,ny0,nx1,ny1], ...]}` | Confirms. First 4 = original endpoints, last 4 = new |
| Furniture | `{"mapfurniture_v2":list}`, or fallback `{"mapfurniture":list,"mfet":[editTypes]}` | Confirms v2; `mfet` fallback is new |
| Obstacle ignore | `{"obstacleignore":[x,y,type,ignore]}` (ignore 1, restore 0) | Confirms. Sibling `{"obstacleconfirm":[x,y,type,1]}` is new |
| Outer borders (3D/AI) | `{"mapwall":walls}`, `{"borderflag":updateType}` | New |

### 6.5 Map lifecycle

| Edit | Payload | Verdict |
|---|---|---|
| Select map | `{"sm":{},"mapid":M}` | Confirms |
| Delete / reset map | `{"cm":{},"mapid":M}` or `{"cm":{},"mapid":-1}` on newer firmware | Confirms; `-1` variant new |
| Rename map | `{"nrism":{"<map_id>":{"name":"<text>"}}}` (plain text) | Confirms |
| Rotate map | `{"smra":{"<mapid>":{"ra":0\|90\|180\|270}}}` | Confirms |
| Temp map | keep `{"cw":5}`, discard `{"cw":0}`, replace `{"cw":1}`; manual save `{"hsm":{},"mapid":M}` | Confirms cw codes; `hsm` is new |
| Backup / restore | action 6.3 `piid 4` (backupMap); props restoreMap 6.10, restoreMapStatus 6.11, restoreMapByArr 6.16 | Confirms; 6.16 is new. Restore flow in §8 |

### 6.6 Other commands new since maps.md

Temporary cleaning and recommendation or AI confirmations, all through action 6.2:
`{"addclean":[[x0,y0,x1,y1], ...]}` (temp sweep rects), `{"skipclean":1}`, `{"hide_sneak":[...]}`,
`{"stepConfirm":1}`, `{"cancel_rec":1}`, `{"confirm_sgm":1}`, `{"smart_show":0}`, `{"nopush":1}`.

## 7. Corrections and open points

Corrections to prior research:

- Map AES IV is a single global `Const.AES_IV` (empty in this build), not a per-model table (§4).
- Header `left`/`top` are pixel-centre coordinates; the app shifts the origin by `-grid/2` (§4.1).
- P-frames (encoding B) are additive on r2579a, resolving maps.md open question 9 (§4.2).
- `customeClean` field 5 is `cleanType` and there is a 7th field `mopTemp` (§6.2).
- `autonsr: true` is sent by the app in its auto-naming flow (§6.1).
- Ellipse carpets are `[cx, cy, sx, sy]`, not two corners (§6.3).
- `vw` commits resend every stored key, carpets included (§6.3).
- Hidden rooms are appended to `cleanareaorder` only when the stored order lists them (§6.2).

Open points (need a live capture against r2579a to settle):

- ~~Carpet add and remove: `cpt` or `vw`?~~ Resolved from the plugin: the app commits carpets with
  a standalone `cpt` command (§6.3), and also resends them inside `vw` when it commits walls.
- The trailing fields on `vw.line` / `vw.rect` / `vw.mop` entries. Partly settled by a live write
  on r2579a: a `rect` sent with four coordinates is stored with a fifth field, 0
  ([l40-ultra-ae.md](l40-ultra-ae.md)). What index 4 means, and whether walls and no-mop zones
  behave the same, is open. `dreame-map` resends stored fields and sends new entries with four.
- Ellipse carpet sizes: radii or full widths. A round carpet was stored on r2579a
  ([l40-ultra-ae.md](l40-ultra-ae.md)), but the stored values do not show which.
- W-frame (type 87) pixel decode and the trajectory (`tr` / `mult_tr`) token grammar were not traced
  in this pass.

## 8. Map restore flow

Two entry points write `restoreMap` (6.10). Both send the JSON text
`{"map_id":M,"map_url":"<url>"}` and then read `restoreMapStatus` (6.11) every 3 s until it is 3
(success) or 4 or 5 (failure). On success the app requests a new I-frame.

- **Recovery page** (`RecoveryMapPage`). It downloads the recovery list named by 6.9 through
  `getInterimFileUrl`, decodes each entry's `thb` thumbnail, and lists the entries of the current
  map. To restore one it gets a signed URL for its `objname`: `getFileUrl` when the name contains
  `mb.tbz2` (a manual backup), otherwise `getInterimFileUrl`.
- **Recover from cloud.** `POST /dreame-user-iot/iotuserbind/restoreMap` with
  `{"model", "optionalModels"}` returns map archives the cloud keeps for the account, possibly from
  other models. The app offers one when the robot's map is empty, and restores it with the same
  6.10 write.

- **Several maps at once** (`RecoveryMapListPage`). When the cloud returns more than one map, the
  app writes `restoreMapByArr` (6.16) with a JSON array `[{"map_url","map_id"}, ...]` and watches
  6.11 the same way. This restores a whole multi-floor set in one write.

The app clears the recover offer when the robot has a task, cleans or returns to the dock. The
plugin checks nothing about the URL host; the robot downloads from whatever URL it gets.
