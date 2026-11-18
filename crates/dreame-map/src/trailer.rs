//! The JSON trailer that follows the pixel grid (maps.md §7 and §8).
//!
//! Every struct keeps unknown keys in a flattened `extra` map, so a decode followed by an encode
//! loses nothing. [`Trailer::from_value`] is lenient: a known key whose value does not fit its
//! typed field is moved to `extra` unchanged and listed in [`Trailer::unparsed`].
//!
//! Positions are integer mm unless noted. Values that some firmware sends as JSON text inside a
//! string (`robot`, `charger`, `cleanset`, `mra`, `curid`) use [`Stringly`], which remembers the
//! form it arrived in.

use crate::{decode_err, Frame, Result};
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map as JsonMap, Value};
use std::collections::BTreeMap;

/// Unknown keys of a JSON object.
pub type Extra = JsonMap<String, Value>;

/// An axis-aligned rectangle or a line: `[x0, y0, x1, y1]` in mm.
pub type Quad = [i64; 4];

/// A stored rectangle or line entry: `[x0, y0, x1, y1, ...]`.
///
/// The robot may store more than four fields. r2579a turned a sent `vw.rect` entry
/// `[x0, y0, x1, y1]` into `[x0, y0, x1, y1, 0]`, and the app reads a delete flag at index 4 or 5
/// (map-plugin.md §6.3). The fields after the coordinates are kept as stored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuadEntry {
    pub quad: Quad,
    pub rest: Vec<Value>,
    /// True when the coordinates were stored as decimals (`100.0`); written back the same way.
    pub float_coords: bool,
}

/// A whole number stored as an integer or as a decimal with no fraction (`-400.0`).
pub fn whole(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| {
        let f = v.as_f64()?;
        (f.fract() == 0.0 && f.abs() < 1e15).then_some(f as i64)
    })
}

fn coord_value(x: i64, float: bool) -> Value {
    if float {
        serde_json::Number::from_f64(x as f64).map_or(Value::from(x), Value::Number)
    } else {
        Value::from(x)
    }
}

impl From<Quad> for QuadEntry {
    fn from(quad: Quad) -> Self {
        Self { quad, rest: Vec::new(), float_coords: false }
    }
}

impl QuadEntry {
    /// The entry as a JSON array: the coordinates, then the kept fields.
    pub fn to_value(&self) -> Value {
        let mut v: Vec<Value> = self.quad.iter().map(|&x| coord_value(x, self.float_coords)).collect();
        v.extend(self.rest.iter().cloned());
        Value::Array(v)
    }
}

impl Serialize for QuadEntry {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

impl<'de> Deserialize<'de> for QuadEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = Vec::<Value>::deserialize(d)?;
        if v.len() < 4 {
            return Err(serde::de::Error::custom("an entry needs at least 4 fields"));
        }
        let mut quad = [0i64; 4];
        for (i, q) in quad.iter_mut().enumerate() {
            *q = whole(&v[i]).ok_or_else(|| serde::de::Error::custom(format!("entry field {i} is not a whole number")))?;
        }
        let float_coords = v[..4].iter().any(|x| x.as_i64().is_none());
        Ok(Self { quad, rest: v[4..].to_vec(), float_coords })
    }
}

// ---------------------------------------------------------------------------------------------
// Stringly

/// A value that may arrive as JSON or as JSON text inside a string.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stringly<T> {
    pub value: T,
    /// True when the wire carried JSON text in a string. Serialization keeps that form.
    pub as_string: bool,
}

impl<T> Stringly<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            as_string: false,
        }
    }
}

impl<T: Serialize> Serialize for Stringly<T> {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        if self.as_string {
            let text = serde_json::to_string(&self.value).map_err(serde::ser::Error::custom)?;
            s.serialize_str(&text)
        } else {
            self.value.serialize(s)
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Stringly<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v {
            Value::String(s) => {
                let value = serde_json::from_str(&s).map_err(serde::de::Error::custom)?;
                Ok(Self {
                    value,
                    as_string: true,
                })
            }
            other => {
                let value = serde_json::from_value(other).map_err(serde::de::Error::custom)?;
                Ok(Self {
                    value,
                    as_string: false,
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rooms

/// One entry of `seg_inf` (maps.md §7.4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SegInfo {
    /// Room type code. 0 means "custom name".
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub room_type: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    /// Base64 of the UTF-8 custom name. Use [`SegInfo::name_text`] and [`SegInfo::set_name_text`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Floor material code (0 none, 1 wood, 2 tile, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<i64>,
    /// Wood grain direction, degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<i64>,
    /// Neighbour room ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nei_id: Option<Vec<u32>>,
    #[serde(rename = "roomID", default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl SegInfo {
    /// The decoded custom name, if present and valid UTF-8 base64.
    pub fn name_text(&self) -> Option<String> {
        let b = crate::envelope::base64_decode(self.name.as_deref()?).ok()?;
        String::from_utf8(b).ok()
    }

    /// Stores `text` as base64 UTF-8.
    pub fn set_name_text(&mut self, text: &str) {
        self.name = Some(encode_name(text));
    }
}

/// Base64 (standard alphabet) of the UTF-8 text, as `seg_inf.name` and `nsr` use it.
pub fn encode_name(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Restrictions

/// A user-added carpet rectangle from `vw.addcpt`: `[x0, y0, x1, y1, id, ellipse, type, ...]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AddedCarpet {
    pub rect: Quad,
    /// True when the robot stored the four coordinates as decimals (`-400.0`). r2579a does this
    /// for round carpets. They are written back the same way.
    pub float_coords: bool,
    pub id: i64,
    /// 1 for an ellipse inscribed in the rectangle.
    pub ellipse: i64,
    /// Carpet type (7 generic on r2579a). Absent in the 6-field form.
    pub kind: Option<i64>,
    /// Further fields, kept as sent.
    pub rest: Vec<Value>,
}

impl Serialize for AddedCarpet {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut v: Vec<Value> = self.rect.iter().map(|&x| coord_value(x, self.float_coords)).collect();
        v.push(self.id.into());
        v.push(self.ellipse.into());
        if let Some(k) = self.kind {
            v.push(k.into());
        }
        v.extend(self.rest.iter().cloned());
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for AddedCarpet {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = Vec::<Value>::deserialize(d)?;
        let int = |i: usize| -> std::result::Result<i64, D::Error> {
            v.get(i)
                .and_then(Value::as_i64)
                .ok_or_else(|| serde::de::Error::custom(format!("addcpt field {i}")))
        };
        if v.len() < 4 {
            return Err(serde::de::Error::custom("addcpt needs at least 4 fields"));
        }
        let coord = |i: usize| -> std::result::Result<i64, D::Error> {
            whole(&v[i]).ok_or_else(|| serde::de::Error::custom(format!("addcpt field {i} is not a whole number")))
        };
        Ok(Self {
            rect: [coord(0)?, coord(1)?, coord(2)?, coord(3)?],
            float_coords: v[..4].iter().any(|x| x.as_i64().is_none()),
            id: if v.len() > 4 { int(4)? } else { 0 },
            ellipse: if v.len() > 5 { int(5)? } else { 0 },
            kind: if v.len() > 6 { Some(int(6)?) } else { None },
            rest: v.iter().skip(7).cloned().collect(),
        })
    }
}

/// `vw`: virtual walls, no-go and no-mop zones, user carpets (maps.md §7.5).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VirtualWalls {
    /// Virtual walls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Vec<QuadEntry>>,
    /// No-go zones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Vec<QuadEntry>>,
    /// No-mop zones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mop: Option<Vec<QuadEntry>>,
    /// Cliff lines; shape not confirmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cliff: Option<Vec<Vec<i64>>>,
    /// User-added carpet rectangles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addcpt: Option<Vec<AddedCarpet>>,
    /// "Ignore carpet here" areas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nocpt: Option<Vec<QuadEntry>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `vws`: thresholds and ramps.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VirtualThresholds {
    /// Virtual thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vwsl: Option<Vec<QuadEntry>>,
    /// Impassable thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npthrsd: Option<Vec<QuadEntry>>,
    /// Ramps `[x0, y0, x1, y1, angle]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp: Option<Vec<Vec<i64>>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `ct` (or `curtain`): curtain lines.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Curtains {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Vec<QuadEntry>>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `ct` means two things: a curtain object in saved maps and a number (cleaning time) in live and
/// history frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CtValue {
    Time(serde_json::Number),
    Curtains(Curtains),
}

impl CtValue {
    pub fn curtains(&self) -> Option<&Curtains> {
        match self {
            Self::Curtains(c) => Some(c),
            Self::Time(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Floor plan

/// `walls_info` in its object form (maps.md §8).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WallsInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_flag: Option<i64>,
    #[serde(default)]
    pub storeys: Vec<Storey>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Storey {
    #[serde(default)]
    pub rooms: Vec<FloorRoom>,
    #[serde(default)]
    pub doors: Vec<FloorDoor>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One room of the floor plan: a closed ring of walls.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FloorRoom {
    pub room_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_type: Option<i64>,
    #[serde(default)]
    pub walls: Vec<FloorWall>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One wall segment. `kind` 0 looks interior and 1 exterior (an inference, maps.md §8.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FloorWall {
    #[serde(rename = "type")]
    pub kind: i64,
    pub beg_pt_x: i64,
    pub beg_pt_y: i64,
    pub end_pt_x: i64,
    pub end_pt_y: i64,
    #[serde(default)]
    pub normal_x: f64,
    #[serde(default)]
    pub normal_y: f64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One door. `door_id` and `nei_door_id` are the two room ids it connects (maps.md §8.1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FloorDoor {
    #[serde(default)]
    pub door_type: i64,
    pub door_id: u32,
    #[serde(default)]
    pub nei_door_id: u32,
    #[serde(default)]
    pub door_status: i64,
    pub beg_pt_x: i64,
    pub beg_pt_y: i64,
    pub end_pt_x: i64,
    pub end_pt_y: i64,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------------------------------------
// Carpets and furniture

/// One `carpet_polygon` entry: `[[x0, y0, x1, y1, ...], type(, hidden)]`. Vertices are float mm.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CarpetPolygon {
    /// Flat vertex list `x0, y0, x1, y1, ...`.
    pub points: Vec<f64>,
    pub kind: i64,
    pub hidden: Option<i64>,
    pub rest: Vec<Value>,
}

impl CarpetPolygon {
    /// Vertices as `(x, y)` pairs.
    pub fn vertices(&self) -> Vec<(f64, f64)> {
        (0..self.points.len() / 2)
            .map(|i| (self.points[2 * i], self.points[2 * i + 1]))
            .collect()
    }
}

impl Serialize for CarpetPolygon {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut v = vec![
            serde_json::to_value(&self.points).map_err(serde::ser::Error::custom)?,
            self.kind.into(),
        ];
        if let Some(h) = self.hidden {
            v.push(h.into());
        }
        v.extend(self.rest.iter().cloned());
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for CarpetPolygon {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = Vec::<Value>::deserialize(d)?;
        let points: Vec<f64> = v
            .first()
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .ok_or_else(|| serde::de::Error::custom("carpet polygon without points"))?;
        let int = |i: usize| {
            v.get(i)
                .map(|x| {
                    x.as_i64()
                        .ok_or_else(|| serde::de::Error::custom("carpet field"))
                })
                .transpose()
        };
        Ok(Self {
            points,
            kind: int(1)?.unwrap_or(0),
            hidden: int(2)?,
            rest: v.iter().skip(3).cloned().collect(),
        })
    }
}

/// Number of named fields in a `funiture_info` entry.
pub const FURNITURE_FIELDS: usize = 15;
/// Fields written as floats by the robot (indices 6 to 12).
pub const FURNITURE_FLOAT_MASK: u16 = 0b0001_1111_1100_0000;

/// One `funiture_info` entry (maps.md §7.8). Field meanings follow Tasshack.
#[derive(Debug, Clone, PartialEq)]
pub struct FurnitureEntry {
    /// The 15 fields as numbers: id, type (wire code), segment, width, height, 0, centre x,
    /// centre y, 0, angle, 0, 0, scale, edit type, modified.
    pub fields: [f64; FURNITURE_FIELDS],
    /// Bit i set means field i is written as a float. Keeps the wire form on re-encode.
    pub float_mask: u16,
    /// Fields after the 15th, kept as sent.
    pub rest: Vec<Value>,
}

impl Default for FurnitureEntry {
    fn default() -> Self {
        let mut fields = [0.0; FURNITURE_FIELDS];
        fields[12] = 1.0;
        Self {
            fields,
            float_mask: FURNITURE_FLOAT_MASK,
            rest: Vec::new(),
        }
    }
}

impl FurnitureEntry {
    pub fn id(&self) -> i64 {
        self.fields[0] as i64
    }
    /// Wire type code. Codes 8 and 25 are swapped against Tasshack's `FurnitureType`.
    pub fn kind(&self) -> i64 {
        self.fields[1] as i64
    }
    pub fn segment(&self) -> i64 {
        self.fields[2] as i64
    }
    pub fn width(&self) -> f64 {
        self.fields[3]
    }
    pub fn height(&self) -> f64 {
        self.fields[4]
    }
    pub fn center(&self) -> (f64, f64) {
        (self.fields[6], self.fields[7])
    }
    pub fn angle(&self) -> f64 {
        self.fields[9]
    }
    pub fn scale(&self) -> f64 {
        self.fields[12]
    }
    pub fn edit_type(&self) -> i64 {
        self.fields[13] as i64
    }
    pub fn modified(&self) -> i64 {
        self.fields[14] as i64
    }

    /// The entry as JSON numbers, ints and floats as recorded in `float_mask`.
    pub fn to_values(&self) -> Vec<Value> {
        let mut v: Vec<Value> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                if self.float_mask & (1 << i) != 0 {
                    serde_json::Number::from_f64(x)
                        .map(Value::Number)
                        .unwrap_or(Value::Null)
                } else {
                    Value::from(x.round() as i64)
                }
            })
            .collect();
        v.extend(self.rest.iter().cloned());
        v
    }
}

impl Serialize for FurnitureEntry {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        self.to_values().serialize(s)
    }
}

impl<'de> Deserialize<'de> for FurnitureEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = Vec::<Value>::deserialize(d)?;
        if v.len() < FURNITURE_FIELDS {
            return Err(serde::de::Error::custom(format!(
                "funiture_info entry has {} fields",
                v.len()
            )));
        }
        let mut fields = [0.0; FURNITURE_FIELDS];
        let mut float_mask = 0u16;
        for (i, x) in v.iter().take(FURNITURE_FIELDS).enumerate() {
            let Value::Number(n) = x else {
                return Err(serde::de::Error::custom(format!(
                    "funiture_info field {i} is not a number"
                )));
            };
            if n.is_f64() {
                float_mask |= 1 << i;
            }
            fields[i] = n.as_f64().unwrap_or(0.0);
        }
        Ok(Self {
            fields,
            float_mask,
            rest: v[FURNITURE_FIELDS..].to_vec(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Trailer

/// The typed JSON trailer. Every field is optional: keys vary by firmware and frame type.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Trailer {
    /// Frame time, epoch ms (or uptime after a restart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<i64>,
    /// `[left, top]`; overrides the header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<[i64; 2]>,
    /// Display rotation in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mra: Option<Stringly<i64>>,
    /// Current saved map id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curid: Option<Stringly<i64>>,
    /// 1: pixel grid uses encoding B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fsm: Option<i64>,
    /// Saved map status: 0 or 1 map being built; 2 localised on a saved map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ris: Option<i64>,
    /// 1: restored map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpur: Option<i64>,
    /// 0: new map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risp: Option<i64>,
    /// 5 or 6 (without `fsm`): temporary map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suw: Option<i64>,
    /// 1: recovery map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub us: Option<i64>,
    /// No robot position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nr: Option<i64>,
    /// No charger position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nc: Option<i64>,
    /// Frame is a cleaning-history map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iscleanlog: Option<bool>,
    /// Robot position `[x, y]`; a JSON string in live frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robot: Option<Stringly<Vec<i64>>>,
    /// Charger position `[x, y(, angle)]`; a JSON string in live frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charger: Option<Stringly<Vec<i64>>>,
    /// Path string, see [`crate::path`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tr: Option<String>,
    /// Active segments; index 0 of each entry is the room id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sa: Option<Vec<Vec<i64>>>,
    /// Hidden room ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delsr: Option<Vec<u32>>,
    /// Per-room settings `{id: [suction, water, times, order, mode, mopping, ...]}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanset: Option<Stringly<BTreeMap<u32, Vec<i64>>>>,
    /// Cleaning order `[{id: order}, ...]`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "via_value"
    )]
    pub cleanareaorder: Option<Vec<BTreeMap<u32, i64>>>,
    /// Rooms.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "via_value"
    )]
    pub seg_inf: Option<BTreeMap<u32, SegInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vw: Option<VirtualWalls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vws: Option<VirtualThresholds>,
    /// Curtains in the saved map, or the cleaning time in history frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ct: Option<CtValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curtain: Option<Curtains>,
    /// Floor plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls_info: Option<WallsInfo>,
    /// Detected carpets by id.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "via_value"
    )]
    pub carpet_polygon: Option<BTreeMap<u32, CarpetPolygon>>,
    /// Furniture (spelling as on the wire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub funiture_info: Option<Vec<FurnitureEntry>>,
    /// `[[object_type, id, cleaning, prefs], ...]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carpetcleanset: Option<Vec<Vec<i64>>>,
    /// Embedded saved map envelope. Decode with [`Trailer::decode_rism`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rism: Option<String>,
    /// Embedded Wi-Fi map envelope (W-frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whm: Option<String>,
    /// Router position `[x, y]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whmp: Option<Vec<i64>>,
    /// Unknown keys and known keys whose value did not fit the typed field.
    #[serde(flatten)]
    pub extra: Extra,
    /// Known keys that failed typed parsing and were kept in `extra` instead.
    #[serde(skip)]
    pub unparsed: Vec<String>,
}

/// Deserializes through [`Value`]. Serde's flatten buffering cannot parse integer map keys from
/// strings, but `serde_json::Value` can.
fn via_value<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    let v = Value::deserialize(d)?;
    serde_json::from_value(v)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// JSON names of the typed [`Trailer`] fields.
pub const KNOWN_KEYS: &[&str] = &[
    "timestamp_ms",
    "origin",
    "mra",
    "curid",
    "fsm",
    "ris",
    "rpur",
    "risp",
    "suw",
    "us",
    "nr",
    "nc",
    "iscleanlog",
    "robot",
    "charger",
    "tr",
    "sa",
    "delsr",
    "cleanset",
    "cleanareaorder",
    "seg_inf",
    "vw",
    "vws",
    "ct",
    "curtain",
    "walls_info",
    "carpet_polygon",
    "funiture_info",
    "carpetcleanset",
    "rism",
    "whm",
    "whmp",
];

impl Trailer {
    /// Parses a JSON object leniently. Known keys that do not fit go to `extra`.
    pub fn from_value(v: Value) -> Result<Self> {
        let Value::Object(mut map) = v else {
            return Err(decode_err("trailer is not a JSON object"));
        };
        let mut bad = Extra::new();
        let mut names = Vec::new();
        for &k in KNOWN_KEYS {
            let Some(x) = map.get(k) else { continue };
            let mut one = Extra::new();
            one.insert(k.to_string(), x.clone());
            if serde_json::from_value::<Trailer>(Value::Object(one)).is_err() {
                bad.insert(k.to_string(), map.remove(k).expect("key exists"));
                names.push(k.to_string());
            }
        }
        let mut t: Trailer = serde_json::from_value(Value::Object(map))
            .map_err(|e| decode_err(format!("trailer: {e}")))?;
        t.extra.extend(bad);
        t.unparsed = names;
        Ok(t)
    }

    /// Parses trailer bytes. Empty input (after trailing NULs and whitespace) gives an empty trailer.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let end = bytes
            .iter()
            .rposition(|&b| b != 0 && !b.is_ascii_whitespace())
            .map_or(0, |i| i + 1);
        if end == 0 {
            return Ok(Self::default());
        }
        let v: Value = serde_json::from_slice(&bytes[..end])
            .map_err(|e| decode_err(format!("trailer JSON: {e}")))?;
        Self::from_value(v)
    }

    /// The trailer as a JSON value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("trailer serialization cannot fail")
    }

    /// The trailer as compact JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("trailer serialization cannot fail")
    }

    /// Decodes the embedded saved map (`rism`), if present.
    pub fn decode_rism(&self) -> Option<Result<Frame>> {
        self.rism
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Frame::decode(s, None, None))
    }

    /// Decodes the embedded Wi-Fi map (`whm`), if present.
    pub fn decode_whm(&self) -> Option<Result<Frame>> {
        self.whm
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Frame::decode(s, None, None))
    }

    /// Robot position from the trailer, `[x, y]`.
    pub fn robot_xy(&self) -> Option<(i64, i64)> {
        let v = &self.robot.as_ref()?.value;
        Some((*v.first()?, *v.get(1)?))
    }

    /// Charger position from the trailer, `[x, y]`.
    pub fn charger_xy(&self) -> Option<(i64, i64)> {
        let v = &self.charger.as_ref()?.value;
        Some((*v.first()?, *v.get(1)?))
    }

    /// True when the trailer carries `cleanareaorder`, which implies map version 2.
    pub fn has_clean_order_v2(&self) -> bool {
        self.cleanareaorder.is_some()
    }
}

/// Compares two JSON values, treating integer and float forms of the same number as equal and
/// parsing JSON text inside strings. Used to check semantic round trips.
pub fn json_semantic_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => {
            if x == y {
                return true;
            }
            match (
                serde_json::from_str::<Value>(x),
                serde_json::from_str::<Value>(y),
            ) {
                (Ok(p), Ok(q)) if p.is_object() || p.is_array() => json_semantic_eq(&p, &q),
                _ => false,
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_semantic_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_semantic_eq(v, w)))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "seg_inf": {
                "1": {"type": 0, "index": 0, "name": encode_name("Lounge"), "material": 1, "direction": 90, "nei_id": [2], "future": 7},
                "2": {"type": 4, "index": 1, "material": 2, "direction": 0, "nei_id": [1]}
            },
            "vw": {"line": [], "rect": [[0, 0, 100, 100]], "mop": [], "cliff": [], "addcpt": [[100, -200, 1100, 900, 1, 0, 7]], "nocpt": [], "newkind": [1]},
            "vws": {"vwsl": [], "npthrsd": [], "ramp": []},
            "ct": {"line": []},
            "delsr": [9, 13],
            "cleanareaorder": [{"1": 1}, {"2": 2}],
            "cleanset": "{\"1\":[1,3,1,1,2,546],\"2\":[1,3,1,2,2,546]}",
            "carpet_polygon": {"1": [[0.0, 0.0, 50.0, 0.0, 50.0, 50.0], 7]},
            "funiture_info": [[0, 2, 2, 1600, 1200, 0, 1500.0, 2500.0, 0.0, 90.0, 0.0, 0.0, 1.0, 1, 0]],
            "walls_info": {"version_flag": 3, "storeys": [{"rooms": [{"room_id": 1, "floor_type": 3, "walls": [
                {"type": 1, "beg_pt_x": 0, "beg_pt_y": 0, "end_pt_x": 100, "end_pt_y": 0, "normal_x": 0.0, "normal_y": -1.0}]}],
                "doors": [{"door_type": 0, "door_id": 1, "nei_door_id": 2, "door_status": 0, "beg_pt_x": 0, "beg_pt_y": 0, "end_pt_x": 0, "end_pt_y": 200}]}]},
            "robot": "[-300,400]",
            "charger": [-300, 400, 45],
            "mra": "0",
            "fsm": 1,
            "ris": 2,
            "iscleanlog": true,
            "tr": "S0,0L10,0",
            "ai_obstacle": [["1", "2"]],
            "unknown_key": {"deep": [1, 2.5, "x"]}
        })
    }

    #[test]
    fn round_trip_keeps_unknown_keys() {
        let v = sample();
        let t = Trailer::from_value(v.clone()).unwrap();
        assert!(t.unparsed.is_empty(), "{:?}", t.unparsed);
        assert_eq!(t.extra.get("unknown_key"), v.get("unknown_key"));
        assert_eq!(t.seg_inf.as_ref().unwrap()[&1].extra["future"], 7);
        assert_eq!(t.vw.as_ref().unwrap().extra["newkind"], json!([1]));
        let back = t.to_value();
        assert!(json_semantic_eq(&v, &back), "{back:#}");
        // Strings stay strings.
        assert!(back["robot"].is_string());
        assert!(back["cleanset"].is_string());
        assert_eq!(back["mra"], "0");
        // Furniture keeps ints and floats as sent.
        assert_eq!(back["funiture_info"], v["funiture_info"]);
        assert_eq!(
            back["funiture_info"].to_string(),
            v["funiture_info"].to_string()
        );
    }

    #[test]
    fn typed_access() {
        let t = Trailer::from_value(sample()).unwrap();
        let seg = &t.seg_inf.as_ref().unwrap()[&1];
        assert_eq!(seg.name_text().as_deref(), Some("Lounge"));
        assert_eq!(t.robot_xy(), Some((-300, 400)));
        assert_eq!(t.charger_xy(), Some((-300, 400)));
        assert_eq!(t.cleanset.as_ref().unwrap().value[&2][3], 2);
        let cpt = &t.vw.as_ref().unwrap().addcpt.as_ref().unwrap()[0];
        assert_eq!(
            (cpt.rect, cpt.id, cpt.ellipse, cpt.kind),
            ([100, -200, 1100, 900], 1, 0, Some(7))
        );
        // r2579a stores a round carpet's coordinates as decimals; they read as whole numbers
        // and are written back as decimals. A real fraction is refused.
        let round: AddedCarpet = serde_json::from_value(json!([-400.0, 1850.0, 600.0, 700.0, 3, 1, 7])).unwrap();
        assert_eq!((round.rect, round.float_coords), ([-400, 1850, 600, 700], true));
        assert_eq!(serde_json::to_value(&round).unwrap(), json!([-400.0, 1850.0, 600.0, 700.0, 3, 1, 7]));
        assert!(serde_json::from_value::<AddedCarpet>(json!([-400.5, 1850, 600, 700, 3, 1, 7])).is_err());
        let q: QuadEntry = serde_json::from_value(json!([0.0, 0, 100, 50, 0])).unwrap();
        assert_eq!(q.to_value(), json!([0.0, 0.0, 100.0, 50.0, 0]));
        let f = &t.funiture_info.as_ref().unwrap()[0];
        assert_eq!(
            (f.kind(), f.segment(), f.width(), f.center(), f.angle()),
            (2, 2, 1600.0, (1500.0, 2500.0), 90.0)
        );
        assert_eq!(t.carpet_polygon.as_ref().unwrap()[&1].vertices().len(), 3);
        assert_eq!(
            t.walls_info.as_ref().unwrap().storeys[0].doors[0].nei_door_id,
            2
        );
    }

    #[test]
    fn misfit_known_key_goes_to_extra() {
        let v = json!({"delsr": "oops", "fsm": 1});
        let t = Trailer::from_value(v.clone()).unwrap();
        assert_eq!(t.unparsed, vec!["delsr".to_string()]);
        assert!(t.delsr.is_none());
        assert_eq!(t.extra["delsr"], "oops");
        assert_eq!(t.to_value(), v);
        // `ct` is a number (cleaning time) in history frames and an object in saved maps.
        let t = Trailer::from_value(json!({"ct": 42})).unwrap();
        assert!(t.unparsed.is_empty());
        assert!(matches!(t.ct, Some(CtValue::Time(_))));
        assert_eq!(t.to_value(), json!({"ct": 42}));
        let t = Trailer::from_value(json!({"ct": {"line": []}})).unwrap();
        assert_eq!(t.ct.unwrap().curtains().unwrap().line, Some(vec![]));
    }

    #[test]
    fn unicode_names() {
        let mut s = SegInfo::default();
        s.set_name_text("Suíte 2");
        assert_eq!(s.name_text().as_deref(), Some("Suíte 2"));
    }

    #[test]
    fn empty_and_padded_slices() {
        assert_eq!(Trailer::from_slice(b"").unwrap(), Trailer::default());
        assert_eq!(
            Trailer::from_slice(b"{\"fsm\":1}\0\0").unwrap().fsm,
            Some(1)
        );
        assert!(Trailer::from_slice(b"[1]").is_err());
    }
}
