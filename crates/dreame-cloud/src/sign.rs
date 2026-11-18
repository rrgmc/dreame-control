//! Request body signing (protocols.md section 3).
//!
//! Every JSON request body that is an object gets two extra fields:
//! `sign = md5_hex(spliced(body) + ms + secret)` and `timestamp = ms`.

use md5::{Digest, Md5};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lowercase hex MD5 of a UTF-8 string.
pub fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// Current Unix time in milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Compact JSON with sorted keys and non-ASCII kept, like Python
/// `json.dumps(v, separators=(",", ":"), sort_keys=True, ensure_ascii=False)`.
fn compact_sorted(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            let parts: Vec<String> =
                keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), compact_sorted(&m[*k]))).collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(compact_sorted).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// The `spliced` canonical form of a body (tasshack `protocol.py`).
///
/// Keys are sorted and joined with `&`. Nested objects become `k=[...]`, or
/// `k=]` when empty. Arrays are kept as compact JSON at the top level and
/// omitted when nested. Strings are raw at the top level and JSON-quoted when
/// nested.
pub fn spliced(obj: &Map<String, Value>, top: bool) -> String {
    let mut keys: Vec<_> = obj.keys().collect();
    keys.sort();
    let mut parts = Vec::with_capacity(keys.len());
    for k in keys {
        let v = &obj[k];
        match v {
            Value::Object(m) if m.is_empty() => parts.push(format!("{k}=]")),
            Value::Object(m) => parts.push(format!("{k}=[{}]", spliced(m, false))),
            Value::Array(_) => {
                if top {
                    parts.push(format!("{k}={}", compact_sorted(v)));
                }
            }
            Value::Bool(b) => parts.push(format!("{k}={b}")),
            Value::Null => parts.push(format!("{k}=null")),
            Value::String(s) if top => parts.push(format!("{k}={s}")),
            Value::String(_) => parts.push(format!("{k}={v}")),
            Value::Number(n) => parts.push(format!("{k}={n}")),
        }
    }
    parts.join("&")
}

/// Signs JSON bodies with the brand's secret (`cid`).
#[derive(Clone)]
pub struct Signer {
    secret: String,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer").finish_non_exhaustive()
    }
}

impl Signer {
    pub fn new(secret: impl Into<String>) -> Self {
        Self { secret: secret.into() }
    }

    /// The signature of `body` at time `ms`.
    pub fn signature(&self, body: &Map<String, Value>, ms: u64) -> String {
        md5_hex(&format!("{}{}{}", spliced(body, true), ms, self.secret))
    }

    /// Adds `sign` and `timestamp` to `body`, using time `ms`.
    pub fn sign_at(&self, body: &mut Map<String, Value>, ms: u64) {
        let sig = self.signature(body, ms);
        body.insert("sign".into(), Value::String(sig));
        body.insert("timestamp".into(), Value::from(ms));
    }

    /// Adds `sign` and `timestamp` to `body`, using the current time.
    pub fn sign(&self, body: &mut Map<String, Value>) {
        self.sign_at(body, now_ms());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "EETjszu*XI5znHsI";

    /// Worked example from protocols.md section 3.3.
    #[test]
    fn worked_example_matches() {
        let p = json!({"did":"1234567890","id":42,"data":{"did":"1234567890","id":42,
            "method":"get_properties","params":[{"did":"1234567890","siid":2,"piid":1}]}});
        let s = spliced(p.as_object().unwrap(), true);
        assert_eq!(s, r#"data=[did="1234567890"&id=42&method="get_properties"]&did=1234567890&id=42"#);
        let signer = Signer::new(SECRET);
        assert_eq!(signer.signature(p.as_object().unwrap(), 1_700_000_000_000), "39ded7b48dd039c03d1a0404ba54fc6e");
    }

    #[test]
    fn top_level_array_is_kept() {
        let p = json!({"did":"1234567890","keys":[{"did":"2.1","siid":2,"piid":1}]});
        assert_eq!(spliced(p.as_object().unwrap(), true), r#"did=1234567890&keys=[{"did":"2.1","piid":1,"siid":2}]"#);
    }

    #[test]
    fn empty_object_bool_and_null() {
        let p = json!({"a":{},"b":true,"c":null,"d":{"e":false}});
        assert_eq!(spliced(p.as_object().unwrap(), true), "a=]&b=true&c=null&d=[e=false]");
    }

    #[test]
    fn sign_adds_fields() {
        let mut body = json!({"did":"1234567890","id":42,"data":{"did":"1234567890","id":42,
            "method":"get_properties","params":[{"did":"1234567890","siid":2,"piid":1}]}})
        .as_object()
        .unwrap()
        .clone();
        Signer::new(SECRET).sign_at(&mut body, 1_700_000_000_000);
        assert_eq!(body["sign"], "39ded7b48dd039c03d1a0404ba54fc6e");
        assert_eq!(body["timestamp"], 1_700_000_000_000u64);
    }
}
