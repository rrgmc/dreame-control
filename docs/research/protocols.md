# Dreame robot vacuum protocols: transport reference

Status: research reference, 2026-11-15.
Audience: the agent that implements the transport and protocol layers of the Rust library.
Scope: the Dreamehome cloud (primary path), the Mi cloud (secondary), and local miIO (secondary).
Out of scope: the MIoT property catalogue and the map payload format, except where they touch
transport.

## 0. How to read this document

### 0.1 Verified versus reported

- **Verified** facts were observed against the user's own robot, a Dreame L40 Ultra AE
  (`dreame.vacuum.r2579a`, firmware `4.3.9_1664`), on a Dreamehome account registered outside the US. They are
  marked "Verified on r2579a, 2026-11-15". Where a verified fact contradicts a source, the
  verified fact wins.
- All other claims are **reported** by open-source clients or the APK analysis. Each has a
  citation. Treat them as likely, not certain.

### 0.2 Sources

Paths are relative to `local/refs/` in the repository root unless stated.

| Short name | Location | What it is |
|---|---|---|
| proto | `dreame-vacuum/custom_components/dreame_vacuum/dreame/protocol.py` | tasshack Home Assistant integration v2.0.3, all transport code. Most complete source. |
| tasshack | `dreame-vacuum/custom_components/dreame_vacuum/` | The rest of that integration (`device.py`, `map.py`, `types.py`, `config_flow.py`). |
| node-dreame | `node-dreame/` | TypeScript client v0.5.0, Dreamehome only. Good wire notes in `docs/auth-flow.md`. |
| homey | `com.dreame.vacuum.cloud/lib/DreameApi.js`, `DreameMqtt.js` | Homey app, Dreamehome only. |
| ioBroker | https://github.com/TA2k/ioBroker.dreame `main.js` (read 2026-11-15) | Upstream of node-dreame. |
| python-miio | https://github.com/rytilahti/python-miio `miio/protocol.py`, `miio/miioprotocol.py` | Local miIO reference. |
| APK | `local/apk/libapp.strings` | `strings` of the Flutter AOT library of Dreamehome 2.6.7.0. Cited as `apk:<line>`. |
| app | `local/apk/blutter/out/asm/` | Full Blutter decompile of the same library. Cited as `app:<path>`. See [decompile.md](decompile.md). |
| spike | An early prototype client, since replaced by `crates/dreame-cloud` and not kept in this repository | Login, signing, device list, cached props, `sendCommand`, download URL. Cited as `spike:<line>`; the citations record where a fact was first checked and no longer resolve. |

### 0.3 Terms

- **did**: device id, a decimal string assigned by the cloud. Written `<did>` here.
- **uid**: account id returned at login. Written `<uid>` here.
- **MIoT**: Xiaomi's property model. A device has services (`siid`). A service has properties
  (`piid`), actions (`aiid`) and events (`eiid`). A property is named `siid.piid`, for example
  `2.1`.
- **App constant**: a value compiled into the official app and shared by every install, such as
  the OAuth client secret. It is not a user credential. App constants can change when Dreame
  ships a new app build (node-dreame `docs/auth-flow.md:51-53`). Keep them in configuration.
- **OSS**: Aliyun Object Storage Service. Maps and other files live there.
- **Dialect**: a consistent set of request headers that mimics one version of the official app.

---

## 1. Dreamehome cloud: hosts and regions

### 1.1 Hosts

| Purpose | Host | Source |
|---|---|---|
| REST API | `https://<region>.iot.dreame.tech:13267` | proto:919-920; node-dreame `src/config.ts:4-13` |
| MQTT broker | from device field `bindDomain`, for example `10000.mt.<region>.iot.dreame.tech:19973` | proto:1114; node-dreame `docs/auth-flow.md:93` |
| Static product images | `https://oss.iot.dreame.tech/pub/pic/000000/ali_dreame/<model>/...` | ioBroker `main.js:910` |
| Signed file URLs | Aliyun OSS hosts returned by the API | node-dreame `src/map/oss-fetch.ts:10-12` |
| MOVA brand REST | `https://<region>.iot.mova-tech.com:13267` | proto:1179-1186 |
| TROUVER brand REST | `https://<region>.iot.trouver-tech.com:13267` | proto:1179-1186 |

Verified on r2579a, 2026-11-15: REST at `https://us.iot.dreame.tech:13267`; `bindDomain` =
`10000.mt.us.iot.dreame.tech:19973`.

The APK also contains `https://app.dreame.tech` (apk:20583) and `eu.iot.mova-tech.com:13267`
(apk:36793). The APK notes read the latter as "the EU host for Dreame". That reading is wrong for
Dreame accounts: every open-source client uses `eu.iot.dreame.tech`, and `mova-tech.com` is the
MOVA brand domain (proto:1179-1186). No client uses `app.dreame.tech` for the IoT API. Do not use
either host for Dreame device control.

### 1.2 Region codes

| Region | Notes | Source |
|---|---|---|
| `eu`, `cn`, `us`, `ru`, `sg`, `kr`, `by` | offered by tasshack | `config_flow.py:807-815` |
| `in` | listed, untested | node-dreame `src/config.ts:4-13` |
| `de` → `eu`, `tw` → `cn` | aliases in node-dreame | node-dreame `src/config.ts:4-13` |

KR quirk: the KR broker host does not resolve, so tasshack rewrites `10100` to `10000` in the
broker host and also subscribes to the `sg` topic (proto:1066-1070, 1136-1137).

### 1.3 Picking a region

The region is a property of the account, not of the user's location. A login against the wrong
region fails with an OAuth error such as `invalid_user` / "Dreame user not found"
(https://community.home-assistant.io/t/custom-component-dreame-vacuum/473026).

Verified on r2579a, 2026-11-15: an account registered outside the US lives in `us`. The token
response returned `region: "us"` with the account's own `country` and `lang` codes.

Algorithm:

1. If the caller gives a region, use it.
2. Otherwise try `us`, `eu`, `sg`, then the rest of §1.2, until one login succeeds
   (spike:268-271 tries `us`, `eu`, `sg`).
3. After login, take `region` from the token response and use it for all later calls
   (spike:160-163). Persist it with the refresh token.

The APK has region lookup endpoints (`/dreame-auth/countryCode` apk:38610,
`/dreame-smarthome/aliIot/findUidAliRegion` apk:42579,
`/dreame-system/appCommonUrl/queryUrlByCountryBatch` apk:1348). No client uses them; their
request shapes are unknown.

---

## 2. Request headers (dialects)

Two dialects are known. Both are reported to work.

### 2.1 Dialect A: Android app `cv=a_102060300` (default)

Source: proto:925-947, 962-966. Verified on r2579a, 2026-11-15: this dialect, with signing (§3),
works for login, device list, device info, OTC info, cached props, `sendCommand` and
`getDownloadUrl`. The prototype client sent it (spike:130-136, 180-188).

```
user-agent:     Dart/3.9 (dart:io)
dreame-meta:    cv=a_102060300;canvasHash=<h8c>;webglHash=<h8w>;visitorIdHash=<vs>
accept-encoding: gzip
dreame-rlc:     <base64(AES-128-ECB(key="EETjszu*XI5znHsI", PKCS7("<region>|<lang>|<country>")))>
tenant-id:      <tenant_id from login, else 000000>
authorization:  Basic ZHJlYW1lX2FwcHYxOkFQXmR2QHpAU1FZVnhOODg=
content-type:   application/json | application/x-www-form-urlencoded
dreame-auth:    <access_token>
```

| Field | Rule | Source |
|---|---|---|
| `<h8c>` | first 8 hex chars of `md5_hex(username + "c")` | proto:927-933; spike:120 |
| `<h8w>` | first 8 hex chars of `md5_hex(username + "w")` | proto:927-933; spike:121 |
| `<vs>` | `md5_hex` of 16 random bytes, fixed for the process | proto:757; spike:122 |
| `dreame-rlc` | base64. Sent only after the first login, because region, lang and country come from the token response. | proto:939-941, 1230-1232 |
| `dreame-auth` | raw access token, no `bearer` prefix. Omitted before login. | proto:962-966 |
| `tenant-id` | `000000` for Dreame. MOVA `000002`, TROUVER `000005`. | proto:1182, 1186 |
| `authorization` | decodes to `dreame_appv1:AP^dv@z@SQYVxN88` (client id and secret). Also present in the APK at apk:21430. | proto S[4] |

tasshack writes HTTP/1.1 by hand and also sends `content-length` right after `accept-encoding`
and `host` right after `authorization` (proto:787-807). Header order is probably irrelevant for a
normal HTTP stack. Verified on r2579a, 2026-11-15: `reqwest` default order and no `dreame-rlc`
header worked.

MOVA adds `dreame-psd: new` (proto:944-945). TROUVER omits the `;canvasHash...` suffix
(proto:1193, 1199, 1205).

### 2.2 Dialect B: iOS app `cv=i_829` (fallback)

Source: node-dreame `src/headers.ts:26-43`; ioBroker `main.js:800-810`.

```
user-agent:    Dart/3.2 (dart:io)
authorization: Basic ZHJlYW1lX2FwcHYxOkFQXmR2QHpAU1FZVnhOODg=
content-type:  application/x-www-form-urlencoded | application/json
dreame-auth:   bearer                 (before login, literal)
dreame-auth:   bearer <access_token>  (after login)
dreame-meta:   cv=i_829
dreame-rlc:    <hex(AES-128-ECB(key="EETjszu*XI5znHsI", PKCS7("eu|en|GB")))>
tenant-id:     000000
```

Differences from A: `dreame-rlc` is **hex**, not base64, and is sent on login too, built from
region defaults (node-dreame `src/crypto.ts:30-36`, `src/config.ts:16-37`).

homey sends neither `dreame-meta` nor `dreame-rlc`, uses UA
`Dreame_Smarthome/2.1.9 (iPhone; iOS 18.4.1; Scale/3.00)` and `Dreame-Auth: <token>` without
`bearer` (homey `DreameApi.js:7, 59-73`). So the server tolerates several header shapes.

### 2.3 Brand constants

| Constant | Dreame | MOVA | TROUVER | Source |
|---|---|---|---|---|
| Host suffix | `.iot.dreame.tech` | `.iot.mova-tech.com` | `.iot.trouver-tech.com` | proto S[0], S[50], S[52] |
| Port | `13267` | `13267` | `13267` | proto S[1] |
| Basic auth | `ZHJlYW1lX2FwcHYxOkFQXmR2QHpAU1FZVnhOODg=` | `bW92YV9hcHA6VjdLb0NoTFc4dkhBQ3FHYg==` | `dHJvdXZlcl9hcHA6NXB0MGV6d0w1TzRHMG96aw==` | proto S[4], S[70], S[78] |
| Tenant id | `000000` | `000002` | `000005` | proto S[5], proto:1182, 1186 |
| cid (signing secret, RLC key) | `EETjszu*XI5znHsI` | `gigxlmqwZ]7oWZUF` | `MMd02jyRqzuf1QBS` | proto S[55]-S[57] |
| `cv` version code | `a_102060300` | `a_10605530` | `a_10300532` | proto S[65]-S[67] |
| User-Agent | `Dart/3.9 (dart:io)` | `Dart/3.10 (dart:io)` | `Dart/3.6 (dart:io)` | proto S[79], S[69], S[80] |
| Password salt | `RAylYC%fmSKp7%Tq` | same | same | proto S[2] |
| `sendCommand` prefix fallback | `10000` | `20000` | unknown | node-dreame `src/config.ts:59-62` |

"S[n]" is index n of tasshack's decoded string table `DREAME_STRINGS` (proto:42-44, decoded at
proto:1177-1178 with `json.loads(gzip.decompress(base64.b64decode(s)))`).

---

## 3. Request signing

Source: proto:909-917, 968-994. First implemented at spike:31-92 with a unit test at spike:298-310; now in
`crates/dreame-cloud/src/sign.rs`.
Verified on r2579a, 2026-11-15: signed requests succeed. node-dreame, ioBroker and homey do not
sign and also work, so the server may not enforce signing yet. Sign anyway; it is cheap.

### 3.1 Which requests

Every JSON request whose body is an object. Not the OAuth form request. Not OSS downloads.

### 3.2 Algorithm

```
ms             = current Unix time in milliseconds
base           = spliced(body, top=true) + decimal(ms) + cid      # cid = "EETjszu*XI5znHsI"
body.sign      = md5_hex(base)                                     # lowercase hex
body.timestamp = ms                                                # JSON integer
```

`spliced(obj, top)`:

| Value type | Top level | Nested |
|---|---|---|
| (all) | keys in sorted order, parts joined with `&` | same |
| object | `k=[<spliced(v, false)>]`, or `k=]` if empty | same |
| array | `k=<compact JSON, sorted keys, non-ASCII kept>` | **omitted** |
| bool | `k=true` / `k=false` | same |
| null | `k=null` | same |
| string | `k=<raw string>` | `k="<JSON-quoted string>"` |
| number | `k=<number>` | `k=<number>` |

Compact JSON means separators `,` and `:` with no spaces, like Python
`json.dumps(v, separators=(",", ":"), sort_keys=True, ensure_ascii=False)`.

### 3.3 Worked example

```
body    = {"did":"1234567890","id":42,"data":{"did":"1234567890","id":42,
           "method":"get_properties","params":[{"did":"1234567890","siid":2,"piid":1}]}}
spliced = data=[did="1234567890"&id=42&method="get_properties"]&did=1234567890&id=42
ms      = 1700000000000
base    = data=[did="1234567890"&id=42&method="get_properties"]&did=1234567890&id=421700000000000EETjszu*XI5znHsI
sign    = 39ded7b48dd039c03d1a0404ba54fc6e
```

### 3.4 Hash: MD5 or SHA-384

The decompile confirms this algorithm: `ParamsSignInterceptor.signParams` builds the spliced string,
appends `cid`, hashes it with `package:crypto` and writes the lowercase hex to `sign`, with
`timestamp` alongside (`app:dreame_flutter_base_network/src/interceptor/params_sign_interceptor.dart`).

One detail is new. The hash is chosen at runtime by a boolean: one branch loads `_MD5`, the other
loads `_Sha384`. MD5 is the one verified on r2579a and the one all open clients use. SHA-384 of the
same `base` string is a code-confirmed alternative. If an MD5 signature is ever rejected, try
SHA-384 before assuming the scheme changed.

Note that `data.params` is an array inside a nested object, so it is omitted.

Second example (top-level array, kept):

```
body    = {"did":"1234567890","keys":[{"did":"2.1","siid":2,"piid":1}]}
spliced = did=1234567890&keys=[{"did":"2.1","piid":1,"siid":2}]
```

Both are asserted by the tests `worked_example_matches` and `top_level_array_is_kept` in
`crates/dreame-cloud/src/sign.rs`.

---

## 4. TLS

### 4.1 REST API

- The server certificate chains to a public CA. tasshack verifies it against `certifi`
  (proto:186-200).
- tasshack implements its own TLS client with a fixed ClientHello that mimics Android OkHttp
  (proto:135-689, ClientHello at proto:266-320). The likely purpose is to pass TLS fingerprint
  (JA3/JA4) filtering.
- Verified on r2579a, 2026-11-15: plain `rustls` through `reqwest` worked for every call in the
  `us` region. No fingerprint blocking was seen. Keep the TLS layer pluggable in case this changes
  (see §18, risk 2).

### 4.2 MQTT broker

- The broker certificate chains to a Dreame-private root. Its subject is `CN=GlobalSign Root CA`,
  it is self-signed and valid 1969-2969. tasshack bundles it as `DREAME_CLIENT_ROOT_CA`
  (proto:46-65) and tries it first when `private_ca=True` (proto:174-185, 691-696).
- node-dreame, ioBroker and homey disable verification instead (node-dreame `src/mqtt.ts:236-240`;
  ioBroker `main.js:4189`; homey `DreameMqtt.js:79`).
- Recommendation: build a `rustls` root store that contains only that PEM, and use it for MQTT.
  Copy the PEM from proto:46-65.

### 4.3 APK pinning and client certificate

From the APK analysis (not used by any open-source client):

- The Dart HTTP stack has server-trust pinning (apk:37973 `verifyServerTrustPinning`, apk:34744,
  apk:33658, apk:17173). A SHA-384 public-key pin appears near apk:10316. Pinning only matters to
  the app; a client is free to verify against public roots.
- The APK bundles a shared mutual-TLS client identity at
  `assets/flutter_assets/assets/cert/client.pem`, `client.key` and `cacert.pem` (apk:17310,
  apk:27742, apk:22802; extracted under `local/apk/extracted/assets/flutter_assets/assets/cert/`).
  Subject `CN=client`, issuer `CN=GlobalSign Organization Validation CA`, RSA 2048.
  Do not copy the private key into the library or its docs.
- Which channel needs the client certificate is unknown. Verified on r2579a, 2026-11-15: REST
  works without it. MQTT is not yet verified (§11).
- Other bundled roots: `assets/root.crt` (GlobalSign Root CA), `assets/server_cn.crt` (DigiCert
  Global Root G2), `assets/server_global.crt` (GoDaddy Root G2).

---

## 5. Login and token refresh

### 5.1 Password login (OAuth2 password grant)

```
POST https://<region>.iot.dreame.tech:13267/dreame-auth/oauth/token
content-type: application/x-www-form-urlencoded
(dialect A headers, without dreame-auth)

grant_type=password&scope=all&platform=ANDROID&type=account&username=<urlencoded email>&password=<hash>
[&country=<CC>&lang=<ll>]
```

- `<hash>` = `md5_hex(plaintext_password + "RAylYC%fmSKp7%Tq")`, lowercase (proto:1214;
  node-dreame `src/crypto.ts:17-19`; spike:124).
- The body is built by string concatenation in this order (proto:1213-1217). `country` and `lang`
  are appended only when known from an earlier token response.
- node-dreame sends `platform=IOS` and `&country=GB&lang=en` (node-dreame `src/auth.ts:57-66`).
- Verified on r2579a, 2026-11-15: the username must be the **email address**. The app's
  "account id" or "user name" did not work. Code: spike:114-172.

The APK notes say "no client-side password hash constant found" and suggest sending the password
in clear. The verified login used the salted MD5, so ignore that suggestion.

### 5.2 Token response

```json
{
  "access_token": "<jwt>",
  "refresh_token": "<jwt>",
  "expires_in": 7200,
  "token_type": "bearer",
  "uid": "<uid>",
  "tenant_id": "000000",
  "country": "<country>",
  "region": "us",
  "lang": "<lang>",
  "domain": "..."
}
```

Source: node-dreame `docs/auth-flow.md:57-68`. Verified on r2579a, 2026-11-15: `region`,
`country`, `lang`, `tenant_id` and `expires_in: 7200` as shown.

Keep: `access_token`, `refresh_token`, `uid`, `tenant_id`, `region`, `country`, `lang`, and the
expiry time. `uid` may be a number or a string; normalise to a string (spike:152).

### 5.3 Login errors

- HTTP 400, 401 or 403 with an OAuth body `{"error": "...", "error_description": "..."}`.
  tasshack treats these as bad credentials (proto:1235-1250).
- Wrong region gives `invalid_user` / "Dreame user not found" (see §1.3).
- 5xx and 429 are transient (proto:1235-1250).
- No client handles captcha, SMS or 2FA for Dreamehome (node-dreame `docs/auth-flow.md:201-202`).

### 5.4 Refresh

Same URL and headers. Body (proto:1211-1212, S[77]):

```
grant_type=refresh_token&scope=all&platform=ANDROID&type=account&refresh_token=<refresh_token>
```

Other clients send shorter bodies: node-dreame `grant_type=refresh_token&refresh_token=<t>`
(`src/auth.ts:76-79`); homey `platform=IOS&scope=all&grant_type=refresh_token&refresh_token=<t>`
(`DreameApi.js:144`).

Verified on r2579a, 2026-11-15: the tasshack body works. The reply is HTTP 200 with a new
`access_token` **and a new `refresh_token`**. Refresh tokens rotate, so a client must store the new
refresh token after every refresh. Other reply fields: `jti`, `token_type`, `region`, `country`,
`lang`, `expires_in` (7199), `scope`, `tenant_id`, `is_new_register`, `client_id` (`dreame_appv1`),
`role_name`, `role_id`, `dept_id`, `uid`, `u` (numeric user id), `t` (issue time in seconds),
`user_token`. API calls with the new access token work. The old refresh token stays valid after a
refresh (tested 2026-11-17): it still gives a new access token, and using it does not revoke the
newer refresh token. Code: `crates/dreame-cloud` (token refresh).

When to refresh:

| Client | Rule | Source |
|---|---|---|
| tasshack | when 0 < remaining ≤ 600 s, and on HTTP 401 | proto:949-960, 1619-1625 |
| node-dreame | 100 s before expiry | `src/client.ts:27, 107` |
| homey | 120 s before expiry | `DreameApi.js:8` |

If the refresh fails and `error_description` contains `refresh token`, do a full password login
(proto:1236-1241). tasshack persists only the refresh token, so a restart does not need the
password (proto:1020-1022). The refresh token lifetime is not documented.

The access token is also the MQTT password. After a refresh, reconnect MQTT with the new token
(proto:1034-1039, 1079-1083).

---

## 6. Generic API response and errors

Most JSON endpoints answer with:

```json
{"code": 0, "success": true, "msg": "操作成功", "data": ...}
```

- `code: 0` means success. Check `code`, not only the HTTP status.
- Verified on r2579a, 2026-11-15: `msg` is in Chinese whatever `lang` is. Do not show it to users
  as-is; map known codes instead.
- HTTP 401 means the access token expired or is invalid: refresh, then retry once
  (proto:955-960).
- HTTP 429, 502, 503, 504: retry with backoff `min(0.5 * 2^(n-1), 2)` s (proto:1585-1606).
- tasshack marks the cloud as disconnected after 5 consecutive failures (proto:1629-1633) and
  spaces queued calls 100 ms apart (proto:899).

Known `code` values:

| code | Meaning | Source |
|---|---|---|
| `0` | success | all |
| `80001` | cloud accepted the command but the robot did not ACK in time. **Not** "offline". The robot often executes it anyway and echoes the new state over MQTT. Also returned when `action` params are sent as an array (§8.3). | node-dreame `README.md:113, 119-137`, `docs/auth-flow.md:161-186`, `CHANGELOG.md:485-534` |

---

## 7. Device list, info and OTC info

### 7.1 Device list

```
POST /dreame-user-iot/iotuserbind/device/listV2
{"sharedStatus":1,"current":1,"size":100,"lang":"en"}     (+ sign, timestamp)
```

Body from node-dreame `src/devices.ts:52-58` and spike:198-210. tasshack sends no body
(proto:1309-1313); homey sends `{}` (`DreameApi.js:244`). Verified on r2579a, 2026-11-15 with the
body above.

Response: records at `data.page.records`. Older shapes put `records` at `data.records` or at the
top level (node-dreame `src/devices.ts:64-66`; spike:203-206).

| Field | Meaning | Source |
|---|---|---|
| `did` | device id (string) | all |
| `model` | model string. Verified: `dreame.vacuum.r2579a`. Read it at runtime; L40 Ultra AE units may also report `r2579h`, `r500za`, `r500zh` (https://dv.tasshack.com/guide/more/supported-devices). | all |
| `mac`, `sn`, `ver` | MAC, serial, firmware. Verified: `ver` = `4.3.9_1664`, serial starts `R2579`. | proto:1297-1303 |
| `customName`, `deviceInfo.displayName` | names | proto:1272-1274 |
| `masterUid`, `masterName`, `master` | owner | proto:1111, 1284-1292 |
| `bindDomain` | MQTT broker `host:port`. Its first DNS label is the `sendCommand` prefix. Verified: `10000.mt.us.iot.dreame.tech:19973`. | proto:1114, 1370 |
| `property` | JSON **string**. Contains `iotId`, `lwt` (1 = online), `mac`. Verified: `{"lwt":1,"mac":...}`. | proto:1115-1119 |
| `online`, `latestStatus`, `battery`, `videoStatus`, `featureCode2` | cached cloud state. Verified: `featureCode2` = 31. | node-dreame `src/devices.ts:107-133` |

The APK also has `/dreame-user-iot/iotuserbind/getDeviceListByHomeV2` (APK notes §5). Not used by
any client.

### 7.2 Device info

```
POST /dreame-user-iot/iotuserbind/device/info     {"did":"<did>"}
POST /dreame-user-iot/iotstatus/devOTCInfo        {"did":"<did>"}
```

Source: proto:1315-1347; homey `DreameApi.js:253`. Verified on r2579a, 2026-11-15 (called from
the prototype client).

`devOTCInfo` returns `data.otcInfo.params`, the robot's own `_otc.info` report. Verified fields:
`fw_ver`, `hw_ver` (`Linux`), `model`, `netif.localIp`, `netif.mask`, `netif.gw`, `ap.siid`,
`ap.bssid`, `ap.rssi`. tasshack merges these into the device info and falls back to the list
record on failure (proto:1323-1347).

---

## 8. Commands: `sendCommand`

`sendCommand` relays an MIoT call to the robot and waits for its answer.

### 8.1 Path

```
POST /dreame-iot-com-<prefix>/device/sendCommand
```

`<prefix>` is the first DNS label of `bindDomain` (proto:1368-1370, 1397-1403; spike:222-226).
Verified on r2579a, 2026-11-15: `/dreame-iot-com-10000/device/sendCommand`.
Fallbacks: `/dreame-iot-com/device/sendCommand` when `bindDomain` is unknown (tasshack), or a
hard-coded `10000` (node-dreame `src/config.ts:59-62`).

### 8.2 Envelope

```json
{
  "did": "<did>",
  "id": 34,
  "data": {
    "did": "<did>",
    "id": 34,
    "method": "get_properties",
    "params": [ ... ]
  },
  "sign": "<md5 hex>",
  "timestamp": 1700000000000
}
```

- `id`: any integer. tasshack uses a counter that starts at a random 1..100 (proto:727, 1372;
  spike:170, 227). node-dreame uses a random 31-bit integer (`src/crypto.ts:42-44`).
- `data.from: "XXXXXX"` is sent by node-dreame and ioBroker only (`src/config.ts:51-57`). Not
  needed: verified on r2579a without it.
- Source: proto:1383-1392; node-dreame `src/commands.ts:83-105`; spike:220-234.

### 8.3 Methods and `params`

On the Dreame cloud, `did` inside each item is the real device id. On the Mi cloud and local miIO
it is a free label, usually `"<siid>.<piid>"` (proto:2822-2900).

| Method | `params` | Source |
|---|---|---|
| `get_properties` | `[{"did":"<did>","siid":2,"piid":1}, ...]` | proto:1383-1392; verified |
| `set_properties` | `[{"did":"<did>","siid":4,"piid":4,"value":1}, ...]` | proto:2822-2900 |
| `action` | `{"did":"<did>","siid":2,"aiid":1,"in":[]}` (a single **object**) | node-dreame `docs/auth-flow.md:161-181` |

- Batch size for `get_properties`: tasshack uses at most 15 per call, or 10 when a large
  property is included (`device.py:860-881`).
- `action` with an array instead of an object yields a misleading `code: 80001`
  (node-dreame `docs/auth-flow.md:161-181`).
- `in` items are usually `{"piid":N,"value":V}`, for example the map request in §12.5.
- `set_properties` and `action` are verified on r2579a since 2026-11-16: settings writes, map
  edits (action 6.2) and control actions, each approved by the user (see
  [l40-ultra-ae.md](l40-ultra-ae.md)).

### 8.4 Response

Verified on r2579a, 2026-11-15:

```json
{"code":0,"success":true,
 "data":{"id":33,"result":[{"siid":2,"did":"<did>","piid":1,"value":8,"code":0}]}}
```

- Values are typed JSON (numbers as numbers), unlike `iotstatus/props` (§9.1).
- Read `data.result`. Some firmware puts `result` at the top level (node-dreame
  `src/commands.ts:42-44`).
- `success: true` without `data.result`: tasshack retries once (proto:1424-1426).
- Per-item `code` follows MIoT: `0` is success, negative is an error.
- `code: 80001`: see §6. Treat as "unknown outcome" and confirm through MQTT or a later read.

---

## 9. Cloud-side reads (not relayed to the robot)

These read the cloud's copy of the robot state. They work while the robot is busy or asleep.
All are under `/dreame-user-iot/`, JSON, signed.

### 9.1 `iotstatus/props`: cached property values

```
POST /dreame-user-iot/iotstatus/props
{"did":"<did>","keys":"2.1,3.1,6.3"}
```

Verified on r2579a, 2026-11-15 (spike:212-217):

- `keys` must be **one comma-separated string**. A JSON array returns `{"code":0,"data":[]}`
  without any error. This settles the conflict between homey (array of objects,
  `DreameApi.js:288-327`) and tasshack (single string, `map.py:565`).
- Up to 60 keys in one call worked.
- Reply items: `{"key":"2.1","value":"8","updateDate":<ms>}`.
- `value` is always a **string**. Numbers come as `"8"`. JSON values come as JSON-encoded strings,
  for example `"{\"enable\":false,...}"`. Parse by the property's declared type.
- Keys with no cached value are absent from the reply.

### 9.2 `iotstatus/history`: property, event and action history

```
POST /dreame-user-iot/iotstatus/history
{"uid":"<uid>","did":"<did>","from":<s>,"limit":<n>,"siid":4,"eiid":1,"region":"us","type":3}
```

Use `piid` or `aiid` instead of `eiid` for properties or actions. Source: proto:1526-1554; ioBroker
`main.js:6190-6207`.

**Verified on r2579a, 2026-11-17, for the cleaning record (event 4.e1):**

- `region` must be the **server region code** (`us`), lowercase. The account's country code, which
  the file URL calls accept, returns `success: true` with an empty list.
- `from` is in Unix **seconds**. The reply lists entries newest first, at most `limit`
  (200 came back for a year).
- Reply: `{"code":0,"success":true,"data":{"list":[{"id":"<19 digits>","region":"us",
  "history":"<JSON text>","createTime":<ms>}, ...]}}`. `history` is the event's argument list as
  JSON text, `[{"piid":n,"value":v}, ...]`, the same as the MQTT `arguments` (§11): duration,
  area, start time, result, the history map object (piid 9) and the summary JSON (piid 10).
- The history map (piid 9) downloads through `iotfile/getDownloadUrl` and decodes as a frame with
  the rooms and the job's whole path (1744 points for a 30-minute job).
- Property history (`piid` 6.3) returned an empty list with the same request; not explored further.

`dreame::history` reads it (`jobs`, `job_map`); `dreame history` lists jobs.

### 9.3 `iotuserdata`: user data blobs

```
POST /dreame-user-iot/iotuserdata/getDeviceData   {"did":"<did>","model":<props>}
POST /dreame-user-iot/iotuserdata/setDeviceData   {"did":"<did>","model":<props>}
```

The property list goes under the key `model`; this is how tasshack does it, not a typo
(proto:1556-1572, S[30]). ioBroker uses it for mower maps (`main.js:5186-5192`). Not yet verified
on r2579a. The APK lists `setDeviceData` at apk:52005.

---

## 10. Cached property snapshot (r2579a)

Verified on r2579a, 2026-11-15: a sweep of siid 1-30, piid 1-60 through `iotstatus/props` returned
values for these keys. Use it as a test fixture and to decide which keys to poll. Meanings belong
in the property catalogue, not here.

| siid | piids with values |
|---|---|
| 1 | 2 (model), 5 (serial) |
| 2 | 1, 2, 3, 6, 8 |
| 3 | 1, 2, 3 |
| 4 | 1-4, 6, 7, 11, 12, 17, 18, 20, 22-29, 31, 33-41, 45-48, 50-53, 55-58, 60 |
| 5 | 1, 2, 3 |
| 6 | 3 (map object name), 7, 8 (saved-map list pointer), 9 |
| 7 | 1, 2, 3, 5, 7 |
| 8 | 1 (time zone) |
| 9-12 | 9.1-2, 10.1-2, 11.1-2, 12.1-4 |
| 14-16 | 14.3-4, 15.1, 15.3, 15.5, 16.1-2 |
| 27, 28, 30 | 27.1-4, 27.7, 27.10-11, 27.16, 28.1, 28.5, 30.1-2 |

`6.1` and `6.13` (map data channels) were empty in the cache.

---

## 11. MQTT push

The robot reports state changes through the Dreame MQTT broker. Clients subscribe to receive them.
Clients do not publish; commands always go through `sendCommand` (node-dreame
`docs/auth-flow.md:109-140`).

Verified on r2579a, 2026-11-15 (prototype MQTT client, now `crates/dreame-mqtt`):

- A **standard** MQTT 3.1.1 CONNECT is accepted (CONNACK 0). tasshack's Will QoS bit is not needed.
- Username `<uid>`, password = access token, client id `p_` + 16 random hex characters, clean
  session, keep-alive 60 s.
- SUBSCRIBE to `/status/<did>/<uid>/dreame.vacuum.r2579a/us/` with QoS 0 is granted QoS 0.
- The broker speaks TLS 1.3. Its certificate chain cannot be checked by `rustls-webpki` (see
  §11.1a), so the client needs a custom verifier.
- No messages arrived in 60 s while the robot was idle and docked.
- A 10-minute capture with the robot docked and idle received 9 messages, all
  `properties_changed`, all on the subscribed topic. The envelope matches §11.3:
  `{"id":4009,"did":<did>,"data":{"id":4009,"method":"properties_changed","params":[{"did":"<did>","siid":4,"piid":64,"value":60}]}}`.
  The top-level `did` was a JSON number, while `params[].did` was a string. Parse both forms.
  - **4.64** arrived every 108 s with a value that rose by one each time (60, 61, 62, ...).
    It is `DRYING_PROGRESS` (device-model.md, siid 4 table): the mop was drying after an earlier
    clean.
  - **99.98** arrived as a burst of four messages (260 to 1,208 bytes). Each value is URL-safe
    base64 of zlib data (it starts with `eN`). Decoded, it is robot telemetry:
    `{"generalRule":[[<tag>, <int>, <string>, <unix time>], ...]}` with tags such as `dev_res`
    (resource counters), `ms`, `ami`, `ns` (a signal value every 60 s), `ue2`, and a final
    `frame_report` "Complete data upload" covering 30 minutes. A control library can ignore 99.98.
  - No state, map or event messages arrived, because the robot did nothing during the capture.

### 11.1b Capture during a cleaning run (verified on r2579a, 2026-11-15)

The user started a one-room clean from the app. The run took about 6.5 minutes from start to the
robot charging on the dock. In that time, about 180 messages arrived. Timeline, in seconds from
the first message:

| Time | Message | Reading |
|---|---|---|
| 2 | `properties_changed` 4.1 = 18, 4.7 = 3, 4.58 = 1, 4.2 = 0, 4.3 = 0, 4.25 = 0, 4.62 = 0, 15.3 = 0 | task setup: status, task status, task type, timers |
| 3 | `event_occured` 2.e3, no arguments | task started |
| 4 | 2.1 = 1, 2.2 = 0, 4.18 = 0; 6.3 = `.../0` | state "sweeping"; errors and faults cleared |
| 5 | `event_occured` 4.e4, no arguments | meaning unknown (device-model.md §2.4) |
| 7 | 3.2 = 2 | charging state: not charging |
| 13-15 | 6.3 = `.../1`, then `.../0` | object name flips (map file uploads) |
| 16-377 | 122 × 6.1, one every 3 s | inline P-frames, `map_id` 5, `frame_id` 1 to 122 |
| 37-318 | 45 × 4.63, rising from 6 to 100 | task progress in percent |
| 52-273 | 4.3 = 1, 2, 3, 4 and 4.2 = 1, 2, 3, 4 | 4.3 `CLEANED_AREA` (m²) and 4.2 `CLEANING_TIME` (min) |
| 73-160 | 4.58 toggles 1 → 17 → 1 | task type changes (probably mop washing) |
| 104, 165, 226 | `_otc.info` | Wi-Fi and IP self-report about every 60 s |
| 318 | 2.1 = 5, 4.1 = 3, 3.2 = 5 | returning to dock |
| 389 | consumables 9.x, 10.x, 11.x, 16.x, 30.x (each with a new piid 99 = 4); totals 12.2-12.4; 4.7 = 0, 4.1 = 6, 4.58 = 0, 15.3 = 1 | task finished; life counters and totals updated |
| 391-407 | 2.1 = 6, 22, 6; 15.5 = 1, then 0 | charging; a short station step (22) in between |
| 392 | `event_occured` 4.e3, no arguments | task ended |
| 393 | `event_occured` 4.e1, arguments below | **cleaning record** |
| 394 | 6.8 with a new md5; 6.3 = `.../1`, then `.../0` | saved map re-saved; map file uploads |

Message format findings:

- **`event_occured` uses a single object for `params`**, not an array:
  `{"id":4210,"method":"event_occured","params":{"did":"<did>","siid":4,"eiid":1,"arguments":[...]}}`.
  A parser that expects an array, as for `properties_changed`, drops every event.
- **`_otc.info` has the usual outer envelope, but its inner `data` object has no `id`** and carries
  `partner_id`:
  `{"id":...,"did":...,"data":{"partner_id":"","params":{"hw_ver","fw_ver","ap":{"siid","bssid","rssi"},"netif":{...},"model"},"method":"_otc.info"}}`.
  (An earlier version of this section said there was no `data` wrapper; the captures show there is.
  `dreame-mqtt` accepts both forms.)
- Message `id`s can arrive out of order (4211 before 4210).
- The cleaning-record event 4.e1 has these arguments (names from device-model.md §2.4):
  `piid 1` STATUS = 18, `2` CLEANING_TIME = 4 min, `3` CLEANED_AREA = 4 m², `6` = 0,
  `13` CLEAN_LOG_STATUS = 1 (completed), `8` CLEANING_START_TIME (Unix s),
  `9` CLEAN_LOG_FILE_NAME = object name of the history map (`ali_dreame/<yyyy>/<mm>/<dd>/<uid>/<did>_<n>.<build>.bin`),
  `10` CLEANING_PROPERTIES = JSON `{"cleaningTime":4,"customeClean":0,"mooClean":2,"pet":0,"cmc":1,"ctyo":2,"multime":<unix s>}`.
  The history map downloads through `iotfile/getDownloadUrl` (the interim endpoint), even though
  tasshack uses the permanent endpoint for history maps on Dreamehome. See maps.md §2.

### 11.1a Broker certificate (verified)

The broker at `10000.mt.us.iot.dreame.tech:19973` presents this chain:

| # | Subject | Issuer | Notes |
|---|---|---|---|
| 0 | `C=CN, ST=Jiangsu, L=Suzhou, O=Dreame, OU=Devops, CN=iot.dreame.tech` | `CN=GlobalSign Organization Validation CA` | RSA 3072, valid 1969-12-31 to 2969-05-02. SANs: `iot.dreame.tech`, `*.dreame.tech`, `*.iot.dreame.tech`, `*.mt.{cn,sg,eu,us,in,ru,kr}.iot.dreame.tech`, the same set for `mova-tech.com`, and IP `127.0.0.1` |
| 1 | `CN=GlobalSign Organization Validation CA` | `CN=GlobalSign Root CA` | private intermediate, not the real GlobalSign |
| 2 | `CN=GlobalSign Root CA` | self | private root; byte-identical to tasshack's `DREAME_CLIENT_ROOT_CA` (proto:46-65) |

- OpenSSL verifies the chain against that root and accepts the host name
  (`openssl s_client -CAfile <root> -verify_hostname 10000.mt.us.iot.dreame.tech`: `Verification: OK`).
- `rustls` with `rustls-webpki` fails the handshake with `invalid peer certificate: BadEncoding`,
  even with the root in its trust store. The 1969 and 2969 validity dates are the likely cause.
- Working approach in Rust: a custom `ServerCertVerifier` that pins the root DER, verifies each
  signature in the chain and the validity dates with `x509-parser` (feature `verify`), matches the
  host name against the leaf's DNS SANs (with `*.` wildcards), and verifies the TLS 1.3 handshake
  signature with `rustls::crypto::verify_tls13_signature_with_raw_key` using the leaf's public key.
  It accepts TLS 1.3 only. See the pinned-root verifier in `crates/dreame-mqtt/src/tls.rs`.
- Do not switch verification off. The other clients do (`rejectUnauthorized: false`), but that would
  hand the access token to anyone who can intercept the connection.

### 11.1 Connection

| Item | Value | Source |
|---|---|---|
| Transport | MQTT 3.1.1 over TLS | proto:1121-1161; node-dreame `src/mqtt.ts:227-240` |
| Host and port | `bindDomain` of the device (`host:port`, port 19973) | proto:1114 |
| Fallback host | `app.mt.<region>.iot.dreame.tech:19973` (MOVA: `app.mt.<region>.iot.mova-tech.com:19974`) | ioBroker `main.js:96, 105, 4184` |
| Server certificate | Dreame-private root (§4.2) | proto:46-65 |
| Client id | tasshack: `"p_" + md5_hex(did + "mqtt" + vs)`, where `vs` is the per-process visitor hash (§2.1). Others: `"p_"` + 16 random hex chars. | proto S[47], S[90]; node-dreame `src/mqtt.ts` |
| Username | `<uid>` | proto:1121-1161 |
| Password | current access token (raw JWT) | proto:1121-1161 |
| Clean session | true | proto:1121-1161 |
| Keep-alive | 60 s | proto:1121-1161 |

CONNECT flag quirk: tasshack sets bit 3 of the connect-flags byte (Will QoS = 1) without the Will
flag (proto:698-705). That violates MQTT 3.1.1 §3.1.2.6, yet the broker accepts it. The reason is
unknown. A standard CONNECT works on the `us` broker (verified on r2579a), so the bit is not needed
there.

Auth failure: CONNACK return code 5 (not authorized) means the token is stale. Log in again, then
reconnect (proto:1079-1083).

The APK fetches broker domains from `/dreame-user-iot/iotmqttdomain/v2/list` (apk:54298). No
client uses it; `bindDomain` is enough.

### 11.2 Topic

```
/status/<did>/<uid>/<model>/<region>/
```

Subscribe with QoS 0. The leading and trailing slashes are both required (node-dreame
`src/mqtt.ts:385-392`). For KR, also subscribe with region `sg` (proto:1136-1137).
Example for this robot: `/status/<did>/<uid>/dreame.vacuum.r2579a/us/`.

### 11.3 Message envelope

```json
{"id":92,"did":"<did>","data":{"id":92,"method":"properties_changed",
  "params":[{"did":"<did>","siid":2,"piid":6,"value":1}]}}
```

Source: node-dreame `docs/auth-flow.md:109-131`. tasshack forwards only `data` (proto:1094-1108).

### 11.4 Message types (`data.method`)

| Method | `params` | Source |
|---|---|---|
| `properties_changed` | array of `{did,siid,piid,value}`. Includes map channels `6.1` (map data), `6.3` (object name), `6.13` (old map data). | node-dreame `src/mqtt.ts:309-363`, `src/spec/map-props.ts:8-56` |
| `event_occured` (sic) | `{siid,eiid,arguments:[...]}` | tasshack `device.py:547-590` |
| `props` | free key/value, for example `{"ota_progress":47}` | node-dreame `src/mqtt.ts:309-363` |
| `_otc.info` | device self-report: `fw_ver`, `model`, `ap`, `netif.localIp`, ... | node-dreame `src/mqtt.ts:309-363` |
| `_sync.update_vacuum_mapinfo` | map list updates | node-dreame `src/mqtt.ts:309-363` |

Limit: the broker truncates messages over 4096 bytes (node-dreame `docs/auth-flow.md:140`). Fetch
large data (maps) through OSS (§12).

---

## 12. OSS file download

### 12.1 Object names

The robot publishes OSS object names in properties. Verified on r2579a, 2026-11-15:

| Property | Value shape | Meaning |
|---|---|---|
| `6.3` | `ali_dreame/<uid>/<did>/0` | latest live map frame |
| `6.8` | `{"obj_name":"ali_dreame/<uid>/<did>/9","md5":"<hex>"}` | saved-map list file |
| `6.9` | `ali_dreame/<uid>/<did>/8` | another map object |

Some models append `,<key>` to the name; the part after the comma is a map decryption key
(tasshack `map.py:320-352, 551-556`). Verified on r2579a, 2026-11-15: no `,key` suffix, and the
downloaded data is not encrypted.

### 12.2 Resolve a signed URL

```
POST /dreame-user-iot/iotfile/getDownloadUrl
{"did":"<did>","model":"<model>","filename":"<object name>","region":"<country>"}
→ {"code":0,"data":"https://...signed..."}
```

Verified on r2579a, 2026-11-15: `region` must be the **country code from the token response**
(for example `"DE"`), not the API region (spike:237-246). `data` is the URL string; spike:243-245 also
accepts `data.url`.

Second resolver for permanent (saved-map) objects, not yet verified (proto:1487-1501):

```
POST /dreame-user-iot/iotfile/getOss1dDownloadUrl
{"did":"<did>","uid":"<uid>","model":"<model>","filename":"<object name minus first char>","region":"<country>"}
```

tasshack strips the first character of the object name for this call.

### 12.3 Download

`GET <signed URL>` with `user-agent: okhttp/4.12.0` and `accept-encoding: gzip`. No Dreame headers
(proto:1467-1485; spike:248-250). tasshack retries up to 4 times. URLs live about an hour; clients
cache them 30 min and append `&current=<unix seconds>` on reuse (tasshack `map.py:590-610`).

Verified on r2579a, 2026-11-15: the body is base64 text. After base64url decoding (`_` → `/`,
`-` → `+`) it inflates with zlib directly. The live frame was about 26 KB of base64 text.

Encrypted variant (other models): AES-256-CBC with key = first 32 hex chars of
`sha256_hex(<key>)` and a per-model 16-byte IV, then zlib (tasshack `map.py:4196-4224`). The IV
table is obfuscated in tasshack `dreame/const.py:736`.

### 12.4 Device file bridge

```
POST /file-bridge/user/getDeiviceFile          (sic: "Deivice")
{"did":"<did>","uid":"<uid>","fileinfo":"{\"filename\":...,\"type\":...}"}
```

Returns the file bytes directly (proto:1431-1465). Not yet verified.

### 12.5 Request a fresh map frame

`action` siid 6, aiid 1:

```json
{"did":"<did>","siid":6,"aiid":1,
 "in":[{"piid":2,"value":"{\"req_type\":1,\"frame_type\":\"I\",\"force_type\":1}"}]}
```

Source: node-dreame `src/map/request.ts:34-75`. Not yet verified (writes were out of scope).

---

## 13. Rate limits and polling

No documented limits. Observed client behaviour:

- tasshack polls every 3-5 s while the robot is active and every 10 s when idle; map refresh every
  10 s active and 30 s idle (`device.py:7930-7950`).
- homey: 15 s timeout, 500 ms × n backoff (`DreameApi.js:196-236`).
- tasshack: backoff on 429/502/503/504 (§6).

Recommendation: prefer MQTT pushes, poll `iotstatus/props` slowly (30-60 s) as a safety net, and
always back off on 429.

---

## 14. APK endpoint inventory

From Dreamehome 2.6.7.0 `libapp.strings`. Only the endpoints marked "used" are known from
working clients; the rest are names only, with unknown request shapes. Line numbers are in
`libapp.strings` where the APK notes gave them.

### 14.1 Auth (`/dreame-auth`)

| Endpoint | Status |
|---|---|
| `/oauth/token` (apk:45426) | used, verified |
| `/oauth/authCode` (apk:48834), `/oauth/email` (apk:37711), `/oauth/logout` (apk:5476) | names only |
| `/v2/oauth/sms` (apk:50907), `/v2/oauth/social/sms` (apk:16533), `/v3/oauth/social/...` (apk:50981) | names only; SMS and social login |
| `/countryCode` (apk:38610) | names only; region lookup |

### 14.2 Devices and state (`/dreame-user-iot`)

| Endpoint | Status |
|---|---|
| `/iotuserbind/device/listV2` | used, verified |
| `/iotuserbind/device/info` | used, verified |
| `/iotuserbind/getDeviceListByHomeV2` | names only |
| `/iotstatus/props` (apk:17456) | used, verified |
| `/iotstatus/devOTCInfo` (apk:21801) | used, verified |
| `/iotstatus/history` | used by tasshack |
| `/iotuserdata/getDeviceData`, `/iotuserdata/setDeviceData` (apk:52005) | used by tasshack |
| `/iotfile/getDownloadUrl` | used, verified |
| `/iotfile/getOss1dDownloadUrl` | used by tasshack |
| `/iotmqttdomain/v2/list` (apk:54298) | names only; MQTT broker list |
| `/aliiotcloud/getUserDevice` (apk:48954), `/aliiotcloud/mvDeviceRegion` (apk:5709) | names only; Aliyun identity |
| `/smarthome/scene/*`, `/smarthome/home`, `/smarthome/room` | names only; homes, rooms, scenes |

### 14.3 Binding and provisioning (`/dreame-user-iot/iotuserbind`)

| Endpoint | Purpose |
|---|---|
| `/pair` (apk:51233) | bind after provisioning |
| `/pairByNonce` (apk:34167) | bind with a nonce challenge |
| `/pair4ble` (apk:30946) | bind after BLE provisioning |
| `/pairQRKey` (apk:33432) | QR binding |
| `/cloudDevBindV2` (apk:36320) | cloud-side bind |
| `/checkDeviceBind` (apk:30945) | bind status |

### 14.4 Commands and files (other services)

| Endpoint | Status |
|---|---|
| `/dreame-iot-com-<prefix>/device/sendCommand` (apk:47181, apk:39891) | used, verified |
| `/file-bridge/user/getDeiviceFile` | used by tasshack |
| `/dreame-smarthome/aliIot/getAuthCodeV3` (apk:24588), `/aliIot/findUidAliRegion` (apk:42579) | names only; Aliyun bridge |
| Aliyun RPC `Service.smarthome.getDeviceData` (apk:46411), `batchGetDeviceDatas` (apk:2132), `batchSetDeviceDatas` (apk:47739), `getFileUrl` (apk:31281) | names only; envelope unknown |

The APK notes conclude that the app controls vacuums over Aliyun RPC rather than `sendCommand`.
Verified on r2579a, 2026-11-15: `sendCommand` works for this robot. Treat the Aliyun RPC as an
unexplored alternative, not a requirement.

### 14.5 Account, product, system

| Prefix | Endpoints |
|---|---|
| `/dreame-user` | `/v2/register/sms`, `/v3/register/email`, `/v2/register/phone`, `/v2/register/email/check/code`, `/v2/change-password`, `/v2/set-password`, `/v2/forgotpass/reset-by-email`, `/v2/forgotpass/reset-by-sms`, `/v1/info`, `/v1/userext/info`, `/v1/set-country`, `/v1/logoff` (deletes the account), `/alldata/download/submit` |
| `/dreame-product` | `/upgrades/h5plugin` (apk:5932), `/upgrades/appplugin` (apk:22919), `/upgrades/sdk` (apk:9366), `/public/common-plugin/getCommonPlugins` (apk:50694), `/public/v1/productCategory/by-models`, `/by-pids-with-model`, `/checkModel`, `/by-pids` |
| `/dreame-system` | `/appCommonUrl/queryUrlByCountryBatch` (apk:1348) |
| Logging | `/dreame-mqtt-log/appLog` (apk:41916), `/dreame-log/common/log/report` |
| Other prefixes | `/dreame-message-push`, `/dreame-messaging`, `/dreame-third-video` (camera, Tencent), `/dreame-third-proxy` |

The library must never call `/v1/logoff` or the password and registration endpoints.

---

## 15. Provisioning and binding (overview)

Provisioning puts a new robot on Wi-Fi and binds it to an account. A library for an already bound
robot does not need it. Summary from the APK analysis (APK notes §4):

- The app uses Aliyun Link Kit provisioning (`assets/config.conf` lists
  `com.aliyun.alink.business.devicecenter.provision.*`): SoftAP, BLE (`BreezeConfigStrategy`),
  broadcast/SmartConfig, QR, phone AP. Discovery uses CoAP (`libcoap.so`), cloud and BLE.
- SoftAP flow: the phone joins the robot's AP and POSTs Wi-Fi credentials to the robot's local
  HTTP endpoints `/pairnet/addwifi` (apk:11920) and `/pair_connect` (apk:3692).
- The app then binds the robot through one of the `/iotuserbind/pair*` endpoints (§14.3).
  `pairByNonce` uses a nonce and signature (`buildSignature`, `nonceStr` at apk:25570, apk:11520).
- Request shapes are unknown. Implementing provisioning would need a traffic capture.

---

## 16. Mi cloud path (Xiaomi account)

Applies only to Dreame robots bound in Mi Home / Xiaomi Home. Source: tasshack
`DreameVacuumMiHomeCloudProtocol` (proto:1657-2683). It is the same algorithm as python-miio's
cloud module. The L40 Ultra AE is probably not bindable to Mi Home: `dreame.vacuum.r2579a` is
absent from https://home.miot-spec.com (checked 2026-11-15).

### 16.1 Login

Cookies preset on `mi.com` and `xiaomi.com`: `sdkVersion=3.8.6`, `deviceId=<16 random a-z>`, and
`passToken`, `userId` when known (proto:1993-1999).
User-Agent: `Android-7.1.1-1.0.0-ONEPLUS A3010-136-<deviceId> APP/xiaomi.smarthome APPV/62830`
(proto:1691).

1. `GET https://account.xiaomi.com/pass/serviceLogin?sid=xiaomiio&_json=true`. Strip the
   `&&&START&&&` prefix and parse JSON. Keep `_sign`. If `code == 0`, the session is still valid:
   take `userId`, `ssecurity`, `location` (proto:1861-1887).
2. `POST https://account.xiaomi.com/pass/serviceLoginAuth2?_json=true`, form fields:
   `user=<email>`, `hash=<MD5(password), uppercase hex>`, `callback=https://sts.api.io.mi.com/sts`,
   `sid=xiaomiio`, `qs=%3Fsid%3Dxiaomiio%26_json%3Dtrue`, `_sign=<from step 1>`
   (proto:1889-1923). Success gives `location`, `userId`, `ssecurity`, `passToken`.
3. `GET <location>`. Keep cookie `serviceToken` from `sts.api.io.mi.com` (proto:1958-1985).

Challenges:

- Captcha: the response has `captchaUrl`. GET it, keep cookie `ick`, show the image, and resend
  with form field `captCode`, query `_dc=<ms>` and cookie `ick` (proto:1908-1952).
- 2FA: the response has `notificationUrl`. Flow: `identity/list` → choose phone (flag 4) or email
  (flag 8) → `identity/auth/verify{Phone|Email}` → `send{Phone|Email}Ticket` → user enters code →
  `verify{Phone|Email}` with `ticket` → follow `location` → read `ssecurity` from the
  `extension-pragma` header → STS for `serviceToken` (proto:2016-2335). Error 70022 or text
  "limit"/"frequent" means a send quota or cooldown.

Session check: `POST /v2/message/v2/check_new_msg` (proto:1792-1859).

### 16.2 Signed and encrypted RPC

Base URL: `https://<cc>.api.io.mi.com/app`, or `https://api.io.mi.com/app` for `cn`
(proto:2602-2603). Countries: `de`, `cn`, `us`, `ru`, `tw`, `sg`, `in`, `i2`
(`config_flow.py:800-801`).

Headers: the UA above, `Accept-Encoding: identity`, `x-xiaomi-protocal-flag-cli: PROTOCAL-HTTP2`,
`content-type: application/x-www-form-urlencoded`, `MIOT-ENCRYPT-ALGORITHM: ENCRYPT-RC4`.
Cookies: `userId`, `serviceToken`, `yetAnotherServiceToken` (same as `serviceToken`), `locale`,
`timezone` (`GMT+hh:mm`), `is_daylight`, `dst_offset`, `channel=MI_APP_STORE` (proto:2554-2570).

Algorithm (proto:2605-2683):

```
nonce        = b64( 8 random bytes || big-endian minimal bytes of floor(ms / 60000) )
signed_nonce = b64( sha256( b64d(ssecurity) || b64d(nonce) ) )
params       = {"data": compact_json(payload)}
params["rc4_hash__"] = b64( sha1( "POST&<path>&k1=v1&...&<signed_nonce>" ) )
for each k in params:
    params[k] = b64( RC4(key = b64d(signed_nonce), drop first 1024 bytes).encrypt(params[k]) )
params["signature"] = b64( sha1( "POST&<path>&k1=enc_v1&...&<signed_nonce>" ) )
params["ssecurity"] = ssecurity
params["_nonce"]    = nonce
response     = b64 RC4 ciphertext; decrypt with a fresh RC4(signed_nonce), drop 1024
```

`<path>` is the URL path after the host, with `/app/` replaced by `/`. Each field uses a fresh RC4
state (proto:2662-2665).

### 16.3 Endpoints (path after `/app/`)

| Path | Payload | Purpose | Source |
|---|---|---|---|
| `v2/homeroom/gethome` | `{"fg":true,"fetch_share":true,"fetch_share_dev":true,"limit":100,"app_ver":7}` | homes | proto:2461-2474 |
| `v2/user/get_device_cnt` | `{"fetch_own":true,"fetch_share":true}` | shared homes | proto:2476-2491 |
| `v2/home/home_device_list` | `{"home_id":N,"home_owner":<uid>,"limit":100,"get_split_device":true,"support_smart_home":true}` | devices per home | proto:2494-2512 |
| `home/device_list` | `{"getVirtualModel":false,"getHuamiDevices":0}` | legacy device list | proto:2514-2534 |
| `v2/home/rpc/<did>` | `{"method":...,"params":...}` | relay an MIoT call | proto:2379-2395 |
| `home/getfileurl[_v3]` | `{"obj_name":...}` | map file URL (`_v3` for `xiaomi.vacuum.*`) | proto:2352-2362 |
| `v2/home/get_interim_file_url[_pro]` | `{"obj_name":...}` | interim map URL | proto:2364-2377 |
| `user/get_user_device_data` | `{"uid","did","time_end","time_start","limit","key","type"}` | history | proto:2403-2419 |
| `device/batchdevicedatas` | `[{"did","props":[...]}]` | user data read | proto:2538-2542 |
| `v2/device/batch_set_props` | `[{"did","props":{...}}]` | user data write | proto:2544-2548 |

Device records include `did`, `model`, `mac`, `localip` and the 32-hex-char miIO `token`
(proto:2421-2430). That token enables local miIO (§17). Mi map object names are
`<uid>/<did>/0` (proto:1788-1790).

---

## 17. Local miIO

### 17.1 Verified: this robot has no local control surface

Verified on r2579a, 2026-11-15 (prototype LAN probe):

- The robot answers ICMP ping on the LAN.
- It does **not** answer the miIO hello on UDP 54321, directed or broadcast.
- TCP connects to ports 22, 23, 80, 443, 554, 1883, 6668, 8000, 8080, 8443, 8883, 8888, 9999,
  10000, 54321 and 55555 all time out. Packets are dropped silently, not refused.
- Conclusion: the L40 Ultra AE on Dreamehome is cloud-only. This matches tasshack, which never
  creates a local transport for Dreamehome accounts (proto:2721-2738), and the APK, which has no
  miIO path (APK notes §4).

Keep miIO in the library only for Mi-bound Dreame models that have a token from the Mi cloud.

### 17.2 Packet format

Source: python-miio `miio/protocol.py`, `miio/miioprotocol.py`. Transport: UDP, port 54321.

```
offset size  field
0      2     magic        0x2131 (big-endian)
2      2     length       header + payload, big-endian
4      4     unknown      0x00000000; 0xFFFFFFFF in hello
8      4     device id    big-endian
12     4     stamp        big-endian, device time counter
16     16    checksum     md5(header[0..16] || token || encrypted_payload)
32     n     payload      AES-128-CBC(PKCS7(json + "\0"))
```

Crypto:

```
key = md5(token)
iv  = md5(key || token)
```

`token` is the 16 raw bytes of the 32-hex-char token.

Hello: `21 31 00 20` followed by 28 bytes of `0xFF` (32 bytes total). The reply carries the device
id and stamp. Old firmware put the token in the checksum field; current firmware sends zeros or
`0xFF`.

Payload JSON: `{"id":N,"method":"...","params":[...]}`. Outgoing stamp = last device stamp + elapsed
seconds. python-miio wraps ids at 9999, adds 100 to the id after a socket error and handshakes
again, and retries on error codes -30001 and -9999. MIoT methods are the same as §8.3, with
`did` = `"<siid>.<piid>"` (proto:2822-2900). tasshack probes with `miIO.info` (proto:2740-2744).

### 17.3 Other local routes

- The robot is an MQTT **client** of the Dreame broker. No local broker is documented.
- Rooted firmware (Valetudo) is the only known fully local route. Valetudo does not list the
  L40 Ultra AE (https://valetudo.cloud/pages/general/supported-robots, read 2026-11-15).

---

## 18. Open questions and risks

1. **MQTT on other brokers.** Connection, private root, client id and topic are verified on
   r2579a on the `us` broker (§11). Other regions are untested. The APK's bundled mTLS client
   certificate (§4.3) may be needed for some brokers.
2. **TLS fingerprinting.** tasshack hand-rolls TLS to mimic Android. Verified that `rustls` works
   in `us` today. Other regions or later dates may block it. Keep the TLS connector pluggable.
3. **Request signing enforcement.** The server accepted an unsigned device-list request on
   2026-11-17 ([l40-ultra-ae.md](l40-ultra-ae.md)). Other endpoints may still check `sign`, and
   the app always sends it. Always send it.
4. **App constant rotation.** Salt, client secret, cid and `cv` come from the app binary and change
   with app releases. Keep them in a config struct with the values in §2.3 as defaults.
5. **`dreame-meta` fingerprint fields.** Server-side use of `canvasHash`, `webglHash`,
   `visitorIdHash` is unknown. Keep them stable per installation.
6. **Writes.** `set_properties` and `action` work on r2579a since 2026-11-16 (§8.3). Settings,
   actions and map edits not yet written are listed in [l40-ultra-ae.md](l40-ultra-ae.md)
   ("Not yet tested").
7. **80001 ambiguity.** A command can succeed while the HTTP answer says it timed out. Confirm
   through MQTT or a later read.
8. **Refresh token lifetime.** The refresh body works, refresh tokens rotate, and an old refresh
   token stays valid after a refresh (see the refresh section above). The lifetime and the behaviour after expiry are
   untested on this account.
9. **CONNECT flag patch.** The `us` broker accepts a standard CONNECT without the Will-QoS bit
   tasshack sets (§11). Other brokers are untested.
10. **Captcha, 2FA, social login.** No Dreamehome client handles them. Accounts created with
    Google, Apple or phone sign-in may have no usable password.
11. **Region discovery.** Trial login across regions works but is slow and may look like abuse.
    The APK region endpoints (§1.3) are unexplored.
12. **Rate limits.** Unknown thresholds. Design for 429 with backoff.
13. **Aliyun RPC path.** The app may prefer Aliyun `Service.smarthome.*` RPC for some features.
    Its envelope is unknown.
14. **Server messages in Chinese.** Map by `code`; never rely on `msg` text.
15. **Terms of service.** Third-party use of app constants is unofficial and may be blocked at
    any time.

---

## 19. Implementation checklist (transport layer)

1. `BrandConfig` struct with every constant from §2.3, defaulting to Dreame. No constants in code
   paths.
2. HTTP client: `reqwest` + `rustls`, public roots, gzip, 20 s timeout, behind a trait so the TLS
   stack can be swapped.
3. Header builder for dialect A (§2.1), with dialect B (§2.2) selectable. `dreame-rlc` added once
   region, lang and country are known.
4. `spliced` and `sign_body` (§3), now in `crates/dreame-cloud/src/sign.rs`, with the worked-example test.
5. Auth: password login with salted MD5 (§5.1), region trial and adoption of the returned region
   (§1.3), refresh before expiry and on 401, fallback to password login. Persist the refresh token,
   region, country, lang, tenant and uid. Never log tokens or passwords.
6. Response handling: parse `code`; typed errors for OAuth failure, 401, 429/5xx (retry with
   backoff), `80001` (unknown outcome), and other codes.
7. Device API: `listV2` with the three response shapes, `device/info`, `devOTCInfo`. Parse
   `property` as a nested JSON string. Derive the `sendCommand` prefix from `bindDomain`.
8. `sendCommand`: id counter, envelope (§8.2), `get_properties` batching (15), `set_properties`,
   `action` with a single object. Read `data.result` or top-level `result`.
9. Cached reads: `iotstatus/props` with a comma-separated `keys` string and string values (§9.1);
   `history`; `getDeviceData` / `setDeviceData`.
10. OSS: `getDownloadUrl` with the country code as `region`, `getOss1dDownloadUrl`, plain GET with
    `okhttp/4.12.0`, URL cache of 30 min, base64url + zlib decode, optional AES.
11. MQTT: TLS with the private root only, client id, uid and token credentials, topic
    `/status/<did>/<uid>/<model>/<region>/`, reconnect on token refresh and on CONNACK 5, envelope
    parser for the methods in §11.4.
12. Secondary transports behind the same trait: Mi cloud RPC (§16) and local miIO (§17). Both are
    optional features; neither applies to the r2579a.
