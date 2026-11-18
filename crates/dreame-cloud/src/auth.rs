//! Login, refresh and token storage (protocols.md section 5).

use crate::region::Region;
use crate::sign::md5_hex;
use dreame_core::{AppConstants, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Path of the OAuth token endpoint, for both login and refresh.
pub const TOKEN_PATH: &str = "/dreame-auth/oauth/token";

/// Refresh the access token when less than this much time remains.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(600);

/// The password as the server expects it: `md5_hex(password + salt)`.
pub fn password_hash(password: &str, salt: &str) -> String {
    md5_hex(&format!("{password}{salt}"))
}

/// The form body of a password login. `country` and `lang` are appended only
/// when known from an earlier token response.
pub fn login_form(c: &AppConstants, username: &str, password: &str, country: Option<&str>, lang: Option<&str>) -> String {
    let mut form = format!(
        "grant_type=password&scope=all&platform={}&type=account&username={}&password={}",
        urlencoding::encode(&c.platform),
        urlencoding::encode(username),
        password_hash(password, &c.password_salt)
    );
    if let (Some(country), Some(lang)) = (country, lang) {
        form.push_str(&format!("&country={}&lang={}", urlencoding::encode(country), urlencoding::encode(lang)));
    }
    form
}

/// The form body of a refresh (tasshack shape, verified on r2579a).
pub fn refresh_form(c: &AppConstants, refresh_token: &str) -> String {
    format!(
        "grant_type=refresh_token&scope=all&platform={}&type=account&refresh_token={}",
        urlencoding::encode(&c.platform),
        urlencoding::encode(refresh_token)
    )
}

/// The `dreame-meta` header. Its hashes are derived from `seed`, which is the
/// username when known and the uid otherwise.
pub fn meta_header(c: &AppConstants, with_hashes: bool, seed: &str) -> String {
    if !with_hashes {
        return c.meta_version.clone();
    }
    let canvas = &md5_hex(&format!("{seed}c"))[..8];
    let webgl = &md5_hex(&format!("{seed}w"))[..8];
    let visitor = md5_hex(&format!("{:032x}", rand::random::<u128>()));
    format!("{};canvasHash={canvas};webglHash={webgl};visitorIdHash={visitor}", c.meta_version)
}

/// The parsed token response of a login or refresh.
#[derive(Clone)]
pub(crate) struct TokenResponse {
    pub access_token: String,
    /// Present after login. Refresh rotates it, so it is normally present
    /// after a refresh too.
    pub refresh_token: Option<String>,
    pub expires_in: Duration,
    pub uid: Option<String>,
    pub tenant_id: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub lang: Option<String>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field("region", &self.region)
            .field("country", &self.country)
            .finish_non_exhaustive()
    }
}

fn str_field(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Parses a token endpoint reply. Errors never contain token values.
pub(crate) fn parse_token_response(status: u16, body: &[u8]) -> Result<TokenResponse> {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let access = v.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty());
    let ok = (200..300).contains(&status);
    if let (true, Some(access)) = (ok, access) {
        return Ok(TokenResponse {
            access_token: access.to_string(),
            refresh_token: str_field(&v, "refresh_token"),
            expires_in: Duration::from_secs(v.get("expires_in").and_then(Value::as_u64).unwrap_or(7200)),
            uid: str_field(&v, "uid"),
            tenant_id: str_field(&v, "tenant_id"),
            region: str_field(&v, "region"),
            country: str_field(&v, "country"),
            lang: str_field(&v, "lang"),
        });
    }
    let desc = str_field(&v, "error_description")
        .or_else(|| str_field(&v, "msg"))
        .or_else(|| str_field(&v, "error"))
        .unwrap_or_else(|| "no error description".into());
    let kind = str_field(&v, "error").unwrap_or_default();
    match status {
        429 | 500..=599 => Err(Error::Http(format!("token endpoint HTTP {status}: {desc}"))),
        _ if kind.is_empty() => Err(Error::Auth(format!("HTTP {status}: {desc}"))),
        _ => Err(Error::Auth(format!("HTTP {status}: {kind}: {desc}"))),
    }
}

/// What an app persists to resume a session without the password.
///
/// Refresh tokens rotate, so the stored value must be replaced after every
/// refresh. [`crate::CloudClient`] does this through its [`TokenStore`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSession {
    pub refresh_token: String,
    pub region: Region,
    pub uid: String,
    pub tenant_id: String,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession")
            .field("refresh_token", &"<redacted>")
            .field("region", &self.region)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

/// Keeps the refresh token between runs, for example in the Android Keystore
/// or the iOS Keychain. Implementations handle their own storage errors.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<StoredSession>;
    fn save(&self, session: &StoredSession);
    fn clear(&self);
}

/// A [`TokenStore`] that keeps the session in memory only.
#[derive(Default)]
pub struct MemoryTokenStore {
    inner: Mutex<Option<StoredSession>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let has = self.inner.lock().map(|g| g.is_some()).unwrap_or(false);
        f.debug_struct("MemoryTokenStore").field("has_session", &has).finish()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Option<StoredSession> {
        self.inner.lock().ok()?.clone()
    }
    fn save(&self, session: &StoredSession) {
        if let Ok(mut g) = self.inner.lock() {
            *g = Some(session.clone());
        }
    }
    fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            *g = None;
        }
    }
}

/// True when the access token should be refreshed before the next call.
pub(crate) fn needs_refresh(expires_at: Instant, now: Instant) -> bool {
    expires_at.saturating_duration_since(now) < REFRESH_MARGIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_form_body() {
        let c = AppConstants::default();
        let form = login_form(&c, "a+b@example.com", "secret", None, None);
        let hash = md5_hex("secretRAylYC%fmSKp7%Tq");
        assert_eq!(
            form,
            format!("grant_type=password&scope=all&platform=ANDROID&type=account&username=a%2Bb%40example.com&password={hash}")
        );
        assert!(!form.contains("secret&"));
        let form = login_form(&c, "x@y.z", "p", Some("DE"), Some("de"));
        assert!(form.ends_with("&country=DE&lang=de"));
    }

    #[test]
    fn refresh_form_body() {
        let c = AppConstants::default();
        assert_eq!(
            refresh_form(&c, "a.b+c"),
            "grant_type=refresh_token&scope=all&platform=ANDROID&type=account&refresh_token=a.b%2Bc"
        );
    }

    #[test]
    fn meta_hashes() {
        let c = AppConstants::default();
        let m = meta_header(&c, true, "user@example.com");
        let canvas = &md5_hex("user@example.comc")[..8];
        assert!(m.starts_with(&format!("cv=a_102060300;canvasHash={canvas};webglHash=")));
        assert_eq!(meta_header(&c, false, "x"), "cv=a_102060300");
    }

    #[test]
    fn token_response_parses() {
        let body = br#"{"access_token":"AT","refresh_token":"RT","expires_in":7200,"token_type":"bearer",
            "uid":12345,"tenant_id":"000000","country":"DE","region":"us","lang":"de"}"#;
        let t = parse_token_response(200, body).unwrap();
        assert_eq!(t.access_token, "AT");
        assert_eq!(t.refresh_token.as_deref(), Some("RT"));
        assert_eq!(t.uid.as_deref(), Some("12345"));
        assert_eq!(t.region.as_deref(), Some("us"));
        assert_eq!(t.expires_in, Duration::from_secs(7200));
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("AT") && !dbg.contains("RT"), "{dbg}");
    }

    #[test]
    fn token_errors() {
        let e = parse_token_response(401, br#"{"error":"invalid_user","error_description":"Dreame user not found"}"#)
            .unwrap_err();
        assert!(matches!(&e, Error::Auth(m) if m.contains("invalid_user")), "{e}");
        assert!(matches!(parse_token_response(503, b"busy").unwrap_err(), Error::Http(_)));
        assert!(matches!(parse_token_response(200, b"{}").unwrap_err(), Error::Auth(_)));
    }

    #[test]
    fn refresh_margin() {
        let now = Instant::now();
        assert!(needs_refresh(now + Duration::from_secs(599), now));
        assert!(!needs_refresh(now + Duration::from_secs(601), now));
        assert!(needs_refresh(now, now + Duration::from_secs(5)));
    }

    #[test]
    fn stored_session_round_trip_and_redaction() {
        let s = StoredSession {
            refresh_token: "RTSECRET".into(),
            region: Region::Us,
            uid: "u1".into(),
            tenant_id: "000000".into(),
        };
        assert!(!format!("{s:?}").contains("RTSECRET"));
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""region":"us""#));
        assert_eq!(serde_json::from_str::<StoredSession>(&json).unwrap(), s);
        let store = MemoryTokenStore::new();
        store.save(&s);
        assert_eq!(store.load(), Some(s));
        store.clear();
        assert_eq!(store.load(), None);
    }
}
