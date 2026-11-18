# Findings from the user's Dreame L40 Ultra AE

These results come from the user's own robot. The summary and the sections up to "Follow-up results" are read-only, from 2026-11-15, and were produced with prototype tools that the `dreame` CLI has since replaced. The later sections record writes and actions that the user approved one at a time, from 2026-11-16.

User identifiers appear as `<did>` (device id), `<uid>` (account id) and `<serial>`.

## Summary

| Question | Answer |
|---|---|
| Model id | `dreame.vacuum.r2579a` (cloud device list and property 1.2 agree) |
| Firmware | `4.3.9_1664`; `hw_ver` is `Linux` |
| tasshack capability set | 152 (69 entries), not the L40 Ultra's set 170. No `MAP_V2`. `LOCAL_STORAGE` needs build 1681, so it is off on 1664. See [device-model.md](device-model.md). |
| Account type | Dreamehome account, registered outside the US |
| Cloud region | `us`: `https://us.iot.dreame.tech:13267` |
| Login username | The account's email address. The "account id" and "user name" shown in the app did not work. |
| Local control | None. The robot answers ping but ignores miIO and drops every probed TCP port. |
| Command path | `/dreame-iot-com-10000/device/sendCommand` (from `bindDomain`) |
| MQTT broker | `10000.mt.us.iot.dreame.tech:19973` (from `bindDomain`). A standard CONNECT works. TLS needs a custom verifier because rustls rejects the broker's private chain. |
| TLS fingerprint blocking | None seen. A plain rustls client with the Android header set worked. |
| Map encryption | None. The map files inflate with zlib directly. |
| Live map pixel encoding | Encoding B ("frame map", `fsm` flag) |
| Saved map pixel encoding | Encoding E (bit 7 wall, bit 6 carpet, bits 5-0 room id) |
| Saved maps | One (multi-floor is off), 13 room ids, 2 of them deleted (`delsr: [9, 13]`), 11 in the cleaning order |

## Identity and connectivity

- The cloud device list gives `model: dreame.vacuum.r2579a`, `ver: 4.3.9_1664`, `featureCode2: 31` and `bindDomain: 10000.mt.us.iot.dreame.tech:19973`. Its `property` field is `{"lwt":1,"mac":...}`, where `lwt` 1 means online.
- `iotstatus/devOTCInfo` returns the robot's self-report: `fw_ver`, `hw_ver`, `model`, `netif.localIp`, `netif.mask`, `netif.gw`, and `ap.{siid, bssid, rssi}`. A client can use `netif.localIp` to find the robot on the LAN.
- The serial number (property 1.5) starts with `R2579`.
- The robot is on 2.4 GHz Wi-Fi.

### LAN probe

A LAN probe tested the robot at its real address (confirmed by the router's client list and by the cloud's MAC address):

- ICMP ping: answered.
- miIO hello to UDP 54321, sent to the robot and as a broadcast: no reply.
- TCP connect to ports 22, 23, 80, 443, 554, 1883, 6668, 8000, 8080, 8443, 8883, 8888, 9999, 10000, 54321 and 55555: every attempt timed out. The robot drops these packets silently instead of refusing them.

Conclusion: this robot has no local control surface. The library must control it through the Dreamehome cloud. A local transport is still worth keeping in the design for other models and for rooted robots (Valetudo).

A second probe on 2026-11-17 gave the same result. Local miIO and the Mi cloud are out of scope for now (see rust-library-design.md, Decisions).

## Cloud API behaviour

These observations settle open questions in [protocols.md](protocols.md):

1. **Login**: OAuth2 password grant at `/dreame-auth/oauth/token` with `platform=ANDROID`. The token response holds `region: us`, the account's `country` and `lang`, `tenant_id: 000000` and `expires_in: 7200`.
2. **Headers and signing**: the prototype sent the Android header set (`dreame-meta: cv=a_102060300;...`, `tenant-id`, Basic `authorization`, `dreame-auth: <token>` without a `bearer` prefix) and signs every JSON body (`sign`, `timestamp`). Every call succeeded. The server also accepts an unsigned body (tested 2026-11-17; see "Cloud and map checks without a robot write").
3. **TLS**: the prototype used `reqwest` with `rustls`, as the library does. It does not copy the Android TLS ClientHello as tasshack does. The server accepted it.
4. **`iotstatus/props`** (cloud property cache): `keys` must be one comma-separated string, for example `"2.1,3.1,6.3"`. A JSON array returns `{"code":0,"data":[]}` with no error. Each reply item is `{"key":"2.1","value":"8","updateDate":<ms>}`. All values are strings, and JSON values are JSON text inside the string. 60 keys in one call worked.
5. **`sendCommand` `get_properties`** is relayed to the robot and works. The values come back typed, so numbers are numbers:

   ```json
   {"code":0,"success":true,"data":{"id":33,"result":[
     {"siid":2,"did":"<did>","piid":1,"value":8,"code":0},
     {"siid":3,"did":"<did>","piid":1,"value":100,"code":0}]},"msg":null}
   ```

6. **`iotfile/getDownloadUrl`** with `{did, model, filename: <object name>, region: "<country>"}` returns a signed URL. Here `region` is the country code from the token, not the server region. A GET on the URL returns the base64 map text.
7. Server messages are in Chinese (`"msg":"操作成功"`, "operation succeeded") whatever the `lang`.

## Properties present on r2579a

A sweep of the cloud cache over siid 1-30 and piid 1-60 found 93 properties with values. [device-model.md](device-model.md) has the full table with names and meanings and marks which entries exist on r2579a. Notable values:

| siid.piid | Value | Note |
|---|---|---|
| 2.1 | `8` | state |
| 2.2 | `68` | error code; 68 is present after a normal finished task, so it is not a fault here |
| 3.1 | `99` | battery % |
| 3.3 | `{"enable":false,"startTime":"22:00","endTime":"08:00"}` | off-peak charging JSON |
| 4.22 | `271` | AI obstacle settings as an integer bitmask |
| 4.23 | `5123` (`0x1403`) | packed cleaning mode |
| 4.50 | `[{"k":"StainIdentify","v":1},{"k":"MopFullyScalable","v":1},...]` | settings store as key/value pairs |
| 5.1-5.3 | `false`, `22:00`, `08:00` | do not disturb |
| 6.3 | `ali_dreame/<uid>/<did>/0` | object name of the last map frame (no AES key suffix) |
| 6.7 | `0` | multi-floor maps off |
| 6.8 | `{"obj_name":"ali_dreame/<uid>/<did>/9","md5":...}` | saved-map list pointer |
| 6.9 | `ali_dreame/<uid>/<did>/8` | recovery (backup) list pointer |
| 7.2, 7.7 | `<voice>`, `<region>` | voice pack and region label (the region label is a city name in Chinese) |
| 8.1 | `<IANA name>` | time zone |
| 9.x, 10.x, 11.x, 16.x | remaining life values | consumables (brushes, filter, sensors, mop pad) |
| 12.1-12.4 | totals | cleaning history counters |
| 27.x, 28.x, 30.x | | station and other services |

The sweep did not cover siid above 30. Services such as 99 (if present) still need a check.

## Maps

[maps.md](maps.md) has the full format. What the robot showed:

- **No encryption.** Property 6.3 has no `,<key>` suffix, and the downloaded text inflates with zlib directly.
- **Last frame (6.3)**: 25,976 characters of base64. Header: `map_id` 4, `frame_id` 350, type `I`, grid 50 mm, 185 x 296 pixels. The JSON trailer has `fsm` and `ris`. Pixels decode correctly only with encoding B (bits 7-2 room id, bits 1-0 class), which gives 12 rooms. So the live map is not a "map v3" (`MAP_V2`) frame.
- **Saved-map list (6.8 file)**: `{"mapstr":[{"map":"<16,204-char envelope>","angle":"0","name":""}],"curr_id":1,"server":null}`. The saved map is inline, so no second download is needed.
- **Saved map**: 225 x 349 pixels at 50 mm. Pixels decode correctly with encoding E. Encoding A (v3) reads the carpet bit as a wall bit and draws carpets as walls.
- **Saved map JSON keys**: `vw`, `vws`, `ct`, `pointinfo`, `whmp`, `whm`, `walls_info`, `delsr`, `funiture_info`, `cleanareaorder`, `carpet_polygon`, `last_complete_clean_time`, `ar_match`, `seg_inf`, `robot`, `charger`, `origin`.
- **Rooms (`seg_inf`)**: each room has `type`, `index`, `material`, `direction`, `nei_id` (neighbour rooms) and an optional base64 UTF-8 `name`. Custom names are free text in any language.
- **Carpets**: `vw.addcpt` holds user-added carpet rectangles `[x1, y1, x2, y2, a, b, c]`. `carpet_polygon` holds detected carpet outlines as flat x,y lists in millimetres.
- **Floor plan (`walls_info`)**: `version_flag` 3, one storey, a list of rooms with `room_id`, `floor_type` and wall segments `{type, beg_pt_x, beg_pt_y, end_pt_x, end_pt_y, normal_x, normal_y}`. This is a vector floor plan of the whole home. A desktop editor can draw it and edit room shapes on it.
- **Furniture**: `funiture_info` (spelling as sent) is a list of 15-number arrays, for example `[0, 2, 2, 2050, 1850, 0, 2075.0, 9075.0, 0.0, 90.0, 0.0, 0.0, 1.0, 1, 0]`. [maps.md](maps.md) maps the fields; some of them are not yet decoded.

## How to reproduce

The read-only checks above map to `dreame` CLI commands: `devices`, `props`, `map` and `watch`
(see the README). The CLI reads the account from `local/env` (git-ignored) and writes
outputs to `local/out/`, which holds personal data and stays out of git.

## Follow-up results (2026-11-15)

- **Token refresh works** and rotates both tokens. See protocols.md §5.4.
- **Properties above siid 30**: only siids 99 (diagnostics) and 10001 (camera, vendor `tx`) exist. A relayed read finds many properties the cloud cache misses, including 6.11, 6.14 and most of siid 28. See device-model.md §2.17.
- **Recovery list**: one original and one edited version of map 1, no backup. Each entry's archive is a bzip2 tar of the robot's map store: PNG grids (occupancy and room segmentation), JSON (doors, walls, carpets, rooms) and binary maps. See maps.md §12.1.
- **MQTT**: connected and subscribed with a pinned-root TLS verifier. While idle, the robot pushes 4.64 every 108 s and telemetry on 99.98. See protocols.md §11.
- **MQTT during a clean**: a one-room clean of about 6.5 minutes produced about 180 messages: 122 P-frames on 6.1 (one every 3 s), progress on 4.63, state, area and time changes, start and end events, and a cleaning-record event (4.e1) that names a downloadable history map. Events use an object, not an array, for `params`, and `_otc.info` has no `data` wrapper. See protocols.md §11.1b and maps.md §2.2.

## First write: room rename and undo (2026-11-16)

Approved by the user, one write at a time, with the robot idle on the dock. Tool:
`dreame map edit rename-room 2 "Test rename" --apply`, then
`dreame map edit restore-names <before snapshot> --apply`.

- **Transport works.** Action 6.2 with one input (`piid 4`, the `nsr` JSON) through the cloud
  relay returned `{"code":0,"siid":6,"aiid":2,"out":[{"piid":6,"value":0}]}`. The result also
  echoes `did`.
- **The saved map follows.** Property 6.8 changed within the 60 s wait after each write, and the
  downloaded saved map showed the change. This answers maps.md open question 4 for `nsr`.
- **`seg_inf.index` on custom-named rooms is stored and is overwritten by `nsr`.** The first
  command sent `index: 0` for every custom-named room, as tasshack does, and the robot stored 0
  for rooms 2, 3, 5 and 6 (before: 6, 7, 4, 3). The app sends the stored index, and
  `dreame-map` now does too. Its meaning is still unknown.
- **The undo restored everything.** After `restore-names`, the saved map's header, pixels and
  every trailer key equal the snapshot taken before the first write.

## Second write: no-go zone and removal (2026-11-16)

Approved by the user, one write at a time, robot idle on the dock. Tool:
`dreame map edit add-no-go 2000 2000 2500 2500 --apply`, then `remove-no-go 0 --apply`. Each
command was the whole stored `vw` object with only `rect` changed.

- **The robot accepts the whole-object `vw` commit** (result code 0, 6.8 changed within 60 s) and
  keeps the resent carpets (`addcpt`) unchanged.
- **The robot adds a field to zone entries.** The zone was sent as `[2000, 2000, 2500, 2500]` and
  stored as `[2000, 2000, 2500, 2500, 0]`. The app's delete filter reads index 5 for `rect`
  (map-plugin.md §6.3), so index 4 is a different field; its meaning is unknown.
- **The extra field broke our parser.** `dreame-map` typed `vw` entries as exactly four integers,
  so the stored zone made the whole `vw` object unreadable to the model. Fixed: entries keep any
  later fields and unchanged entries are resent with them. The CLI now refuses to edit a map with
  unread trailer keys.
- **Removal restored the map exactly.** After `remove-no-go`, the saved map's header, pixels and
  every trailer key equal the snapshot taken before the zone was added.

## Third write: per-room settings and undo (2026-11-16)

Approved by the user, one write at a time, robot idle on the dock. Tool:
`dreame map edit room-settings 2 --suction 2 --apply`, then `--suction 1 --apply`. Each command
was `customeClean` with all 11 rooms in the six-field form, only room 2 changed.

- **Settings live in the live frame only.** The saved map has no `cleanset`. After each write the
  live frame (6.3) changed within the wait and 6.8 did not.
- **The robot keeps the cleaning order.** Room 2's stored entry went from `[1, 3, 1, 1, 2, 546]` to
  `[2, 3, 1, 1, 2, 546]`: the order field (index 3), which the command does not carry, was kept.
- **The undo restored the settings exactly.** The live `cleanset` after the undo equals the one
  before the first write.
- **`ulmm` does not name this edit.** It held `cancel_rec` (an earlier app action) before the
  first write and was absent from every live frame after it.

## Fourth and fifth writes: hide a room, change the cleaning order (2026-11-16)

Approved by the user, one write at a time, robot idle on the dock.

- **Hiding** (`delsr` from `[9, 13]` to `[9, 12, 13]`, then back) worked. Hiding a room also
  **removes it from `cleanareaorder`**. Showing it again **puts it back at its old position** by
  itself (room 12 returned to position 11), so no separate order restore was needed.
- **Cleaning order** (rooms 7 and 8 swapped, then restored) worked and changed only those two
  rooms' positions.
- After both undos the saved map equals the one before the tests, except for a new, empty
  `sneak_areas: []` key that the robot added when it rewrote the map.
- The live frame (6.3) changed first after each write. Twice the saved map already showed the
  edit before 6.8 changed, so the read-back was correct, but whether that always holds is not
  established.

## First control command: clean one room (2026-11-16)

Approved by the user. Tool: `dreame control clean-rooms 1 --apply`, robot idle on the dock.

- Payload, as the official app sends it: action **4.1** with `piid 1` = 18 (segment cleaning) and
  `piid 10` = `{"selects":[[1,1,1,3,1]]}` (room, passes, suction, water, then 1 in the order
  field). Passes, suction and water came from room 1's stored settings.
- The result was `{"code":0,"siid":4,"aiid":1,"out":[]}`. Five seconds later the state was
  1 (sweeping), status 18 (segment cleaning), task 3 (segment cleaning).
- The robot cleaned room 1 only and docked on its own: about 2 min of cleaning, 2 m², no
  errors, about 6 min from start to "charging completed".
- Afterwards the live frame (6.3) held the run's track: map id 2, 118 path points, all inside
  room 1. The drive to and from the dock was not in it.

## Control session: zone clean, pause, resume, dock (2026-11-16)

Each command approved by the user, sent with `dreame control <cmd> --apply`. Every result was
`{"code":0,...,"out":[]}`; the status 5 s later is given.

| Command | Payload | Status 5 s later |
|---|---|---|
| Zone clean, 800 x 800 mm inside room 1 | 4.1, `piid 1` = 19, `piid 10` = `{"areas":[[-700,8150,100,8950,1,1,2]]}` | state 1 (sweeping), status 19 (zone cleaning), task 2 |
| Pause | 2.2, no input | state 3 (paused), status 1, task 7 (zone cleaning paused) |
| Start | 2.1, no input | state 1, status 19, task 2: **it resumed the zone clean**, not a full clean |
| Dock | 3.1, no input | state 5 (returning), status 3 (back home), task 0 (completed) |

- Dock ends the job: the task was reported completed while the robot drove home.
- The robot was charging on the dock about 2 minutes later; the job was recorded as 0 min, 1 m².
- Zone coordinates in world mm from the live track lined up with room 1 on the saved map, so
  zone cleans share the saved map's coordinate frame.

## Sixth and seventh writes: split a room, merge it back (2026-11-16)

Approved by the user. Both edits were made in `dreame-lab-editor`, robot idle on the dock.

- **Split:** `{"dsrid":[1250,5850,1250,4050,4],"mapid":1}`, a vertical line across room 4.
  `dsrid` alone is enough; `dsr` is not needed (maps.md §14, question 3).
  - Room 4 kept its id, type, index, cleaning-order position and settings, and went from
    2527 to 1565 pixels.
  - The new half became **room 14**, the next free id, with 962 pixels, so the halves add up to
    the old room. It has no name or type. It is visible, last in `cleanareaorder` (position 12),
    and has default settings in the live `cleanset`.
  - `nei_id` was updated on room 4, room 14 and their neighbours. No other room changed.
- **Merge:** `{"msr":[4,14],"mapid":1}`. Room 4 survived with its old type, order and
  settings, and room 14 disappeared from the saved map and the live `cleanset`.
- **The merge restored the map exactly.** The saved map after the merge has no differences
  from the one before the split: the same rooms, pixel counts, neighbours, order and settings.
- An app that splits a room must name and place the new room itself, since the robot gives it
  no name or type.

## Eighth write: move the boundary between two rooms (2026-11-16)

Approved by the user and made in `dreame-lab-editor`, robot idle on the dock. The user kept the result.
The editor sent five commands, reading the map back after each one.

| Step | Command | Result |
|---|---|---|
| 1 | `{"msr":[2,5],"mapid":1}` | Room 5 merged into room 2 |
| 2 | `{"dsrid":[75,6350,-1200,6350,2],"mapid":1}` | Split along the new line, trimmed to the room's edge |
| 3 | `nsr` with every named room | The new half got room 5's old name back |
| 4 | `{"nsm":{"5":{"material":2,"direction":0}},"mapid":1}` | Room 5's floor material restored |
| 5 | `cleanareaorder` | Room 5 back in sixth place |

- **The split reused the merged-away id.** The new half became room 5 again, not the next free
  id as in the plain split above. So after `msr` the freed id is the one a split hands out next.
- Room 2 went from 3196 to 3099 pixels and room 5 from 1523 to 1620: a strip of 97 pixels
  (about 0.24 m²) moved between them. No other room changed.
- Per-room settings needed no restore: both rooms had the same values, and the new half got
  them too.
- **A split line may end in a neighbouring room.** The first attempt in the editor was refused by
  the library, which required both ends on a wall. The app only requires the ends to lie outside
  the room, then trims them to the room's edge (map-plugin.md §6.1). The library now does the
  same, and the robot accepted the trimmed line.
- **The live `cleanset` order field does not follow `cleanareaorder`.** After step 5 the saved
  map's order was right, but the live frame still listed room 5 eleventh with later rooms moved
  up. The earlier order change (rooms 7 and 8) left that field unchanged as well. Which of the two
  the robot follows when it cleans is not known.

## Map shapes: carpets, thresholds, curtains (2026-11-16)

Approved by the user. Each shape was added in `dreame-lab-editor`, checked, then deleted, with the robot
idle on the dock. Every add was stored exactly as sent, with no extra fields, and changed nothing
else. Every delete restored the saved map exactly, and the map after the last delete equals the
one before the first add.

| Shape | Command | Stored as |
|---|---|---|
| Carpet rectangle | `{"cpt":{"addcpt":[...,[-875,3125,-175,6025,3,0,7]],"nocpt":[]}}` | `vw.addcpt`, entry as sent |
| Ignored-carpet area | `{"cpt":{"addcpt":[...],"nocpt":[[-825,3175,-225,5375]]}}` | `vw.nocpt` |
| Threshold | `{"vws":{"vwsl":[[100,6100,100,5600]],"npthrsd":[],"ramp":[]}}` | `vws.vwsl` |
| Impassable threshold | `{"vws":{"vwsl":[],"npthrsd":[[100,6000,100,5450]],"ramp":[]}}` | `vws.npthrsd` |
| Curtain | `{"curtain":{"line":[[4900,-200,4900,1850]]}}` | `ct.line` |

- The new carpet took the lowest free id (3) and kept the generic type 7. Its corners were snapped
  to cell corners before sending, as the app does.
- Unlike no-go zones, none of these entries gained a trailing field.
- Untested: carpet type and carpet cleaning settings, and whether these shapes change how the
  robot cleans. Ramps and round carpets are covered below.

## Furniture, ramps, round carpets (2026-11-16)

Approved by the user and made in `dreame-lab-editor`, robot idle on the dock. Each change was checked,
then undone, and every undo restored the saved map exactly.

| Edit | Command | Stored as |
|---|---|---|
| Move furniture item 3 by +200 mm in x | `mapfurniture_v2` with all nine items, item 3 at x 2325 | `funiture_info`, exactly as sent |
| Add furniture (type 7, 600 x 600 mm) | the nine items plus `[0,7,8,600,600,0,-675.0,1225.0,0.0,0.0,0.0,0.0,1.0,1,0]` | as sent: id 0, room 8 from the centre |
| Ramp at 90 degrees | `{"vws":{"vwsl":[],"npthrsd":[],"ramp":[[-650,1750,-150,2550,90]]}}` | `vws.ramp`, angle kept |
| Round carpet | `{"cpt":{"addcpt":[...,[-400,1850,600,700,3,1,7]],"nocpt":[]}}` | `vw.addcpt` as `[-400.0,1850.0,600.0,700.0,3,1,7]` |

- **Round carpets come back as decimals.** The robot rewrote the four coordinates as decimals and
  reordered the keys of `vw`. The parser first refused the whole `vw` object, which switched
  editing off as designed; it now accepts whole-number decimals and writes them back unchanged.
- Furniture resent with every field as stored kept the fields the model does not read. A new item
  with id 0, like the stored ones, was accepted, so ids are not unique on this robot.
- **Still open: are a round carpet's sizes widths or radii?** The robot does not mark user carpets
  in the saved grid (the carpet cell count did not change), so the map cannot show it. The app's
  own drawing of the carpet would.

## Settings writes (2026-11-16)

Approved by the user. Each setting was changed with `dreame set … --apply`, read back live,
changed back, and read back again. Every write returned code 0, every read back showed the new
value, and the full settings listing afterwards matched the one before.

| Setting | Property | Values written | Note |
|---|---|---|---|
| Volume | 7.1 | 50, then 60 | |
| Child lock | 4.27 | `true`, then `false` | boolean |
| Carpet sensitivity | 4.28 | 3, then 2 | |
| Mop drying time | 4.40 | 2, then 3 (hours) | |
| Cleans between auto-empties | 15.2 | 2, then 1 | not in the cloud cache until written |
| Per-room settings | 4.26 | 1, then 0 | stored as a number, not a boolean |
| Do not disturb | 5.1, 5.2, 5.3 | on, then off; 22:00 and 08:00 resent | three writes per change |
| Off-peak charging | 3.3 | `{"enable":true,…}`, then `false`, same times | |
| AI pet detection | 4.22 | 287, then 271 | whole bitmask, one bit changed |
| `MeticulousTwist` | 4.50 | `{"k":"MeticulousTwist","v":7}`, then -7 | one `{"k","v"}` object |

- 4.26 and 4.34 read as `0`/`1` although the profile lists them as switches, so the library writes
  on/off settings in the type the robot stores.
- 15.2 exists (discovery found it) but was missing from the cloud cache until it was written;
  reading a setting live before changing it avoids showing an unknown value.

## Map backup and restore (2026-11-16)

The user approved W1 and W8 from maps.md §16.2. The robot was docked, fully charged and idle.
Commands: `dreame map recovery`, `dreame map backup 1 --apply`, `dreame map restore 2 --apply`.
Copies of the saved map and of every archive were kept in `local/out/` before each write.

- **Backup (action 6.3, `[{"piid":4,"value":"1"}]`) worked.** The result code was 0, and 6.14 went
  from 0 through 2 to 3 within 6 s. The recovery list gained a third entry for map 1 with
  `first` 2 (backup).
- **The backup's object name has no `mb.tbz2` suffix** on r2579a. It ends in a plain number, like
  the other entries, and downloads through `getDownloadUrl`. Its files are identical to those of
  the "edited" entry, which the robot had refreshed after today's edits. Only the tar metadata
  differs.
- **Restoring the unmodified backup (6.10) worked.** The write returned code 0, and the first
  read of 6.11, 3 s later, was already 3 (success); 2 (running) was not seen. 6.8 kept the same
  value for 60 s, and the saved map's rooms, walls, restrictions, carpets and furniture were
  unchanged.
- **Per-room settings were missing from the live frame afterwards.** The live frame (6.3) the
  robot uploaded after the restore had no `cleanset` key. That frame has `ris` 2 and an
  embedded `rism`, like an end-of-clean frame. The archive holds no per-room settings either
  (`1.dat2` has only `switchcleanset` 0 and an empty `cleanarea_detail`).
- **The restore started a new map session.** The last live frame before it was map 3, frame 99,
  with `cleanset` for 11 rooms. The frame after it is map 4, frame 105, without `cleanset`.
- **Requesting a frame did not bring the settings back.** With the user's approval, action 6.1
  was sent three times (`dreame map request-frame`). The plain request (`{"frame_type":"I"}`)
  returned code 0 and empty `out` values, and no frame arrived over MQTT or in 6.3 within 60 s:
  the robot was asleep on the dock. The forced request (`req_type` 1, `force_type` 1) returned the
  object name of the same frame 105, still without `cleanset`.
- So the restore most likely dropped the per-room settings, which the archive does not hold.
  This is not proven: a sleeping robot may just resend its last frame. Any client that restores
  a map should save the per-room settings first and re-apply them afterwards with `customeClean`.
- **Re-applying them worked.** With the user's approval, `dreame map edit restore-settings` sent
  one `customeClean` with the 11 rooms' values from the last live frame before the restore. The
  result code was 0, 6.3 changed, 6.8 did not, and the new live frame held every value again.
  Only index 3 of each `cleanset` entry, which the command does not send, came back as 0 for
  every room instead of the earlier values 1 to 11. The saved `cleanareaorder` was unchanged by
  the restore and by this write.

## Map restore from a local file (2026-11-16)

The user approved W8b from maps.md §16.2: restore the same unmodified backup archive, this time
served from this computer. The robot was docked and idle. Command:
`dreame map restore-file <archive> --apply`. It serves the file over plain HTTP on the computer's LAN
address, answers only the robot's address, and writes that URL to 6.10.

- **The robot accepts a URL outside Dreame's storage.** It made one request:
  `GET /<name>.tbz2` with the agent `Wget/1.20.3 (linux-gnu)` and no `Range` header, and received
  all 328,109 bytes. No other address asked for the file. The computer's firewall did not block it.
- 6.11 went from 3 (left from the earlier restore) through 2 to 3. 6.8 kept its value, and the
  saved map's rooms, walls, restrictions, carpets and furniture were unchanged.
- The per-room settings were dropped again, as after W8. `dreame map edit restore-settings` with
  the live frame saved just before the restore put them back (result code 0).
- The recovery list did not change: a restore adds no entry.

## First edited archive: W9a (2026-11-16)

The user approved W9a from maps.md §16.2. The robot was docked and idle.

- **The edit.** A one-off script copied the W1 backup and changed 9 cells
  (a 3 x 3 block at PNG (35, 75)) from room 12 to its neighbour room 10 in
  `segmented_map_large.png`, `dec_map_large.png` and `last_seg_map_large.png`. Every other member
  and all tar metadata stayed byte for byte the same. The re-encoded PNGs are smaller (947 instead
  of 3,416 bytes) but decode to the same pixels apart from the block.
- **The robot accepted it.** `dreame map restore-file` served it; the robot fetched it with one
  `GET` (Wget) and 6.11 read 3. The live map session counter rose from 5 to 6, which confirms a
  restore took place: 6.11 already read 3 from the previous restore and 2 was never seen.
- **The edit did not show.** The `rism` in the next live frame was identical to the saved map
  before the restore in every cell, and the 9 cells (saved-map cells (36..38, 76..78)) were still
  room 12. 6.8 was not republished.
- **Inconclusive** (corrected after W9b, below). The `rism` checked here turned out to be a copy of
  the `rism` inside the archive's `last_app_map`, which the edit did not touch. So this result does
  not show whether the robot reads the PNGs.
- **Reverted.** The unmodified backup was restored the same way (one `GET`, 6.8 unchanged, no
  change in the saved map), and `restore-settings` put the per-room settings back (result code
  0).

## Second edited archive: W9b (2026-11-16)

The user approved W9b from maps.md §16.2. The robot was docked and idle.

- **The edit.** A one-off script moved door 13 in
  `divide/1/doors_info_large.json` by 4 cells (20 cm) along x: both end points and all 23 cells
  of `lines_`. The JSON kept the archive's formatting and size; every other member and all tar
  metadata stayed the same.
- **The robot accepted it.** One `GET` (Wget); 6.11 went through 1 (not seen before) to 3; the
  live map session counter rose to 8.
- **No change seen.** The `rism` of the next live frame equalled the saved map before the
  restore, and 6.8 was not republished.
- **The check itself is flawed.** That `rism` is byte for byte the `rism` inside the archive's
  `last_app_map`, and the live frame differs from `last_app_map` only in volatile keys. The robot
  most likely copies `last_app_map` back after a restore. So W9b, like W9a, is inconclusive.
- The robot was left on the W9b archive until the user decides how to check it (see maps.md
  §16.2).

## Forcing a re-save after W9b: W9c (2026-11-16)

The user approved W9c from maps.md §16.2. The robot was still on the W9b archive, docked and idle.

- **A names write with no change re-saves the map.** `dreame map edit restore-names` with a
  snapshot of the current names returned code 0, and 6.8 changed.
- **The re-saved map shows no door shift.** The new 6.8 equals the saved map before W9b in every
  cell and every trailer key.
- **The robot keeps the edited member as it is.** The robot's own archive saved after this write
  (the new "edited" recovery entry, object 103) is byte for byte the W9b archive we sent, door 13
  included at its shifted place, except `1.dat2`. In `1.dat2` only `seg_info` (room names; our
  write), `last_app_map` (a new live frame), `last_clean_area` and the order of `extend_msgs`
  changed. The segmentation PNGs were not recomputed from the shifted door either.
- **So the door lines do not drive the saved map** after a restore and a re-save. The robot
  stores the member and keeps the room layout it already had. W9a (PNGs) was not re-saved, so it
  stays inconclusive. The candidates left are the saved map embedded in `last_app_map` (its
  `rism`), which the robot copied back after every restore (maps.md §12.2), the segmentation
  PNGs, and the binary members.
- **Revert.** `restore-file` with the unchanged archive failed this time: 6.11 went 1, then 5,
  and the robot made no request to the computer. The computer and the robot share a subnet. The
  likely cause is the computer's firewall, whose rules apply per program file: this run used a
  different CLI build from the earlier successful runs. Restoring the
  same W1 backup through Dreame's signed URL (`dreame map restore 1`) worked (6.11 2, then 3), and
  `restore-settings` put the per-room settings back (code 0).
- The recovery list now holds the original, the W1 backup and a new "edited" entry; restoring
  entries in it shifts their indexes, so a client should pick entries by object name, not index.

## Embedded saved map: W9d (2026-11-16)

The user approved W9d from maps.md §16.2.

- **The archive.** A one-off script changed 9 cells (saved-map (36..38, 76..78))
  from room 12 to room 10 in the `rism` inside `1.dat2`'s `last_app_map`, keeping their wall and
  carpet bits. The raw text of `1.dat2` is unchanged outside that one string; the live frame's
  header, pixels and other trailer keys, the rism's header and trailer, every other member and all
  tar metadata are unchanged.
- **The robot never fetched it.** 6.11 went 1, then 5, and no request reached the computer. The map and
  the per-room settings stayed as they were (live map session 9, 11 rooms in `cleanset`).
- **Cause, confirmed.** The computer's firewall holds one inbound rule per program file. Builds
  that were allowed worked, and the others were blocked. The block rules most likely come from
  firewall prompts that were dismissed. `restore-file` needs an allow rule for the binary that
  runs it; this is the user's setting to change.
- **Second try, after the user allowed the build.** `restore-file` now serves on the fixed
  port 8770. The robot fetched the archive (one `GET`) and 6.11 went 2, then 3.
- **The edit did not show.** A names write with no change re-saved 6.8 (code 0, 6.8 changed), and
  the new 6.8 equals the saved map before W9d in every cell and every trailer key. So the `rism`
  inside `last_app_map` does not drive the saved map either.
- **Reverted** through Dreame's URL (W1 backup, object 102). 6.11 showed only the old 3, but the
  live map session rose from 10 to 11, which confirms the restore; `restore-settings` put the
  per-room settings back (code 0).

## Restoring the April original: W10 (2026-11-16)

The user approved W10 from maps.md §16.2. The robot was docked and idle.

- `dreame map restore 0 --apply` restored the "original" recovery entry (object 100, April)
  through Dreame's URL. 6.11 showed only the old 3, but 6.8 changed at once.
- **The new 6.8 is the April map.** 211 x 307 cells (225 x 349 before), 12 rooms (room 13 gone),
  room 9 no longer hidden, other room names, types, cleaning order and sizes, the April furniture,
  and no user carpets in `vw`. It equals the `rism` inside the archive's `last_app_map` byte for
  byte and matches the archive's segmentation PNG in every non-wall cell.
- **So a restore replaces the robot's map**, and 6.8 is republished when the map differs. The
  earlier small edits (W9b, W9d) did not change it because the robot rebuilds from members other
  than the door lines and the embedded `rism`.
- **Reverted.** Restoring the W1 backup (object 102) through Dreame's URL brought back a 6.8 equal
  to the map before W10 in every cell and trailer key. `restore-settings` put the per-room
  settings back (code 0).

## PNG edit with a forced re-save: W9e (2026-11-16)

The user approved W9e from maps.md §16.2. The robot was docked and idle.

- `restore-file` served the W9a archive (9 cells moved from room 12 to room 10 in the three
  segmentation PNGs) from a build that the user had allowed in the firewall, on
  port 8770. The robot fetched it (one `GET`); 6.8 was not republished. The live map session rose
  from 12 to 13, so the restore took place.
- A names write with no change re-saved 6.8 (code 0). The new 6.8 equalled the map before in every
  cell and trailer key; the 9 cells were still room 12.
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## PNG edit after loading another map: W11 (2026-11-16)

The user approved W11 from maps.md §16.2. The robot was docked and idle.

- `dreame map restore 0 --apply` loaded the April original (6.8 became 211 x 307).
- `restore-file` then served the W9a archive (today's map, 9 cells moved from room 12 to room 10 in
  the segmentation PNGs). The robot fetched it and 6.8 changed at once back to today's map: 225 x
  349, 13 rooms.
- That 6.8 equalled today's unedited map in every cell and trailer key. The 9 cells were still
  room 12, also after a forced re-save (names write, code 0).
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## Embedded map edit after loading another map: W12 (2026-11-16)

The user approved W12 from maps.md §16.2. The robot was docked and idle.

- `dreame map restore 0 --apply` loaded the April original (6.8 became 211 x 307).
- `restore-file` then served the W9d archive (today's map, 9 cells moved only in the `rism` inside
  `1.dat2`'s `last_app_map`). The robot fetched it and 6.8 changed at once back to today's map.
- That 6.8 equalled today's unedited map in every cell and trailer key, not the edited `rism` it
  was sent; the 9 cells were still room 12, also after a forced re-save.
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## Door line edit after loading another map: W13 (2026-11-16)

The user approved W13 from maps.md §16.2. The robot was docked and idle.

- `dreame map restore 0 --apply` loaded the April original (6.8 became 211 x 307).
- `restore-file` then served the W9b archive (today's map, door 13 moved 4 cells in
  `doors_info_large.json`). The robot fetched it and 6.8 changed at once back to today's map.
- That 6.8 equalled today's unedited map in every cell and trailer key; rooms 10 and 11 kept
  their sizes, also after a forced re-save.
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## One consistent edit of every editable member: W14 (2026-11-16)

The user approved W14 (group 1 of the proposed tests). The robot was docked and idle.

- A one-off script moved the border between rooms 12 and 10 by 3 cells into
  room 12 in the segmentation PNGs, the embedded `rism`, three door records and the wall end
  points of rooms 10 and 12 in `out_walls_large.json`. Other members and tar metadata unchanged.
- `dreame map restore 0 --apply` loaded the April original; `restore-file` then served the W14
  archive (one `GET`). 6.8 changed at once to today's map.
- The room grid equalled today's in every cell (rooms 10 and 12 kept their sizes), but 6.8's
  `walls_info` carried the edited door and wall coordinates (−2975 to −3125 mm). Both stayed after
  a forced re-save.
- Reverted with the W1 backup through Dreame's URL: 6.8 equalled today's map in every cell and
  trailer key. `restore-settings` put the per-room settings back (code 0).

## User line edit after loading another map: W15 (2026-11-16)

The user approved W15 from maps.md §16.2. The robot was docked and idle.

- A one-off script moved user line 7 of `doors_info_large.json` (border of rooms
  8 and 5) by 3 cells; nothing else changed.
- `dreame map restore 0 --apply` loaded the April original; `restore-file` then served the W15
  archive (one `GET`). 6.8 changed at once to today's map.
- That 6.8 equalled today's unedited map in every cell and trailer key; rooms 5 and 8 kept their
  sizes, also after a forced re-save.
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## Occupancy edit after loading another map: W16 (2026-11-17)

The user approved W16. The robot was docked and idle. Its "edited" recovery entry had been
renewed overnight (now object 101), but the map before W16 equalled the map of the day before in
every cell and trailer key, so reverting to the W1 backup lost nothing.

- A one-off script set a stray 6-cell obstacle cluster outside the rooms to
  unknown in `slam_map.png`, `slam_map_ori.png` and `origin_img_slam.png`; nothing else changed.
- `dreame map restore 0 --apply` loaded the April original; `restore-file` then served the W16
  archive (one `GET`). 6.8 changed at once to today's map.
- That 6.8 equalled today's unedited map in every cell and trailer key; the 6 cells were still
  walls, also after a forced re-save.
- Reverted with the W1 backup through Dreame's URL; `restore-settings` put the per-room settings
  back (code 0).

## Mixed archives and the room grid image: W17 and W18 (2026-11-17)

The user asked to continue the tests. The robot was docked and idle each time.

- **W17a.** A one-off script replaced `1.dat2` in the W1 backup with April's.
  Restored from the computer while the robot held today's map: 6.8 kept today's grid exactly but took
  April's names, types, cleaning order, `vw`, charger, Wi-Fi map and `last_complete_clean_time`.
- **W17c.** The W1 backup with April's `divide/` folder. The 6.10 write returned 80001. Afterwards
  the robot was idle and charging, 6.11 read 3, 6.8 held April's grid exactly (211 x 307) with
  today's metadata, and the live frame showed map session 3 with 0 rooms.
- **Revert after W17.** The W1 backup through Dreame's URL brought 6.8 back to today's map in every
  cell and trailer key; `restore-settings` put the settings back, and the live frame showed 11
  rooms again.
- **W18.** A one-off script moved a 3 x 3 block from room 12 to room 10 in
  `segmented_map_with_obstacle_large.png` only. After the April original, `restore-file` served it
  (one `GET`); 6.8 differed from today's map in exactly those 9 cells, and still did after a forced
  re-save. Reverted with the W1 backup (6.8 equal to today's map again) and `restore-settings`
  (code 0).

## Cleaning by an edited room border: W19 (2026-11-17)

The user chose this test and approved the clean. The robot was docked and idle.

- A one-off script moved the border between room 12 (custom-named, about
  3.7 m²) and room 10 by 0.5 m into room 12 in `segmented_map_with_obstacle_large.png`
  only. After the April original, `restore-file` loaded it; 6.8 differed from today's map in exactly
  the 290 cells. `restore-settings` put the per-room settings back.
- `dreame control clean-rooms 10 --apply` started a clean of room 10 only (`selects`
  `[[10,1,1,3,1]]`). The robot cleaned, returned and was charging about 20 minutes later.
- The live frame's path (390 points) covered the strip to its far edge and across its whole free
  width, and never entered the rest of room 12.
- The map the robot saved after the run had the old border again. Reverted with the W1 backup
  (6.8 equal to the earlier map in every cell and trailer key) and `restore-settings`; this also
  dropped the run's own small map updates.

## Durable room border edit: W20 (2026-11-17)

The user approved W20. The robot was docked and idle.

- A one-off script moved the room 12/10 border 0.5 m into room 12 in all four
  segmentation images. After the April original, `restore-file` loaded it (one `GET`); 6.8 had
  room 10 at 3,454 and room 12 at 1,224 cells. `restore-settings` put the settings back.
- `dreame control clean-rooms 10 --apply` cleaned room 10; the robot was charging again about
  15 minutes later. The track covered the strip (44 of 429 path points).
- The map saved after the run kept 261 of the 290 edited cells in room 10. About 4,600 other cells
  changed (carpet flags, wall bits, room edges), and the map grew by one row. The rendered map looked
  normal.
- The user chose to revert: the W1 backup through Dreame's URL brought 6.8 back to the pre-test map
  in every cell and trailer key, and `restore-settings` put the settings back (code 0). This also
  dropped the run's re-mapping.

## Archive edit from the CLI: W21 and W22 (2026-11-17)

The user approved both runs. The robot was docked and idle.

- W21: `dreame map edit-archive grow 10 12 up 10 --apply` backed up the map, moved the room 12/10
  border 0.5 m into room 12 in the backup, restored the April original and then the edited
  archive, and put the room settings back (code 0). 6.8 had room 10 at 3,454 and room 12 at 1,224
  cells and no other change. The new backup replaced the earlier one in the recovery list.
- W22: `dreame control clean-rooms 10 --apply` cleaned room 10 in about 10 minutes. The map
  saved after the run kept all 290 edited cells; 13 other cells changed (12 carpet flags, 1 room
  edge cell).
- The user kept the edited map.

## Wall bits through the archive: W23 (2026-11-17)

The user chose the test. The robot was docked and idle.

- `dreame map edit-archive wall -1125 7875 275 7875 --apply` marked door 2, the only way into
  room 1, as wall (22 cells). 6.8 had the wall after the restore. The live frame briefly showed
  0 as every room's cleaning-order position; `cleaning-order` resent the old order (code 0).
- `dreame control clean-rooms 1 --apply` cleaned room 1 in about 5 minutes without an error. The
  path went through the wall line, and the map saved after the run had no wall bits left on it.
- Nothing needed reverting. The room 12/10 border from W21 is still in place.

## New room id through the archive: W24 (2026-11-17)

The user chose the test. The robot was docked and idle.

- `dreame map edit-archive split 7 14 x 2600 --apply` split room 7 at x = 2,600 mm; its east half
  became room 14 in the grid images only. 6.8 showed room 14 after the restore.
- `dreame control clean-rooms 14 --apply` cleaned exactly room 14 in about 13 minutes.
- The map saved after the run merged room 14 back into room 7, and room 7 also took 1,671 cells
  of room 11.
- The user chose to revert: `restore-file` of the W24 fresh backup, then `restore-settings` and
  `cleaning-order` (code 0 each). 6.8 equalled the map before W24 in every cell and trailer key.

## Cloud and map checks without a robot write (2026-11-17)

None of these checks writes to the robot.

- **P-frame merge.** The test `mqtt_p_frames_merge` in `crates/dreame-map/tests/real_data.rs` merges the 122 captured P-frames onto the earlier live frame and compares the result with the history map (frame 123). With the add rule, 7 cells differ from the history map. With the replace rule, 1,509 differ, and the base frame alone differs in 1,047. The history path is an in-order subsequence of the merged path: 252 of its 255 points, and the lengths differ by 3 mm.
- **Unsigned request bodies.** The device list (`iotuserbind/device/listV2`) without `sign` and `timestamp` succeeded with code 0. The server does not require the signature, at least for this read. The library still signs every body, as the app does.
- **Old refresh tokens.** After a refresh rotated the refresh token, the token from before the refresh still worked: it gave a new access token, and the device list worked with it. After that, the newest refresh token also still worked. A refresh therefore does not revoke the token it replaces, and using an old token does not revoke a newer one. The live test `live_unsigned_body_and_old_refresh_token` in `crates/dreame-cloud/tests/live.rs` repeats both checks.

## Stop during a room clean (2026-11-17)

Approved by the user. The robot was docked, idle and fully charged.

- `dreame control clean-rooms 1 --apply` started a room clean (code 0). About 25 s later the state was 1 (sweeping), status 18 (segment cleaning), task 3.
- `dreame control stop --apply` sent action **4.2** with no input. The result was `{"code":0,"siid":4,"aiid":2,"out":[]}`.
- Five seconds later the state was 2 (idle), status 17 (standby), task 0 (completed), with no error. The user saw the robot stop where it was.
- It stayed in that state for the next minute, off the dock and not charging. Stop ends the job but does not send the robot home, unlike dock (3.1), which ends the job and returns. The job was recorded as 0 min, 0 m².

## Not yet tested

- Writes other than those recorded above: actions other than room clean, zone clean, start, pause, stop, dock, frame request (6.1), map backup (6.3), map restore (6.10) and remote control (see [remote-and-camera.md](remote-and-camera.md)), settings other than those in "Settings writes", and map edits other than `nsr`, `nsm`, `vw.rect`, `customeClean`, `delsr`, `cleanareaorder`, `dsrid`, `msr`, `cpt`, `vws`, `curtain` and `mapfurniture_v2`. Each needs the user's approval, one at a time.
- How long a refresh token lives, and whether an old one stops working after some time or some number of refreshes.
