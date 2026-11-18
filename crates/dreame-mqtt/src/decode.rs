//! Decoding of push payloads into [`Push`] values (protocols.md sections 11.1b, 11.3 and 11.4).
//!
//! This module has no I/O, so it can be tested on captured payloads.

use dreame_core::{Error, PropId, RawValue, Result};
use serde_json::{Map, Value};

/// One decoded push message.
#[derive(Debug, Clone, PartialEq)]
pub enum Push {
    /// `properties_changed`: new values, in the order the robot sent them.
    PropertiesChanged(Vec<(PropId, RawValue)>),
    /// `event_occured` (sic). `arguments` holds `(piid, value)` pairs.
    Event { siid: u32, eiid: u32, arguments: Vec<(u32, RawValue)> },
    /// `props`: free key and value pairs, for example `{"ota_progress":47}`.
    Props(Map<String, Value>),
    /// `_otc.info`: the robot's network self-report. `raw` holds the whole
    /// `params` object; it contains the IP address and access point details,
    /// so do not log it.
    OtcInfo {
        fw_ver: Option<String>,
        hw_ver: Option<String>,
        model: Option<String>,
        local_ip: Option<String>,
        rssi: Option<i64>,
        raw: Value,
    },
    /// Any other message, as the complete payload.
    Other(Value),
}

/// Decodes one MQTT payload.
///
/// Most messages are wrapped as `{"id","did","data":{"id","method","params"}}`.
/// A payload without `data` is read as the inner message itself. The
/// top-level `did` may be a number or a string; it is not used.
///
/// Usually the result has one item. An `event_occured` message whose
/// `params` is an array yields one [`Push::Event`] per element. Malformed
/// items inside a known message are skipped. The function fails only when
/// the payload is not JSON.
pub fn decode_message(payload: &[u8]) -> Result<Vec<Push>> {
    let v: Value = serde_json::from_slice(payload).map_err(|e| Error::Decode(format!("mqtt payload: {e}")))?;
    Ok(decode_value(v))
}

/// Decodes a payload that is already parsed as JSON.
pub fn decode_value(v: Value) -> Vec<Push> {
    let msg = match v.get("data") {
        Some(d) if d.is_object() => d,
        _ => &v,
    };
    let params = msg.get("params").unwrap_or(&Value::Null);
    match msg.get("method").and_then(Value::as_str) {
        Some("properties_changed") => {
            let items = params.as_array().map(Vec::as_slice).unwrap_or_default();
            let changes: Vec<_> = items
                .iter()
                .filter_map(|p| {
                    let id = PropId::new(as_u32(p.get("siid")?)?, as_u32(p.get("piid")?)?);
                    Some((id, RawValue::from_json(p.get("value").unwrap_or(&Value::Null))))
                })
                .collect();
            vec![Push::PropertiesChanged(changes)]
        }
        Some("event_occured") => match params {
            Value::Array(a) => a.iter().filter_map(event).collect(),
            p => event(p).into_iter().collect(),
        },
        Some("props") => match params {
            Value::Object(m) => vec![Push::Props(m.clone())],
            _ => vec![Push::Other(v)],
        },
        Some("_otc.info") => {
            let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
            vec![Push::OtcInfo {
                fw_ver: s("fw_ver"),
                hw_ver: s("hw_ver"),
                model: s("model"),
                local_ip: params.pointer("/netif/localIp").and_then(Value::as_str).map(str::to_string),
                rssi: params.pointer("/ap/rssi").and_then(as_i64),
                raw: params.clone(),
            }]
        }
        _ => vec![Push::Other(v)],
    }
}

fn event(p: &Value) -> Option<Push> {
    let siid = as_u32(p.get("siid")?)?;
    let eiid = as_u32(p.get("eiid")?)?;
    let arguments = p
        .get("arguments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| Some((as_u32(x.get("piid")?)?, RawValue::from_json(x.get("value").unwrap_or(&Value::Null)))))
                .collect()
        })
        .unwrap_or_default();
    Some(Push::Event { siid, eiid, arguments })
}

/// Reads an id that may be a JSON number or a numeric string.
fn as_u32(v: &Value) -> Option<u32> {
    as_i64(v).and_then(|i| u32::try_from(i).ok())
}

fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode(v: Value) -> Vec<Push> {
        decode_message(v.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn properties_changed_with_numeric_did() {
        let p = decode(json!({"id":4009,"did":1234567,"data":{"id":4009,"method":"properties_changed",
            "params":[{"did":"1234567","siid":4,"piid":64,"value":60},
                      {"did":"1234567","siid":6,"piid":3,"value":"[\"obj\",0]"}]}}));
        assert_eq!(
            p,
            vec![Push::PropertiesChanged(vec![
                (PropId::new(4, 64), RawValue::Int(60)),
                (PropId::new(6, 3), RawValue::Str("[\"obj\",0]".into())),
            ])]
        );
    }

    #[test]
    fn properties_changed_with_string_did() {
        let p = decode(json!({"id":92,"did":"1234567","data":{"id":92,"method":"properties_changed",
            "params":[{"did":"1234567","siid":2,"piid":6,"value":1}]}}));
        assert_eq!(p, vec![Push::PropertiesChanged(vec![(PropId::new(2, 6), RawValue::Int(1))])]);
    }

    #[test]
    fn event_params_is_an_object() {
        let p = decode(json!({"id":4210,"did":1234567,"data":{"id":4210,"method":"event_occured",
            "params":{"did":"1234567","siid":4,"eiid":1,"arguments":[
                {"piid":1,"value":18},{"piid":2,"value":4},{"piid":13,"value":1},
                {"piid":9,"value":"ali_dreame/2026/01/01/1/2_3.0.bin"}]}}}));
        assert_eq!(
            p,
            vec![Push::Event {
                siid: 4,
                eiid: 1,
                arguments: vec![
                    (1, RawValue::Int(18)),
                    (2, RawValue::Int(4)),
                    (13, RawValue::Int(1)),
                    (9, RawValue::Str("ali_dreame/2026/01/01/1/2_3.0.bin".into())),
                ],
            }]
        );
    }

    #[test]
    fn event_without_arguments() {
        let p = decode(json!({"id":4211,"did":1234567,"data":{"id":4211,"method":"event_occured",
            "params":{"did":"1234567","siid":2,"eiid":3,"arguments":[]}}}));
        assert_eq!(p, vec![Push::Event { siid: 2, eiid: 3, arguments: vec![] }]);
    }

    #[test]
    fn event_params_as_array_is_tolerated() {
        let p = decode(json!({"data":{"method":"event_occured","params":[{"siid":4,"eiid":3},{"siid":4,"eiid":4}]}}));
        assert_eq!(p.len(), 2);
    }

    fn otc_params() -> Value {
        json!({"hw_ver":"Linux","fw_ver":"4.3.9_1234","model":"dreame.vacuum.x",
            "ap":{"siid":"ssid","bssid":"00:00:00:00:00:00","rssi":-52},
            "netif":{"localIp":"192.0.2.10","mask":"255.255.255.0","gw":"192.0.2.1"}})
    }

    #[test]
    fn otc_info_inside_data_without_id() {
        let p = decode(json!({"id":4100,"did":1234567,"data":{"partner_id":"","params":otc_params(),"method":"_otc.info"}}));
        assert_eq!(
            p,
            vec![Push::OtcInfo {
                fw_ver: Some("4.3.9_1234".into()),
                hw_ver: Some("Linux".into()),
                model: Some("dreame.vacuum.x".into()),
                local_ip: Some("192.0.2.10".into()),
                rssi: Some(-52),
                raw: otc_params(),
            }]
        );
    }

    #[test]
    fn otc_info_at_top_level() {
        let p = decode(json!({"partner_id":"","params":otc_params(),"method":"_otc.info"}));
        assert!(matches!(&p[0], Push::OtcInfo { rssi: Some(-52), .. }));
    }

    #[test]
    fn props_and_other() {
        let p = decode(json!({"id":1,"did":"1","data":{"id":1,"method":"props","params":{"ota_progress":47}}}));
        assert!(matches!(&p[0], Push::Props(m) if m["ota_progress"] == 47));
        let v = json!({"id":2,"did":"1","data":{"id":2,"method":"_sync.update_vacuum_mapinfo","params":{}}});
        assert_eq!(decode(v.clone()), vec![Push::Other(v)]);
    }

    #[test]
    fn malformed_items_are_skipped_and_bad_json_fails() {
        let p = decode(json!({"data":{"method":"properties_changed","params":[{"siid":"2","piid":1,"value":true},{"value":1}]}}));
        assert_eq!(p, vec![Push::PropertiesChanged(vec![(PropId::new(2, 1), RawValue::Bool(true))])]);
        assert!(decode_message(b"not json").is_err());
    }
}
