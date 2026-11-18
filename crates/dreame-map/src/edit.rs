//! Typed builders for map edit commands (maps.md §11).
//!
//! Every edit is action 6.2 (`UPDATE_MAP_DATA`) with one input, piid 4, whose value is a compact
//! JSON string. Each builder returns an [`EditCommand`]; [`EditCommand::to_json`] gives that
//! string and [`EditCommand::action_in`] the whole `in` entry.
//!
//! Full-replacement commands (`vw`, `cpt`, `vws`, `curtain`, `mapfurniture_v2`, `delsr`,
//! `cleanareaorder`) must carry the complete current list, so they are built from a [`Map`]
//! that holds the freshest saved map plus the user's changes.
//!
//! Builders validate what they can before anything is sent; see [`validate`] for the checks.

use crate::grid::PixelClass;
use crate::model::{CarpetArea, CarpetShape, Furniture, Line, Map, Point, Rect};
use crate::trailer::{encode_name, Quad, QuadEntry};
use crate::{Error, Result};
use serde_json::{json, Map as JsonMap, Value};

/// Maximum room count before a split is refused (Tasshack, without `MAP_V2`).
pub const MAX_ROOMS: usize = 49;
/// Maximum number of cruise points.
pub const MAX_CRUISE_POINTS: usize = 20;
/// Maximum number of user-added carpet areas (the app's `isAddCptLimit`).
pub const MAX_CARPET_AREAS: usize = 20;

/// One edit command: the piid-4 JSON object.
#[derive(Debug, Clone, PartialEq)]
pub struct EditCommand(pub Value);

impl EditCommand {
    /// Compact JSON for the piid-4 value.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.0).expect("JSON values always serialize")
    }

    /// The action input entry `{"piid":4,"value":"<json>"}`.
    pub fn action_in(&self) -> Value {
        json!({"piid": 4, "value": self.to_json()})
    }

    pub fn value(&self) -> &Value {
        &self.0
    }
}

fn invalid(msg: impl Into<String>) -> Error {
    Error::Unsupported(msg.into())
}

fn quads<'a>(it: impl IntoIterator<Item = [i64; 4]> + 'a) -> Value {
    Value::Array(it.into_iter().map(|q| json!(q)).collect())
}

// ---------------------------------------------------------------------------------------------
// Map-level commands

/// `{"sm":{},"mapid":M}`: select a saved map.
pub fn select_map(map_id: i64) -> EditCommand {
    EditCommand(json!({"sm": {}, "mapid": map_id}))
}

/// `{"nrism":{"<M>":{"name":"<text>"}}}`: rename a saved map. `None` clears the name.
pub fn rename_map(map_id: i64, name: Option<&str>) -> EditCommand {
    EditCommand(json!({"nrism": {map_id.to_string(): {"name": name}}}))
}

/// `{"smra":{"<M>":{"ra":0|90|180|270}}}`: set the display rotation.
pub fn rotate_map(map_id: i64, degrees: i64) -> Result<EditCommand> {
    if ![0, 90, 180, 270].contains(&degrees) {
        return Err(invalid(format!(
            "rotation must be 0, 90, 180 or 270, got {degrees}"
        )));
    }
    Ok(EditCommand(
        json!({"smra": {map_id.to_string(): {"ra": degrees}}}),
    ))
}

// ---------------------------------------------------------------------------------------------
// Rooms

/// New name or type for a room.
#[derive(Debug, Clone, PartialEq)]
pub enum RoomName {
    /// Custom name (sent as base64 UTF-8 with type 0).
    Custom(String),
    /// Room type code 1..15 and index (0 for the first room of that type).
    Typed { room_type: i64, index: i64 },
}

/// `nsr`: rename or retype rooms. Like Tasshack, it sends every named or typed room of the map,
/// with the requested changes applied.
///
/// Payload: `{"nsr":{"<id>":{"name":"<b64>","type":0,"index":I},"<id2>":{"type":T,"index":I}},"mapid":M}`,
/// where a custom name keeps the room's stored `index`.
pub fn rename_rooms(map: &Map, changes: &[(u32, RoomName)]) -> Result<EditCommand> {
    for (id, name) in changes {
        validate::room_exists(map, *id)?;
        match name {
            RoomName::Custom(n) if n.trim().is_empty() => {
                return Err(invalid("custom room name is empty"))
            }
            RoomName::Typed { room_type, .. } if !(1..=15).contains(room_type) => {
                return Err(invalid(format!("room type must be 1..15, got {room_type}")))
            }
            _ => {}
        }
    }
    let mut nsr = JsonMap::new();
    for room in map.rooms.values() {
        let change = changes
            .iter()
            .rev()
            .find(|(id, _)| *id == room.id)
            .map(|(_, n)| n.clone());
        let current = match (room.room_type, &room.name) {
            (Some(0) | None, Some(n)) if !n.is_empty() => Some(RoomName::Custom(n.clone())),
            (Some(t), _) if t > 0 => Some(RoomName::Typed {
                room_type: t,
                index: room.index.unwrap_or(0),
            }),
            _ => None,
        };
        let Some(name) = change.or(current) else {
            continue;
        };
        // Custom names keep the room's stored index, as the app's commitRename does
        // (`index: _item.num`). Sending 0 resets it (seen on r2579a).
        let mut entry = match name {
            RoomName::Custom(n) => {
                json!({"name": encode_name(&n), "type": 0, "index": room.index.unwrap_or(0)})
            }
            RoomName::Typed { room_type, index } => json!({"type": room_type, "index": index}),
        };
        if let Some(uid) = &room.unique_id {
            entry["roomID"] = uid.clone();
        }
        nsr.insert(room.id.to_string(), entry);
    }
    Ok(EditCommand(json!({"nsr": nsr, "mapid": map.map_id})))
}

/// Renames one room to a custom name. See [`rename_rooms`].
pub fn rename_room(map: &Map, id: u32, name: &str) -> Result<EditCommand> {
    rename_rooms(map, &[(id, RoomName::Custom(name.to_string()))])
}

/// `{"msr":[A,B],"mapid":M}`: merge two neighbour rooms. Room ids change afterwards.
pub fn merge_rooms(map: &Map, a: u32, b: u32) -> Result<EditCommand> {
    validate::room_exists(map, a)?;
    validate::room_exists(map, b)?;
    if a == b {
        return Err(invalid("cannot merge a room with itself"));
    }
    if !map.are_neighbours(a, b) {
        return Err(invalid(format!("rooms {a} and {b} are not neighbours")));
    }
    Ok(EditCommand(json!({"msr": [a, b], "mapid": map.map_id})))
}

/// `{"dsrid":[x0,y0,x1,y1,id],"mapid":M}`: split a room along a line (world mm). The line is
/// trimmed to the room's edge first, and the ends are rounded down, as the app does (see
/// [`validate::split_line`]).
pub fn split_room(map: &Map, id: u32, line: Line) -> Result<EditCommand> {
    validate::room_exists(map, id)?;
    if map.rooms.len() >= MAX_ROOMS && map.map_version < 3 {
        return Err(invalid(format!(
            "the map already has {} rooms",
            map.rooms.len()
        )));
    }
    let l = validate::split_line(map, id, &line)?;
    let q = [l.a.x, l.a.y, l.b.x, l.b.y].map(|v| v.floor() as i64);
    Ok(EditCommand(
        json!({"dsrid": [q[0], q[1], q[2], q[3], id], "mapid": map.map_id}),
    ))
}

/// The map as it would look after merging room `b` into room `a` with [`merge_rooms`]: `b`'s
/// floor and boundary pixels become `a`'s, and room `b` is gone. Room `a` keeps its metadata,
/// as it does on r2579a. This is a local preview; nothing is sent.
pub fn merged_preview(map: &Map, a: u32, b: u32) -> Result<Map> {
    validate::room_exists(map, a)?;
    validate::room_exists(map, b)?;
    let (sa, sb) = (a as u8, b as u8);
    let mut m = map.clone();
    for px in &mut m.grid.pixels {
        px.class = match px.class {
            PixelClass::Floor(s) if s == sb => PixelClass::Floor(sa),
            PixelClass::Wall(Some(s)) if s == sb => PixelClass::Wall(Some(sa)),
            PixelClass::HiddenWall(s) if s == sb => PixelClass::HiddenWall(sa),
            c => c,
        };
    }
    let rb = m.rooms.remove(&b).expect("checked above");
    let ra = m.rooms.get_mut(&a).expect("checked above");
    ra.pixel_count += rb.pixel_count;
    ra.bounds = match (ra.bounds, rb.bounds) {
        (Some(x), Some(y)) => Some(Rect::new(x.x0.min(y.x0), x.y0.min(y.y0), x.x1.max(y.x1), x.y1.max(y.y1))),
        (x, y) => x.or(y),
    };
    let mut nei: Vec<u32> = ra.neighbours.iter().chain(&rb.neighbours).copied().filter(|&n| n != a && n != b).collect();
    nei.sort_unstable();
    nei.dedup();
    ra.neighbours = nei;
    for r in m.rooms.values_mut() {
        if r.neighbours.contains(&b) {
            r.neighbours.retain(|&n| n != b);
            if r.id != a && !r.neighbours.contains(&a) {
                r.neighbours.push(a);
                r.neighbours.sort_unstable();
            }
        }
    }
    Ok(m)
}

/// Checks a boundary move between neighbour rooms `a` and `b`: merge them, then split the merged
/// room `a` along `line`. Returns the two commands in order. The split is checked against
/// [`merged_preview`], so both ends of `line` must lie outside both rooms; the split command
/// carries the line trimmed to the merged room's edge. Both rooms must be visible.
pub fn move_boundary(map: &Map, a: u32, b: u32, line: Line) -> Result<[EditCommand; 2]> {
    let merge = merge_rooms(map, a, b)?;
    for id in [a, b] {
        if map.rooms[&id].hidden {
            return Err(invalid(format!("room {id} is hidden; show it first")));
        }
    }
    let merged = merged_preview(map, a, b)?;
    let split = split_room(&merged, a, line)?;
    Ok([merge, split])
}

/// The map cells that change rooms when the border of neighbour rooms `a` and `b` moves to
/// `line`, straight or bent, as [`crate::segmap::border_cells`] cuts them. A boundary move
/// through the recovery archive paints these cells, so room ids, names and settings stay;
/// [`move_boundary`] instead merges and splits the rooms through map edit commands.
pub fn border_cells(map: &Map, a: u32, b: u32, line: &[Point]) -> Result<crate::segmap::BorderCells> {
    validate::room_exists(map, a)?;
    validate::room_exists(map, b)?;
    let id = |r: u32| u8::try_from(r).map_err(|_| invalid(format!("room id {r} is too high")));
    let t = map.transform();
    let line: Vec<(f64, f64)> = line.iter().map(|p| (p.x, p.y)).collect();
    crate::segmap::border_cells(
        map.grid.width,
        map.grid.height,
        |c, r| map.grid.get(c, r).class.segment(),
        |c, r| t.cell_center(c, r),
        (id(a)?, id(b)?),
        &line,
    )
}

/// `{"delsr":[ids],"mapid":M}`: the full list of hidden rooms.
pub fn hide_rooms(map: &Map, hidden: &[u32]) -> Result<EditCommand> {
    for &id in hidden {
        validate::room_exists(map, id)?;
    }
    let mut ids = hidden.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() >= map.rooms.len() && !map.rooms.is_empty() {
        return Err(invalid("cannot hide every room"));
    }
    Ok(EditCommand(json!({"delsr": ids, "mapid": map.map_id})))
}

/// `{"cleanareaorder":[{"<id>":1},{"<id>":2},...]}`: cleaning order (version 2).
///
/// `order` must list each visible room exactly once; hidden rooms are not allowed in it. Like
/// the app (`commitCleanOrder`, map-plugin.md §6.2), hidden rooms that the stored order already
/// lists are kept at the end, in ascending id order, numbered after the visible rooms. Hidden
/// rooms that the stored order does not list stay out.
pub fn cleaning_order(map: &Map, order: &[u32]) -> Result<EditCommand> {
    let mut seen = std::collections::BTreeSet::new();
    for &id in order {
        validate::room_exists(map, id)?;
        if map.rooms[&id].hidden {
            return Err(invalid(format!("room {id} is hidden")));
        }
        if !seen.insert(id) {
            return Err(invalid(format!("room {id} is listed twice")));
        }
    }
    let visible: std::collections::BTreeSet<u32> = map
        .rooms
        .values()
        .filter(|r| !r.hidden)
        .map(|r| r.id)
        .collect();
    if seen != visible {
        let missing: Vec<_> = visible.difference(&seen).collect();
        return Err(invalid(format!("cleaning order misses rooms {missing:?}")));
    }
    let hidden_listed = map
        .rooms
        .values()
        .filter(|r| r.hidden && r.order.is_some())
        .map(|r| &r.id);
    let list: Vec<Value> = order
        .iter()
        .chain(hidden_listed)
        .enumerate()
        .map(|(i, id)| json!({id.to_string(): i + 1}))
        .collect();
    Ok(EditCommand(json!({"cleanareaorder": list})))
}

/// `{"nsm":{"<id>":{"material":m,"direction":0|90}},"mapid":M}`: floor material per room.
pub fn floor_material(map: &Map, rooms: &[(u32, i64, i64)]) -> Result<EditCommand> {
    let mut nsm = JsonMap::new();
    for &(id, material, direction) in rooms {
        validate::room_exists(map, id)?;
        if !(0..=7).contains(&material) {
            return Err(invalid(format!("material must be 0..7, got {material}")));
        }
        if direction != 0 && direction != 90 {
            return Err(invalid(format!(
                "direction must be 0 or 90, got {direction}"
            )));
        }
        nsm.insert(
            id.to_string(),
            json!({"material": material, "direction": direction}),
        );
    }
    Ok(EditCommand(json!({"nsm": nsm, "mapid": map.map_id})))
}

/// Per-room cleaning settings for `customeClean` (map-plugin.md §6.2).
///
/// The robot stores them in `cleanset` as `[suction, water, times, order, cleanType, mopEffect,
/// mopTemp]`. The command drops `order` and puts the room id first. The last three fields are
/// optional: the app sends `cleanType` only when the robot lets it change per room, `mopEffect`
/// only with mop-effect support, and `mopTemp` only with mop-temperature support. r2579a stores
/// six fields (no `mopTemp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomSettings {
    pub id: u32,
    /// 0..3
    pub suction: i64,
    /// Wetness: raw 1..32 on robots with 32 levels, else 1..3.
    pub water: i64,
    /// 1..3
    pub times: i64,
    /// 0 sweep, 1 mop, 2 sweep and mop.
    pub clean_type: Option<i64>,
    /// `preference | (water << 4) | (mopEffect << 8)`.
    pub mop_effect: Option<i64>,
    /// Mop temperature; only on robots that support it.
    pub mop_temp: Option<i64>,
}

impl RoomSettings {
    /// Builds the settings from a stored `cleanset` entry. Returns `None` when the entry has
    /// fewer than three fields.
    pub fn from_cleanset(id: u32, v: &[i64]) -> Option<Self> {
        Some(Self {
            id,
            suction: *v.first()?,
            water: *v.get(1)?,
            times: *v.get(2)?,
            clean_type: v.get(4).copied(),
            mop_effect: v.get(5).copied(),
            mop_temp: v.get(6).copied(),
        })
    }

    /// The current settings of every room of the map that has a `cleanset` entry.
    pub fn current(map: &Map) -> Vec<Self> {
        map.rooms
            .values()
            .filter_map(|r| Self::from_cleanset(r.id, r.settings.as_deref()?))
            .collect()
    }
}

/// `{"customeClean":[[id,suction,water,times,cleanType,mopEffect,mopTemp], ...]}`.
///
/// Each entry ends at the last optional field that is set. Like the app, send every room: start
/// from [`RoomSettings::current`] and change the rooms you need.
pub fn room_settings(map: &Map, settings: &[RoomSettings]) -> Result<EditCommand> {
    let mut list = Vec::new();
    for s in settings {
        validate::room_exists(map, s.id)?;
        if !(0..=3).contains(&s.suction)
            || !(1..=32).contains(&s.water)
            || !(1..=3).contains(&s.times)
            || s.clean_type.is_some_and(|t| !(0..=2).contains(&t))
        {
            return Err(invalid(format!("room {} settings out of range", s.id)));
        }
        let optional = [s.clean_type, s.mop_effect, s.mop_temp];
        let n = optional.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        let mut entry = vec![i64::from(s.id), s.suction, s.water, s.times];
        for field in &optional[..n] {
            let Some(v) = field else {
                return Err(invalid(format!(
                    "room {}: a later settings field is set without an earlier one",
                    s.id
                )));
            };
            entry.push(*v);
        }
        list.push(json!(entry));
    }
    Ok(EditCommand(json!({"customeClean": list})))
}

// ---------------------------------------------------------------------------------------------
// Restrictions, carpets, furniture

/// `{"vw":{"line":[...],"rect":[...],"mop":[...],"addcpt":[...],"nocpt":[...],...}}` from the
/// map's restrictions and carpet areas.
///
/// The app's commit (`commitVirtualWall`, map-plugin.md §6.3) resends every key stored under
/// `vw`, carpets and `cliff` included. This builder does the same: it starts from
/// [`Map::stored_vw`], then writes the model's virtual walls, no-go and no-mop zones, carpet areas
/// and ignored-carpet areas over it. Keys the model does not interpret are resent as stored.
pub fn restrictions(map: &Map) -> Result<EditCommand> {
    let r = &map.restrictions;
    for rect in r.no_go.iter().chain(&r.no_mop) {
        validate::rect_in_map(map, rect)?;
    }
    for l in &r.virtual_walls {
        validate::line_in_map(map, l)?;
    }
    for rect in &map.ignored_carpet {
        validate::rect_in_map(map, rect)?;
    }
    let mut vw = match map.stored_vw.as_ref().map(serde_json::to_value) {
        Some(Ok(Value::Object(o))) => o,
        _ => JsonMap::new(),
    };
    let stored = map.stored_vw.as_ref();
    vw.insert(
        "line".into(),
        with_stored_fields(
            r.virtual_walls.iter().map(Line::to_quad),
            stored.and_then(|s| s.line.as_ref()),
        ),
    );
    vw.insert(
        "rect".into(),
        with_stored_fields(
            r.no_go.iter().map(Rect::to_quad),
            stored.and_then(|s| s.rect.as_ref()),
        ),
    );
    vw.insert(
        "mop".into(),
        with_stored_fields(
            r.no_mop.iter().map(Rect::to_quad),
            stored.and_then(|s| s.mop.as_ref()),
        ),
    );
    if vw.contains_key("addcpt") || !map.carpet_areas.is_empty() {
        vw.insert("addcpt".into(), carpet_entries(map)?);
    }
    if vw.contains_key("nocpt") || !map.ignored_carpet.is_empty() {
        vw.insert(
            "nocpt".into(),
            with_stored_fields(
                map.ignored_carpet.iter().map(Rect::to_quad),
                stored.and_then(|s| s.nocpt.as_ref()),
            ),
        );
    }
    Ok(EditCommand(json!({ "vw": vw })))
}

/// Serializes entries. An entry whose coordinates match a stored entry is sent with that entry's
/// extra fields, as the app resends stored entries whole; a new entry is sent with four fields.
fn with_stored_fields(
    entries: impl IntoIterator<Item = Quad>,
    stored: Option<&Vec<QuadEntry>>,
) -> Value {
    let mut pool: Vec<&QuadEntry> = stored.into_iter().flatten().collect();
    Value::Array(
        entries
            .into_iter()
            .map(|q| match pool.iter().position(|e| e.quad == q) {
                Some(i) => pool.remove(i).to_value(),
                None => json!(q),
            })
            .collect(),
    )
}

/// `{"cpt":{"addcpt":[[x0,y0,x1,y1,id,ellipse,type,...]],"nocpt":[[x0,y0,x1,y1]]}}` from the
/// map's carpet areas and ignored-carpet areas. Replaces both lists.
///
/// This is the app's own carpet commit (`didCommitCpt`, map-plugin.md §6.3): a standalone `cpt`
/// command, although the robot stores the lists under `vw`. Like the app, it resends each stored
/// entry with all its fields, so the type and any later fields survive.
pub fn carpets(map: &Map) -> Result<EditCommand> {
    let add = carpet_entries(map)?;
    for r in &map.ignored_carpet {
        validate::rect_in_map(map, r)?;
    }
    Ok(EditCommand(
        json!({"cpt": {"addcpt": add, "nocpt": quads(map.ignored_carpet.iter().map(Rect::to_quad))}}),
    ))
}

/// Validates the map's carpet areas and serializes them as `addcpt` entries.
fn carpet_entries(map: &Map) -> Result<Value> {
    if map.carpet_areas.len() > MAX_CARPET_AREAS {
        return Err(invalid(format!(
            "at most {MAX_CARPET_AREAS} carpet areas, got {}",
            map.carpet_areas.len()
        )));
    }
    let mut ids = std::collections::BTreeSet::new();
    for c in &map.carpet_areas {
        match &c.shape {
            CarpetShape::Rect(r) => validate::rect_in_map(map, r)?,
            CarpetShape::Ellipse { center, size } => {
                if size.0 <= 0.0 || size.1 <= 0.0 {
                    return Err(invalid(format!("carpet ellipse {} has no area", c.id)));
                }
                if !map.bounds().contains(*center) {
                    return Err(invalid(format!("carpet ellipse {} leaves the map", c.id)));
                }
            }
        }
        if c.id <= 0 || !ids.insert(c.id) {
            return Err(invalid(format!(
                "carpet id {} is not unique and positive",
                c.id
            )));
        }
    }
    Ok(Value::Array(
        map.carpet_areas
            .iter()
            .map(|c| {
                let mut v: Vec<Value> = c.shape.to_quad().iter().map(|&x| x.into()).collect();
                v.push(c.id.into());
                v.push(i64::from(c.shape.is_ellipse()).into());
                if let Some(k) = c.kind {
                    v.push(k.into());
                }
                v.extend(c.extra.iter().cloned());
                Value::Array(v)
            })
            .collect(),
    ))
}

/// The lowest free id for a new user carpet (`CARPET_ID`: existing ids are kept).
pub fn next_carpet_id(areas: &[CarpetArea]) -> i64 {
    (1..)
        .find(|i| !areas.iter().any(|c| c.id == *i))
        .expect("an unbounded range always yields")
}

/// `{"vws":{"vwsl":[...],"npthrsd":[...],"ramp":[[x0,y0,x1,y1,angle]]}}`. Replaces all three.
/// Like [`restrictions`], entries that match stored ones keep their stored fields, and stored
/// keys the model does not read are resent.
pub fn thresholds(map: &Map) -> Result<EditCommand> {
    let r = &map.restrictions;
    for l in r.thresholds.iter().chain(&r.impassable_thresholds) {
        validate::line_in_map(map, l)?;
    }
    let stored = map.stored_vws.as_ref();
    for rp in &r.ramps {
        validate::rect_in_map(map, &rp.rect)?;
    }
    // A ramp that matches a stored one is resent as stored, with any fields after the angle.
    let mut pool: Vec<&Vec<i64>> = stored.and_then(|s| s.ramp.as_ref()).into_iter().flatten().collect();
    let same = |e: &Vec<i64>, q: &Quad, angle: i64| {
        e.len() >= 5 && Rect::from_quad(&[e[0], e[1], e[2], e[3]]).to_quad() == *q && e[4] == angle
    };
    let ramps: Vec<Value> = r
        .ramps
        .iter()
        .map(|rp| {
            let q = rp.rect.to_quad();
            match pool.iter().position(|e| same(e, &q, rp.angle)) {
                Some(i) => json!(pool.remove(i)),
                None => json!([q[0], q[1], q[2], q[3], rp.angle]),
            }
        })
        .collect();
    let mut vws = match stored.map(serde_json::to_value) {
        Some(Ok(Value::Object(o))) => o,
        _ => JsonMap::new(),
    };
    vws.insert(
        "vwsl".into(),
        with_stored_fields(r.thresholds.iter().map(Line::to_quad), stored.and_then(|s| s.vwsl.as_ref())),
    );
    vws.insert(
        "npthrsd".into(),
        with_stored_fields(
            r.impassable_thresholds.iter().map(Line::to_quad),
            stored.and_then(|s| s.npthrsd.as_ref()),
        ),
    );
    vws.insert("ramp".into(), Value::Array(ramps));
    Ok(EditCommand(json!({ "vws": vws })))
}

/// `{"curtain":{"line":[...]}}`. Stored by the robot as `ct`. Like [`restrictions`], entries
/// that match stored ones keep their stored fields, and other stored keys are resent.
pub fn curtains(map: &Map) -> Result<EditCommand> {
    for l in &map.restrictions.curtains {
        validate::line_in_map(map, l)?;
    }
    let stored = map.stored_ct.as_ref();
    let mut ct = match stored.map(serde_json::to_value) {
        Some(Ok(Value::Object(o))) => o,
        _ => JsonMap::new(),
    };
    ct.insert(
        "line".into(),
        with_stored_fields(map.restrictions.curtains.iter().map(Line::to_quad), stored.and_then(|s| s.line.as_ref())),
    );
    Ok(EditCommand(json!({ "curtain": ct })))
}

/// `{"mapfurniture_v2":[[id,type,seg,w,h,0,x,y,0,angle,0,0,scale,edit_type,modified], ...]}`.
/// Full list; type codes are sent as stored (wire codes). Each entry is its stored entry with
/// the model's fields written over it (see [`Furniture::to_entry`]), so unread fields survive.
pub fn furniture(map: &Map) -> Result<EditCommand> {
    for f in &map.furniture {
        if f.width <= 0.0 || f.height <= 0.0 || f.scale <= 0.0 {
            return Err(invalid(format!("furniture at {:?} has no size", f.center)));
        }
        if !map.bounds().contains(f.center) {
            return Err(invalid(format!("furniture at {:?} leaves the map", f.center)));
        }
    }
    let list: Vec<Value> = map.furniture.iter().map(|f: &Furniture| Value::Array(f.to_entry().to_values())).collect();
    Ok(EditCommand(json!({"mapfurniture_v2": list})))
}

// ---------------------------------------------------------------------------------------------
// Validation

/// Checks shared by the builders. Errors are [`Error::Unsupported`] with a reason.
pub mod validate {
    use super::*;
    use crate::grid::PixelClass;
    use crate::model::Point;
    use std::collections::BTreeSet;

    pub fn room_exists(map: &Map, id: u32) -> Result<()> {
        if map.rooms.contains_key(&id) {
            Ok(())
        } else {
            Err(invalid(format!("room {id} does not exist")))
        }
    }

    /// The rectangle must have area and lie inside the map grid.
    pub fn rect_in_map(map: &Map, r: &Rect) -> Result<()> {
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return Err(invalid(format!("rectangle {:?} has no area", r.to_quad())));
        }
        if !map.bounds().contains_rect(r) {
            return Err(invalid(format!(
                "rectangle {:?} leaves the map",
                r.to_quad()
            )));
        }
        Ok(())
    }

    /// The line must have length and both ends inside the map grid.
    pub fn line_in_map(map: &Map, l: &Line) -> Result<()> {
        if l.length() <= 0.0 {
            return Err(invalid("line has zero length"));
        }
        let b = map.bounds();
        if !b.contains(l.a) || !b.contains(l.b) {
            return Err(invalid(format!("line {:?} leaves the map", l.to_quad())));
        }
        Ok(())
    }

    /// Rooms whose floor pixels the line crosses, sampled every quarter cell.
    pub fn rooms_crossed(map: &Map, l: &Line) -> BTreeSet<u32> {
        let tr = map.transform();
        let step = tr.grid_size / 4.0;
        let n = (l.length() / step).ceil().max(1.0) as usize;
        let mut out = BTreeSet::new();
        for i in 0..=n {
            let t = i as f64 / n as f64;
            let p = Point::new(l.a.x + (l.b.x - l.a.x) * t, l.a.y + (l.b.y - l.a.y) * t);
            if let Some((c, r)) = tr.cell_at(p.x, p.y) {
                if let PixelClass::Floor(s) = map.grid.get(c, r).class {
                    out.insert(s as u32);
                }
            }
        }
        out
    }

    /// The split line to send, trimmed as the app trims it (`calculateCrossAreaLine` and
    /// `lookForValidPos` in the r2579a plugin). Both drawn ends must lie outside the room's
    /// floor. Each end then moves toward the other in 25 mm steps along the line's main axis and
    /// stops at the last point before the room's floor, so the sent ends lie on the room's edge.
    /// The robot refuses ends that are not on the edge (result 5). What lies beyond the room
    /// does not matter: a drawn end may lie in another room, as in the app.
    pub fn split_line(map: &Map, id: u32, l: &Line) -> Result<Line> {
        line_in_map(map, l)?;
        let tr = map.transform();
        let inside = |p: Point| {
            tr.cell_at(p.x, p.y).map(|(c, r)| map.grid.get(c, r).class)
                == Some(PixelClass::Floor(id as u8))
        };
        if inside(l.a) || inside(l.b) {
            return Err(invalid(format!(
                "each end of the split line must lie outside room {id}"
            )));
        }
        let trim = |from: Point, to: Point| -> Result<Point> {
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            let span = dx.abs().max(dy.abs());
            let mut last = from;
            for i in 1..=(span / SPLIT_STEP_MM) as usize {
                let t = i as f64 * SPLIT_STEP_MM / span;
                let p = Point::new(from.x + dx * t, from.y + dy * t);
                if inside(p) {
                    return Ok(last);
                }
                last = p;
            }
            Err(invalid(format!("split line does not cross room {id}")))
        };
        let a = trim(l.a, l.b)?;
        let b = trim(l.b, l.a)?;
        Ok(Line { a, b })
    }

    /// The app's step when it trims a split line to the room's edge.
    const SPLIT_STEP_MM: f64 = 25.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::saved_map;
    use crate::model::{Point, Ramp};

    #[test]
    fn map_commands() {
        assert_eq!(select_map(1).to_json(), r#"{"sm":{},"mapid":1}"#);
        assert_eq!(
            rename_map(1, Some("Casa")).to_json(),
            r#"{"nrism":{"1":{"name":"Casa"}}}"#
        );
        assert_eq!(
            rename_map(1, None).to_json(),
            r#"{"nrism":{"1":{"name":null}}}"#
        );
        assert_eq!(
            rotate_map(1, 90).unwrap().to_json(),
            r#"{"smra":{"1":{"ra":90}}}"#
        );
        assert!(rotate_map(1, 45).is_err());
        assert_eq!(
            select_map(1).action_in(),
            json!({"piid": 4, "value": "{\"sm\":{},\"mapid\":1}"})
        );
    }

    #[test]
    fn rename_sends_all_named_rooms() {
        let m = saved_map();
        let c = rename_room(&m, 1, "Suíte").unwrap();
        let b64 = encode_name("Suíte");
        assert_eq!(
            c.to_json(),
            format!(
                r#"{{"nsr":{{"1":{{"name":"{b64}","type":0,"index":0}},"2":{{"type":15,"index":1}}}},"mapid":1}}"#
            )
        );
        let c = rename_rooms(
            &m,
            &[(
                1,
                RoomName::Typed {
                    room_type: 4,
                    index: 0,
                },
            )],
        )
        .unwrap();
        assert_eq!(
            c.to_json(),
            r#"{"nsr":{"1":{"type":4,"index":0},"2":{"type":15,"index":1}},"mapid":1}"#
        );
        assert!(rename_room(&m, 7, "x").is_err());
        assert!(rename_room(&m, 1, " ").is_err());
        // A custom-named room keeps its stored index, whether renamed or resent unchanged.
        let mut m = saved_map();
        m.rooms.get_mut(&1).unwrap().index = Some(6);
        let c = rename_room(&m, 2, "Lounge").unwrap();
        assert_eq!(c.value()["nsr"]["1"]["index"], 6);
        let c = rename_room(&m, 1, "Lounge").unwrap();
        assert_eq!(c.value()["nsr"]["1"]["index"], 6);
    }

    #[test]
    fn merge_and_split() {
        let m = saved_map();
        assert_eq!(
            merge_rooms(&m, 1, 2).unwrap().to_json(),
            r#"{"msr":[1,2],"mapid":1}"#
        );
        assert!(merge_rooms(&m, 1, 1).is_err());
        // Vertical line through room 1 from the bottom wall to the top wall. Each end moves in
        // 25 mm steps to the last point before room 1's floor (rows 1-2, y 50..150).
        let ok = Line::new(75.0, 10.0, 75.0, 190.0);
        assert_eq!(
            split_room(&m, 1, ok).unwrap().to_json(),
            r#"{"dsrid":[75,35,75,165,1],"mapid":1}"#
        );
        // An end in room 2 is fine, as in the app: it is trimmed to room 1's edge (x 150).
        assert_eq!(
            split_room(&m, 1, Line::new(10.0, 75.0, 290.0, 75.0)).unwrap().to_json(),
            r#"{"dsrid":[35,75,165,75,1],"mapid":1}"#
        );
        // Misses room 1 altogether.
        assert!(split_room(&m, 1, Line::new(175.0, 10.0, 175.0, 190.0)).is_err());
        // Ends inside the room.
        assert!(split_room(&m, 1, Line::new(75.0, 10.0, 75.0, 100.0)).is_err());
        // Leaves the map.
        assert!(split_room(&m, 1, Line::new(75.0, -100.0, 75.0, 190.0)).is_err());
    }

    #[test]
    fn boundary_move() {
        let m = saved_map();
        let merged = merged_preview(&m, 1, 2).unwrap();
        assert_eq!(merged.rooms.keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(merged.rooms[&1].pixel_count, m.rooms[&1].pixel_count + m.rooms[&2].pixel_count);
        assert_eq!(merged.room_at(Point::new(225.0, 125.0)), Some(1));
        assert_eq!(merged.floor_overlap(1, &m, 2), m.rooms[&2].pixel_count);
        // A vertical line through column 2, which is room 1 today: room 2 grows to the left.
        let [merge, split] = move_boundary(&m, 1, 2, Line::new(125.0, 10.0, 125.0, 190.0)).unwrap();
        assert_eq!(merge.to_json(), r#"{"msr":[1,2],"mapid":1}"#);
        assert_eq!(split.to_json(), r#"{"dsrid":[125,35,125,165,1],"mapid":1}"#);
        // Through room 2's side: also fine after the merge.
        assert!(move_boundary(&m, 1, 2, Line::new(175.0, 10.0, 175.0, 190.0)).is_ok());
        // Ends inside the merged room, or only over walls.
        assert!(move_boundary(&m, 1, 2, Line::new(125.0, 10.0, 125.0, 100.0)).is_err());
        assert!(move_boundary(&m, 1, 2, Line::new(25.0, 10.0, 25.0, 190.0)).is_err());
        let mut hidden = m.clone();
        hidden.rooms.get_mut(&2).unwrap().hidden = true;
        assert!(move_boundary(&hidden, 1, 2, Line::new(125.0, 10.0, 125.0, 190.0)).is_err());
    }

    #[test]
    fn hide_and_order() {
        let mut m = saved_map();
        assert_eq!(
            hide_rooms(&m, &[2]).unwrap().to_json(),
            r#"{"delsr":[2],"mapid":1}"#
        );
        assert!(hide_rooms(&m, &[1, 2]).is_err());
        assert_eq!(
            cleaning_order(&m, &[1, 2]).unwrap().to_json(),
            r#"{"cleanareaorder":[{"1":1},{"2":2}]}"#
        );
        assert!(cleaning_order(&m, &[1]).is_err());
        assert!(cleaning_order(&m, &[1, 1, 2]).is_err());
        // A hidden room the stored order lists goes to the end.
        m.rooms.get_mut(&2).unwrap().hidden = true;
        assert_eq!(m.rooms[&2].order, Some(1));
        assert_eq!(
            cleaning_order(&m, &[1]).unwrap().to_json(),
            r#"{"cleanareaorder":[{"1":1},{"2":2}]}"#
        );
        assert!(cleaning_order(&m, &[1, 2]).is_err());
        // A hidden room the stored order does not list stays out.
        m.rooms.get_mut(&2).unwrap().order = None;
        assert_eq!(
            cleaning_order(&m, &[1]).unwrap().to_json(),
            r#"{"cleanareaorder":[{"1":1}]}"#
        );
    }

    /// r2579a stores a sent no-go zone `[x0,y0,x1,y1]` as `[x0,y0,x1,y1,0]` (seen 2026-11-16).
    #[test]
    fn stored_zone_with_extra_field() {
        use crate::trailer::{Trailer, VirtualWalls};
        let t = Trailer::from_value(json!({"vw": {"line": [], "rect": [[100, 50, 200, 150, 0]],
            "mop": [], "cliff": [], "addcpt": [[50, 50, 150, 150, 1, 0, 7]], "nocpt": []}}))
        .unwrap();
        assert!(t.unparsed.is_empty(), "vw must parse: {:?}", t.unparsed);
        let vw: &VirtualWalls = t.vw.as_ref().unwrap();
        assert_eq!(vw.rect.as_ref().unwrap()[0].rest, [json!(0)]);
        assert_eq!(t.to_value()["vw"]["rect"], json!([[100, 50, 200, 150, 0]]));

        let mut m = saved_map();
        m.stored_vw = Some(vw.clone());
        m.restrictions.no_go = vec![Rect::new(100.0, 50.0, 200.0, 150.0)];
        // Unchanged: resent with the stored field.
        assert_eq!(
            restrictions(&m).unwrap().value()["vw"]["rect"],
            json!([[100, 50, 200, 150, 0]])
        );
        // A new zone goes with four fields; the stored one keeps its field.
        m.restrictions
            .no_go
            .push(Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(
            restrictions(&m).unwrap().value()["vw"]["rect"],
            json!([[100, 50, 200, 150, 0], [0, 0, 50, 50]])
        );
        // Removed: gone from the list; carpets stay.
        m.restrictions.no_go.clear();
        let v = restrictions(&m).unwrap();
        assert_eq!(v.value()["vw"]["rect"], json!([]));
        assert_eq!(v.value()["vw"]["addcpt"], json!([[50, 50, 150, 150, 1, 0, 7]]));
    }

    #[test]
    fn restrictions_payload() {
        let mut m = saved_map();
        m.restrictions
            .no_go
            .push(Rect::new(250.0, 150.0, 200.0, 100.0));
        m.restrictions.no_mop = vec![Rect::new(0.0, 0.0, 50.0, 50.0)];
        // Every stored vw key is resent: cliff as stored, carpets from the model.
        assert_eq!(
            restrictions(&m).unwrap().to_json(),
            r#"{"vw":{"line":[[0,0,100,0]],"rect":[[100,50,200,150],[200,100,250,150]],"mop":[[0,0,50,50]],"cliff":[],"addcpt":[[50,50,150,150,1,0,7]],"nocpt":[]}}"#
        );
        // An unknown stored key survives; a carpet removed in the model is removed on the wire.
        let mut stored = m.stored_vw.clone().unwrap();
        stored.extra.insert("newkind".into(), json!([1]));
        m.stored_vw = Some(stored);
        m.carpet_areas.clear();
        let v = restrictions(&m).unwrap();
        assert_eq!(v.value()["vw"]["newkind"], json!([1]));
        assert_eq!(v.value()["vw"]["addcpt"], json!([]));
        // Without a stored vw, only the modelled lists are sent.
        m.stored_vw = None;
        assert_eq!(
            restrictions(&m).unwrap().to_json(),
            r#"{"vw":{"line":[[0,0,100,0]],"rect":[[100,50,200,150],[200,100,250,150]],"mop":[[0,0,50,50]]}}"#
        );
        m.restrictions
            .no_go
            .push(Rect::new(250.0, 150.0, 400.0, 100.0));
        assert!(restrictions(&m).is_err());
    }

    #[test]
    fn carpets_payload() {
        let mut m = saved_map();
        let id = next_carpet_id(&m.carpet_areas);
        assert_eq!(id, 2);
        // An ellipse is centre plus size; its numbers must not be reordered like corners.
        m.carpet_areas.push(CarpetArea {
            id,
            shape: CarpetShape::Ellipse {
                center: Point::new(225.0, 100.0),
                size: (25.0, 50.0),
            },
            kind: Some(7),
            extra: vec![json!(3)],
        });
        m.ignored_carpet.push(Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(
            carpets(&m).unwrap().to_json(),
            r#"{"cpt":{"addcpt":[[50,50,150,150,1,0,7],[225,100,25,50,2,1,7,3]],"nocpt":[[0,0,50,50]]}}"#
        );
        m.carpet_areas[1].id = 1;
        assert!(carpets(&m).is_err());
        m.carpet_areas[1].id = 2;
        m.carpet_areas[1].shape = CarpetShape::Ellipse {
            center: Point::new(-500.0, 100.0),
            size: (25.0, 50.0),
        };
        assert!(carpets(&m).is_err());
        m.carpet_areas.truncate(1);
        for i in 2..=21 {
            m.carpet_areas.push(CarpetArea {
                id: i,
                ..m.carpet_areas[0].clone()
            });
        }
        assert!(carpets(&m).is_err());
    }

    #[test]
    fn other_payloads() {
        let mut m = saved_map();
        assert_eq!(
            floor_material(&m, &[(1, 1, 90)]).unwrap().to_json(),
            r#"{"nsm":{"1":{"material":1,"direction":90}},"mapid":1}"#
        );
        assert!(floor_material(&m, &[(1, 1, 45)]).is_err());
        // The stored six-field entry [suction, water, times, order, cleanType, mopEffect].
        let current = RoomSettings::current(&m);
        assert_eq!(current.len(), 2);
        let s = current[1];
        assert_eq!(
            (s.id, s.clean_type, s.mop_effect, s.mop_temp),
            (2, Some(2), Some(546), None)
        );
        assert_eq!(
            room_settings(&m, &[s]).unwrap().to_json(),
            r#"{"customeClean":[[2,1,3,1,2,546]]}"#
        );
        let with_temp = RoomSettings {
            mop_temp: Some(1),
            ..s
        };
        assert_eq!(
            room_settings(&m, &[with_temp]).unwrap().to_json(),
            r#"{"customeClean":[[2,1,3,1,2,546,1]]}"#
        );
        let basic = RoomSettings {
            clean_type: None,
            mop_effect: None,
            ..s
        };
        assert_eq!(
            room_settings(&m, &[basic]).unwrap().to_json(),
            r#"{"customeClean":[[2,1,3,1]]}"#
        );
        // A gap in the optional fields is refused.
        let gap = RoomSettings {
            mop_effect: None,
            mop_temp: Some(1),
            ..s
        };
        assert!(room_settings(&m, &[gap]).is_err());
        assert!(room_settings(&m, &[RoomSettings { times: 0, ..s }]).is_err());
        m.restrictions
            .thresholds
            .push(Line::new(0.0, 0.0, 0.0, 100.0));
        m.restrictions.ramps = vec![Ramp {
            rect: Rect::new(0.0, 0.0, 50.0, 50.0),
            angle: 10,
        }];
        assert_eq!(
            thresholds(&m).unwrap().to_json(),
            r#"{"vws":{"vwsl":[[0,0,0,100]],"npthrsd":[],"ramp":[[0,0,50,50,10]]}}"#
        );
        assert_eq!(
            curtains(&m).unwrap().to_json(),
            r#"{"curtain":{"line":[]}}"#
        );
        // Stored fields and keys survive: a flag the robot appended, and a key the model does
        // not read.
        let mut k = m.clone();
        k.stored_vws = Some(
            serde_json::from_value(json!({"vwsl": [[0, 0, 0, 100, 0]], "npthrsd": [], "ramp": [], "x": 1})).unwrap(),
        );
        k.restrictions.impassable_thresholds.push(Line::new(0.0, 0.0, 100.0, 0.0));
        assert_eq!(
            thresholds(&k).unwrap().to_json(),
            r#"{"vws":{"vwsl":[[0,0,0,100,0]],"npthrsd":[[0,0,100,0]],"ramp":[[0,0,50,50,10]],"x":1}}"#
        );
        k.stored_ct = Some(serde_json::from_value(json!({"line": [[0, 0, 0, 100, 0]]})).unwrap());
        k.restrictions.curtains = vec![Line::new(0.0, 0.0, 0.0, 100.0), Line::new(0.0, 0.0, 50.0, 0.0)];
        assert_eq!(curtains(&k).unwrap().to_json(), r#"{"curtain":{"line":[[0,0,0,100,0],[0,0,50,0]]}}"#);
        // Carpet corners snap to cell corners: the grid corner is at (0, 0), cells are 50 mm.
        assert_eq!(k.snap_to_cells(&Rect::new(12.0, 30.0, 140.0, 96.0)).to_quad(), [0, 50, 150, 100]);
        assert_eq!(
            furniture(&m).unwrap().to_json(),
            r#"{"mapfurniture_v2":[[0,2,1,200,100,0,100.0,100.0,0.0,90.0,0.0,0.0,1.0,1,0]]}"#
        );
        // A moved and rotated item keeps the fields the model does not read: field 5 and a
        // 16th field here.
        let mut k = m.clone();
        let stored: crate::trailer::FurnitureEntry =
            serde_json::from_value(json!([0, 2, 1, 200, 100, 7, 100.0, 100.0, 0.0, 90.0, 0.0, 0.0, 1.0, 1, 0, 1])).unwrap();
        k.furniture[0].stored = stored;
        k.furniture[0].center = Point::new(150.0, 125.0);
        k.furniture[0].angle = 180.0;
        assert_eq!(
            furniture(&k).unwrap().to_json(),
            r#"{"mapfurniture_v2":[[0,2,1,200,100,7,150.0,125.0,0.0,180.0,0.0,0.0,1.0,1,0,1]]}"#
        );
        k.furniture[0].center = Point::new(-5000.0, 0.0);
        assert!(furniture(&k).is_err(), "outside the map");
        assert_eq!(
            validate::rooms_crossed(&m, &Line::new(10.0, 75.0, 290.0, 75.0)),
            [1, 2].into()
        );
        let _ = Point::default();
    }
}
