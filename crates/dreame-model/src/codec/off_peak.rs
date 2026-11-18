//! OFF_PEAK_CHARGING (3.3): `{"enable":bool,"startTime":"HH:MM","endTime":"HH:MM"}`.

use super::{compact, decode_err, json, HhMm};
use dreame_core::{RawValue, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OffPeakCharging {
    pub enable: bool,
    #[serde(rename = "startTime")]
    pub start: HhMm,
    #[serde(rename = "endTime")]
    pub end: HhMm,
    /// Fields not listed above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl OffPeakCharging {
    pub fn decode(v: &RawValue) -> Result<Self> {
        serde_json::from_value(json(v)?).map_err(|e| decode_err("3.3", e))
    }

    /// Compact JSON text, the form Tasshack writes.
    pub fn encode(&self) -> RawValue {
        RawValue::Str(compact(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_value() {
        let text = r#"{"enable":false,"startTime":"22:00","endTime":"08:00"}"#;
        let o = OffPeakCharging::decode(&RawValue::from_cache(text)).unwrap();
        assert!(!o.enable);
        assert_eq!(o.start, HhMm::new(22, 0).unwrap());
        assert_eq!(o.end, HhMm::new(8, 0).unwrap());
        assert_eq!(o.encode(), RawValue::Str(text.into()));
        assert!(OffPeakCharging::decode(&RawValue::Str(r#"{"enable":1}"#.into())).is_err());
    }
}
