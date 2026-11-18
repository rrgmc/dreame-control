//! Shared types for Dreame robot vacuum clients.
//!
//! See `docs/research/rust-library-design.md` for how the crates fit together.
//! This crate has no I/O. It defines MIoT addressing, the raw value type,
//! device info, the common error type and the app constants used by the cloud.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A MIoT property address: service id and property id, written `siid.piid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PropId {
    pub siid: u32,
    pub piid: u32,
}

impl PropId {
    pub const fn new(siid: u32, piid: u32) -> Self {
        Self { siid, piid }
    }
}

impl fmt::Display for PropId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.siid, self.piid)
    }
}

impl std::str::FromStr for PropId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let (a, b) = s.split_once('.').ok_or_else(|| Error::Decode(format!("bad property id {s:?}")))?;
        let p = |x: &str| x.trim().parse::<u32>().map_err(|_| Error::Decode(format!("bad property id {s:?}")));
        Ok(Self::new(p(a)?, p(b)?))
    }
}

/// A MIoT action address: service id and action id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionId {
    pub siid: u32,
    pub aiid: u32,
}

impl ActionId {
    pub const fn new(siid: u32, aiid: u32) -> Self {
        Self { siid, aiid }
    }
}

/// A MIoT event address: service id and event id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId {
    pub siid: u32,
    pub eiid: u32,
}

/// A property value as the robot or cloud reports it.
///
/// The relayed `get_properties` call returns typed JSON values, while the
/// cloud cache returns every value as a string. [`RawValue::from_cache`]
/// normalises cache strings so both sources compare equal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// JSON objects and arrays, whether sent as JSON or as JSON text in a string.
    Json(serde_json::Value),
}

impl RawValue {
    /// Converts a typed JSON value from a relayed call or an MQTT push.
    pub fn from_json(v: &serde_json::Value) -> Self {
        use serde_json::Value as V;
        match v {
            V::Bool(b) => Self::Bool(*b),
            V::Number(n) => n.as_i64().map(Self::Int).unwrap_or_else(|| Self::Float(n.as_f64().unwrap_or(f64::NAN))),
            V::String(s) => Self::Str(s.clone()),
            V::Null => Self::Str(String::new()),
            other => Self::Json(other.clone()),
        }
    }

    /// Converts a string from the cloud cache (`iotstatus/props`), where every
    /// value is text. Integers, booleans and JSON objects or arrays are parsed.
    pub fn from_cache(s: &str) -> Self {
        let t = s.trim();
        if let Ok(i) = t.parse::<i64>() {
            return Self::Int(i);
        }
        match t {
            "true" => return Self::Bool(true),
            "false" => return Self::Bool(false),
            _ => {}
        }
        if t.starts_with('{') || t.starts_with('[') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
                return Self::Json(v);
            }
        }
        if t.contains('.') {
            if let Ok(f) = t.parse::<f64>() {
                return Self::Float(f);
            }
        }
        Self::Str(s.to_string())
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Bool(b) => Some(*b as i64),
            Self::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The value as JSON. A string holding JSON text is parsed.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as V;
        match self {
            Self::Bool(b) => V::Bool(*b),
            Self::Int(i) => V::from(*i),
            Self::Float(f) => serde_json::Number::from_f64(*f).map(V::Number).unwrap_or(V::Null),
            Self::Str(s) => serde_json::from_str(s).ok().filter(|v: &V| v.is_object() || v.is_array()).unwrap_or(V::String(s.clone())),
            Self::Json(v) => v.clone(),
        }
    }
}

/// One result item of a relayed `get_properties` call.
#[derive(Debug, Clone, PartialEq)]
pub struct PropResult {
    pub id: PropId,
    /// MIoT result code. 0 means a value was returned. On r2579a, -1 also
    /// covers properties that exist but cannot be read (push-only or write-only).
    pub code: i64,
    pub value: Option<RawValue>,
}

/// A device as listed by the cloud.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub did: String,
    pub model: String,
    pub firmware: String,
    pub name: String,
    /// MQTT broker `host:port`; its first DNS label is also the command-path prefix.
    pub bind_domain: Option<String>,
    pub mac: Option<String>,
    pub online: bool,
    /// The complete record as the cloud sent it, for fields not mapped above.
    pub raw: serde_json::Value,
}

impl DeviceInfo {
    pub fn is_vacuum(&self) -> bool {
        self.model.contains(".vacuum.")
    }
}

/// Common error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Login or refresh rejected (wrong credentials, or wrong region).
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("http error: {0}")]
    Http(String),
    /// The API returned a non-zero `code`.
    #[error("api error {code}: {msg}")]
    Api { code: i64, msg: String },
    /// Cloud code 80001: the robot did not acknowledge in time. The command may
    /// still have run; confirm through a later read or an MQTT push.
    #[error("robot did not acknowledge in time (80001); outcome unknown")]
    NoAck,
    #[error("decode error: {0}")]
    Decode(String),
    #[error("not supported: {0}")]
    Unsupported(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Constants taken from the Dreamehome app. They can change with app updates,
/// so callers may override them. See docs/research/protocols.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConstants {
    /// `Authorization` header for the OAuth endpoint (`dreame_appv1` client).
    pub basic_auth: String,
    /// Appended to the plaintext password before MD5.
    pub password_salt: String,
    /// Appended to the spliced body before MD5 when signing JSON bodies.
    pub sign_secret: String,
    /// `cv=` part of the `dreame-meta` header.
    pub meta_version: String,
    pub user_agent: String,
    pub tenant_id: String,
    /// `platform` field of the OAuth form.
    pub platform: String,
}

impl Default for AppConstants {
    fn default() -> Self {
        Self {
            basic_auth: "Basic ZHJlYW1lX2FwcHYxOkFQXmR2QHpAU1FZVnhOODg=".into(),
            password_salt: "RAylYC%fmSKp7%Tq".into(),
            sign_secret: "EETjszu*XI5znHsI".into(),
            meta_version: "cv=a_102060300".into(),
            user_agent: "Dart/3.9 (dart:io)".into(),
            tenant_id: "000000".into(),
            platform: "ANDROID".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_id_round_trip() {
        let p: PropId = "6.3".parse().unwrap();
        assert_eq!(p, PropId::new(6, 3));
        assert_eq!(p.to_string(), "6.3");
        assert!("6".parse::<PropId>().is_err());
    }

    #[test]
    fn cache_values_normalise() {
        assert_eq!(RawValue::from_cache("8"), RawValue::Int(8));
        assert_eq!(RawValue::from_cache("-1"), RawValue::Int(-1));
        assert_eq!(RawValue::from_cache("false"), RawValue::Bool(false));
        assert_eq!(RawValue::from_cache("22:00"), RawValue::Str("22:00".into()));
        assert_eq!(RawValue::from_cache("-113.95"), RawValue::Float(-113.95));
        assert!(matches!(RawValue::from_cache(r#"{"enable":false}"#), RawValue::Json(_)));
        assert_eq!(RawValue::from_json(&serde_json::json!(8)), RawValue::from_cache("8"));
    }

    #[test]
    fn json_text_in_string_parses() {
        let v = RawValue::Str(r#"{"obj_name":"x","md5":"y"}"#.into());
        assert_eq!(v.to_json()["obj_name"], "x");
        assert_eq!(RawValue::Str("plain".into()).to_json(), serde_json::json!("plain"));
    }
}
