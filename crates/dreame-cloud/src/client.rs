//! The Dreamehome cloud client.

use crate::auth::{
    login_form, meta_header, needs_refresh, parse_token_response, refresh_form, StoredSession, TokenResponse,
    TokenStore, TOKEN_PATH,
};
use crate::http::{HttpRequest, HttpResponse, HttpTransport, Method, ReqwestTransport};
use crate::region::{Brand, Region};
use crate::sign::Signer;
use dreame_core::{ActionId, AppConstants, DeviceInfo, Error, PropId, PropResult, RawValue, Result};
use serde_json::{json, Value};
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Maximum properties per relayed `get_properties` or `set_properties` call.
pub const RELAY_BATCH: usize = 15;
/// Maximum keys per `iotstatus/props` call (60 verified on r2579a).
pub const CACHE_BATCH: usize = 60;

const LIST_PATH: &str = "/dreame-user-iot/iotuserbind/device/listV2";
const INFO_PATH: &str = "/dreame-user-iot/iotuserbind/device/info";
const OTC_PATH: &str = "/dreame-user-iot/iotstatus/devOTCInfo";
const PROPS_PATH: &str = "/dreame-user-iot/iotstatus/props";
const HISTORY_PATH: &str = "/dreame-user-iot/iotstatus/history";
const DOWNLOAD_PATH: &str = "/dreame-user-iot/iotfile/getDownloadUrl";
const OSS1D_PATH: &str = "/dreame-user-iot/iotfile/getOss1dDownloadUrl";
const OSS_USER_AGENT: &str = "okhttp/4.12.0";
/// Retries after the first attempt for transient HTTP statuses.
const MAX_TRANSIENT_RETRIES: u32 = 3;

/// Options for building a [`CloudClient`].
#[derive(Clone)]
pub struct CloudConfig {
    pub brand: Brand,
    /// App constants. Defaults to [`Brand::constants`] of `brand`.
    pub constants: AppConstants,
    /// HTTP transport. Defaults to [`ReqwestTransport`] with `timeout`.
    pub transport: Option<Arc<dyn HttpTransport>>,
    /// Receives the session after every login and refresh.
    pub token_store: Option<Arc<dyn TokenStore>>,
    pub timeout: Duration,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self::for_brand(Brand::Dreame)
    }
}

impl CloudConfig {
    pub fn for_brand(brand: Brand) -> Self {
        Self { brand, constants: brand.constants(), transport: None, token_store: None, timeout: Duration::from_secs(30) }
    }

    pub fn with_transport(mut self, t: Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(t);
        self
    }

    pub fn with_token_store(mut self, s: Arc<dyn TokenStore>) -> Self {
        self.token_store = Some(s);
        self
    }
}

impl fmt::Debug for CloudConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudConfig")
            .field("brand", &self.brand)
            .field("custom_transport", &self.transport.is_some())
            .field("token_store", &self.token_store.is_some())
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

/// One value from the cloud property cache (`iotstatus/props`).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedProp {
    pub id: PropId,
    pub value: RawValue,
    /// When the cloud last stored the value, in Unix milliseconds.
    pub updated_ms: Option<i64>,
}

/// The robot's own `_otc.info` report from `iotstatus/devOTCInfo`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OtcInfo {
    pub fw_ver: Option<String>,
    pub hw_ver: Option<String>,
    pub model: Option<String>,
    /// The robot's LAN address (`netif.localIp`).
    pub local_ip: Option<String>,
    /// The complete `params` object.
    pub raw: Value,
}

struct AuthState {
    region: Region,
    access_token: String,
    refresh_token: String,
    expires_at: Instant,
    uid: String,
    tenant_id: String,
    country: Option<String>,
    lang: Option<String>,
}

/// An authenticated Dreamehome cloud session.
///
/// All methods take `&self`, so one client can be shared between tasks
/// (wrap it in an `Arc`). The access token is refreshed automatically when
/// less than 600 s remain, and once after an HTTP 401.
pub struct CloudClient {
    brand: Brand,
    constants: AppConstants,
    transport: Arc<dyn HttpTransport>,
    store: Option<Arc<dyn TokenStore>>,
    signer: Signer,
    meta: String,
    state: RwLock<AuthState>,
    refresh_lock: tokio::sync::Mutex<()>,
    next_id: AtomicU32,
}

impl fmt::Debug for CloudClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.read_state();
        f.debug_struct("CloudClient")
            .field("brand", &self.brand)
            .field("region", &s.region)
            .field("country", &s.country)
            .field("expires_in", &s.expires_at.saturating_duration_since(Instant::now()))
            .field("tokens", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Context needed to call the token endpoint before a client exists.
struct TokenCtx<'a> {
    transport: &'a dyn HttpTransport,
    constants: &'a AppConstants,
    meta: &'a str,
}

impl TokenCtx<'_> {
    async fn request(&self, base: &str, tenant_id: &str, form: String) -> Result<TokenResponse> {
        let req = HttpRequest {
            method: Method::Post,
            url: format!("{base}{TOKEN_PATH}"),
            headers: vec![
                ("user-agent".into(), self.constants.user_agent.clone()),
                ("dreame-meta".into(), self.meta.to_string()),
                ("tenant-id".into(), tenant_id.to_string()),
                ("authorization".into(), self.constants.basic_auth.clone()),
                ("content-type".into(), "application/x-www-form-urlencoded".into()),
            ],
            body: form.into_bytes(),
        };
        let resp = self.transport.send(req).await?;
        parse_token_response(resp.status, &resp.body)
    }
}

fn build_transport(cfg: &CloudConfig) -> Result<Arc<dyn HttpTransport>> {
    match &cfg.transport {
        Some(t) => Ok(t.clone()),
        None => Ok(Arc::new(ReqwestTransport::new(cfg.timeout)?)),
    }
}

/// The region to use after a token response: the token's region, unless the
/// caller gave an explicit host.
fn effective_region(asked: &Region, from_token: Option<&str>) -> Region {
    match (asked, from_token) {
        (Region::Custom(_), _) | (_, None) => asked.clone(),
        (_, Some(code)) => Region::parse(code),
    }
}

impl CloudClient {
    // ---- construction --------------------------------------------------

    /// Logs in with a password in one region, using the default config.
    /// The username is the account's email address.
    pub async fn login(region: Region, username: &str, password: &str) -> Result<Self> {
        Self::login_with(CloudConfig::default(), region, username, password).await
    }

    /// Logs in with a password in one region.
    pub async fn login_with(cfg: CloudConfig, region: Region, username: &str, password: &str) -> Result<Self> {
        Self::login_any_with(cfg, &[region], username, password).await
    }

    /// Tries a password login in each region in order and keeps the first
    /// that works. An empty list means [`Region::default_order`]. After login
    /// the client uses the region named in the token response.
    pub async fn login_any(regions: &[Region], username: &str, password: &str) -> Result<Self> {
        Self::login_any_with(CloudConfig::default(), regions, username, password).await
    }

    /// Like [`Self::login_any`], with a config.
    pub async fn login_any_with(cfg: CloudConfig, regions: &[Region], username: &str, password: &str) -> Result<Self> {
        let regions = if regions.is_empty() { Region::default_order() } else { regions.to_vec() };
        let transport = build_transport(&cfg)?;
        let meta = meta_header(&cfg.constants, cfg.brand != Brand::Trouver, username);
        let ctx = TokenCtx { transport: transport.as_ref(), constants: &cfg.constants, meta: &meta };
        let mut failures = Vec::new();
        for region in &regions {
            let base = region.base_url(cfg.brand);
            let form = login_form(&cfg.constants, username, password, None, None);
            match ctx.request(&base, &cfg.constants.tenant_id, form).await {
                Ok(tr) => {
                    let actual = effective_region(region, tr.region.as_deref());
                    info!(tried = %region, region = %actual, "login succeeded");
                    let refresh = tr.refresh_token.clone().ok_or_else(|| Error::Auth("login reply has no refresh token".into()))?;
                    return Ok(Self::from_parts(cfg, transport, meta, actual, tr, refresh, String::new()));
                }
                Err(e) => {
                    warn!(region = %region, error = %e, "login failed");
                    failures.push(format!("{region}: {e}"));
                }
            }
        }
        Err(Error::Auth(format!("login failed in every region tried ({})", failures.join("; "))))
    }

    /// Resumes a session from a stored refresh token. This performs a refresh
    /// at once, which rotates the refresh token; the new one goes to the
    /// config's token store and is available from [`Self::stored_session`].
    pub async fn from_refresh_token(cfg: CloudConfig, stored: StoredSession) -> Result<Self> {
        let transport = build_transport(&cfg)?;
        // Without the username, the meta hashes are derived from the uid.
        let meta = meta_header(&cfg.constants, cfg.brand != Brand::Trouver, &stored.uid);
        let ctx = TokenCtx { transport: transport.as_ref(), constants: &cfg.constants, meta: &meta };
        let base = stored.region.base_url(cfg.brand);
        let tenant = if stored.tenant_id.is_empty() { cfg.constants.tenant_id.clone() } else { stored.tenant_id.clone() };
        let tr = ctx.request(&base, &tenant, refresh_form(&cfg.constants, &stored.refresh_token)).await?;
        let region = effective_region(&stored.region, tr.region.as_deref());
        let refresh = tr.refresh_token.clone().unwrap_or(stored.refresh_token);
        let mut tr = tr;
        if tr.tenant_id.is_none() {
            tr.tenant_id = Some(tenant);
        }
        Ok(Self::from_parts(cfg, transport, meta, region, tr, refresh, stored.uid))
    }

    /// Resumes the session held by the config's token store.
    pub async fn from_store(cfg: CloudConfig) -> Result<Self> {
        let stored = cfg
            .token_store
            .as_ref()
            .and_then(|s| s.load())
            .ok_or_else(|| Error::Auth("no stored session".into()))?;
        Self::from_refresh_token(cfg, stored).await
    }

    fn from_parts(
        cfg: CloudConfig,
        transport: Arc<dyn HttpTransport>,
        meta: String,
        region: Region,
        tr: TokenResponse,
        refresh_token: String,
        fallback_uid: String,
    ) -> Self {
        let state = AuthState {
            region,
            access_token: tr.access_token,
            refresh_token,
            expires_at: Instant::now() + tr.expires_in,
            uid: tr.uid.unwrap_or(fallback_uid),
            tenant_id: tr.tenant_id.unwrap_or_else(|| cfg.constants.tenant_id.clone()),
            country: tr.country,
            lang: tr.lang,
        };
        let client = Self {
            brand: cfg.brand,
            signer: Signer::new(cfg.constants.sign_secret.clone()),
            constants: cfg.constants,
            transport,
            store: cfg.token_store,
            meta,
            state: RwLock::new(state),
            refresh_lock: tokio::sync::Mutex::new(()),
            next_id: AtomicU32::new(rand::random::<u32>() % 100 + 1),
        };
        client.save_session();
        client
    }

    // ---- accessors -----------------------------------------------------

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, AuthState> {
        self.state.read().unwrap_or_else(|p| p.into_inner())
    }

    pub fn brand(&self) -> Brand {
        self.brand
    }

    /// Account id. It is also the MQTT username.
    pub fn uid(&self) -> String {
        self.read_state().uid.clone()
    }

    /// The account's region, as reported by the server.
    pub fn region(&self) -> Region {
        self.read_state().region.clone()
    }

    /// The account's country code, for example `DE`. `download_url` sends it
    /// as `region`.
    pub fn country(&self) -> Option<String> {
        self.read_state().country.clone()
    }

    pub fn lang(&self) -> Option<String> {
        self.read_state().lang.clone()
    }

    pub fn tenant_id(&self) -> String {
        self.read_state().tenant_id.clone()
    }

    /// The current access token. It is also the MQTT password. Never log it.
    pub fn access_token(&self) -> String {
        self.read_state().access_token.clone()
    }

    /// Time left before the access token expires.
    pub fn expires_in(&self) -> Duration {
        self.read_state().expires_at.saturating_duration_since(Instant::now())
    }

    /// What to persist to resume this session later.
    pub fn stored_session(&self) -> StoredSession {
        let s = self.read_state();
        StoredSession {
            refresh_token: s.refresh_token.clone(),
            region: s.region.clone(),
            uid: s.uid.clone(),
            tenant_id: s.tenant_id.clone(),
        }
    }

    fn base_url(&self) -> String {
        self.read_state().region.base_url(self.brand)
    }

    fn save_session(&self) {
        if let Some(store) = &self.store {
            store.save(&self.stored_session());
        }
    }

    // ---- token refresh -------------------------------------------------

    /// Refreshes the access token now. Both tokens rotate; the new refresh
    /// token is saved to the token store. If this fails with a message about
    /// the refresh token, the caller must log in with the password again.
    pub async fn refresh(&self) -> Result<()> {
        self.refresh_inner(None).await
    }

    /// Refreshes, unless `seen` is given and another task already replaced
    /// that access token.
    async fn refresh_inner(&self, seen: Option<String>) -> Result<()> {
        let _guard = self.refresh_lock.lock().await;
        let (base, refresh_token, tenant, current) = {
            let s = self.read_state();
            (s.region.base_url(self.brand), s.refresh_token.clone(), s.tenant_id.clone(), s.access_token.clone())
        };
        if seen.is_some_and(|t| t != current) {
            return Ok(());
        }
        let ctx = TokenCtx { transport: self.transport.as_ref(), constants: &self.constants, meta: &self.meta };
        let tr = ctx.request(&base, &tenant, refresh_form(&self.constants, &refresh_token)).await?;
        {
            let mut s = self.state.write().unwrap_or_else(|p| p.into_inner());
            s.region = effective_region(&s.region, tr.region.as_deref());
            s.access_token = tr.access_token;
            if let Some(r) = tr.refresh_token {
                s.refresh_token = r;
            }
            s.expires_at = Instant::now() + tr.expires_in;
            if let Some(u) = tr.uid {
                s.uid = u;
            }
            if let Some(t) = tr.tenant_id {
                s.tenant_id = t;
            }
            if tr.country.is_some() {
                s.country = tr.country;
            }
            if tr.lang.is_some() {
                s.lang = tr.lang;
            }
        }
        info!("access token refreshed");
        self.save_session();
        Ok(())
    }

    async fn ensure_fresh(&self) -> Result<()> {
        let (expires_at, token) = {
            let s = self.read_state();
            (s.expires_at, s.access_token.clone())
        };
        if needs_refresh(expires_at, Instant::now()) {
            debug!("access token close to expiry; refreshing");
            self.refresh_inner(Some(token)).await?;
        }
        Ok(())
    }

    // ---- generic request -----------------------------------------------

    fn json_request(&self, path: &str, body: &Value, sign: bool) -> Result<(HttpRequest, String)> {
        let mut body = body.clone();
        if let (true, Value::Object(m)) = (sign, &mut body) {
            self.signer.sign(m);
        }
        let token = self.access_token();
        let mut headers = vec![
            ("user-agent".to_string(), self.constants.user_agent.clone()),
            ("dreame-meta".to_string(), self.meta.clone()),
            ("tenant-id".to_string(), self.tenant_id()),
            ("authorization".to_string(), self.constants.basic_auth.clone()),
            ("content-type".to_string(), "application/json".to_string()),
            ("dreame-auth".to_string(), token.clone()),
        ];
        if self.brand == Brand::Mova {
            headers.push(("dreame-psd".into(), "new".into()));
        }
        let bytes = serde_json::to_vec(&body).map_err(|e| Error::Decode(e.to_string()))?;
        Ok((HttpRequest { method: Method::Post, url: format!("{}{path}", self.base_url()), headers, body: bytes }, token))
    }

    /// Signed JSON POST with auto refresh, one retry after HTTP 401, and
    /// backoff on transient statuses. `idempotent` allows retries on 502,
    /// 503 and 504 as well as 429, and on transport errors such as a pooled
    /// connection the server closed while idle. A write is not retried on a
    /// transport error, since the request may have arrived. Returns the
    /// whole reply once `code` is 0.
    pub async fn post_api(&self, path: &str, body: Value, idempotent: bool) -> Result<Value> {
        self.ensure_fresh().await?;
        let mut refreshed = false;
        let mut retries = 0;
        loop {
            let (req, token) = self.json_request(path, &body, true)?;
            debug!(path, "POST");
            let resp = match self.transport.send(req).await {
                Ok(r) => r,
                Err(err) if idempotent && retries < MAX_TRANSIENT_RETRIES => {
                    retries += 1;
                    let delay = backoff(retries);
                    warn!(path, error = %err, ?delay, "transport error; retrying");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(err) => return Err(err),
            };
            if resp.status == 401 && !refreshed {
                debug!(path, "HTTP 401; refreshing token and retrying once");
                refreshed = true;
                self.refresh_inner(Some(token)).await?;
                continue;
            }
            let transient = resp.status == 429 || (idempotent && matches!(resp.status, 502..=504));
            if transient && retries < MAX_TRANSIENT_RETRIES {
                retries += 1;
                let delay = backoff(retries);
                warn!(path, status = resp.status, ?delay, "transient HTTP status; retrying");
                tokio::time::sleep(delay).await;
                continue;
            }
            return parse_api(path, &resp);
        }
    }

    /// Protocol probe: one JSON POST without the `sign` and `timestamp`
    /// fields, with no refresh or retry. It shows whether the server still
    /// accepts unsigned bodies (l40-ultra-ae.md). Send only read requests.
    #[doc(hidden)]
    pub async fn post_api_unsigned(&self, path: &str, body: Value) -> Result<Value> {
        let (req, _) = self.json_request(path, &body, false)?;
        let resp = self.transport.send(req).await?;
        parse_api(path, &resp)
    }

    // ---- devices -------------------------------------------------------

    /// Lists every device bound to the account (vacuums and others; see
    /// [`DeviceInfo::is_vacuum`]).
    pub async fn list_devices(&self) -> Result<Vec<DeviceInfo>> {
        let v = self.post_api(LIST_PATH, json!({"sharedStatus": 1, "current": 1, "size": 100, "lang": "en"}), true).await?;
        parse_device_list(&v)
    }

    /// Reads one device record (`iotuserbind/device/info`).
    pub async fn device_info(&self, did: &str) -> Result<DeviceInfo> {
        let v = self.post_api(INFO_PATH, json!({"did": did}), true).await?;
        let data = v.get("data").filter(|d| d.is_object()).ok_or_else(|| Error::Decode("device info reply has no data".into()))?;
        Ok(parse_device(data))
    }

    /// Reads the robot's own `_otc.info` report (firmware, LAN address, Wi-Fi).
    pub async fn otc_info(&self, did: &str) -> Result<OtcInfo> {
        let v = self.post_api(OTC_PATH, json!({"did": did}), true).await?;
        parse_otc_info(&v)
    }

    // ---- properties and commands ---------------------------------------

    /// Relays an MIoT call to the robot through `sendCommand` and returns the
    /// whole reply. Prefer the typed methods below.
    pub async fn send_command(&self, dev: &DeviceInfo, method: &str, params: Value, idempotent: bool) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let did = dev.did.as_str();
        let body = json!({"did": did, "id": id, "data": {"did": did, "id": id, "method": method, "params": params}});
        self.post_api(&command_path(dev.bind_domain.as_deref()), body, idempotent).await
    }

    /// Like [`Self::send_command`], but retries once when the reply reports
    /// success without a `result`, and returns the `result`.
    async fn send_command_result(&self, dev: &DeviceInfo, method: &str, params: Value) -> Result<Value> {
        for attempt in 0..2 {
            let v = self.send_command(dev, method, params.clone(), true).await?;
            if let Some(r) = command_result(&v) {
                return Ok(r.clone());
            }
            debug!(method, attempt, "sendCommand succeeded without result");
        }
        Err(Error::Decode(format!("sendCommand {method} returned no result")))
    }

    /// Reads properties from the robot itself (relayed through the cloud),
    /// in batches of 15. Values come back typed.
    pub async fn get_properties(&self, dev: &DeviceInfo, ids: &[PropId]) -> Result<Vec<PropResult>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(RELAY_BATCH) {
            let params: Vec<Value> =
                chunk.iter().map(|p| json!({"did": dev.did, "siid": p.siid, "piid": p.piid})).collect();
            let r = self.send_command_result(dev, "get_properties", Value::Array(params)).await?;
            out.extend(parse_prop_results(chunk, &r)?);
        }
        Ok(out)
    }

    /// Writes properties on the robot, in batches of 15.
    ///
    /// This is a write: it changes the robot's settings. It is not verified
    /// on r2579a yet. A [`Error::NoAck`] means the outcome is unknown.
    pub async fn set_properties(&self, dev: &DeviceInfo, values: &[(PropId, RawValue)]) -> Result<Vec<PropResult>> {
        let mut out = Vec::with_capacity(values.len());
        for chunk in values.chunks(RELAY_BATCH) {
            let mut params = Vec::with_capacity(chunk.len());
            for (p, v) in chunk {
                let value = serde_json::to_value(v).map_err(|e| Error::Decode(e.to_string()))?;
                params.push(json!({"did": dev.did, "siid": p.siid, "piid": p.piid, "value": value}));
            }
            // Setting the same values twice is harmless, so the retry on a
            // missing result applies here too.
            let r = self.send_command_result(dev, "set_properties", Value::Array(params)).await?;
            let ids: Vec<PropId> = chunk.iter().map(|(p, _)| *p).collect();
            out.extend(parse_prop_results(&ids, &r)?);
        }
        Ok(out)
    }

    /// Runs an MIoT action on the robot. `params` is sent as a single object
    /// `{did, siid, aiid, in}`; an array yields a misleading 80001.
    ///
    /// This is a write: it makes the robot do something. It is never retried
    /// automatically except after HTTP 401 or 429, which the server did not
    /// process. Returns the `result` value, or `Null` when there is none.
    /// A [`Error::NoAck`] means the outcome is unknown; the robot often runs
    /// the action anyway.
    pub async fn action(&self, dev: &DeviceInfo, id: ActionId, input: Vec<Value>) -> Result<Value> {
        let params = json!({"did": dev.did, "siid": id.siid, "aiid": id.aiid, "in": input});
        let v = self.send_command(dev, "action", params, false).await?;
        Ok(command_result(&v).cloned().unwrap_or(Value::Null))
    }

    /// Reads the cloud's cached property values (`iotstatus/props`), in
    /// batches of 60 sent at the same time. This does not reach the robot.
    /// Keys without a cached value are absent from the result.
    pub async fn cached_properties(&self, did: &str, ids: &[PropId]) -> Result<Vec<CachedProp>> {
        let calls = ids.chunks(CACHE_BATCH).map(|chunk| async move {
            // One comma-separated string; a JSON array returns `data: []`.
            let keys = chunk.iter().map(PropId::to_string).collect::<Vec<_>>().join(",");
            let v = self.post_api(PROPS_PATH, json!({"did": did, "keys": keys}), true).await?;
            parse_cached_props(&v)
        });
        let batches = futures_util::future::try_join_all(calls).await?;
        Ok(batches.into_iter().flatten().collect())
    }

    /// Reads the cloud's stored history of one event (`iotstatus/history`), such as the
    /// cleaning record 4.e1 a robot sends at the end of each job. `from` is in Unix seconds;
    /// at most `limit` entries come back. Read-only. Returns the entries as the cloud sends them.
    pub async fn event_history(&self, did: &str, siid: u32, eiid: u32, from: i64, limit: u32) -> Result<Vec<Value>> {
        Ok(history_entries(&self.history_reply(did, "eiid", siid, eiid, from, limit).await?))
    }

    /// The whole reply of the history API for a property (`kind` `"piid"`), an event
    /// (`"eiid"`) or an action (`"aiid"`), for looking at its layout. Read-only.
    pub async fn history_reply(&self, did: &str, kind: &str, siid: u32, iid: u32, from: i64, limit: u32) -> Result<Value> {
        let mut body = json!({
            "uid": self.uid(),
            "did": did,
            "from": from,
            "limit": limit,
            "siid": siid,
            // The server region code ("us"), not the account's country as the file calls use.
            "region": self.region().code().unwrap_or("us"),
            "type": 3,
        });
        body[kind] = json!(iid);
        self.post_api(HISTORY_PATH, body, true).await
    }

    // ---- files ---------------------------------------------------------

    fn country_or_region(&self) -> String {
        let s = self.read_state();
        s.country.clone().unwrap_or_else(|| s.region.code().unwrap_or("us").to_ascii_uppercase())
    }

    /// Resolves an OSS object name (for example property 6.3) to a signed
    /// URL. The URL is valid for about an hour.
    pub async fn download_url(&self, did: &str, model: &str, object: &str) -> Result<String> {
        let body = json!({"did": did, "model": model, "filename": object, "region": self.country_or_region()});
        let v = self.post_api(DOWNLOAD_PATH, body, true).await?;
        parse_url(&v)
    }

    /// Resolves a permanent (saved-map) object through
    /// `getOss1dDownloadUrl`. The first character of the object name is
    /// dropped, as tasshack does. Not yet verified on r2579a.
    pub async fn permanent_download_url(&self, did: &str, model: &str, object: &str) -> Result<String> {
        let filename: String = object.chars().skip(1).collect();
        let body = json!({"did": did, "uid": self.uid(), "model": model, "filename": filename,
            "region": self.country_or_region()});
        let v = self.post_api(OSS1D_PATH, body, true).await?;
        parse_url(&v)
    }

    /// Downloads a signed URL without Dreame headers. Retries up to three
    /// times on transport errors and transient statuses.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let req = HttpRequest {
                method: Method::Get,
                url: url.to_string(),
                headers: vec![("user-agent".into(), OSS_USER_AGENT.into())],
                body: Vec::new(),
            };
            let res = self.transport.send(req).await;
            let err = match res {
                Ok(r) if r.is_success() => return Ok(r.body),
                Ok(r) if r.status == 429 || r.status >= 500 => Error::Http(format!("download HTTP {}", r.status)),
                Ok(r) => return Err(Error::Http(format!("download HTTP {}", r.status))),
                Err(e) => e,
            };
            if attempt > MAX_TRANSIENT_RETRIES {
                return Err(err);
            }
            warn!(attempt, error = %err, "download failed; retrying");
            tokio::time::sleep(backoff(attempt)).await;
        }
    }

    /// Downloads a signed URL as UTF-8 text (map files are base64 text).
    pub async fn fetch_text(&self, url: &str) -> Result<String> {
        String::from_utf8(self.fetch_bytes(url).await?).map_err(|_| Error::Decode("download is not UTF-8".into()))
    }
}

/// Backoff for retry `n` (1-based): `min(0.5 * 2^(n-1), 2)` seconds.
fn backoff(n: u32) -> Duration {
    let ms = 500u64.saturating_mul(1 << n.saturating_sub(1).min(4));
    Duration::from_millis(ms.min(2000))
}

/// `/dreame-iot-com-<prefix>/device/sendCommand`, where the prefix is the
/// first DNS label of `bindDomain`.
pub fn command_path(bind_domain: Option<&str>) -> String {
    match bind_domain.and_then(|d| d.split('.').next()).filter(|p| !p.is_empty()) {
        Some(p) => format!("/dreame-iot-com-{p}/device/sendCommand"),
        None => "/dreame-iot-com/device/sendCommand".into(),
    }
}

/// Checks the generic reply envelope `{code, success, msg, data}`.
fn parse_api(path: &str, resp: &HttpResponse) -> Result<Value> {
    let v: Value = match serde_json::from_slice(&resp.body) {
        Ok(v) => v,
        Err(_) if !resp.is_success() => return Err(Error::Http(format!("HTTP {} from {path}", resp.status))),
        Err(_) => return Err(Error::Decode(format!("non-JSON reply from {path}"))),
    };
    if let Some(code) = v.get("code").and_then(Value::as_i64) {
        if code == 80001 {
            return Err(Error::NoAck);
        }
        if code != 0 {
            let msg = v.get("msg").and_then(Value::as_str).unwrap_or_default().to_string();
            return Err(Error::Api { code, msg });
        }
    }
    if !resp.is_success() {
        return Err(Error::Http(format!("HTTP {} from {path}", resp.status)));
    }
    Ok(v)
}

/// `data.result`, or a top-level `result` on some firmware. `null` counts as
/// missing.
fn command_result(v: &Value) -> Option<&Value> {
    v.pointer("/data/result").or_else(|| v.get("result")).filter(|r| !r.is_null())
}

fn as_u32(v: &Value) -> Option<u32> {
    v.as_u64().map(|n| n as u32).or_else(|| v.as_str()?.parse().ok())
}

/// Parses `get_properties` / `set_properties` result items.
///
/// Some robots answer a property of a service they lack with an item that has
/// `siid` and `code` but no `piid` (seen on r2579a for services 13, 17, 24-26,
/// 31-38 and 40). When the reply has one item per requested id, such an item
/// is matched to the requested id at the same position. Otherwise it is
/// skipped, so one odd item never fails the whole batch.
fn parse_prop_results(requested: &[PropId], result: &Value) -> Result<Vec<PropResult>> {
    let items = result.as_array().ok_or_else(|| Error::Decode("sendCommand result is not a list".into()))?;
    let same_len = items.len() == requested.len();
    let mut out = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let id = match (it.get("siid").and_then(as_u32), it.get("piid").and_then(as_u32)) {
            (Some(siid), Some(piid)) => PropId::new(siid, piid),
            _ if same_len && it.get("code").is_some() => requested[i],
            _ => {
                tracing::debug!(index = i, "skipping get_properties item without siid/piid");
                continue;
            }
        };
        let code = it.get("code").and_then(Value::as_i64).unwrap_or(0);
        let value = match it.get("value") {
            Some(v) if code == 0 && !v.is_null() => Some(RawValue::from_json(v)),
            _ => None,
        };
        out.push(PropResult { id, code, value });
    }
    Ok(out)
}

/// The entries of a history reply: the first list found under `data` (its key is not
/// documented), or `data` itself when it is a list.
fn history_entries(v: &Value) -> Vec<Value> {
    match v.get("data") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.values().find_map(|x| x.as_array().cloned()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn parse_cached_props(v: &Value) -> Result<Vec<CachedProp>> {
    let items = match v.get("data") {
        Some(Value::Array(a)) => a,
        Some(Value::Null) | None => return Ok(Vec::new()),
        Some(_) => return Err(Error::Decode("iotstatus/props data is not a list".into())),
    };
    let mut out = Vec::with_capacity(items.len());
    for it in items {
        let id = match it.get("key").and_then(Value::as_str) {
            Some(k) => k.parse::<PropId>()?,
            None => match (it.get("siid").and_then(as_u32), it.get("piid").and_then(as_u32)) {
                (Some(s), Some(p)) => PropId::new(s, p),
                _ => return Err(Error::Decode("iotstatus/props item without key".into())),
            },
        };
        let value = match it.get("value") {
            None | Some(Value::Null) => continue,
            Some(Value::String(s)) => RawValue::from_cache(s),
            Some(other) => RawValue::from_json(other),
        };
        let updated_ms = it.get("updateDate").and_then(|d| d.as_i64().or_else(|| d.as_str()?.parse().ok()));
        out.push(CachedProp { id, value, updated_ms });
    }
    Ok(out)
}

/// Maps one device record from the list or info endpoint.
pub(crate) fn parse_device(rec: &Value) -> DeviceInfo {
    let s = |k: &str| match rec.get(k) {
        Some(Value::String(x)) if !x.is_empty() => Some(x.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    // `property` is JSON text such as {"lwt":1,"mac":"..."}.
    let property: Value = match rec.get("property") {
        Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
        Some(v @ Value::Object(_)) => v.clone(),
        _ => Value::Null,
    };
    let online = rec
        .get("online")
        .and_then(Value::as_bool)
        .or_else(|| property.get("lwt").and_then(Value::as_i64).map(|l| l == 1))
        .unwrap_or(false);
    let name = s("customName")
        .or_else(|| rec.pointer("/deviceInfo/displayName").and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string))
        .unwrap_or_default();
    DeviceInfo {
        did: s("did").unwrap_or_default(),
        model: s("model").unwrap_or_default(),
        firmware: s("ver").unwrap_or_default(),
        name,
        bind_domain: s("bindDomain"),
        mac: s("mac").or_else(|| property.get("mac").and_then(Value::as_str).map(str::to_string)),
        online,
        raw: rec.clone(),
    }
}

/// Finds the records in the known device list shapes: `data.page.records`,
/// `data.records` or top-level `records`.
pub(crate) fn parse_device_list(v: &Value) -> Result<Vec<DeviceInfo>> {
    let recs = v
        .pointer("/data/page/records")
        .or_else(|| v.pointer("/data/records"))
        .or_else(|| v.get("records"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Decode("unexpected device list shape".into()))?;
    Ok(recs.iter().map(parse_device).collect())
}

fn parse_otc_info(v: &Value) -> Result<OtcInfo> {
    let params = v
        .pointer("/data/otcInfo/params")
        .ok_or_else(|| Error::Decode("devOTCInfo reply has no otcInfo.params".into()))?;
    let params = match params {
        Value::String(t) => serde_json::from_str(t).map_err(|_| Error::Decode("otcInfo.params is not JSON".into()))?,
        other => other.clone(),
    };
    let s = |p: &str| params.pointer(p).and_then(Value::as_str).map(str::to_string);
    Ok(OtcInfo {
        fw_ver: s("/fw_ver"),
        hw_ver: s("/hw_ver"),
        model: s("/model"),
        local_ip: s("/netif/localIp"),
        raw: params,
    })
}

fn parse_url(v: &Value) -> Result<String> {
    let data = v.get("data");
    data.and_then(Value::as_str)
        .or_else(|| data?.get("url")?.as_str())
        .filter(|u| !u.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Decode("no URL in download reply".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_entries_come_from_data_list() {
        // The r2579a reply layout (protocols.md §9.2).
        let v = json!({"code": 0, "success": true, "data": {"list": [{"id": "1", "history": "[]", "createTime": 1}]}});
        assert_eq!(history_entries(&v).len(), 1);
        assert!(history_entries(&json!({"data": {"list": []}})).is_empty());
        assert!(history_entries(&json!({"code": 1})).is_empty());
    }
    use crate::auth::MemoryTokenStore;
    use crate::http::mock::MockTransport;

    const TOKEN: &str = r#"{"access_token":"AT1","refresh_token":"RT1","expires_in":7200,"token_type":"bearer",
        "uid":"U1","tenant_id":"000000","country":"DE","region":"us","lang":"de"}"#;
    const TOKEN2: &str = r#"{"access_token":"AT2","refresh_token":"RT2","expires_in":7199,"uid":"U1",
        "tenant_id":"000000","country":"DE","region":"us","lang":"de"}"#;
    const DEV: &str = "D1";

    fn body_json(r: &HttpRequest) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            did: DEV.into(),
            model: "dreame.vacuum.r2579a".into(),
            bind_domain: Some("10000.mt.us.iot.dreame.tech:19973".into()),
            ..Default::default()
        }
    }

    async fn logged_in(mock: &Arc<MockTransport>) -> CloudClient {
        mock.push(200, TOKEN);
        let cfg = CloudConfig::default().with_transport(mock.clone());
        CloudClient::login_with(cfg, Region::Us, "user@example.com", "pw").await.unwrap()
    }

    #[tokio::test]
    async fn login_request_and_state() {
        let mock = Arc::new(MockTransport::new());
        let store = Arc::new(MemoryTokenStore::new());
        mock.push(200, TOKEN);
        let cfg = CloudConfig::default().with_transport(mock.clone()).with_token_store(store.clone());
        let c = CloudClient::login_with(cfg, Region::Us, "user@example.com", "pw").await.unwrap();
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].url, "https://us.iot.dreame.tech:13267/dreame-auth/oauth/token");
        assert_eq!(reqs[0].header("content-type"), Some("application/x-www-form-urlencoded"));
        assert_eq!(reqs[0].header("dreame-auth"), None);
        assert!(reqs[0].header("dreame-meta").unwrap().starts_with("cv=a_102060300;canvasHash="));
        let form = String::from_utf8(reqs[0].body.clone()).unwrap();
        assert!(form.starts_with("grant_type=password&scope=all&platform=ANDROID&type=account&username=user%40example.com&password="));
        assert!(!form.contains("pw&") && !form.ends_with("=pw"));
        assert_eq!(c.uid(), "U1");
        assert_eq!(c.region(), Region::Us);
        assert_eq!(c.country().as_deref(), Some("DE"));
        assert_eq!(c.access_token(), "AT1");
        let saved = store.load().unwrap();
        assert_eq!(saved.refresh_token, "RT1");
        assert_eq!(saved.region, Region::Us);
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("AT1") && !dbg.contains("RT1"), "{dbg}");
    }

    #[tokio::test]
    async fn login_any_falls_back_and_uses_token_region() {
        let mock = Arc::new(MockTransport::new());
        mock.push(401, r#"{"error":"invalid_user","error_description":"Dreame user not found"}"#);
        mock.push(503, "unavailable");
        // The sg host accepts the login but says the account lives in us.
        mock.push(200, TOKEN);
        let cfg = CloudConfig::default().with_transport(mock.clone());
        let c = CloudClient::login_any_with(cfg, &[Region::Eu, Region::Cn, Region::Sg], "u@x.y", "pw").await.unwrap();
        let hosts: Vec<String> = mock.requests().iter().map(|r| r.url.clone()).collect();
        assert!(hosts[0].starts_with("https://eu.iot.dreame.tech"));
        assert!(hosts[1].starts_with("https://cn.iot.dreame.tech"));
        assert!(hosts[2].starts_with("https://sg.iot.dreame.tech"));
        assert_eq!(c.region(), Region::Us);

        mock.push(200, r#"{"code":0,"success":true,"data":{"page":{"records":[]}}}"#);
        c.list_devices().await.unwrap();
        assert!(mock.requests()[3].url.starts_with("https://us.iot.dreame.tech:13267/"));
    }

    #[tokio::test]
    async fn login_any_reports_all_failures() {
        let mock = Arc::new(MockTransport::new());
        mock.push(401, r#"{"error":"invalid_user","error_description":"Dreame user not found"}"#);
        mock.push(400, r#"{"error":"invalid_grant","error_description":"bad password"}"#);
        let cfg = CloudConfig::default().with_transport(mock.clone());
        let e = CloudClient::login_any_with(cfg, &[Region::Us, Region::Eu], "u@x.y", "pw").await.unwrap_err();
        let Error::Auth(msg) = e else { panic!("expected Auth") };
        assert!(msg.contains("us:") && msg.contains("eu:") && msg.contains("bad password"), "{msg}");
    }

    #[tokio::test]
    async fn custom_host_is_kept() {
        let mock = Arc::new(MockTransport::new());
        mock.push(200, TOKEN);
        let cfg = CloudConfig::default().with_transport(mock.clone());
        let c = CloudClient::login_with(cfg, Region::parse("http://127.0.0.1:9"), "u@x.y", "pw").await.unwrap();
        assert_eq!(c.region(), Region::Custom("http://127.0.0.1:9".into()));
    }

    #[tokio::test]
    async fn signed_json_headers_and_device_list() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(
            200,
            r#"{"code":0,"success":true,"msg":"ok","data":{"page":{"records":[
                {"did":"D1","model":"dreame.vacuum.r2579a","ver":"4.3.9_1664","customName":"",
                 "deviceInfo":{"displayName":"L40 Ultra AE"},"bindDomain":"10000.mt.us.iot.dreame.tech:19973",
                 "property":"{\"lwt\":1,\"mac\":\"AA:BB\"}","online":true},
                {"did":2,"model":"dreame.feeder.x","customName":"Feeder","property":"{\"lwt\":0}"}]}}}"#,
        );
        let devs = c.list_devices().await.unwrap();
        let req = &mock.requests()[1];
        assert_eq!(req.path(), "/dreame-user-iot/iotuserbind/device/listV2");
        assert_eq!(req.header("dreame-auth"), Some("AT1"));
        assert_eq!(req.header("tenant-id"), Some("000000"));
        assert_eq!(req.header("content-type"), Some("application/json"));
        let b = body_json(req);
        assert_eq!(b["size"], 100);
        assert_eq!(b["sign"].as_str().unwrap().len(), 32);
        assert!(b["timestamp"].is_u64());

        assert_eq!(devs.len(), 2);
        assert_eq!(devs[0].name, "L40 Ultra AE");
        assert_eq!(devs[0].firmware, "4.3.9_1664");
        assert_eq!(devs[0].mac.as_deref(), Some("AA:BB"));
        assert!(devs[0].online && devs[0].is_vacuum());
        assert_eq!(devs[1].did, "2");
        assert_eq!(devs[1].name, "Feeder");
        assert!(!devs[1].online && !devs[1].is_vacuum());
    }

    #[test]
    fn device_list_shapes() {
        let rec = json!({"did":"D1","model":"dreame.vacuum.x"});
        for v in [
            json!({"code":0,"data":{"page":{"records":[rec]}}}),
            json!({"code":0,"data":{"records":[rec]}}),
            json!({"records":[rec]}),
        ] {
            assert_eq!(parse_device_list(&v).unwrap()[0].did, "D1");
        }
        assert!(parse_device_list(&json!({"code":0,"data":{}})).is_err());
    }

    #[tokio::test]
    async fn cached_properties_shape_and_batching() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(
            200,
            r#"{"code":0,"success":true,"data":[
                {"key":"2.1","value":"8","updateDate":1763200000000},
                {"key":"3.3","value":"{\"enable\":false,\"startTime\":\"22:00\"}","updateDate":1763200000001},
                {"key":"5.2","value":"22:00","updateDate":1763200000002},
                {"key":"6.3","value":"ali_dreame/U/D/0","updateDate":null}]}"#,
        );
        mock.push(200, r#"{"code":0,"success":true,"data":[]}"#);
        let ids: Vec<PropId> = (1..=61).map(|p| PropId::new(4, p)).collect();
        let mut ids2 = vec![PropId::new(2, 1)];
        ids2.extend(ids.iter().skip(1));
        let got = c.cached_properties(DEV, &ids2).await.unwrap();
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 3, "61 keys need two calls");
        let b = body_json(&reqs[1]);
        assert_eq!(b["did"], DEV);
        let keys = b["keys"].as_str().expect("keys must be one string");
        assert!(keys.starts_with("2.1,4.2,4.3"));
        assert_eq!(keys.split(',').count(), 60);
        assert_eq!(body_json(&reqs[2])["keys"], "4.61");

        assert_eq!(got.len(), 4);
        assert_eq!(got[0], CachedProp { id: PropId::new(2, 1), value: RawValue::Int(8), updated_ms: Some(1763200000000) });
        assert_eq!(got[1].value.to_json()["startTime"], "22:00");
        assert_eq!(got[2].value, RawValue::Str("22:00".into()));
        assert_eq!(got[3].updated_ms, None);
    }

    #[tokio::test]
    async fn get_properties_typed_and_batched() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(
            200,
            r#"{"code":0,"success":true,"data":{"id":33,"result":[
                {"siid":2,"did":"D1","piid":1,"value":8,"code":0},
                {"siid":3,"did":"D1","piid":1,"value":100,"code":0},
                {"siid":6,"did":"D1","piid":8,"value":"{\"obj_name\":\"x\"}","code":0},
                {"siid":4,"did":"D1","piid":64,"code":-1}]},"msg":null}"#,
        );
        mock.push(200, r#"{"code":0,"success":true,"result":[{"siid":9,"piid":1,"value":true,"code":0}]}"#);
        let mut ids: Vec<PropId> = (1..=15).map(|p| PropId::new(2, p)).collect();
        ids.push(PropId::new(9, 1));
        let got = c.get_properties(&device(), &ids).await.unwrap();
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 3);
        assert_eq!(reqs[1].path(), "/dreame-iot-com-10000/device/sendCommand");
        let b = body_json(&reqs[1]);
        assert_eq!(b["did"], DEV);
        assert_eq!(b["data"]["method"], "get_properties");
        assert_eq!(b["data"]["id"], b["id"]);
        assert_eq!(b["data"]["params"].as_array().unwrap().len(), 15);
        assert_eq!(b["data"]["params"][0], json!({"did":"D1","siid":2,"piid":1}));
        assert_eq!(body_json(&reqs[2])["data"]["params"].as_array().unwrap().len(), 1);

        assert_eq!(got[0], PropResult { id: PropId::new(2, 1), code: 0, value: Some(RawValue::Int(8)) });
        assert_eq!(got[1].value, Some(RawValue::Int(100)));
        assert_eq!(got[2].value.as_ref().unwrap().to_json()["obj_name"], "x");
        assert_eq!(got[3], PropResult { id: PropId::new(4, 64), code: -1, value: None });
        assert_eq!(got[4].value, Some(RawValue::Bool(true)));
    }

    #[tokio::test]
    async fn retry_once_when_success_without_result() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, r#"{"code":0,"success":true,"data":{"id":1}}"#);
        mock.push(200, r#"{"code":0,"success":true,"data":{"id":2,"result":[{"siid":2,"piid":1,"value":6,"code":0}]}}"#);
        let got = c.get_properties(&device(), &[PropId::new(2, 1)]).await.unwrap();
        assert_eq!(got[0].value, Some(RawValue::Int(6)));
        let reqs = mock.requests();
        assert_ne!(body_json(&reqs[1])["id"], body_json(&reqs[2])["id"]);

        mock.push(200, r#"{"code":0,"success":true,"data":{}}"#);
        mock.push(200, r#"{"code":0,"success":true,"data":{"result":null}}"#);
        assert!(matches!(c.get_properties(&device(), &[PropId::new(2, 1)]).await, Err(Error::Decode(_))));
    }

    #[tokio::test]
    async fn transport_errors_retry_reads_but_not_writes() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        let before = mock.requests().len();
        mock.push(0, "error sending request");
        mock.push(200, r#"{"code":0,"success":true,"data":{}}"#);
        assert!(c.post_api("/read", json!({}), true).await.is_ok());
        assert_eq!(mock.requests().len() - before, 2, "the read was sent again");

        let before = mock.requests().len();
        mock.push(0, "error sending request");
        assert!(matches!(c.post_api("/write", json!({}), false).await, Err(Error::Http(_))));
        assert_eq!(mock.requests().len() - before, 1, "the write was not resent");
    }

    #[tokio::test]
    async fn api_codes_map_to_errors() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, r#"{"code":80001,"success":false,"msg":"设备离线"}"#);
        assert!(matches!(c.get_properties(&device(), &[PropId::new(2, 1)]).await, Err(Error::NoAck)));
        mock.push(200, r#"{"code":10007,"success":false,"msg":"bad"}"#);
        match c.cached_properties(DEV, &[PropId::new(2, 1)]).await {
            Err(Error::Api { code: 10007, msg }) => assert_eq!(msg, "bad"),
            other => panic!("unexpected {other:?}"),
        }
        mock.push(404, "not found");
        assert!(matches!(c.device_info(DEV).await, Err(Error::Http(_))));
    }

    #[tokio::test]
    async fn refresh_on_401_and_rotation() {
        let mock = Arc::new(MockTransport::new());
        let store = Arc::new(MemoryTokenStore::new());
        mock.push(200, TOKEN);
        let cfg = CloudConfig::default().with_transport(mock.clone()).with_token_store(store.clone());
        let c = CloudClient::login_with(cfg, Region::Us, "u@x.y", "pw").await.unwrap();
        mock.push(401, r#"{"code":401,"msg":"unauthorized"}"#);
        mock.push(200, TOKEN2);
        mock.push(200, r#"{"code":0,"data":[{"key":"3.1","value":"99","updateDate":1}]}"#);
        let got = c.cached_properties(DEV, &[PropId::new(3, 1)]).await.unwrap();
        assert_eq!(got[0].value, RawValue::Int(99));
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 4);
        assert_eq!(reqs[1].header("dreame-auth"), Some("AT1"));
        assert_eq!(reqs[2].path(), TOKEN_PATH);
        assert!(String::from_utf8(reqs[2].body.clone()).unwrap().ends_with("&refresh_token=RT1"));
        assert_eq!(reqs[3].header("dreame-auth"), Some("AT2"));
        assert_eq!(c.access_token(), "AT2");
        assert_eq!(store.load().unwrap().refresh_token, "RT2");

        // A second 401 right after the refresh is not retried again.
        mock.push(401, r#"{"code":401,"msg":"unauthorized"}"#);
        mock.push(200, TOKEN);
        mock.push(401, r#"{"code":401,"msg":"unauthorized"}"#);
        assert!(c.cached_properties(DEV, &[PropId::new(3, 1)]).await.is_err());
        assert_eq!(mock.remaining(), 0);
    }

    #[tokio::test]
    async fn auto_refresh_near_expiry() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        c.state.write().unwrap().expires_at = Instant::now() + Duration::from_secs(300);
        mock.push(200, TOKEN2);
        mock.push(200, r#"{"code":0,"data":[]}"#);
        c.cached_properties(DEV, &[PropId::new(2, 1)]).await.unwrap();
        let reqs = mock.requests();
        assert_eq!(reqs[1].path(), TOKEN_PATH);
        assert_eq!(reqs[2].header("dreame-auth"), Some("AT2"));
        assert!(c.expires_in() > Duration::from_secs(7000));
    }

    #[tokio::test]
    async fn from_refresh_token_resumes() {
        let mock = Arc::new(MockTransport::new());
        let store = Arc::new(MemoryTokenStore::new());
        store.save(&StoredSession { refresh_token: "RT0".into(), region: Region::Us, uid: "U1".into(), tenant_id: "000000".into() });
        mock.push(200, TOKEN2);
        let cfg = CloudConfig::default().with_transport(mock.clone()).with_token_store(store.clone());
        let c = CloudClient::from_store(cfg).await.unwrap();
        let form = String::from_utf8(mock.requests()[0].body.clone()).unwrap();
        assert!(form.ends_with("refresh_token=RT0"));
        assert_eq!(c.access_token(), "AT2");
        assert_eq!(c.country().as_deref(), Some("DE"));
        assert_eq!(store.load().unwrap().refresh_token, "RT2");
    }

    #[tokio::test]
    async fn action_params_is_an_object() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, r#"{"code":0,"success":true,"data":{"id":5,"result":{"code":0,"out":[]}}}"#);
        let input = vec![json!({"piid":2,"value":"{\"req_type\":1}"})];
        let r = c.action(&device(), ActionId::new(6, 1), input).await.unwrap();
        assert_eq!(r["code"], 0);
        let b = body_json(&mock.requests()[1]);
        assert_eq!(b["data"]["method"], "action");
        assert_eq!(b["data"]["params"], json!({"did":"D1","siid":6,"aiid":1,"in":[{"piid":2,"value":"{\"req_type\":1}"}]}));
    }

    #[tokio::test]
    async fn set_properties_sends_typed_values() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, r#"{"code":0,"data":{"result":[{"siid":5,"piid":1,"code":0}]}}"#);
        let got = c
            .set_properties(&device(), &[(PropId::new(5, 1), RawValue::Bool(true)), (PropId::new(5, 2), RawValue::Str("{\"a\":1}".into()))])
            .await
            .unwrap();
        assert_eq!(got[0].code, 0);
        let b = body_json(&mock.requests()[1]);
        assert_eq!(b["data"]["method"], "set_properties");
        assert_eq!(b["data"]["params"][0]["value"], true);
        // A string stays a string, even when it holds JSON text.
        assert_eq!(b["data"]["params"][1]["value"], "{\"a\":1}");
    }

    #[tokio::test]
    async fn download_url_uses_country() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, r#"{"code":0,"data":"https://oss.example/x?sig=1"}"#);
        mock.push(200, r#"{"code":0,"data":{"url":"https://oss.example/y"}}"#);
        let u = c.download_url(DEV, "dreame.vacuum.r2579a", "ali_dreame/U/D/0").await.unwrap();
        assert_eq!(u, "https://oss.example/x?sig=1");
        let u = c.permanent_download_url(DEV, "dreame.vacuum.r2579a", "ali_dreame/U/D/9").await.unwrap();
        assert_eq!(u, "https://oss.example/y");
        let reqs = mock.requests();
        let b = body_json(&reqs[1]);
        assert_eq!(b["region"], "DE");
        assert_eq!(b["filename"], "ali_dreame/U/D/0");
        let b = body_json(&reqs[2]);
        assert_eq!(reqs[2].path(), OSS1D_PATH);
        assert_eq!(b["filename"], "li_dreame/U/D/9");
        assert_eq!(b["uid"], "U1");
    }

    #[tokio::test]
    async fn fetch_has_no_dreame_headers() {
        let mock = Arc::new(MockTransport::new());
        let c = logged_in(&mock).await;
        mock.push(200, "eJxLy");
        assert_eq!(c.fetch_text("https://oss.example/x").await.unwrap(), "eJxLy");
        let r = &mock.requests()[1];
        assert_eq!(r.method, Method::Get);
        assert_eq!(r.header("dreame-auth"), None);
        assert_eq!(r.header("user-agent"), Some(OSS_USER_AGENT));
        mock.push(403, "denied");
        assert!(matches!(c.fetch_bytes("https://oss.example/x").await, Err(Error::Http(_))));
    }

    #[test]
    fn otc_info_parses() {
        let v = json!({"code":0,"data":{"otcInfo":{"partner_id":"","params":{"hw_ver":"Linux","fw_ver":"4.3.9_1664",
            "ap":{"siid":"wifi","bssid":"x","rssi":-50},"netif":{"localIp":"192.0.2.10","mask":"255.255.255.0","gw":"192.0.2.1"},
            "model":"dreame.vacuum.r2579a"}}}});
        let o = parse_otc_info(&v).unwrap();
        assert_eq!(o.fw_ver.as_deref(), Some("4.3.9_1664"));
        assert_eq!(o.local_ip.as_deref(), Some("192.0.2.10"));
        assert_eq!(o.raw["ap"]["rssi"], -50);
    }

    #[test]
    fn paths_and_backoff() {
        assert_eq!(command_path(Some("10000.mt.us.iot.dreame.tech:19973")), "/dreame-iot-com-10000/device/sendCommand");
        assert_eq!(command_path(None), "/dreame-iot-com/device/sendCommand");
        assert_eq!(backoff(1), Duration::from_millis(500));
        assert_eq!(backoff(2), Duration::from_millis(1000));
        assert_eq!(backoff(3), Duration::from_millis(2000));
        assert_eq!(backoff(9), Duration::from_millis(2000));
    }

    #[test]
    fn prop_items_without_piid_are_matched_by_position() {
        let ids = [PropId::new(13, 1), PropId::new(2, 1)];
        let r = json!([{"siid":13,"did":"D1","code":-1},{"siid":2,"did":"D1","piid":1,"value":8,"code":0}]);
        let out = parse_prop_results(&ids, &r).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, PropId::new(13, 1));
        assert_eq!(out[0].code, -1);
        assert_eq!(out[1].value, Some(RawValue::Int(8)));
        // Different length: the odd item is skipped, the rest still parses.
        let out = parse_prop_results(&ids[..1], &r).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, PropId::new(2, 1));
    }
}
