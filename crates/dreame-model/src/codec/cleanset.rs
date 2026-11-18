//! Per-room clean settings: map JSON `cleanset` and the `customeClean` edit
//! (device-model.md 4.10). Shape: `{"<segment>": [suction, water, repeats,
//! order, cleaning_mode?, mopping_settings?, ...]}`.

use super::{compact, decode_err};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use dreame_core::Result;

/// The three nibbles of `mopping_settings`, `n0` lowest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoppingNibbles {
    pub n0: u8,
    pub n1: u8,
    pub n2: u8,
}

/// The meaning of the nibbles (T/map:6602-6615).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoppingReading {
    /// `n2 == 0`: custom route on, route `n0 - 1`, water `n1`.
    CustomRoute { route: i8, water: u8 },
    /// `n2` in 1..3: the water level or route.
    Level(u8),
    Other,
}

impl MoppingNibbles {
    pub fn split(v: i64) -> Self {
        Self { n0: (v & 0xF) as u8, n1: ((v >> 4) & 0xF) as u8, n2: ((v >> 8) & 0xF) as u8 }
    }
    pub fn value(self) -> i64 {
        (self.n0 as i64) | (self.n1 as i64) << 4 | (self.n2 as i64) << 8
    }
    pub fn reading(self) -> MoppingReading {
        match self.n2 {
            0 => MoppingReading::CustomRoute { route: self.n0 as i8 - 1, water: self.n1 },
            1..=3 => MoppingReading::Level(self.n2),
            _ => MoppingReading::Other,
        }
    }
}

/// One room's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCleanSet {
    pub suction: i64,
    /// Water or wetness field. On r2579a this holds 3, which looks like a
    /// 1..3 level, not wetness 1..32. Verify before writing.
    pub water: i64,
    pub repeats: i64,
    pub order: i64,
    pub cleaning_mode: Option<i64>,
    pub mopping_settings: Option<i64>,
    /// Items after index 5, kept as sent.
    pub extra: Vec<Value>,
}

impl RoomCleanSet {
    pub fn from_list(items: &[Value]) -> Result<Self> {
        let n = |i: usize| items.get(i).and_then(Value::as_i64);
        let need = |i: usize| n(i).ok_or_else(|| decode_err("cleanset item", items));
        Ok(Self {
            suction: need(0)?,
            water: need(1)?,
            repeats: need(2)?,
            order: need(3)?,
            cleaning_mode: n(4),
            mopping_settings: n(5),
            extra: items.iter().skip(6).cloned().collect(),
        })
    }

    pub fn nibbles(&self) -> Option<MoppingNibbles> {
        self.mopping_settings.map(MoppingNibbles::split)
    }

    pub fn to_list(&self) -> Vec<Value> {
        let mut v: Vec<Value> = vec![self.suction.into(), self.water.into(), self.repeats.into(), self.order.into()];
        if let Some(m) = self.cleaning_mode {
            v.push(m.into());
            if let Some(s) = self.mopping_settings {
                v.push(s.into());
                v.extend(self.extra.iter().cloned());
            }
        }
        v
    }
}

/// Decodes a `cleanset` object keyed by segment id.
pub fn decode(v: &Value) -> Result<BTreeMap<u32, RoomCleanSet>> {
    let obj = v.as_object().ok_or_else(|| decode_err("cleanset", v))?;
    let mut out = BTreeMap::new();
    for (k, items) in obj {
        let seg: u32 = k.parse().map_err(|_| decode_err("cleanset key", k))?;
        let list = items.as_array().ok_or_else(|| decode_err("cleanset item", items))?;
        out.insert(seg, RoomCleanSet::from_list(list)?);
    }
    Ok(out)
}

/// Encodes a cleanset object as compact JSON text.
pub fn encode(rooms: &BTreeMap<u32, RoomCleanSet>) -> String {
    let mut m = Map::new();
    for (k, r) in rooms {
        m.insert(k.to_string(), Value::Array(r.to_list()));
    }
    compact(&m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn r2579a_room() {
        // Room 1 on r2579a: [1, 3, 1, 2, 2, 546] (verified, live map JSON).
        let rooms = decode(&json!({"1": [1, 3, 1, 2, 2, 546], "2": [1, 3, 1, 1, 2, 546]})).unwrap();
        let r = &rooms[&1];
        assert_eq!((r.suction, r.water, r.repeats, r.order), (1, 3, 1, 2));
        assert_eq!(r.cleaning_mode, Some(2));
        let n = r.nibbles().unwrap();
        assert_eq!((n.n0, n.n1, n.n2), (2, 2, 2));
        assert_eq!(n.reading(), MoppingReading::Level(2));
        assert_eq!(n.value(), 546);
        assert_eq!(encode(&rooms), r#"{"1":[1,3,1,2,2,546],"2":[1,3,1,1,2,546]}"#);
    }

    #[test]
    fn custom_route_reading_and_short_lists() {
        assert_eq!(MoppingNibbles::split(0x032).reading(), MoppingReading::CustomRoute { route: 1, water: 3 });
        let rooms = decode(&json!({"5": [0, 16, 2, 1]})).unwrap();
        assert_eq!(rooms[&5].cleaning_mode, None);
        assert_eq!(rooms[&5].to_list(), vec![json!(0), json!(16), json!(2), json!(1)]);
        assert!(decode(&json!({"x": [1, 2, 3, 4]})).is_err());
        assert!(decode(&json!({"1": [1, 2]})).is_err());
    }
}
