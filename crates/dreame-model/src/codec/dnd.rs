//! Do not disturb: the single window in 5.1-5.3 and the task list in 5.4
//! (device-model.md 2.5). r2579a uses the single window.

use super::{boolean, compact, decode_err, json, HhMm};
use dreame_core::{PropId, RawValue, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const DND_ENABLED: PropId = PropId::new(5, 1);
pub const DND_START: PropId = PropId::new(5, 2);
pub const DND_END: PropId = PropId::new(5, 3);
pub const DND_TASK: PropId = PropId::new(5, 4);

/// The single DND window. Fields are filled one property at a time, so
/// each may be missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DndWindow {
    pub enabled: Option<bool>,
    pub start: Option<HhMm>,
    pub end: Option<HhMm>,
}

impl DndWindow {
    /// Applies one of 5.1, 5.2 or 5.3. Returns false for any other id.
    pub fn apply(&mut self, id: PropId, v: &RawValue) -> Result<bool> {
        match id {
            DND_ENABLED => self.enabled = Some(boolean(v)?),
            DND_START => self.start = Some(HhMm::decode(v)?),
            DND_END => self.end = Some(HhMm::decode(v)?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The three property writes for this window.
    pub fn encode(enabled: bool, start: HhMm, end: HhMm) -> [(PropId, RawValue); 3] {
        [
            (DND_ENABLED, RawValue::Bool(enabled)),
            (DND_START, RawValue::Str(start.to_string())),
            (DND_END, RawValue::Str(end.to_string())),
        ]
    }
}

/// One entry of the 5.4 task list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DndTask {
    pub id: i64,
    #[serde(rename = "en")]
    pub enabled: bool,
    #[serde(rename = "st")]
    pub start: HhMm,
    #[serde(rename = "et")]
    pub end: HhMm,
    /// 7-bit weekday mask.
    #[serde(rename = "wk")]
    pub weekdays: u8,
    #[serde(rename = "ss", default)]
    pub ss: i64,
    /// Fields not listed above.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Decodes 5.4.
pub fn decode_tasks(v: &RawValue) -> Result<Vec<DndTask>> {
    let j = json(v)?;
    if !j.is_array() {
        return Err(decode_err("5.4", v));
    }
    serde_json::from_value(j).map_err(|e| decode_err("5.4", e))
}

/// Encodes 5.4 as compact JSON text.
pub fn encode_tasks(tasks: &[DndTask]) -> RawValue {
    RawValue::Str(compact(&tasks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_window() {
        // 5.1-5.3 on r2579a: false, 22:00, 08:00 (cache strings).
        let mut w = DndWindow::default();
        assert!(w.apply(DND_ENABLED, &RawValue::from_cache("false")).unwrap());
        assert!(w.apply(DND_START, &RawValue::from_cache("22:00")).unwrap());
        assert!(w.apply(DND_END, &RawValue::from_cache("08:00")).unwrap());
        assert!(!w.apply(PropId::new(5, 9), &RawValue::Int(0)).unwrap());
        assert_eq!(w.enabled, Some(false));
        assert_eq!(w.start.unwrap().to_string(), "22:00");
        assert_eq!(w.end.unwrap().to_string(), "08:00");
        let writes = DndWindow::encode(true, w.start.unwrap(), w.end.unwrap());
        assert_eq!(writes[0], (DND_ENABLED, RawValue::Bool(true)));
        assert_eq!(writes[1].1, RawValue::Str("22:00".into()));
    }

    #[test]
    fn task_list_round_trip() {
        let text = r#"[{"id":1,"en":true,"st":"22:00","et":"08:00","wk":127,"ss":0}]"#;
        let tasks = decode_tasks(&RawValue::Str(text.into())).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].weekdays, 127);
        assert!(tasks[0].enabled);
        assert_eq!(encode_tasks(&tasks), RawValue::Str(text.into()));
        assert!(decode_tasks(&RawValue::Str("{}".into())).is_err());
    }
}
