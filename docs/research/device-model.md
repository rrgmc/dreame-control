# Dreame robot vacuum device model

This document is the reference for the device-model layer of the Rust library. That layer covers
MIoT properties, actions, events, enums, composite value codecs and capability detection. Transport,
authentication and the map binary format are out of scope; see the other research documents.

## 0. Sources, conventions and terms

### 0.1 Citation tags

| Tag | Source | Notes |
|---|---|---|
| **R** | Read-only prototype tools against the user's robot, 2026-11-15 | **Verified.** Model `dreame.vacuum.r2579a` (L40 Ultra AE), firmware `4.3.9_1664` |
| **T** | `local/refs/dreame-vacuum/custom_components/dreame_vacuum/` (Tasshack, Python, Home Assistant), tag v2.0.3, commit `fec086c` | Largest mapping; one global table for all models |
| **T/types**, **T/device**, **T/map**, **T/const** | `T/dreame/types.py`, `T/dreame/device.py`, `T/dreame/map.py`, `T/dreame/const.py` | |
| **N** | `local/refs/node-dreame/src/` (TypeScript, Dreamehome cloud) | Says it verified values on r2532a (X50 Ultra Complete) and r2449a (X40 Ultra Complete) |
| **H** | `local/refs/com.dreame.vacuum.cloud/` (Homey app, JavaScript) | |
| **S/r2320** | MIoT spec `urn:miot-spec-v2:device:vacuum:0000A006:dreame-r2320:1` from `https://miot-spec.org/miot-spec-v2/instance?type=<urn>` | Newest public Dreame spec. No public spec exists for r24xx or r25xx models. |
| **APK** | Dreamehome Android app, asset `assets/flutter_assets/assets/home_device/common_device_protocol.json` | Value-to-label maps keyed by `"<siid>.<piid>"` |

Line numbers for T refer to the commit above. A claim with an R citation was observed on the user's
robot. A claim without R comes from the cited code and is not verified on r2579a.

### 0.2 Terms

- **MIoT**: Xiaomi's device model. A device has **services**. Each service has **properties**,
  **actions** and **events**.
- **siid**: service id. **piid**: property id. **aiid**: action id. **eiid**: event id.
- **DIID**: the string key `"<siid>.<piid>"`, for example `"4.23"` (T/types:2722).
  `"4.a1"` means action 1 of service 4. `"4.e1"` means event 1 of service 4.
- **Codec**: code that turns a raw wire value into a typed value and back.
- **Capability**: a feature flag, for example "has a self-wash base".
- **Capability set**: Tasshack's per-model list of capability flags. It lives in the `DEVICE_INFO`
  blob (T/const:736).
- **fw build**: the integer after `_` in the firmware string. For `4.3.9_1664` it is `1664`.
  Tasshack calls it `info.version` (T/device:11234-11239).
- **Cache sweep**: the R prototype read the cloud property cache (`iotstatus/props`) for siid 1-30 and
  piid 1-60. A property that appears in the sweep exists on the robot. A property that does not
  appear may still exist; it may simply never have been reported to the cloud.

### 0.3 Observed column legend

The property tables have an **r2579a** column. Its values are:

| Marker | Meaning |
|---|---|
| a value, for example `8` | **Verified** present, with the value from the cache sweep (R) |
| `absent` | Inside the swept range (siid 1-30, piid 1-60) but not in the cache. Probably not supported. |
| `n/s` | Not swept (siid above 30 or piid above 60). Unknown. |

Identifiers in values are replaced by `<did>`, `<uid>` and `<serial>`.

---

## 1. Overview of the MIoT model

### 1.1 Wire operations

| Operation | Shape | Notes |
|---|---|---|
| Read properties | `get_properties` with `[{did, siid, piid}]`; reply items `{siid, piid, value, code}` | `code == 0` means success. Values are typed (numbers as numbers) when relayed through `sendCommand` (R). |
| Write properties | `set_properties` with `[{did, siid, piid, value}]` | |
| Call action | `action` with `{did, siid, aiid, in: [{piid, value}]}`; reply has `out` | In the Dreame cloud envelope, `params` is one object, not an array (N/commands.ts:164-189). |
| Push | `properties_changed` and `event_occured` (sic) over MQTT | Event arguments are `{piid, value}` lists (N/mqtt.ts:103-118). |
| Cloud cache | `iotstatus/props` with `keys` as **one comma-separated string**, for example `"2.1,3.1"` | Values come back as **strings**. JSON values are JSON-encoded strings. A JSON array for `keys` returns an empty list silently (R). |

The library must therefore accept the same property as a typed value (from the device) or as a
string (from the cache), and normalise both.

### 1.2 Value kinds

Dreame uses plain MIoT types plus several encodings inside them:

| Kind | Examples |
|---|---|
| int, bool, string | 2.1, 4.11, 8.1 |
| JSON inside a string | 3.3, 4.50, 5.4, 6.8, 10001.1 |
| packed integer | 4.23 (cleaning mode group), schedule config ints, mopping-settings nibbles |
| comma list | 4.18 faults, station operation `"op,flag"` |
| base64 text inside JSON | shortcut and room names |
| int or JSON, depending on generation | 4.22 |

Tasshack writes compact JSON (`separators=(",",":")`). The decoder must branch on the runtime type
for 4.22 and similar properties.

### 1.3 The user's robot (verified)

| Fact | Value | Source |
|---|---|---|
| Model | `dreame.vacuum.r2579a` (cloud device list and property 1.2 agree) | R |
| Product | Dreame L40 Ultra AE | R; Tasshack supported-device list, https://dv.tasshack.com/guide/more/supported-devices |
| Firmware | `4.3.9_1664`, so fw build 1664 | R |
| Serial prefix | `R2579` | R |
| Local surface | none: no miIO reply on UDP 54321, all probed TCP ports dropped | R |
| Map frames | not encrypted: 6.3 has no `,<key>` suffix; frames inflate with zlib directly | R |

The earlier guess that the AE is an `r2492` variant was wrong. Implementers must read the model at
runtime and must not hard-code it.

---

## 2. Property, action and event tables

Column notes:

- **Acc**: R read, W write, N notify. "Spec" means from S/r2320. Otherwise access comes from
  Tasshack's `READ_ONLY_PROPERTIES` (T/types:2461-2494) and `READ_WRITE_PROPERTIES`
  (T/types:2497-2571). Blank means unknown.
- **Type**: MIoT format when the spec gives it; otherwise inferred from code.
- **Name**: Tasshack's `DreameVacuumProperty` name (T/types:1095-1468). The siid/piid mapping is at
  T/types:1596-1970. Actions are at T/types:1973-2016.

### 2.1 siid 1: device information (standard MIoT)

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | manufacturer | str | R | `"dreame"` (N/spec/device-props.ts:12) | absent |
| 2 | model | str | R | full model string (N/spec/device-props.ts:13-14) | `dreame.vacuum.r2579a` (R, direct read; not in cache) |
| 3 | serial-number / did | str | R | | absent |
| 4 | firmware-revision | str | R | N: build number only | absent |
| 5 | serial-no | str | RN | printed serial | `<serial>` |

Tasshack does not map siid 1. It takes the firmware version from cloud device info or `_otc.info`
(T/device:579-585).

### 2.2 siid 2: vacuum (core state)

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | STATE | uint8 | RN (spec) | `DreameVacuumState`, see 3.1 | `8` (DRYING) |
| 2 | ERROR | uint8 | RN | `DreameVacuumErrorCode`, see 3.4. Values above the spec range 100 occur. | `68` |
| 3 | spec `mode` | uint8 | RWN | Spec: 0 Silent, 1 Basic, 2 Strong, 3 Full Speed. Tasshack uses 4.4 instead. | `2` (equals 4.4, see note A) |
| 4 | spec `room-ids` | str | | Input of spec action 2.a3. Unused by T, N and H. | absent |
| 6 | N: `CLEAN_MODE_SETTING` | int | RW | Plain cleaning mode 0..3; N says a write updates the low bits of 4.23 (N/spec/vacuum-props.ts:55-84). Not in T. | `3` (matches 4.23 low bits) |
| 8 | H: `MOPPING_TYPE` | int | | 0 standard, 1 deep, 2 quick (H `drivers/vacuum/device.js:17, 236-237`) | `0` |

Note A: 2.3 and 4.4 both read 2 on r2579a. This suggests 2.3 mirrors the suction level, but one
coincident value does not prove it.

| aiid | Name | In | Notes |
|---|---|---|---|
| 1 | START | none | start or resume a full clean |
| 2 | PAUSE | none | |
| 3 | spec start-room-sweep | `[2.4]` | not used by any reference library |

Events: `2.e2` is "status changed", with no arguments (N/mqtt.ts:103-118).

### 2.3 siid 3: battery

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | BATTERY_LEVEL | uint8 % | RN | 0..100 | `99` |
| 2 | CHARGING_STATUS | uint8 | RN | 1 charging, 2 not charging, 3 charging completed (older firmware), 5 returning to charge (T/types:219-226; spec agrees on 1, 2, 5). Tasshack derives "completed" from level vs 28.47 (T/device:8272-8284). | `1` |
| 3 | OFF_PEAK_CHARGING | JSON-str | RW | `{"enable":bool,"startTime":"HH:MM","endTime":"HH:MM"}` (T/device:4295-4319) | `{"enable":false,"startTime":"22:00","endTime":"08:00"}` |

aiid 1 CHARGE returns the robot to the dock. Event `3.e1` is low battery (spec).

### 2.4 siid 4: vacuum-extend (Dreame's main service)

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | STATUS | int32 | RN | `DreameVacuumStatus`, see 3.2. Also the first `in` of 4.a1. | `14` (SLEEPING) |
| 2 | CLEANING_TIME | uint32 min | RN | current or last job | `15` |
| 3 | CLEANED_AREA | uint32 m² | RN | current or last job | `11` |
| 4 | SUCTION_LEVEL | int8 | RWN | 0 quiet, 1 standard, 2 strong, 3 turbo (T/types:485-492) | `2` (strong) |
| 5 | WATER_VOLUME | int8 | RWN | 1 low, 2 medium, 3 high (T/types:515-521). Only used without a self-wash base (T/device:2258-2259). | absent |
| 6 | WATER_TANK | int8 | RN | 0 not installed, 1 installed, 2 mop installed, 3 installed, 10 mop installed, 99 in station (T/types:505-512) | `1` (installed) |
| 7 | TASK_STATUS | int8 | RN | `DreameVacuumTaskStatus`, see 3.3 | `0` (completed) |
| 8 | CLEANING_START_TIME | int | event | Unix seconds; arrives in 4.e1 (N) | absent (event only) |
| 9 | CLEAN_LOG_FILE_NAME | str | event | `"<object>[,<aes_key>]"` (T/types:4509-4517) | absent (event only) |
| 10 | CLEANING_PROPERTIES | JSON-str | W, event | `in` of 4.a1, 4.a4, 4.a8; task summary in 4.e1. See 4.6, 4.7. | absent (write only) |
| 11 | RESUME_CLEANING | bool | RWN | resume after recharge | `1` |
| 12 | CARPET_BOOST | bool | RWN | | `1` |
| 13 | CLEAN_LOG_STATUS | uint16 | event | 0 interrupted, 1 completed, 2 manually ended, 3 failed (T/types:4657-4661) | absent (event only) |
| 14 | SERIAL_NUMBER | str | RN | | absent |
| 15 | REMOTE_CONTROL | JSON-str | W | `{"spdv":v,"spdw":w,"audio":"true"/"false","random":n}` (T/device:5942-5968) | absent (write only) |
| 16 | MOP_CLEANING_REMAINDER | uint32 | RWN | spec range 0..120 | absent |
| 17 | CLEANING_PAUSED | int32 | RN | spec keep-sweeper-time | `0` |
| 18 | FAULTS | str | RN | comma list of active codes (T/device:1482-1524). See 4.5. | `68` |
| 19 | NATION_MATCHED | str | RN | | absent |
| 20 | RELOCATION_STATUS | int32 | RN | 0 located, 1 locating, 10 failed, 11 success (T/types:556-563) | `0` |
| 21 | OBSTACLE_AVOIDANCE | bool | RWN | | absent |
| 22 | AI_OBSTACLE_DETECTION | uint32 or JSON-str | RWN | Bitmask on new models. See 4.3. | `271` |
| 23 | CLEANING_MODE | uint32 | RWN | Packed. See 4.1. | `5123` |
| 24 | UPLOAD_MAP | | | | `1` |
| 25 | SELF_WASH_BASE_STATUS | uint32 | RN | 0 idle, 1 washing, 2 drying, 3 returning, 4 paused, 5 adding water, 6 returning to drain, 7 returning for dry mop (T/types:672-683) | `2` (drying) |
| 26 | CUSTOMIZED_CLEANING | bool | RWN | use per-room settings | `0` |
| 27 | CHILD_LOCK | bool | RWN | | `0` |
| 28 | CARPET_SENSITIVITY | uint32 | RWN | 1 low, 2 medium, 3 high | `2` |
| 29 | TIGHT_MOPPING | uint32 | RWN | | `0` |
| 30 | CLEANING_CANCEL | uint32 | RN | | absent |
| 31 | Y_CLEAN | | | | `0` |
| 32 | WATER_ELECTROLYSIS | | RW | | absent |
| 33 | CARPET_RECOGNITION | bool | RWN | N: 3 seen only in "Ignore" mode | `1` |
| 34 | SELF_CLEAN | bool | RWN | wash mop during the job | `1` |
| 35 | WARN_STATUS | uint32 | RN | meaning unknown | `101` |
| 36 | CARPET_CLEANING | uint32 | RWN | T: 1 avoid, 2 adaptation, 3 remove mop, 4 adaptation without route, 5 vacuum and mop, 6 ignore, 7 cross (T/types:542-553). N (r2532a): 2 = vacuum and lift mop. | `2` |
| 37 | AUTO_ADD_DETERGENT | uint32 | RWN | 0/1; 2 unavailable (T/types:2081-2083) | `1` |
| 38 | CAPABILITY | int | | meaning unknown | `-1` |
| 39 | SAVE_WATER_TIPS | | | | `0` |
| 40 | DRYING_TIME | uint32 h | RWN | hours; `DRYING_TIME_LIST` may override the choices | `3` |
| 41 | LOW_WATER_WARNING | uint32 | RN | 0/1 none, 2 no water, 3 after clean, 4 for clean, 5 low, 6 tank missing (T/types:907-917) | `0` |
| 42-44 | MAP_INDEX, MAP_NAME, CRUISE_TYPE | | history | task summary fields | absent |
| 45 | AUTO_MOUNT_MOP | bool | RW | | `0` |
| 46 | MOP_WASH_LEVEL | int | RW | 0 water-saving, 1 standard, 2 deep (T/types:699-705); "ultra" is AutoSwitch `SuperWash` (T/device:8727-8738) | `1` (standard) |
| 47 | SCHEDULED_CLEAN | int | RN | non-zero when started by schedule or shortcut (T/device:9113-9131) | `0` |
| 48 | SHORTCUTS | JSON-str | RW | See 4.8. | `[]` |
| 49 | INTELLIGENT_RECOGNITION | bool | RW | | absent |
| 50 | AUTO_SWITCH_SETTINGS | JSON-str | RW | key/value store. See 4.2. | JSON list (see 4.2) |
| 51 | AUTO_WATER_REFILLING | int | RW | 1 means plumbed refill on | `0` |
| 52 | MOP_IN_STATION | bool | RN | | `0` |
| 53 | MOP_PAD_INSTALLED | int | RN | 0 means installed (T/device:9211-9216) | `0` (installed) |
| 54 | WATER_CHECK | | RN | | absent |
| 55 | DRY_STOP_REMAINDER | | | | `0` |
| 56 | NUMERIC_MESSAGE_PROMPT | int | N | (N/spec/vacuum-props.ts:367-376) | `0` |
| 57 | MESSAGE_PROMPT | str | N | | `null` |
| 58 | TASK_TYPE | int | RN | `DreameVacuumTaskType`, see 3.3 | `0` (idle) |
| 59 | PET_DETECTIVE | | | | absent |
| 60 | DRAINAGE_STATUS | int | RN, W(0) | 0 idle, 1 draining, 2 success, 3 failed (T/types:920-927) | `0` |
| 61 | DOCK_CLEANING_STATUS | int | N | N: countdown in seconds | n/s |
| 63 | CLEANING_PROGRESS | int % | N | 0..100 | push only; rose 6 to 100 during a clean (verified) |
| 64 | DRYING_PROGRESS | int | N | minutes of drying (N) | push only; +1 every 108 s while drying (verified) |
| 83 | DEVICE_CAPABILITY | int | N | N saw `31`; bits not decoded | n/s |
| 105 | ONBOARD_WETNESS_LEVEL | int | RW | replaces 28.1 with capability `ONBOARD_WETNESS_LEVEL` | n/s |

| aiid | Name | In | Notes |
|---|---|---|---|
| 1 | START_CUSTOM | `[{piid:1, value:<STATUS>}, {piid:10, value:<JSON-str>}?]` | See 4.6 (T/device:4596-4619) |
| 2 | STOP | none | |
| 3 | CLEAR_WARNING | none | N says verified |
| 4 | START_WASHING | `[{piid:10, value:"<op>,<flag>"}]` | Station operations, see 4.7. Firmware build <= 1037 sends no parameter (T/device:5392-5405). |
| 6 | GET_PHOTO_INFO | | obstacle photo metadata |
| 8 | SHORTCUTS | `[{piid:10, value:'{"cmd":..,"params":{..}}'}]` | See 4.8 (T/device:3785-3801) |

Events: `4.e1` is the task summary, with arguments for piids 1, 2, 3, 6, 8, 9, 10, 13, 42, 43, 44 and
others (T/types:4457-4558). `4.e3` is "task ended". `4.e4` has unknown meaning.

### 2.5 siid 5: do not disturb (DND)

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | DND | bool | RWN | legacy single window | `false` |
| 2 | DND_START | str `HH:MM` | RWN | | `22:00` |
| 3 | DND_END | str `HH:MM` | RWN | | `08:00` |
| 4 | DND_TASK | JSON-str | RW | `[{"id":1,"en":bool,"st":"HH:MM","et":"HH:MM","wk":127,"ss":0}]`; `wk` is a 7-bit weekday mask (T/device:1712-1727, 4227-4271). When present, 5.1-5.3 are ignored. | absent |

aiid 1 DELETE_DND_TASK. The switches 28.14-28.16 need capability `DND_FUNCTIONS`, which r2579a has
for any firmware (section 5.3).

r2579a appears to use the legacy single window (5.1-5.3), not the task list (5.4).

### 2.6 siid 6: map

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | MAP_DATA | str | N | inline base64 frame, optionally `,<key>` | absent (empty in cache) |
| 2 | FRAME_INFO | JSON-str | W | `in` of 6.a1 | absent (write only) |
| 3 | OBJECT_NAME | str | N | cloud object path of the latest frame | `ali_dreame/<uid>/<did>/0` |
| 4 | MAP_EXTEND_DATA | JSON-str | W | `in` of 6.a2 (map edits, see 4.9) | absent (write only) |
| 5 | ROBOT_TIME | int64 ms | N | | absent |
| 6 | RESULT_CODE | int16 | N | `out` of 6.a2 | absent |
| 7 | MULTI_FLOOR_MAP | bool | RWN | multi-map switch | `0` |
| 8 | MAP_LIST | JSON-str | RN | pointer `{"object_name" or "obj_name": .., "md5": ..}` to the saved-map list (T/device:956-972) | `{"obj_name":"ali_dreame/<uid>/<did>/9","md5":"<md5>"}` |
| 9 | RECOVERY_MAP_LIST | JSON-str or str | R | pointer to the backup list | `ali_dreame/<uid>/<did>/8` (plain string) |
| 10 | MAP_RECOVERY | | W | | absent |
| 11 | MAP_RECOVERY_STATUS | int | RN | 0 idle, 2 running, 3 success, 4/5 fail | absent |
| 13 | OLD_MAP_DATA | str | N | `"0,<base64>"` inline or `"1,<object>[,<key>]"` pointer (T/map:337-345) | absent (empty in cache) |
| 14 | MAP_BACKUP_STATUS | int | RN | 0 idle, 2 running, 3 success, 4 fail | absent |
| 15 | WIFI_MAP | | | | absent |

| aiid | Name | In / Out |
|---|---|---|
| 1 | REQUEST_MAP | in `[{piid:2, value:'{"frame_type":"I"}'}]`; optional keys `req_type`, `force_type`, `map_id`, `frame_id`, `time`, `type`, `index`, `roomID` (T/const:692-699). Out holds piids 1, 3, 5, 13. |
| 2 | UPDATE_MAP_DATA | in `[{piid:4, value:<JSON-str>}]`, out `[{piid:6}]` |
| 3 | BACKUP_MAP | |
| 4 | WIFI_MAP | |

### 2.7 siid 7: audio and voice

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | VOLUME | int32 | RWN | 0..100 | `60` |
| 2 | VOICE_PACKET_ID | str | RWN | language pack id | `PT` |
| 3 | VOICE_CHANGE_STATUS | str | RN | download progress | `{"id":"PT","state":"success","progress":100}` (a JSON string) |
| 4 | VOICE_CHANGE | JSON-str | W | `{"id","url","md5","size"}` (T/device:5970-5979) | absent (write only) |
| 5 | VOICE_ASSISTANT | int | RW | | `0` |
| 7 | CURRENT_CITY | str | | | a city name in Chinese characters |
| 10 | VOICE_ASSISTANT_LANGUAGE | str | RW | `""`, EN, DE, RU, IT, FR, KO, ZH (T/types:851-861) | absent |
| 11-23 | voice assistant details | | | mostly unused by T (T/types:1655-1667) | absent |

aiid 1 LOCATE (beep). aiid 2 PLAY_SOUND. N says both are verified.

### 2.8 siid 8: time and schedules

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | TIMEZONE | str | RW | IANA name | `Europe/Berlin` |
| 2 | SCHEDULE | str | RW | `;`-separated slots, see 4.4 | absent (no schedule set, or not cached) |
| 3 | SCHEDULE_ID | str | | `in` of delete | absent |
| 4 | SCHEDULE_CANCEL_REASON | uint8 | RN | | absent |
| 5 | CRUISE_SCHEDULE | str | RW | camera patrol schedules | absent |

aiid 1 DELETE_SCHEDULED_TASK, aiid 2 DELETE_CRUISE_SCHEDULE, aiid 3 DELETE_PET_CRUISE_SCHEDULE.

### 2.9 Consumables (one service each)

Each service has p1 and p2 (time left and percent left; **the order varies**) and aiid 1 = reset.
Tasshack's local reset defaults are: main brush 300 h, side brush 200 h, filter 150 h, sensor 30,
tank filter 30, wheel 30 (T/device:3862-3895, 3952-3958).

| siid | Consumable | p1 | p2 | r2579a p1 / p2 | Check |
|---|---|---|---|---|---|
| 9 | Main brush | hours left | % left | `192` / `64` | 192 / 300 = 64 %. **Order verified.** |
| 10 | Side brush | hours left | % left | `92` / `46` | 92 / 200 = 46 %. **Order verified.** |
| 11 | Filter | % left | hours left | `28` / `42` | 42 / 150 = 28 %. **Reversed order verified.** |
| 16 | Sensors | % left | time left (T: hours; N: days) | `45` / `13` | 13 / 30 = 43 %. Consistent with a 30-unit lifetime and a truncated p2. Unit still open. |
| 17 | Tank filter | % | time | absent | |
| 18 | Mop pad | % | time | absent | |
| 19 | Silver ion | time | % | absent | |
| 20 | Detergent | % | time | absent | |
| 24 | Squeegee | time | % | absent | |
| 25 | Dirty water channel | time | % | absent | |
| 26 | Onboard dirty water tank | time | % | absent | |
| 29 | Deodorizer | time | % | absent | |
| 30 | Wheel (dirty) | time | % | `20` / `68` | 20 / 30 = 67 %. Consistent with a 30-unit lifetime and a truncated p1. |
| 31-38 | Scale inhibitor, fluffing roller, roller mop filter, water outlet filter, track, washboard, dock filter | | | n/s | |

Sources for the order: the `*_TIME_LEFT` / `*_LEFT` names at T/types:1718-1960 and actions at
T/types:1973-2016. Units: N/spec/consumable-props.ts:1-55.

Notes for r2579a:

- The mop pad (18) and detergent (20) are absent from the cache, although the robot has mop pads
  and auto detergent (4.37 = 1). The app may track these in the cloud, or they may not be cached.
- The capability set lists `DEODORIZER_MODULE`, but siid 29 is absent.

### 2.10 siid 12: lifetime statistics

| piid | Name | Unit | r2579a |
|---|---|---|---|
| 1 | FIRST_CLEANING_DATE | Unix seconds | `1776452925` |
| 2 | TOTAL_CLEANING_TIME | minutes | `6474` |
| 3 | CLEANING_COUNT | count | `250` |
| 4 | TOTAL_CLEANED_AREA | m² | `5082` |
| 5 | TOTAL_RUNTIME | | absent |
| 6 | TOTAL_CRUISE_TIME | | absent |

### 2.11 siids 13 and 14: robot configuration

| DIID | Name | Notes | r2579a |
|---|---|---|---|
| 13.1 | MAP_SAVING | Presence means a VSLAM robot (T/types:3292) | absent, so LiDAR |
| 14.3 | (not in T) | unknown | `200` |
| 14.4 | KEEP_ALIVE | N: "needs attention" flag (N/spec/notification-props.ts:26) | `0` |
| 14.5 | GET_PUBLIC_KEY | | absent |
| 14.6 | MATTER_NETWORK_STARTUP | | absent |
| 14.7 | COUNTRY_CODE | | absent |
| 14.9 | MAINS_POWER_FREQUENCY | | absent |

### 2.12 siid 15: auto-empty

| piid | Name | Type | Acc | Values / notes | r2579a |
|---|---|---|---|---|---|
| 1 | AUTO_DUST_COLLECTING | uint32 | RWN | Mode. Without capability `INTELLIGENT_AUTO_EMPTY`: 0 off, 1 standard, 2 high frequency, 3 low frequency (T/types:776-783). With it: 0 off, 1 standard, 2 intelligent, 3 low, 4 custom, 5 high (T/types:786-795). Selection at T/device:8780-8810. | `2` (high frequency; r2579a lacks `INTELLIGENT_AUTO_EMPTY`) |
| 2 | AUTO_EMPTY_FREQUENCY | uint32 | RWN | cleans between empties | absent |
| 3 | DUST_COLLECTION | uint32 | RN | 0 not available, 1 available, 2 over-use, 3 never (T/types:652-659) | `1` |
| 5 | AUTO_EMPTY_STATUS | uint8 | RN | 0 idle, 1 active, 2 not performed, 3 returning (T/types:662-669) | `0` |

aiid 1 START_AUTO_EMPTY; out `[15.4 result-code]`.

### 2.13 siid 27: station status (read-only)

| piid | Name | Values | r2579a |
|---|---|---|---|
| 1 | CLEAN_WATER_TANK_STATUS | 0 installed, 1 not installed, 2 low water, 3 checking | `0` |
| 2 | DIRTY_WATER_TANK_STATUS | 0 installed, 1 not installed or full | `0` |
| 3 | DUST_BAG_STATUS | 0 installed, 1 not installed, 2 check | `0` |
| 4 | DETERGENT_STATUS | 0 installed, 1 disabled, 2 low | `0` |
| 5 | STATION_DRAINAGE_STATUS | 0 idle, 1 draining | absent |
| 6 | LDS_STATUS | N: X50 mast raised or lowered | absent |
| 7 | AI_MAP_OPTIMIZATION_STATUS | | `0` |
| 8 | SECOND_CLEANING_STATUS | | absent |
| 9 | WATER_TANK_STATUS | with capability `WATERBOX_EXTEND` | absent |
| 10 / 11 | ADD_CLEANING_AREA_STATUS / RESULT | | `0` / `0` |
| 15 | HOT_WATER_STATUS | 0 disabled, 1 enabled | absent |
| 16 | THRESHOLD_CONFIRMATION_STATUS | | `0` |
| 17 / 18 | DUST_BAG_DRYING_LEFT / STATUS | status 0 idle, 1 drying, 2 paused | absent |
| 21-45 | other hardware families | (T/types:1782-1793) | absent up to 27.45 |

Enums: T/types:996-1075.

### 2.14 siid 28: extended settings (gen5)

Most entries are 0/1 switches. All mappings come from T/types:1794-1913. Only two piids in 28.1-28.60
are cached on r2579a.

| piid | Name | Notes | r2579a |
|---|---|---|---|
| 1 | WETNESS_LEVEL | Mop wetness 1..32, default 16. Three-level view: below 6 slightly dry, above 26 wet (T/device:1844-1862). | `15` |
| 2 | CLEAN_CARPETS_FIRST | | absent |
| 5 | CLEANGENIUS_MODE | 2 vacuum and mop, 3 mop after vacuum | `2` (vacuum and mop) |
| 8 | WATER_TEMPERATURE | 0 normal ... 4 max | absent (r2579a lacks `WATER_TEMPERATURE`) |
| 14 / 15 / 16 | DND_DISABLE_RESUME_CLEANING / DND_DISABLE_AUTO_EMPTY / DND_REDUCE_VOLUME | | absent |
| 18 | DYNAMIC_OBSTACLE_CLEANING | in r2579a's capability set | absent |
| 22 | SMART_MOP_WASHING | in r2579a's capability set | absent |
| 27 | SILENT_DRYING | | absent |
| 29 | SIDE_BRUSH_CARPET_ROTATE | | absent |
| 38 | OBSTACLE_CROSSING | X50 only | absent |
| 46 | MOP_TEMPERATURE | | absent |
| 47 | BATTERY_CHARGE_LEVEL | charge limit | absent |
| 61-117 | newer hardware (ring light, robot arm, mop pressure, auto-empty area 28.95, stain recognition, and more) | | 61-117 n/s |

The other 28.x names are listed in T/types:1794-1913. Several features in r2579a's capability set
(28.18, 28.22, 28.14-16) are absent from the cache. Either the cache omits unchanged settings, or
these features live elsewhere on this model. A direct `get_properties` check is needed (section 7).

### 2.15 siids 34, 40, 99 and 10001

The follow-up cache sweep of siid 31-60, 99 and 10001 on r2579a found only siids 99 and 10001
(see §2.17). siid 34 and 40 returned nothing.

| siid | Content | Source |
|---|---|---|
| 34 | BLOCK_EXCEPTION_LIST (34.1), MAP_FLOOR_MATCHING (34.4), CLIMBER_ERROR_CODE (34.5), MULTI_FLOOR_CLEANING (34.6) | T/types:1906-1917 |
| 40 | stair-climber accessory | T/types:1918-1922 |
| 99 | Factory and diagnostics. 99.8 SELF_TEST_STATUS: writing `{"bittest":[17,0]}` starts dock self-repair (T/device:5586-5587). 99.31 `{fw_ver,mcu_ver,speech_ver}`; 99.98 Wi-Fi info (N/spec/device-props.ts:31-51). | T/types:1923-1946 |
| 10001 | Camera stream. 10001.1 STREAM_STATUS JSON `{operType, operation, session, result, status}` (T/device:1764-1782). 10001.9 light brightness, `"101"` = auto. 10001.99 STREAM_PROPERTY. 10001.112 live AI detection feed (N). aiid 1 STREAM_VIDEO, 2 STREAM_AUDIO, 3 STREAM_PROPERTY, 4 STREAM_CODE, 7 VIDEO_VENDOR. | T/types:1947-1970 |

r2579a has `STREAM_ALI` and `STREAM_TX` in its capability set, so video should use Aliyun
LinkVisual or Tencent (N/spec/camera-props.ts:1-16).

### 2.16 Properties present on r2579a (summary)

From the cache sweep (R), the robot has these DIIDs:

- siid 1: 5
- siid 2: 1, 2, 3, 6, 8
- siid 3: 1, 2, 3
- siid 4: 1, 2, 3, 4, 6, 7, 11, 12, 17, 18, 20, 22-29, 31, 33-41, 45-48, 50-53, 55-58, 60
- siid 5: 1, 2, 3
- siid 6: 3, 7, 8, 9
- siid 7: 1, 2, 3, 5, 7
- siid 8: 1
- siids 9, 10, 11, 16, 30: 1, 2
- siid 12: 1, 2, 3, 4
- siid 14: 3, 4
- siid 15: 1, 3, 5
- siid 27: 1, 2, 3, 4, 7, 10, 11, 16
- siid 28: 1, 5

Not present in the swept range: siid 13, 17-26 and 29. §2.17 shows that the cache misses some
properties that do exist, so this list is incomplete.

### 2.17 Follow-up probes on r2579a (verified, 2026-11-15)

Two read-only probes ran after the first sweep.

**Cache sweep, siid 31-60, 99 and 10001 (piid 1-60).** Values found:

| DIID | Value (identifiers redacted) | Reading |
|---|---|---|
| 99.6 | `1778769994` | a Unix time (May 2026), probably the last log upload |
| 99.7 | `ali_dreame/2026/05/14/<uid>/<did>_144648505.1664.tbz2` | object name of an uploaded log archive |
| 99.17 | `2025-12-06 16:09:27,4.3.9_1664_release,,` | firmware build date and version |
| 99.22 | `44` | unknown |
| 99.23 | `24` | unknown |
| 99.31 | `{"fw_ver":"2025-12-06 16:09:27,4.3.9_1664_release","mcu_ver":""}` | versions |
| 99.34 | `1008` | unknown |
| 99.38 | `1` | unknown |
| 10001.1 | `{"operType":"end","operation":"monitor","result":0,"status":0}` | last camera stream state |
| 10001.2 | `{"operType":"intercom","operation":"end","session":"<hex>","result":0,"status":1,"df":31}` | last intercom (two-way audio) session |
| 10001.9 | `101` | camera light brightness, 101 = auto |
| 10001.10 | `0` | unknown |
| 10001.11 | `{"initStatus":1,"vendor":"tx"}` | video vendor: `tx` = Tencent |

**Relayed `get_properties` probe** of 85 DIIDs, sent through `sendCommand` to the robot. Each
result item has a `code`:

| Code | DIIDs |
|---|---|
| 0 (value returned) | 2.3 = 2, 2.8 = 0, 4.35 = 101, 4.38 = 0, 6.7, 6.8, 6.9, 6.11 = 0, 6.12 = 0, 6.14 = 0, 6.15 = `""`, 14.1 = 0, 14.2 = `""`, 14.3 = 200, 14.4 = 0, 20.3 = 0, 28.1-28.6, 28.8, 28.9, 28.12-28.19, 28.21-28.23, 28.25-28.30, 29.3 = 0, 99.2 = `<n>-<n>`, 99.3, 99.4 (Wi-Fi `{bssid, ssid, ip}`), 99.5 (`{SN, model, fw_version, nation, language, wifi_mac}`), 99.7-99.10 |
| -1 | 6.1-6.6, 6.10, 6.13, 6.16, 14.5, 20.4-20.10, 28.7, 28.10, 28.11, 28.20, 28.24, 29.4-29.10, 99.1, 99.6 |
| -2 | 20.1, 20.2, 29.1, 29.2 |

What this shows:

- **The cloud cache is not a full list.** Many properties answer a relayed read but are missing
  from the cache: 6.11, 6.12, 6.14, 6.15, 14.1, 14.2, 20.3, 29.3 and most of siid 28. Capability
  discovery must use relayed reads, not the cache.
- **Code -1 does not mean "absent".** 6.1 and 6.3 exist (6.3 is in the cache) but return -1 on a
  relayed read. They are push-only properties. 6.10 (`MAP_RECOVERY`) is write-only. Treat -1 as
  "not readable", and use the cache, the capability table or the MIoT access flags to tell absent
  from not readable.
- **Code -2** appears only on the first two piids of siid 20 and 29. Its meaning is unknown.
- **Map backup and restore exist**: 6.11 `MAP_RECOVERY_STATUS` = 0 (idle) and 6.14
  `MAP_BACKUP_STATUS` = 0 (idle). 6.15 (`WIFI_MAP`) exists and is empty.
- **The detergent service (siid 20) and siid 29 exist** in part (20.3, 29.3), as capability set 152
  suggests. **28.14-28.16, 28.18 and 28.22** also exist, which settles that open question.
- 99.5 reports `nation: EU` and `language: EN` although the account's country and voice pack are
  different ones. The robot is probably an EU-market unit.
- 2.3 = 2, 2.8 = 0, 4.35 = 101 and 4.38 = 0 now have direct values; their meanings are still open.
  Note that 4.38 read `-1` in the cache and `0` live.

---

## 3. Enums

Treat every enum as an open set: `Known(T)` or `Unknown(i64)`. Labels vary by firmware (3.4).
The Dreamehome app ships its own value-to-label maps per DIID in
`common_device_protocol.json` (APK). That file is the best label source per model.

### 3.1 STATE (2.1)

`DreameVacuumState`, T/types:378-450:

| Value | Name | Value | Name |
|---|---|---|---|
| 0 | treated as CHARGING_COMPLETED (T/device:8399-8400) | 23 | REMOTE_CONTROL |
| 1 | SWEEPING | 24 | SMART_CHARGING |
| 2 | IDLE | 25 | SECOND_CLEANING |
| 3 | PAUSED | 26 | HUMAN_FOLLOWING |
| 4 | ERROR | 27 | SPOT_CLEANING |
| 5 | RETURNING | 28 | RETURNING_AUTO_EMPTY |
| 6 | CHARGING | 29 | WAITING_FOR_TASK |
| 7 | MOPPING | 30 | STATION_CLEANING |
| 8 | DRYING (**r2579a: observed**) | 31 | RETURNING_TO_DRAIN |
| 9 | WASHING | 32 | DRAINING |
| 10 | RETURNING_TO_WASH | 33 | AUTO_WATER_DRAINING |
| 11 | BUILDING (mapping) | 34 | EMPTYING |
| 12 | SWEEPING_AND_MOPPING | 35 / 36 | DUST_BAG_DRYING / _PAUSED |
| 13 | CHARGING_COMPLETED | 37 / 38 | HEADING_TO_EXTRA_CLEANING / EXTRA_CLEANING |
| 14 | UPGRADING | 95 / 96 | FINDING_PET_PAUSED / FINDING_PET |
| 15 | CLEAN_SUMMON | 97 | SHORTCUT |
| 16 | STATION_RESET | 98 / 99 | MONITORING / _PAUSED |
| 17 | RETURNING_INSTALL_MOP | 101 / 102 | INITIAL_DEEP_CLEANING / _PAUSED |
| 18 | RETURNING_REMOVE_MOP | 103 / 104 | SANITIZING / _WITH_DRY |
| 19 | WATER_CHECK | 105 / 106 | CHANGING_MOP / _PAUSED |
| 20 | CLEAN_ADD_WATER | 107 / 108 | FLOOR_MAINTAINING / _PAUSED |
| 21 | WASHING_PAUSED | 109, 113-122, 140-147 | arm, pet guarding, mop install, recharge, dock, stair climber |
| 22 | AUTO_EMPTYING | | |

**Generation split.** Models without capability `NEW_STATE` use `DreameVacuumStateOld` for values
above 18 (T/types:453-482; remap at T/device:8392-8409). r2579a has `NEW_STATE`, so the table above
applies.

N (r2532a) observed only values from this table. 1 vs 12 means vacuum-only vs vacuum and mop. A
typical end-of-job sequence is `12, 5, 6, 22, 9, 6, 8` (N/spec/enums.ts:10-58). Values 1-18 also
match S/r2320.

Tasshack rewrites IDLE into other states from secondary properties (T/device:8411-8436). That is app
behaviour; keep it out of the core (section 6).

### 3.2 STATUS (4.1)

`DreameVacuumStatus`, T/types:614-649. The same values are the first `in` of 4.a1.

| Value | Name | Value | Name |
|---|---|---|---|
| 0 | IDLE | 16 | FACTORY_FUNCTION_TEST |
| 1 | PAUSED | 17 | STANDBY |
| 2 | CLEANING | 18 | SEGMENT_CLEANING |
| 3 | BACK_HOME | 19 | ZONE_CLEANING |
| 4 | PARTIAL_CLEANING | 20 | SPOT_CLEANING |
| 5 | FOLLOW_WALL | 21 | FAST_MAPPING |
| 6 | CHARGING | 22 | CRUISING_PATH |
| 7 | OTA | 23 | CRUISING_POINT |
| 8 | FCT | 24 | SUMMON_CLEAN |
| 9 | WIFI_SET | 25 | SHORTCUT |
| 10 | POWER_OFF | 26 | PERSON_FOLLOW |
| 11 | FACTORY | 27 | PET_GUARDING |
| 12 | ERROR | 28-30 | AUTO / SMART / ZONED_ARRANGEMENT |
| 13 | REMOTE_CONTROL | 1501 | WATER_CHECK |
| 14 | SLEEPING (**r2579a: observed**) | | |
| 15 | SELF_REPAIR | | |

### 3.3 TASK_STATUS (4.7) and TASK_TYPE (4.58)

- **TASK_STATUS** (T/types:566-611): 0 completed (**r2579a: observed**), 1 auto cleaning, 2 zone,
  3 segment, 4 spot, 5 fast mapping, 6-18 paused variants, 20-23 cruising, 24 summon paused,
  25/26 returning to install or remove mop, 27 station cleaning, 30 pet finding, 31-33
  washing-paused variants, 34-44 item pick-up.
- **TASK_TYPE** (T/types:930-972): 0 idle (**r2579a: observed**), 1/2 standard, 3/4 custom,
  5/6 shortcut, 7/8 scheduled, 9/10 smart, 11/12 partial, 13/14 summon, 15/16 water stain,
  17-26 special modes, 30-41 newer modes. N saw 9 and 17 alternate while the mop rotated on r2532a.
- Cleaning history enums (T/types:4633-4661): `StartupMethod`, `CleanupMethod`, `TaskEndType`,
  `CleaningStatus`.
- `TaskInterruptReason` (T/types:2818-2838) and `ObstacleReason` (T/types:2804-2815) appear inside
  4.10 fields `abnormal_end` and `area_clean_detail`.

### 3.4 Errors (2.2) and faults (4.18)

`DreameVacuumErrorCode`, T/types:229-375, has about 150 codes from 0 to 230, plus 1000.

| Range | Area |
|---|---|
| 1-51 | robot hardware (drop, cliff, bumper, wheels, brushes, fan, battery, sensors, LDS) |
| 54-67 | navigation (edge, carpet, laser, no-go zone, route, blocked) |
| 68-99 | mop and stuck conditions |
| 101-129 | bin, dust bag, water tanks, mop pad, station, washboard, drainage, dock |
| 200-230 | newest hardware |

Warnings (not errors) are listed in `WARNING_ERROR_CODE` (T/types:2691-2714).

**Code meanings differ by firmware.** N (r2532a) reads 68 as "task complete" (benign), 18 as robot
lifted, 16 as main wheel tangled, 74 as manual mop install needed and 120 as mop pads missing
(N/spec/enums.ts:138-330). Tasshack names the same codes REMOVE_MOP, FORWARD_SUFFOCATE,
RIGHT_WHEEL_MOTOR, MOP_INSTALL_FAILED and NO_MOP_IN_STATION.

r2579a evidence: 2.2 = 68 and 4.18 = `"68"` while the robot was docked and drying mops after a
completed task (2.1 = 8, 4.7 = 0, 4.25 = 2). This fits N's "task complete" reading. Tasshack also
treats 68 as a warning (T/types:2692) and drops it on self-wash models. So code 68 is not an error
on r2579a. Expose the raw integer; treat labels as hints.

### 3.5 Suction, water and mop enums

| Enum | Property | Values | Source | r2579a |
|---|---|---|---|---|
| SuctionLevel | 4.4 | 0 quiet, 1 standard, 2 strong, 3 turbo | T/types:485-492 | `2` |
| WaterVolume | 4.5 | 1 low, 2 medium, 3 high | T/types:515-521 | not used (wetness model) |
| Wetness | 28.1 | 1..32, default 16 | T/device:1844-1862 | `15` |
| MopPadHumidity | 4.23 byte 2 | 1 slightly dry, 2 moist, 3 wet | T/types:524-530 | byte 2 = 0 |
| CleaningMode (logical) | 4.23 byte 0, 2.6 | 0 sweeping, 1 mopping, 2 sweeping and mopping, 3 mopping after sweeping | T/types:495-502 | `3` |
| MopWashLevel | 4.46 | 0 water-saving, 1 standard, 2 deep | T/types:699-705 | `1` |
| MoppingType (`CleanType`) | 4.50 | 0 daily, 1 accurate, 2 deep | T/types:708-714 | `0` |
| H MoppingType | 2.8 | 0 standard, 1 deep, 2 quick | H device.js:236-237 | `0` |
| CleaningRoute (`CleanRoute`) | 4.50 | 1 standard, 2 intensive, 3 deep, 4 quick | T/types:717-725 | not seen (list truncated) |
| CustomMoppingRoute | | -1 off, 0 standard, 1 intensive, 2 deep | T/types:728-735 | |
| WiderCornerCoverage (`MeticulousTwist`) | 4.50 | 0 off, 1 high frequency, 7 low frequency | T/types:738-744 | |
| MopPadSwing (`MopScalable`) | 4.50 | 0 off, 1 auto, 2 daily, 7 weekly | T/types:747-754 | |
| MopExtendFrequency (`ExtrFreq`) | 4.50 | 7 standard, 1 intelligent, 2 high | T/types:757-763 | `2` (high) |
| SelfCleanFrequency (`BackWashType`) | 4.50 | 1 by area, 2 by time, 3 by room, 4 intelligent | T/types:766-773 | |
| CleanGenius (`SmartHost`) | 4.50 | 0 off, 1 routine, 2 deep | T/types:798-804 | |
| CleanGeniusMode | 28.5 | 2 vacuum and mop, 3 mop after vacuum | T/types:1794-1913 | `2` |
| SecondCleaning (`SmartAutoMop`, `SmartAutoWash`) | 4.50 | 0 off, 1 in deep mode, 2 in all modes; -1 off | T/types:815-821 | `SmartAutoMop` = `-1` (off) |
| CarpetCleaning | 4.36 | see 2.4 | T/types:542-553 | `2` |
| CarpetSensitivity | 4.28 | 1 low, 2 medium, 3 high | T | `2` |
| FloorMaterial | map | 0 none, 1 wood, 2 tile, 5 medium pile, 6 low pile, 7 carpet | T/types:824-833 | room materials 1 and 2 seen in the saved map (R) |
| SegmentType | map | 0-15, Room to Secondary Bedroom | T/types:12-29 | types 0, 1, 4, 5, 6, 12 seen (R) |

### 3.6 Station and dock enums

| Enum | Property | Values | Source | r2579a |
|---|---|---|---|---|
| SelfWashBaseStatus | 4.25 | 0 idle, 1 washing, 2 drying, 3 returning, 4 paused, 5 adding water, 6 returning to drain, 7 returning for dry mop | T/types:672-683 | `2` |
| AutoEmptyMode | 15.1 | see 2.12 | T/types:776-795 | `2` |
| DustCollection | 15.3 | 0 not available, 1 available, 2 over-use, 3 never | T/types:652-659 | `1` |
| AutoEmptyStatus | 15.5 | 0 idle, 1 active, 2 not performed, 3 returning | T/types:662-669 | `0` |
| DrainageStatus | 4.60 | 0 idle, 1 draining, 2 success, 3 failed | T/types:920-927 | `0` |
| LowWaterWarning | 4.41 | see 2.4 | T/types:907-917 | `0` |
| Station tank and bag statuses | 27.x | see 2.13 | T/types:996-1075 | all `0` |
| Station operation | 4.a4 | see 4.7 | T/device:5413-5629 | |

---

## 4. Composite value codecs

### 4.1 CLEANING_MODE (4.23): packed integer

Tasshack's `split_group_value` and `combine_group_value` (T/device:2486-2500):

```
mode = value & 0x03          if mop_pad_lifting, else value & 0x01
mid  = (value >> 8) & !0x300  // clears bits 8-9 of (value >> 8); not a full 8-bit mask
hi   = value >> 16
combine(mode, mid, hi) = ((hi << 8) ^ mid) << 8 ^ mode
```

Bit layout (when `hi < 4`):

| Bits | Field | Meaning |
|---|---|---|
| 0-1 | mode | raw cleaning mode (see table below) |
| 8-15 | mid | self-clean frequency value: m² or minutes, depending on `BackWashType` |
| 16-23 | hi | water level or mop pad humidity 1..3 (models without wetness level) |

Raw mode on `mop_pad_lifting` models, per Tasshack (read at T/device:1082-1100, write at
T/device:2157-2182):

| Raw | Logical |
|---|---|
| 0 | SWEEPING_AND_MOPPING |
| 1 | MOPPING |
| 2 | SWEEPING |
| 3 | MOPPING_AFTER_SWEEPING |

N reads raw 0 as sweeping and 2 as sweeping and mopping (N/spec/vacuum-props.ts:243-273), which is
the opposite.

**Worked example (r2579a, verified value).** 4.23 = `5123` = `0x00_14_03`.

| Field | Computation | Result |
|---|---|---|
| mode | `5123 & 0x03` | `3`: MOPPING_AFTER_SWEEPING in both readings |
| mid | `(5123 >> 8) & !0x300` = `20 & !0x300` | `20`: self-clean every 20 m², Tasshack's default (T/types:3240) |
| hi | `5123 >> 16` | `0`: unused; water is set by 28.1 on wetness models (T/device:2211-2266) |

Cross-checks on the same robot: 2.6 = `3` equals the mode field, which supports N's claim that 2.6
mirrors the low bits. N's baseline 5120 (`0x1400`) on r2532a and r2449a has the same `mid` = 20.

What this settles and what it does not:

- **Settled:** the byte layout, and that the self-clean value sits in byte 1 on r2579a.
- **Not settled:** the 0/2 polarity, because mode 3 is the same in both readings.

Example write, switching to "sweeping and mopping" while keeping mid = 20 and hi = 0:
Tasshack's reading gives `combine(0, 20, 0)` = `5120`; N's reading gives `5122`. Do not write 4.23
until the polarity is verified. Never write it with `mid` = 0: N reports that this breaks the next
clean (N/spec/vacuum-props.ts:243-273). Writing 2.6 may be the safer path, also unverified.

### 4.2 AUTO_SWITCH_SETTINGS (4.50): key/value JSON

- **Read:** a JSON string, either `[{"k":"<Key>","v":<int>}, ...]` or one `{"k":..,"v":..}`
  (T/device:1636-1655).
- **Write:** one `{"k":"<Key>","v":<int>}` per write (T/device:6112-6166). N says the whole array
  must be written back (N/spec/feature-config.ts:13-16). **Settled:** the official app's r2579a
  plugin writes one object (`toggleMeticulousTwist`: `JSON.stringify({k, v})`), and a live write
  of `MeticulousTwist` 7 and back to -7 worked and read back ([l40-ultra-ae.md](l40-ultra-ae.md)).
  The app toggles a key by negating it (`1` and `-1`, `7` and `-7`).
- `-1` usually means off or blocked.

r2579a value (R, list truncated in the capture after 14 entries):

| Key | r2579a | Meaning (T/types:1471-1507; N/spec/feature-config.ts:28-140) | Decoded |
|---|---|---|---|
| `StainIdentify` | 1 | stain avoidance | on |
| `MopFullyScalable` | 1 | seen by N; meaning unknown | |
| `SuperWash` | 0 | ultra clean mode | off |
| `RobotCarpetPressEnable` | 1 | carpet boost mirror, -1/0/1 (N) | on |
| `SmartAutoMop` | -1 | auto re-cleaning | off |
| `CarpetFineClean` | 1 | intensive carpet cleaning | on |
| `ExtrFreq` | 2 | MopExtend frequency | high; enables `mop_extend` (5.4) |
| `AutoDry` | 1 | auto mop drying | on |
| `LessColl` | 1 | collision avoidance | on |
| `FillinLight` | 1 | camera fill light | on |
| `CleanType` | 0 | mopping type | daily |
| `MonitorHumanFollow` | 0 | camera human follow | off |
| `MopEffectSwitch` | 1 | custom mopping mode | on |
| `FluctuationTestResult` | 0 | drainage calibration | |

Other known keys: `SmartHost`, `MeticulousTwist`, `MaterialDirectionClean`, `PetPartClean`,
`SmartAutoWash`, `MopScalable`, `SmartCharge`, `SuctionMax`, `SmartDrying`,
`FluctuationConfirmResult`, `HotWash`, `UVLight`, `CleanRoute`, `MopEffectState`, `BackWashType`,
`LacuneMopScalable`, `MopScalable2`, `MonitorPromptLevel`, `MopExtrSwitch`, `SbrushExtrSwitch`,
`HeavyStainSmart`, `CarpetOnlyClean`, `DetergentNote`, `MopScalableVersion`. Model the store as a
`BTreeMap<String, i64>` and keep unknown keys.

### 4.3 AI_OBSTACLE_DETECTION (4.22): bitmask or JSON

Integer form: `DreameVacuumAIProperty` (T/types:1528-1542).

| Bit | Value | Meaning |
|---|---|---|
| 0 | 1 | furniture detection |
| 1 | 2 | obstacle detection (master) |
| 2 | 4 | obstacle picture |
| 3 | 8 | fluid detection |
| 4 | 16 | pet detection |
| 5 | 32 | obstacle image upload |
| 6 | 64 | AI image |
| 7 | 128 | pet avoidance |
| 8 | 256 | fuzzy obstacle detection |
| 9 | 512 | pet picture |
| 10 | 1024 | pet-focused detection |
| 11 | 2048 | large particles boost |

Write the whole integer with one bit changed (T/device:6094-6097). The official app's plugin does
the same (`encodeAiSwitchByType`), with the same bit order (`AI_SWITCH_TYPE`). Verified on r2579a:
writing 287 turned pet detection on, and 271 turned it off again.

**Worked example (r2579a, verified value).** 4.22 = `271` = `0b1_0000_1111`
= 256 + 8 + 4 + 2 + 1. On: fuzzy obstacle, fluid, obstacle picture, obstacle detection, furniture.
Off: pet detection, image upload, AI image, pet avoidance, pet picture, pet-focused, large
particles. This settles that r2579a uses the **integer** form. It also matches N's note that bits 1
and 256 are always set.

JSON form (older models): `{"obstacle_detect_switch":bool, "obstacle_app_display_switch":bool,
"whether_have_pet":bool, "human_detect_switch":bool, "furniture_detect_switch":bool,
"fluid_detect_switch":bool}`; write a partial object (T/types:1510-1518). The codec must accept
both forms.

### 4.4 SCHEDULE (8.2): dash-separated string

Slots are separated by `;`. Each slot has 9 `-`-separated fields (T/device:1729-1762):

| Index | Tasshack | N (r2532a; N/spec/schedule-props.ts:1-50) |
|---|---|---|
| 0 | id | id |
| 1 | enabled: 1 or 2 on, 3 invalid | same |
| 2 | `HH:MM` | same |
| 3 | weekday mask, 7 chars | Mon-Sun bitmap |
| 4 | `0` means once | 0 one-shot, 1 repeating |
| 5 | map_id | room scope 0 or 1 |
| 6 | suction_level | wetness (16 standard) |
| 7 | water_volume | packed config int |
| 8 | options list or `0` | rooms list or packed per-room ints |

N's packed config int (N/spec/enums.ts:470-512): bits 0-3 route 1-4; bits 4-6 mode (1 vac, 2 vac
and mop, 3 mop, 4 mop after vac); bits 7-23 constant `0xC249`; bits 24-25 suction 0-3; bits 28-31
repeat count. Fields 5-8 are disputed between the two sources.

**Verified on r2579a, 2026-11-17**, for a "Custom" schedule of chosen rooms made in the official
app, by changing one setting at a time and reading 8.2 after each change:
`1-1-22:00-1111111-1-1-0-0-1126240522,1126240524`.

- Field 4 `1` = repeat weekly. Field 5 `1` = chosen rooms with their own settings. Fields 6 and 7
  are `0`.
- Field 8 is one packed number per room, in cleaning order:

| Bits | Field |
|---|---|
| 0-7 | room id |
| 8-11 | passes 1-3 |
| 12-15 | `1` in every sample |
| 16-19 | suction 0-3 (quiet to max) |
| 20-23 | mode: 0 vacuum, 1 mop, 2 vacuum and mop (as `cleanType`) |
| 24-25 | `3` in every sample |
| 26-31 | mop wetness 1-32 (16 default; the app's slider value) |

Neither source's packing matches. The weekday field starts on **Sunday** (its 5th character showed as Thursday in the official app; N says Monday first). The value written
for "off" (assumed `0`) is not yet confirmed. `codec::schedule::RoomTask` reads and writes the rooms.

### 4.5 FAULTS (4.18): comma list

Processing (T/device:1482-1524):

1. Split on `,` and parse integers.
2. Drop codes listed in the `IGNORED_WARNINGS` capability parameter.
3. With capability `LEGACY_WARNINGS`, remap 84 to 85 and 85 to 86. **r2579a does not have it.**
4. Drop 0, 84 and 122.
5. Drop 20 (battery low) while charging.
6. Drop 68 on self-wash models.

r2579a example: `"68"` gives `[68]`. Step 6 drops it, so no active fault.

### 4.6 START_CUSTOM (4.a1) and CLEANING_PROPERTIES (4.10)

In: `[{"piid":1,"value":<STATUS>}, {"piid":10,"value":"<json>"}]` (T/device:4596-4619).

| STATUS | Purpose | 4.10 value | Source |
|---|---|---|---|
| 18 | room clean | `{"selects":[[seg, repeats, suction, water, order], ...]}`. Order must be 1 with 4.26 present on gen5 (T/device:4955-4957). | T/device:4886-4986 |
| 19 | zones | `{"areas":[[x0,y0,x1,y1,repeats,suction,water], ...]}` in map mm | T/device:4763-4884 |
| 20 | spots | `{"points":[[x,y,repeats,suction,water], ...]}` | T/device:4988-5095 |
| 23 | go to point | `{"tpoint":[[x,y,0,0]]}` | T/device:5097-5197 |
| 22 | patrol | `{"tpoint":[[x,y,..], ...]}`, at most 20 points | T/device:5199-5271 |
| 21 | fast mapping | no 4.10 | T/device:5332-5366 |
| 25 | shortcut | `"<shortcut_id>"` | T/device:5273-5330 |
| 2 | clean again | `{"MopAgain":n,"timestamp":..,"CleanArea":[...],"BigArea":[...]}` | T/device:5630-5672 |

For self-wash models, `water` is the mop pad humidity 1..3.

Task summary keys in 4.10 (T/types:4528-4558): `cmc`, `clt`, `abnormal_end`, `customeClean` (sic),
`ismultiple`, `ctyo`, `mooClean`, `multime`, `pet`, `cleanagain`, `area_clean_detail`. The r2579a
live map JSON also carries `cmc`, `customeClean` = 1 and `mooClean` (R).

### 4.7 START_WASHING (4.a4): station operations

The 4.10 value is `"<op>,<flag>"` (T/device:5413-5629):

| Value | Operation |
|---|---|
| `"2,1"` | return and wash |
| `"1,1"` / `"1,0"` | resume or pause washing |
| `"3,1"` / `"3,0"` | start or stop mop drying |
| `"14,1"` / `"14,0"` | start or stop dust-bag drying |
| `"7,1"` | drain |
| `"9,1"` | drain the clean water tank |
| `"5,1"` | washboard self-cleaning |

Self-repair is a write of `{"bittest":[17,0]}` to 99.8, not a station operation.

### 4.8 SHORTCUTS (4.48) and the SHORTCUTS action (4.a8)

- **4.48:** `[{"id":N,"name":"<base64 utf-8>","state":"0|1|.."}]`; state `"0"` or `"1"` means
  running (T/device:5674-5701). r2579a: `[]` (no shortcuts).
- **4.a8** in: `{"cmd":..,"params":{..}}` (T/device:3785-3801). `GET_COMMANDS` returns
  `[{id, mapId}]`; `GET_COMMAND_BY_ID` with `{"id":N}` returns tasks; rename is
  `{"id","name":base64,"type":3}` (T/device:6345-6351); delete is `{"id","type":4}`
  (T/device:6385-6388).
- **Task item:** `[seg_id | -1 all | -2 zone, suction, water, repeats, cleaning_mode, ...]`; zones
  carry `x0..y1` at 5-8 and order at 9 (T/device:5739-5830).

### 4.9 MAP_EXTEND_DATA (6.4) edits via 6.a2

A JSON object whose top-level key selects the operation (T/device:6402-7920): `vw` (walls, no-go,
no-mop), `cpt` (carpets), `carpetcleanset`, `vws` (thresholds, ramps), `curtain`, `nrism` (rename
map), `smra` (rotate), `sm` (select map), `cm` (delete map), `cw` (temporary map), `msr` (merge
rooms), `dsrid` (split room), `nsr` (rename or retype rooms), `nsm` (floor material), `delsr`
(hidden rooms), `cleanareaorder` (sequence), `customeClean` (per-room settings), `obstacleignore`,
`wrp` (router position).

r2579a has `CLEANING_SEQUENCE_V2`, so the sequence uses `[{"<id>":order}]`. The saved map confirms
this form: `cleanareaorder` = `[{"1": 2}, {"2": 1}, ...]` (R).

### 4.10 Per-room cleanset (`customeClean` and map JSON `cleanset`)

Shape: `{"<segment_id>": [suction, water_or_wetness, repeats, order, cleaning_mode?,
mopping_settings?, ...]}` (T/map:6372-6530).

`mopping_settings` packs three nibbles `[n0, n1, n2]`, `n0` lowest (T/map:6602-6615):

| Case | Reading |
|---|---|
| `n2 == 0` | custom route on: route = `n0 - 1`, water = `n1` |
| `n2` in 1..3 | water or route = `n2` |

`CleansetType` (T/types:2875-2882) selects the reading. r2579a has `WETNESS_LEVEL` and
`CLEANING_ROUTE`, so the wetness reading applies.

**Worked example (r2579a, verified value).** The live map `cleanset` for room 1 is
`[1, 3, 1, 2, 2, 546]` (R).

| Index | Value | Meaning |
|---|---|---|
| 0 | 1 | suction standard |
| 1 | 3 | water field: Tasshack maps 2..4 to level `item[1] - 1`, so 3 is level 2; see note B |
| 2 | 1 | repeats |
| 3 | 2 | cleaning order |
| 4 | 2 | cleaning mode `2` (Tasshack's default value; raw or logical encoding not confirmed) |
| 5 | 546 = `0x222` | nibbles n0 = 2, n1 = 2, n2 = 2. `n2` in 1..3, so the level is 2. |

`546` is also Tasshack's default for `segment_mopping_settings` (T/map:6372-6530). All 11 rooms share
`[1, 3, 1, k, 2, 546]` with different orders `k`, which matches `cleanareaorder`.

Note B: Tasshack sets index 1 to 16 by default on wetness models, but r2579a stores 3. So on this
model index 1 looks like a 1..3 level, not wetness 1..32. Verify before writing.

### 4.11 Pointers to cloud files (6.3, 6.8, 6.9)

| DIID | Form on r2579a (R) |
|---|---|
| 6.3 | plain object name `ali_dreame/<uid>/<did>/<n>`; no `,<key>` suffix |
| 6.8 | JSON `{"obj_name":..,"md5":..}`; T also accepts `object_name` (T/device:956-972) |
| 6.9 | plain object name |

The 6.8 file holds `{"mapstr":[{"map":<base64>,"angle":"0","name":""}],"curr_id":1,"server":null}`
on r2579a (R; T/map:1630-1700). Fetching uses the cloud file API, which is out of scope here.

### 4.12 Other flat JSON values

- 3.3 off-peak charging, 5.4 DND tasks, 7.3 voice status `{"id","state","progress"}` (R), 7.4
  voice change, 10001.1 stream status. All are flat JSON; decode with serde into typed structs and
  keep unknown fields.

---

## 5. Capability detection

### 5.1 Tasshack's three layers

1. **Property presence** (T/types:3292-3316). At start-up Tasshack reads nearly every mapped
   property (T/device:400-440) and treats `code == 0` as present.

   | Flag | Rule |
   |---|---|
   | `lidar_navigation` | 13.1 absent |
   | `multi_floor_map` | 6.7 present and LiDAR |
   | `ai_detection` | 4.22 present |
   | `self_wash_base` | 4.25 present |
   | `auto_empty_base` | 15.3 present |
   | `customized_cleaning` | 4.26 present |
   | `tight_mopping` | 4.29 present |
   | `auto_switch_settings` | 4.50 present |
   | `carpet_recognition` | 4.33 or 4.36 present |
   | `wifi_map` | 6.15 present |
   | `backup_map` | 6.14 present |
   | `dnd_task` | 5.4 present |
   | `dnd` | 5.4 or 5.1 present |
   | `shortcuts` | 4.48 present |
   | `off_peak_charging` | 3.3 present |
   | `voice_assistant` | 7.5 present |

2. **Model capability set** (T/types:3317-3333). Each set entry is `[capability_id, value]`. Value
   `1` means on. A larger integer is a minimum fw build. A numeric string is an integer parameter;
   another string is a string parameter.

3. **Derived rules** (T/types:3336-3518). Examples:
   - `auto_empty_base` needs `AUTO_EMPTYING` and 15.3 != 3.
   - `detergent` needs a self-wash base and (`DETERGENT` or 20.1 present).
   - `drainage` needs `DRAINAGE` and 4.60 present.
   - `cleaning_progress` needs 4.63 present.
   - Without a self-wash base, CleanGenius, washing mode, detergent and drainage are cleared.
   - `mop_extend` needs `MOP_PAD_SWING(_PLUS)` and AutoSwitch `ExtrFreq >= 0`.
   - `wetness` needs `WETNESS_LEVEL`, or `MOPPING_SETTINGS` and a non-zero 28.1.
   - `mop_pad_lifting` is also on when there is a self-wash base and an auto-empty base.
   - `robot_type` is SWEEPING_AND_MOPPING when self-wash and mop lifting are both on.

4. **Runtime availability** (T/types:2022-2305 for properties, T/types:2308-2401 for actions):
   state-dependent predicates for the app UI, not protocol rules.

`DeviceCapability` has 175 ids (T/types:2885-3057). Defaults for parameter capabilities are at
T/types:3060-3274.

### 5.2 The `DEVICE_INFO` data source

`DEVICE_INFO` (T/const:736) is base64 of zlib (gzip header) JSON. It is decoded at T/device:668-669.
Shape:

| Index | Content |
|---|---|
| `[0]` | model records `[brand, ui_variant, capability_set_index, iv_index?]`. Brand: 0 Dreame, 1 Xiaomi, 2 MOVA, 3 Trouver (`config_flow.py:781-783`). |
| `[1]` | 433 capability sets, each a list of `[capability_id, value]` |
| `[2]` | 13 AES-CBC IV strings for map decryption |
| `[3]` | dict of 766 model suffixes (text after the last `.`) to an index into `[0]` |

An unknown suffix makes Tasshack refuse the device (T/types:3276-3290). The library should not
refuse; it should fall back to presence detection.

### 5.3 What r2579a gets

Decoded from the local `DEVICE_INFO` (this research, 2026-11-15):

| Item | Value |
|---|---|
| Suffix `r2579a` | record index 175 |
| Record | `[0, 0, 152, 5]`: Dreame brand, standard UI, **capability set 152**, IV index 5 (`NRwnBj5FsNPgBNbT`) |
| Other suffixes on set 152 | `r25799`, `r2579c`, `r2579h`, `r2579k`, `r500za` (`r500zh` uses set 147) |

Capability set 152 has 69 entries. Flags with value 1 (on for any firmware):

```
DRAINAGE, MOPPING_AFTER_SWEEPING, MAX_SUCTION_POWER, OBSTACLE_IMAGE_CROP, MOP_PAD_SWING,
HOT_WASHING, AUTO_EMPTY_MODE, FLOOR_DIRECTION_CLEANING, SEGMENT_VISIBILITY, MOP_PAD_SWING_PLUS,
AUTO_REWASHING, MOP_PAD_LIFTING_PLUS, PET_FURNITURE, CLEANING_ROUTE, MOPPING_SETTINGS, TASK_TYPE,
ULTRA_CLEAN_MODE, EXTENDED_FURNITURES, SELF_CLEAN_FREQUENCY, CLEANGENIUS, FLUID_DETECTION,
INTENSIVE_CARPET_CLEANING, CLEAN_CARPETS_FIRST, WETNESS_LEVEL, AUTO_RENAME_SEGMENT, FLOOR_MATERIAL,
GEN5, FURNITURES_V2, SAVED_FURNITURES, OBSTACLES, WATER_CHECK, AUTO_CARPET_CLEANING,
SEGMENT_MOPPING_SETTINGS, MAX_SUCTION_POWER_EXTENDED, AUTO_RECLEANING, NEW_STATE, CAMERA_STREAMING,
CLEANGENIUS_MODE, WASHING_MODE, SMART_MOP_WASHING, DND_FUNCTIONS, RAMPS, VIRTUAL_TRACKS, WHEEL,
AREA_ROTATION, MOP_PAD_LIFTING, DYNAMIC_OBSTACLE_CLEANING, CARPET_CLEANSET_V2, CARPET_SHAPE,
CARPET_ID, AUTO_EMPTYING, WATER_TANK_DRAINING, AUTO_ADD_DETERGENT, FILL_LIGHT, DEODORIZER_MODULE,
CLEANING_SEQUENCE_V2, WALLS_V3, QUICK_MAP_RECOVERY_V2, CURTAINS, STREAM_TX, STREAM_ALI,
VIDEO_DYNAMIC_VENDOR, VIDEO_DURING_CLEANING, PET_SNAPSHOT, MULTI_SEGMENT_FLOOR_MATERIAL,
FLOOR_MATERIAL_EDIT, DOCK_STATUS, SKIP_AREA
```

Firmware-gated: `LOCAL_STORAGE >= 1681`. The robot runs build 1664, so `LOCAL_STORAGE` is **off**.

Compared with the L40 Ultra (`r2492*`, set 170):

| Only in r2579a (set 152) | Only in r2492 (set 170) |
|---|---|
| `DEODORIZER_MODULE`, `DYNAMIC_OBSTACLE_CLEANING`, `LOCAL_STORAGE` (gated), `QUICK_MAP_RECOVERY_V2`, `STREAM_TX`, `VIDEO_DYNAMIC_VENDOR`, `WHEEL` | `CARPET_TYPE`, `LARGE_PARTICLES_BOOST`, `LEGACY_WARNINGS`, `MOP_PAD_UNMOUNTING`, `SIDE_BRUSH_CARPET_ROTATE`, `SIDE_REACH`, `VIDEO_AUDIO`, `WATER_TEMPERATURE` |

Set 170 gates `SMART_MOP_WASHING`, `DND_FUNCTIONS`, `CURTAINS` (1310), `CARPET_SHAPE` (1377) and
`MULTI_SEGMENT_FLOOR_MATERIAL` (1623). Set 152 enables them for any firmware.

Notable absences in set 152: `MAP_V2`, `INTELLIGENT_AUTO_EMPTY`, `LEGACY_WARNINGS`,
`WATER_TEMPERATURE`, `IGNORED_WARNINGS`, `DETERGENT`, `MOP_PAD_UNMOUNTING`.

**Presence layer on r2579a (from the cache, R):**

| Flag | Result | Evidence |
|---|---|---|
| `lidar_navigation` | true | 13.1 absent |
| `multi_floor_map` | true | 6.7 present (value 0 = switched off) |
| `ai_detection` | true | 4.22 = 271 |
| `self_wash_base` | true | 4.25 = 2 |
| `auto_empty_base` | true | 15.3 = 1, and `AUTO_EMPTYING` |
| `customized_cleaning` | true | 4.26 present |
| `tight_mopping` | true | 4.29 present |
| `auto_switch_settings` | true | 4.50 present |
| `carpet_recognition` | true | 4.33 and 4.36 present |
| `wifi_map` | false (weak) | 6.15 absent; the saved map does contain `whm` |
| `backup_map` | false (weak) | 6.14 absent |
| `dnd_task` | false | 5.4 absent |
| `dnd` | true | 5.1 present |
| `shortcuts` | true | 4.48 present |
| `off_peak_charging` | true | 3.3 present |
| `voice_assistant` | true | 7.5 present |

**Derived layer on r2579a:**

| Flag | Result | Reason |
|---|---|---|
| `mop_extend` | true | `MOP_PAD_SWING` and `ExtrFreq` = 2 |
| `wetness` | true | `WETNESS_LEVEL` |
| `drainage` | true | `DRAINAGE` and 4.60 present |
| `ultra_clean_mode` | true | `ULTRA_CLEAN_MODE` and drainage |
| `detergent` | false per Tasshack | no `DETERGENT` and 20.1 absent; `auto_add_detergent` (4.37) is still on |
| `mop_pad_lifting` | true | in the set; also self-wash plus auto-empty |
| `robot_type` | SWEEPING_AND_MOPPING | self-wash and mop lifting |
| `station_cleaning` | true | self-wash base and `GEN5` |
| `cleaning_progress` | unknown | 4.63 not swept |
| `furnitures_v3` | false | no `MAP_V2` and none of the trigger flags |
| Auto-empty enum | old (0-3) | no `INTELLIGENT_AUTO_EMPTY` |
| Fault remap | none | no `LEGACY_WARNINGS` |

Consistency checks against the robot:

- No `MAP_V2` matches the map finding: live frames decode with the frame-map encoding, not the v3
  encoding (R).
- `WHEEL` matches siid 30 present. No `WATER_TEMPERATURE` matches 28.8 absent.
- `DEODORIZER_MODULE` is in the set, but siid 29 is absent.
- IV index 5 is assigned, but r2579a map frames are **not** encrypted (R). The IV only matters when
  6.3 or 6.1 carries a `,<key>` suffix.

### 5.4 Recommendation for the library

1. Run a batch `get_properties` over the whole profile and record `code == 0` per DIID. Do not rely
   on the cloud cache for presence; absence there is weak evidence.
2. Read the model and fw build first. Look the suffix up in an optional, data-driven capability
   table (a port of `DEVICE_INFO`, with fw thresholds). Unknown suffixes fall back to presence only.
3. Apply derived rules as data, not as scattered code.
4. Keep availability predicates out of the core.

---

## 6. Model-generic vs Dreame-specific

| Layer | Generic MIoT | Dreame-specific |
|---|---|---|
| Addressing | `(siid, piid)`, `(siid, aiid)` with `in: [{piid, value}]` and `out`, `(siid, eiid)`; `get_properties`, `set_properties`, `action`; `properties_changed`, `event_occured` | Cloud `sendCommand` envelope with object `params` (N/commands.ts:164-189); cache API returns strings (R) |
| Services | 1 device info; 2 vacuum (piids 1-4 per spec); 3 battery; consumable p1/p2/reset pattern | 4 vacuum-extend; 5 DND; 6 map; 7 audio; 8 schedule; 12 statistics; 13-40; 99; 10001 |
| Values | int, bool, string | JSON in strings, packed ints (4.23, schedule, mopping nibbles), comma lists (4.18, station ops), base64 names, int-or-JSON (4.22) |
| Capabilities | presence via `code` | model table, fw gating, derived rules |

Suggested crate structure:

- **Wire level:** `PropId { siid, piid }`, `ActionId { siid, aiid }`,
  `Value = Int(i64) | Bool(bool) | Str(String) | Null`. Keep every raw value. Normalise cache strings
  into typed values.
- **Profile level:** a data table of `(name, siid, piid, codec, access)`. Tasshack uses one global
  table and has not needed per-model siid/piid overrides; meanings and labels do vary, so allow
  overrides.
- **Codecs:** CleaningModeGroup (4.23), AutoSwitch map (4.50), AiFlags (4.22, int or JSON), DndTasks,
  OffPeak, Schedule, Shortcuts, CleanSet with mopping nibbles, StartCustom builders, StationOp,
  MapEdit ops, Faults list, StreamStatus, cloud pointer (6.3, 6.8, 6.9).
- **Enums:** open sets (`Known` or `Unknown(i64)`).
- **Optional app layer:** Tasshack's state rewrites (T/device:8392-8443), availability predicates and
  fault filtering.

---

## 7. Open questions after the robot data

Resolved by R:

- The AE model id is `dreame.vacuum.r2579a`, capability set 152 (not r2492, set 170).
- 4.23 layout: byte 1 is the self-clean value (20) on r2579a.
- 4.22 uses the integer bitmask form on r2579a.
- 4.50 is a list of `{"k","v"}` objects; `ExtrFreq` exists, so MopExtend is available.
- Consumable order for siids 9, 10 and 11 matches Tasshack.
- 2.6 and 2.8 exist on r2579a. 2.6 matches the 4.23 mode field.
- Code 68 appears after a normal completed task.
- Map frames are not encrypted on this robot and firmware.
- AutoSwitch (4.50) writes take a single `{"k","v"}` object, as in T: writing
  `{"k":"MeticulousTwist","v":7}` worked on 2026-11-16 ([l40-ultra-ae.md](l40-ultra-ae.md),
  "Settings writes").

Still open:

1. **4.23 mode polarity (0 vs 2).** Read 2.6 and 4.23 while changing the mode in the app between
   "vacuum" and "vacuum and mop".
2. **2.6 as a write path.** Is writing 2.6 safer than writing 4.23?
3. **2.3, 2.8 and 14.3.** 2.3 may mirror 4.4. 2.8 may be H's mopping type. 14.3 = 200 is unknown.
4. **4.35 = 101 and 4.38 = -1.** Meanings unknown.
5. **AutoSwitch list.** The write form is resolved (above), but the captured 4.50 list is
   truncated; capture it whole.
6. **Cleanset index 1.** r2579a stores 3, not a wetness value. Confirm what it controls.
7. ~~**Absent features in the capability set.**~~ Answered by the relayed probe in §2.17:
   28.14-28.16, 28.18, 28.22 and 29.3 exist; the cache just misses them.
8. **Detergent consumable.** 4.37 = 1 but siid 20 is absent. Find where the app shows the
   detergent level.
9. **Sensor and wheel units.** p2 of siid 16 and p1 of siid 30 fit a 30-unit lifetime. Hours or
   days is open.
10. **Schedule fields 5-8** (4.4). Needs a schedule on the robot.
11. **4.58 TASK_TYPE and 4.83 DEVICE_CAPABILITY.** Not observed beyond 4.58 = 0. 4.83 bits could
    allow table-free detection.
12. **Error labels per model.** Extract `common_device_protocol.json` from the APK and use its
    `"2.2"` map for r2579a labels.
13. **Unswept ranges.** siid 4 piids 61-105 and siid 28 piids 61-117. §2.17 swept siids 31-60,
    99 and 10001 for piids 1-60 only, so piids above 60 of siids 99 and 10001 (the app uses
    10001.99 and 10001.1003, for example) and siids above 60 are still unswept.
14. **Coupled side effects.** Some writes change other properties, for example carpet "avoid" sets
    `RobotCarpetPressEnable` to -1 (node-dreame `docs/spec-discovery-methodology.md:258-274`). Re-read after
    writes.
15. **Cloud-only settings** never reach the device (node-dreame `docs/spec-discovery-methodology.md:276-287`).
