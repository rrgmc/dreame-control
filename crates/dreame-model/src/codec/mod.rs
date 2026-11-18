//! Codecs for composite property values (docs/research/device-model.md section 4).
//!
//! Each decoder accepts a [`RawValue`] from any source: a relayed read
//! (typed JSON), the cloud cache (strings) or an MQTT push. Each encoder
//! returns the value to write, but nothing in this crate sends it.

pub mod ai_obstacle;
pub mod auto_switch;
pub mod cleaning_mode;
pub mod cleanset;
pub mod consumables;
pub mod custom_clean;
pub mod dnd;
pub mod faults;
pub mod off_peak;
pub mod record;
pub mod schedule;
pub mod station;

use dreame_core::{Error, RawValue, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

pub(crate) fn decode_err(what: &str, v: impl fmt::Debug) -> Error {
    Error::Decode(format!("{what}: unexpected value {v:?}"))
}

/// The value as an integer. Accepts numbers, booleans and numeric strings.
pub fn int(v: &RawValue) -> Result<i64> {
    match v {
        RawValue::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
        _ => v.as_i64().ok_or_else(|| decode_err("integer", v)),
    }
}

/// The value as a boolean. Accepts `true`/`false`, 0/1 and their strings.
pub fn boolean(v: &RawValue) -> Result<bool> {
    match v {
        RawValue::Bool(b) => Ok(*b),
        RawValue::Str(s) if s.trim().eq_ignore_ascii_case("true") => Ok(true),
        RawValue::Str(s) if s.trim().eq_ignore_ascii_case("false") => Ok(false),
        _ => match int(v)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(decode_err("boolean", v)),
        },
    }
}

/// The value as text. Numbers are formatted.
pub fn text(v: &RawValue) -> String {
    match v {
        RawValue::Str(s) => s.clone(),
        RawValue::Int(i) => i.to_string(),
        RawValue::Bool(b) => b.to_string(),
        RawValue::Float(f) => f.to_string(),
        RawValue::Json(j) => j.to_string(),
    }
}

/// Parses JSON carried as JSON or as JSON text inside a string.
pub fn json(v: &RawValue) -> Result<serde_json::Value> {
    match v {
        RawValue::Json(j) => Ok(j.clone()),
        RawValue::Str(s) => serde_json::from_str(s).map_err(|e| Error::Decode(format!("JSON text: {e}"))),
        other => Ok(other.to_json()),
    }
}

/// Compact JSON text, as Tasshack writes it (`separators=(",",":")`).
pub(crate) fn compact(v: &impl Serialize) -> String {
    serde_json::to_string(v).expect("serialising plain data cannot fail")
}

/// A wall-clock time `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HhMm {
    pub hour: u8,
    pub minute: u8,
}

impl HhMm {
    pub fn new(hour: u8, minute: u8) -> Result<Self> {
        if hour > 23 || minute > 59 {
            return Err(Error::Decode(format!("bad time {hour}:{minute}")));
        }
        Ok(Self { hour, minute })
    }

    pub fn parse(s: &str) -> Result<Self> {
        let (h, m) = s.trim().split_once(':').ok_or_else(|| decode_err("HH:MM", s))?;
        let h = h.parse().map_err(|_| decode_err("HH:MM", s))?;
        let m = m.parse().map_err(|_| decode_err("HH:MM", s))?;
        Self::new(h, m)
    }

    pub fn decode(v: &RawValue) -> Result<Self> {
        Self::parse(&text(v))
    }
}

impl fmt::Display for HhMm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl Serialize for HhMm {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HhMm {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// A cloud object pointer: properties 6.3, 6.8 and 6.9.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudObject {
    pub object: String,
    pub md5: Option<String>,
    /// AES key suffix (`<object>,<key>`), when the frame is encrypted.
    pub key: Option<String>,
}

impl CloudObject {
    /// Accepts a plain object name, `<object>,<key>`, or JSON with
    /// `obj_name` or `object_name` and `md5`.
    pub fn decode(v: &RawValue) -> Result<Self> {
        if let Ok(j) = json(v) {
            if let Some(o) = j.as_object() {
                let object = o
                    .get("obj_name")
                    .or_else(|| o.get("object_name"))
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| decode_err("cloud pointer", v))?;
                let md5 = o.get("md5").and_then(|x| x.as_str()).map(str::to_string);
                return Ok(Self { object: object.to_string(), md5, key: None });
            }
        }
        let s = text(v);
        if s.trim().is_empty() {
            return Err(decode_err("cloud object", v));
        }
        let (object, key) = match s.split_once(',') {
            Some((o, k)) if !k.is_empty() => (o.to_string(), Some(k.to_string())),
            _ => (s.trim_end_matches(',').to_string(), None),
        };
        Ok(Self { object, md5: None, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars() {
        assert_eq!(int(&RawValue::Str("15".into())).unwrap(), 15);
        assert_eq!(int(&RawValue::Bool(true)).unwrap(), 1);
        assert!(boolean(&RawValue::Int(1)).unwrap());
        assert!(!boolean(&RawValue::from_cache("false")).unwrap());
        assert!(boolean(&RawValue::Int(5)).is_err());
    }

    #[test]
    fn hhmm() {
        let t = HhMm::decode(&RawValue::Str("22:00".into())).unwrap();
        assert_eq!(t, HhMm::new(22, 0).unwrap());
        assert_eq!(t.to_string(), "22:00");
        assert_eq!(HhMm::parse("8:05").unwrap().to_string(), "08:05");
        assert!(HhMm::parse("25:00").is_err());
        assert!(HhMm::parse("noon").is_err());
    }

    #[test]
    fn cloud_objects() {
        let plain = CloudObject::decode(&RawValue::Str("ali_dreame/u/d/0".into())).unwrap();
        assert_eq!(plain.object, "ali_dreame/u/d/0");
        assert_eq!(plain.key, None);
        let keyed = CloudObject::decode(&RawValue::Str("ali_dreame/u/d/0,abc".into())).unwrap();
        assert_eq!(keyed.key.as_deref(), Some("abc"));
        let ptr = CloudObject::decode(&RawValue::from_cache(r#"{"obj_name":"ali_dreame/u/d/9","md5":"m"}"#)).unwrap();
        assert_eq!(ptr.object, "ali_dreame/u/d/9");
        assert_eq!(ptr.md5.as_deref(), Some("m"));
        let alt = CloudObject::decode(&RawValue::Str(r#"{"object_name":"x","md5":"m"}"#.into())).unwrap();
        assert_eq!(alt.object, "x");
    }
}
