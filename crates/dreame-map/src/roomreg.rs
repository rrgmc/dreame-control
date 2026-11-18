//! The room registries inside a recovery archive, and adding a room to them.
//!
//! Besides the room grid ([`crate::segmap`]), an archive lists its rooms in several members:
//!
//! - `1.dat2`: `seg_info` (a JSON string: per room the name, type, index, floor and neighbours)
//!   and `cleanarea_order` (the cleaning order);
//! - `divide/1/room_neighbor_table_large.json`: the room ids and each room's neighbours;
//! - `ai/1/ai_rooms_large.txt`: one line per room, `id aitype 1 [base64 name]`;
//! - `ai/1/ai_floors_large.txt`: per room the floor, `wood` (with a direction) or `ceramic`.
//!
//! A room id that the grid has but these members lack loads and can be cleaned, but the robot
//! re-segments it away after its next run (W24); registered, it lasts (W29). [`add_room`] adds a
//! room to all of them and [`remove_room`] takes one out, changing nothing else: `1.dat2` keeps
//! its text outside the two values, and the other members keep their formatting. The per-room
//! floor-plan walls in `out_walls_large.json` are left alone.

use crate::archive::Archive;
use crate::segmap::Segmentation;
use crate::{decode_err, Result};
use dreame_core::Error;
use serde_json::{json, Map as JsonMap, Value};
use std::collections::BTreeSet;

pub const DAT2: &str = "1.dat2";
pub const NEIGHBOURS: &str = "divide/1/room_neighbor_table_large.json";
pub const AI_ROOMS: &str = "ai/1/ai_rooms_large.txt";
pub const HIDDEN: &str = "divide/1/hide_rooms_large.json";
pub const AI_FLOORS: &str = "ai/1/ai_floors_large.txt";

/// The floor of a new room: `seg_info` material 1 (wood, with a grain direction of 0 or 90) or 2
/// (tile). The AI floor list calls them `wood` and `ceramic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor {
    Wood { direction: i64 },
    Tile,
}

impl Floor {
    fn material(self) -> i64 {
        match self {
            Self::Wood { .. } => 1,
            Self::Tile => 2,
        }
    }

    fn direction(self) -> i64 {
        match self {
            Self::Wood { direction } => direction,
            Self::Tile => 0,
        }
    }

    fn ai_entry(self, id: u8) -> Value {
        match self {
            Self::Wood { direction } => json!({"id": id, "material": "wood", "label": 1, "direction": direction}),
            Self::Tile => json!({"id": id, "material": "ceramic", "label": 1}),
        }
    }
}

/// What a new room is called and what floor it has. `None` takes the parent's type or floor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomMeta {
    /// A custom name (room type 0).
    pub name: Option<String>,
    pub floor: Option<Floor>,
}

/// What [`add_room`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The new room's neighbours in the grid.
    pub neighbours: Vec<u8>,
    /// Its place in the cleaning order (1-based).
    pub order: i64,
    /// The members rewritten.
    pub members: Vec<&'static str>,
}

/// The byte range of the value of top-level key `key` in compact JSON text. The key must occur
/// once.
fn value_span(text: &str, key: &str) -> Result<std::ops::Range<usize>> {
    let pat = format!("\"{key}\":");
    let mut hits = text.match_indices(&pat);
    let (i, _) = hits.next().ok_or_else(|| decode_err(format!("{DAT2} has no {key}")))?;
    if hits.next().is_some() {
        return Err(Error::Unsupported(format!("{DAT2} holds \"{key}\" more than once")));
    }
    let start = i + pat.len();
    let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    values.next().ok_or_else(|| decode_err(format!("{DAT2}: no value for {key}")))?.map_err(|e| decode_err(format!("{DAT2} {key}: {e}")))?;
    Ok(start..start + values.byte_offset())
}

fn json_text(name: &str, a: &Archive) -> Result<String> {
    let b = a.get(name).ok_or_else(|| decode_err(format!("the archive has no {name}")))?;
    String::from_utf8(b.to_vec()).map_err(|e| decode_err(format!("{name}: {e}")))
}

fn parse(name: &str, text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| decode_err(format!("{name}: {e}")))
}

/// Pretty JSON with 4-space indents, as the robot writes its `divide/` members.
pub fn pretty4(v: &Value) -> String {
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    serde::Serialize::serialize(v, &mut ser).expect("serialising a Value");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

/// Adds `id` to a JSON list of room ids: in order if the list is sorted, else at the end. The
/// other entries keep their order, so [`remove_id`] undoes it.
fn add_id(list: &mut Value, id: u8) {
    if let Some(v) = list.as_array_mut() {
        let id = u64::from(id);
        if v.iter().any(|x| x.as_u64() == Some(id)) {
            return;
        }
        let ids: Vec<Option<u64>> = v.iter().map(Value::as_u64).collect();
        let at = if ids.is_sorted() { ids.partition_point(|x| x.is_some_and(|x| x < id)) } else { v.len() };
        v.insert(at, json!(id));
    }
}

/// `1.dat2`'s `seg_info` and `cleanarea_order`, and the neighbour table, decoded for an edit.
/// [`Lists::write`] splices the two `1.dat2` values back into the original text and rewrites the
/// table in the robot's layout.
struct Lists {
    dat: String,
    seg_span: std::ops::Range<usize>,
    order_span: std::ops::Range<usize>,
    seg: JsonMap<String, Value>,
    order: Vec<JsonMap<String, Value>>,
    table: Value,
}

impl Lists {
    fn read(a: &Archive) -> Result<Self> {
        let dat = json_text(DAT2, a)?;
        let seg_span = value_span(&dat, "seg_info")?;
        let seg_text: String = serde_json::from_str(&dat[seg_span.clone()]).map_err(|e| decode_err(format!("seg_info: {e}")))?;
        let seg = serde_json::from_str(&seg_text).map_err(|e| decode_err(format!("seg_info: {e}")))?;
        let order_span = value_span(&dat, "cleanarea_order")?;
        let order = serde_json::from_str(&dat[order_span.clone()]).map_err(|e| decode_err(format!("cleanarea_order: {e}")))?;
        let table = parse(NEIGHBOURS, &json_text(NEIGHBOURS, a)?)?;
        if !table["RoomNeighborTable"].is_array() {
            return Err(decode_err(format!("{NEIGHBOURS} has no RoomNeighborTable")));
        }
        Ok(Self { dat, seg_span, order_span, seg, order, table })
    }

    fn write(mut self, a: &mut Archive) -> Result<()> {
        let seg = serde_json::to_string(&serde_json::to_string(&self.seg).expect("a JSON map")).expect("a string");
        let order = serde_json::to_string(&self.order).expect("a JSON list");
        // Replace the later span first so the earlier one's offsets stay valid.
        let mut spans = [(self.seg_span, seg), (self.order_span, order)];
        spans.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
        for (r, v) in spans {
            self.dat.replace_range(r, &v);
        }
        a.set(DAT2, self.dat.into_bytes())?;
        a.set(NEIGHBOURS, pretty4(&self.table).into_bytes())
    }

    fn rows(&mut self) -> &mut Vec<Value> {
        self.table["RoomNeighborTable"].as_array_mut().expect("checked in read")
    }

    /// Adds `id` to the neighbour lists of `rooms`, in `seg_info` and the table.
    fn add_neighbour(&mut self, rooms: &[u8], id: u8) {
        for &r in rooms {
            if let Some(nei) = self.seg.get_mut(&r.to_string()).and_then(|x| x.get_mut("nei_id")) {
                add_id(nei, id);
            }
        }
        for row in self.rows() {
            if row["room_id"].as_u64().is_some_and(|x| rooms.iter().any(|&r| u64::from(r) == x)) {
                add_id(&mut row["room_neighbor_id"], id);
            }
        }
    }
}

/// Removes `id` from a JSON list of room ids.
fn remove_id(list: &mut Value, id: u8) {
    if let Some(v) = list.as_array_mut() {
        v.retain(|x| x.as_u64() != Some(u64::from(id)));
    }
}

/// Registers room `new`, already in the grid of `s`, as a room of its own: in `seg_info` with the
/// type, floor and direction of `parent` (no name; typed rooms get the next free index) unless
/// `meta` sets a name or a floor, last in the cleaning order, in the neighbour table with its grid
/// neighbours, and in the AI room and floor lists like `parent`. Its neighbours get it as a
/// neighbour too.
pub fn add_room(a: &mut Archive, s: &Segmentation, new: u8, parent: u8, meta: &RoomMeta) -> Result<Added> {
    if meta.floor.is_some_and(|f| !matches!(f.direction(), 0 | 90)) {
        return Err(Error::Unsupported("a wood floor's direction is 0 or 90".into()));
    }
    let neighbours: Vec<u8> = s.neighbours(new).into_iter().collect();
    if neighbours.is_empty() {
        return Err(Error::Unsupported(format!("room {new} has no cells or no neighbours in the grid")));
    }
    let (p, n) = (parent.to_string(), new.to_string());
    let mut l = Lists::read(a)?;
    if l.seg.contains_key(&n) {
        return Err(Error::Unsupported(format!("room {new} is already in seg_info")));
    }
    let mut entry = l.seg.get(&p).cloned().ok_or_else(|| Error::Unsupported(format!("room {parent} is not in seg_info")))?;
    let e = entry.as_object_mut().ok_or_else(|| decode_err("seg_info entry is not an object"))?;
    e.remove("name");
    e.remove("roomID");
    let kind = e.get("type").cloned();
    if kind.as_ref().and_then(Value::as_i64).is_some_and(|t| t != 0) {
        let next = l.seg.values().filter(|r| r.get("type") == kind.as_ref()).filter_map(|r| r.get("index")?.as_i64()).max().map_or(0, |i| i + 1);
        e.insert("index".into(), json!(next));
    }
    if let Some(name) = &meta.name {
        e.insert("name".into(), json!(crate::trailer::encode_name(name)));
        e.insert("type".into(), json!(0));
        e.insert("index".into(), json!(0));
    }
    if let Some(f) = meta.floor {
        e.insert("material".into(), json!(f.material()));
        e.insert("direction".into(), json!(f.direction()));
    }
    e.insert("nei_id".into(), json!(neighbours));
    l.add_neighbour(&neighbours, new);
    l.seg.insert(n.clone(), entry);
    let place = l.order.iter().flat_map(|m| m.values()).filter_map(Value::as_i64).max().unwrap_or(0) + 1;
    l.order.push(JsonMap::from_iter([(n.clone(), json!(place))]));
    add_id(&mut l.table["RoomIdAndColorLookUpTable"]["RoomIds"], new);
    l.rows().push(json!({"room_id": new, "room_neighbor_id": neighbours, "is_user_edited": true, "rec_hide_type": 0}));
    l.write(a)?;

    // The AI room list: the parent's line with the new id and no name.
    let text = json_text(AI_ROOMS, a)?;
    let line = text
        .lines()
        .find(|l| l.split_whitespace().next() == Some(p.as_str()))
        .ok_or_else(|| Error::Unsupported(format!("room {parent} is not in {AI_ROOMS}")))?;
    let mut fields: Vec<String> = line.split_whitespace().take(3).map(String::from).collect();
    fields[0] = n.clone();
    if let Some(name) = &meta.name {
        fields.push(crate::trailer::encode_name(name));
    }
    let mut out = text.clone();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{}\n", fields.join(" ")));
    a.set(AI_ROOMS, out.into_bytes())?;
    let mut members = vec![DAT2, NEIGHBOURS, AI_ROOMS];

    // The AI floor list: the chosen floor, else a copy of the parent's entry.
    if let Some(text) = a.get(AI_FLOORS).map(|b| String::from_utf8_lossy(b).into_owned()) {
        let mut v = parse(AI_FLOORS, &text)?;
        let list = v["ai_material"].as_array_mut().ok_or_else(|| decode_err(format!("{AI_FLOORS} has no ai_material")))?;
        let entry = match meta.floor {
            Some(f) => Some(f.ai_entry(new)),
            None => list.iter().find(|e| e["id"].as_u64() == Some(u64::from(parent))).cloned().map(|mut e| {
                e["id"] = json!(new);
                e
            }),
        };
        if let Some(e) = entry {
            list.retain(|e| e["id"].as_u64() != Some(u64::from(new)));
            list.push(e);
            a.set(AI_FLOORS, pretty4(&v).into_bytes())?;
            members.push(AI_FLOORS);
        }
    }

    Ok(Added { neighbours, order: place, members })
}

/// What [`remove_room`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The removed room's place in the cleaning order; later rooms moved up one place.
    pub order: Option<i64>,
    /// The members rewritten.
    pub members: Vec<&'static str>,
}

/// Takes room `room` out of the room lists after its cells went to its neighbour `into` in the
/// grid of `s`: its `seg_info` entry, cleaning-order place (later rooms move up), neighbour-table
/// row, AI room line and hidden-room entry go, and every neighbour list drops it. No new
/// neighbour pairs are added: the lists do not follow grid adjacency exactly, and the robot
/// recomputes them after a run (W29).
pub fn remove_room(a: &mut Archive, s: &Segmentation, room: u8, into: u8) -> Result<Removed> {
    if into == room || !seg_info_rooms(a)?.contains(&into) {
        return Err(Error::Unsupported(format!("room {into} is not in seg_info")));
    }
    forget_room(a, s, room)
}

/// Takes room `room`, which has no cells left in the grid of `s`, out of the room lists, like
/// [`remove_room`]. Used when an erase leaves a room without floor.
pub fn forget_room(a: &mut Archive, s: &Segmentation, room: u8) -> Result<Removed> {
    if s.room_sizes().contains_key(&room) {
        return Err(Error::Unsupported(format!("room {room} still has cells in the grid")));
    }
    let r = room.to_string();
    let mut l = Lists::read(a)?;
    if l.seg.remove(&r).is_none() {
        return Err(Error::Unsupported(format!("room {room} is not in seg_info")));
    }
    for e in l.seg.values_mut() {
        if let Some(nei) = e.get_mut("nei_id") {
            remove_id(nei, room);
        }
    }
    let order = l.order.iter().find_map(|m| m.get(&r)?.as_i64());
    l.order.retain(|m| !m.contains_key(&r));
    if let Some(p) = order {
        for v in l.order.iter_mut().flat_map(|m| m.values_mut()) {
            if let Some(x) = v.as_i64().filter(|x| *x > p) {
                *v = json!(x - 1);
            }
        }
    }
    remove_id(&mut l.table["RoomIdAndColorLookUpTable"]["RoomIds"], room);
    l.rows().retain(|row| row["room_id"].as_u64() != Some(u64::from(room)));
    for row in l.rows() {
        remove_id(&mut row["room_neighbor_id"], room);
    }
    l.write(a)?;
    let mut members = vec![DAT2, NEIGHBOURS];

    let text = json_text(AI_ROOMS, a)?;
    let kept: Vec<&str> = text.lines().filter(|l| l.split_whitespace().next() != Some(r.as_str())).collect();
    if kept.len() != text.lines().count() {
        a.set(AI_ROOMS, format!("{}\n", kept.join("\n")).into_bytes())?;
        members.push(AI_ROOMS);
    }
    if let Some(text) = a.get(AI_FLOORS).map(|b| String::from_utf8_lossy(b).into_owned()) {
        let mut v = parse(AI_FLOORS, &text)?;
        if let Some(list) = v["ai_material"].as_array_mut() {
            let n = list.len();
            list.retain(|e| e["id"].as_u64() != Some(u64::from(room)));
            if list.len() != n {
                a.set(AI_FLOORS, pretty4(&v).into_bytes())?;
                members.push(AI_FLOORS);
            }
        }
    }
    if let Some(text) = a.get(HIDDEN).map(|b| String::from_utf8_lossy(b).into_owned()) {
        let mut h = parse(HIDDEN, &text)?;
        let before = h["HideRooms"].clone();
        remove_id(&mut h["HideRooms"], room);
        if h["HideRooms"] != before {
            a.set(HIDDEN, pretty4(&h).into_bytes())?;
            members.push(HIDDEN);
        }
    }
    Ok(Removed { order, members })
}

/// The room ids in `seg_info` of an archive.
pub fn seg_info_rooms(a: &Archive) -> Result<BTreeSet<u8>> {
    Ok(Lists::read(a)?.seg.keys().filter_map(|k| k.parse().ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_span_finds_one_value() {
        let t = r#"{"a":[{"1":2}],"seg_info":"{\"1\":{}}","b":3}"#;
        assert_eq!(&t[value_span(t, "a").unwrap()], r#"[{"1":2}]"#);
        assert_eq!(&t[value_span(t, "seg_info").unwrap()], r#""{\"1\":{}}""#);
        assert!(value_span(t, "c").is_err());
        assert!(value_span(r#"{"a":1,"x":{"a":2}}"#, "a").is_err(), "twice");
    }

    #[test]
    fn pretty4_matches_the_robot_layout() {
        let v = json!({"HideRooms": [9, 13]});
        assert_eq!(pretty4(&v), "{\n    \"HideRooms\": [\n        9,\n        13\n    ]\n}");
    }
}
