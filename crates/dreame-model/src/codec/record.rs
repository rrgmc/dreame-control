//! Event 4.e1: the cleaning record sent when a job ends (protocols.md 11.1b).
//!
//! Arguments arrive as a list of `{piid, value}` objects. Known piids:
//! 1 STATUS, 2 CLEANING_TIME (min), 3 CLEANED_AREA (m²), 8 start time
//! (Unix s), 9 history map object, 10 CLEANING_PROPERTIES (JSON),
//! 13 CLEAN_LOG_STATUS. Others are kept in `other`.

use super::{decode_err, int, json, text, CloudObject};
use crate::enums::{CleanLogStatus, Open, VacuumStatus};
use dreame_core::{RawValue, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The JSON summary in argument 10.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleaningProperties {
    pub cleaning_time: Option<i64>,
    /// `customeClean` (sic): 1 when per-room settings were used.
    pub custom_clean: Option<i64>,
    /// `mooClean` (sic).
    pub mop_clean: Option<i64>,
    pub pet: Option<i64>,
    pub cmc: Option<i64>,
    pub ctyo: Option<i64>,
    /// `multime`, Unix seconds.
    pub multime: Option<i64>,
    /// Every field, including the ones above.
    pub raw: Map<String, Value>,
}

impl CleaningProperties {
    pub fn decode(v: &RawValue) -> Result<Self> {
        let Value::Object(raw) = json(v)? else { return Err(decode_err("4.10", v)) };
        let g = |k: &str| raw.get(k).and_then(Value::as_i64);
        Ok(Self {
            cleaning_time: g("cleaningTime"),
            custom_clean: g("customeClean"),
            mop_clean: g("mooClean"),
            pet: g("pet"),
            cmc: g("cmc"),
            ctyo: g("ctyo"),
            multime: g("multime"),
            raw,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleaningRecord {
    pub status: Option<Open<VacuumStatus>>,
    pub cleaning_time_min: Option<i64>,
    pub cleaned_area_m2: Option<i64>,
    pub start_time: Option<i64>,
    /// The history map object (contains account and device ids; do not log).
    pub log_object: Option<CloudObject>,
    pub properties: Option<CleaningProperties>,
    pub result: Option<Open<CleanLogStatus>>,
    /// Arguments not decoded above, by piid.
    pub other: BTreeMap<u32, RawValue>,
}

impl CleaningRecord {
    /// Decodes `(piid, value)` pairs.
    pub fn from_args<'a>(args: impl IntoIterator<Item = (u32, &'a RawValue)>) -> Result<Self> {
        let mut r = Self::default();
        for (piid, v) in args {
            match piid {
                1 => r.status = Some(Open::from_code(int(v)?)),
                2 => r.cleaning_time_min = Some(int(v)?),
                3 => r.cleaned_area_m2 = Some(int(v)?),
                8 => r.start_time = Some(int(v)?),
                9 if !text(v).is_empty() => r.log_object = Some(CloudObject::decode(v)?),
                10 => r.properties = Some(CleaningProperties::decode(v)?),
                13 => r.result = Some(Open::from_code(int(v)?)),
                _ => {
                    r.other.insert(piid, v.clone());
                }
            }
        }
        Ok(r)
    }

    /// Decodes the MQTT `arguments` list `[{"piid":n,"value":v}, ...]`.
    pub fn from_json(arguments: &Value) -> Result<Self> {
        let list = arguments.as_array().ok_or_else(|| decode_err("4.e1 arguments", arguments))?;
        let mut pairs = Vec::with_capacity(list.len());
        for a in list {
            let piid = a.get("piid").and_then(Value::as_u64).ok_or_else(|| decode_err("4.e1 argument", a))?;
            let value = a.get("value").map(RawValue::from_json).unwrap_or(RawValue::Str(String::new()));
            pairs.push((piid as u32, value));
        }
        Self::from_args(pairs.iter().map(|(p, v)| (*p, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn r2579a_record() {
        // Values from the r2579a capture (protocols.md 11.1b), ids redacted.
        let args = json!([
            {"piid": 1, "value": 18},
            {"piid": 2, "value": 4},
            {"piid": 3, "value": 4},
            {"piid": 6, "value": 0},
            {"piid": 13, "value": 1},
            {"piid": 8, "value": 1763200000},
            {"piid": 9, "value": "ali_dreame/2026/11/15/UID/DID_1.1664.bin"},
            {"piid": 10, "value": "{\"cleaningTime\":4,\"customeClean\":0,\"mooClean\":2,\"pet\":0,\"cmc\":1,\"ctyo\":2,\"multime\":1763200000}"}
        ]);
        let r = CleaningRecord::from_json(&args).unwrap();
        assert_eq!(r.status, Some(Open::Known(VacuumStatus::SegmentCleaning)));
        assert_eq!(r.cleaning_time_min, Some(4));
        assert_eq!(r.cleaned_area_m2, Some(4));
        assert_eq!(r.result, Some(Open::Known(CleanLogStatus::Completed)));
        assert_eq!(r.start_time, Some(1763200000));
        assert!(r.log_object.unwrap().object.ends_with(".bin"));
        let p = r.properties.unwrap();
        assert_eq!((p.cleaning_time, p.custom_clean, p.mop_clean, p.cmc, p.ctyo), (Some(4), Some(0), Some(2), Some(1), Some(2)));
        assert_eq!(r.other.get(&6), Some(&RawValue::Int(0)));
    }

    #[test]
    fn bad_arguments() {
        assert!(CleaningRecord::from_json(&json!({"piid": 1})).is_err());
        assert!(CleaningRecord::from_json(&json!([{"value": 1}])).is_err());
        assert!(CleaningRecord::from_json(&json!([{"piid": 10, "value": "[1]"}])).is_err());
    }
}
