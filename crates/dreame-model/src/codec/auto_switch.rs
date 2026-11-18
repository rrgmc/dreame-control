//! AUTO_SWITCH_SETTINGS (4.50): a key/value store in JSON (device-model.md 4.2).
//!
//! The read form is a list `[{"k":"<Key>","v":<int>}, ...]` or one object.
//! Tasshack writes one `{"k","v"}` object per change; node-dreame says the
//! whole list must be written. Neither is verified on r2579a, so both
//! encoders exist. Unknown keys are kept.

use super::{compact, decode_err, json};
use dreame_core::{RawValue, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Known keys (T/types:1471-1507; node-dreame feature-config.ts).
pub mod keys {
    pub const STAIN_IDENTIFY: &str = "StainIdentify";
    pub const SUPER_WASH: &str = "SuperWash";
    pub const CARPET_PRESS: &str = "RobotCarpetPressEnable";
    pub const SMART_AUTO_MOP: &str = "SmartAutoMop";
    pub const SMART_AUTO_WASH: &str = "SmartAutoWash";
    pub const CARPET_FINE_CLEAN: &str = "CarpetFineClean";
    pub const EXTR_FREQ: &str = "ExtrFreq";
    pub const AUTO_DRY: &str = "AutoDry";
    pub const LESS_COLL: &str = "LessColl";
    pub const FILL_LIGHT: &str = "FillinLight";
    pub const CLEAN_TYPE: &str = "CleanType";
    pub const CLEAN_ROUTE: &str = "CleanRoute";
    pub const BACK_WASH_TYPE: &str = "BackWashType";
    pub const SMART_HOST: &str = "SmartHost";
    pub const MOP_SCALABLE: &str = "MopScalable";
    pub const METICULOUS_TWIST: &str = "MeticulousTwist";
    pub const MOP_EFFECT_SWITCH: &str = "MopEffectSwitch";
    pub const HOT_WASH: &str = "HotWash";
}

/// The decoded store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutoSwitchSettings {
    /// Integer values by key. `-1` usually means off or blocked.
    pub values: BTreeMap<String, i64>,
    /// Entries whose value is not an integer, kept as JSON.
    pub other: BTreeMap<String, Value>,
}

impl AutoSwitchSettings {
    pub fn decode(v: &RawValue) -> Result<Self> {
        let j = json(v)?;
        let items: Vec<&Value> = match &j {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![&j],
            _ => return Err(decode_err("4.50", v)),
        };
        let mut out = Self::default();
        for it in items {
            let k = it.get("k").and_then(Value::as_str).ok_or_else(|| decode_err("4.50 entry", it))?;
            match it.get("v") {
                Some(x) if x.as_i64().is_some() => {
                    out.values.insert(k.to_string(), x.as_i64().unwrap_or_default());
                }
                Some(x) => {
                    out.other.insert(k.to_string(), x.clone());
                }
                None => return Err(decode_err("4.50 entry", it)),
            }
        }
        Ok(out)
    }

    pub fn get(&self, key: &str) -> Option<i64> {
        self.values.get(key).copied()
    }

    /// Tasshack's write form: one `{"k","v"}` object as JSON text.
    pub fn encode_single(key: &str, value: i64) -> RawValue {
        RawValue::Str(compact(&json!({"k": key, "v": value})))
    }

    /// node-dreame's write form: the whole list with one key changed.
    pub fn encode_full_with(&self, key: &str, value: i64) -> RawValue {
        let mut list: Vec<Value> = Vec::new();
        let mut seen = false;
        for (k, v) in &self.values {
            let v = if k == key {
                seen = true;
                value
            } else {
                *v
            };
            list.push(json!({"k": k, "v": v}));
        }
        for (k, v) in &self.other {
            list.push(json!({"k": k, "v": v}));
        }
        if !seen {
            list.push(json!({"k": key, "v": value}));
        }
        RawValue::Str(compact(&list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The r2579a value from device-model.md 4.2 (truncated after 14 entries).
    const R2579A: &str = r#"[{"k":"StainIdentify","v":1},{"k":"MopFullyScalable","v":1},{"k":"SuperWash","v":0},{"k":"RobotCarpetPressEnable","v":1},{"k":"SmartAutoMop","v":-1},{"k":"CarpetFineClean","v":1},{"k":"ExtrFreq","v":2},{"k":"AutoDry","v":1},{"k":"LessColl","v":1},{"k":"FillinLight","v":1},{"k":"CleanType","v":0},{"k":"MonitorHumanFollow","v":0},{"k":"MopEffectSwitch","v":1},{"k":"FluctuationTestResult","v":0}]"#;

    #[test]
    fn decodes_the_r2579a_list() {
        let s = AutoSwitchSettings::decode(&RawValue::from_cache(R2579A)).unwrap();
        assert_eq!(s.values.len(), 14);
        assert_eq!(s.get(keys::EXTR_FREQ), Some(2));
        assert_eq!(s.get(keys::SMART_AUTO_MOP), Some(-1));
        assert_eq!(s.get(keys::CLEAN_TYPE), Some(0));
        assert_eq!(s.get("MopFullyScalable"), Some(1));
        // The relay sends the same JSON as a string.
        assert_eq!(AutoSwitchSettings::decode(&RawValue::Str(R2579A.into())).unwrap(), s);
    }

    #[test]
    fn single_object_and_unknown_values() {
        let s = AutoSwitchSettings::decode(&RawValue::Str(r#"{"k":"AutoDry","v":0}"#.into())).unwrap();
        assert_eq!(s.get("AutoDry"), Some(0));
        let s = AutoSwitchSettings::decode(&RawValue::Str(r#"[{"k":"X","v":"text"}]"#.into())).unwrap();
        assert_eq!(s.other["X"], json!("text"));
        assert!(AutoSwitchSettings::decode(&RawValue::Int(3)).is_err());
    }

    #[test]
    fn encoders() {
        assert_eq!(AutoSwitchSettings::encode_single("AutoDry", 0), RawValue::Str(r#"{"k":"AutoDry","v":0}"#.into()));
        let s = AutoSwitchSettings::decode(&RawValue::Str(r#"[{"k":"A","v":1},{"k":"B","v":2}]"#.into())).unwrap();
        let full = s.encode_full_with("B", 5);
        assert_eq!(full, RawValue::Str(r#"[{"k":"A","v":1},{"k":"B","v":5}]"#.into()));
        let back = AutoSwitchSettings::decode(&full).unwrap();
        assert_eq!(back.get("B"), Some(5));
        let added = AutoSwitchSettings::decode(&s.encode_full_with("C", 3)).unwrap();
        assert_eq!(added.get("C"), Some(3));
    }
}
