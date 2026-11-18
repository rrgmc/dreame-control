//! Map edits: build the action 6.2 payload from the freshest saved map, and optionally send it.
//!
//! Without `apply` nothing is written: the payload is only printed. With `apply` the flow is:
//! refuse if the robot is busy; save the saved map and the live frame as "before" snapshots; send
//! the command; check the result code; wait until property 6.8 (the saved-map pointer) or 6.3
//! (the live frame) changes; read both again; save "after" snapshots; print what changed.
//!
//! The robot is reached through the [`MapAccess`] trait so the flow can be tested with a fake.

use anyhow::{anyhow, bail, Context, Result};
use dreame_map::edit::{self, EditCommand, RoomSettings};
use dreame_map::model::{CarpetArea, CarpetShape, Furniture, Line, Point, Ramp, Rect};
use dreame_map::{Frame, Map};
use crate::maplist::SavedMaps;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The carpet type the app gives a new carpet (map-plugin.md §6.3; 7 is the generic type).
pub const NEW_CARPET_TYPE: i64 = 7;

/// A new furniture item of wire type `kind`, centred at `center`, as the editor places one. Its
/// id is 0, like every item r2579a stores, and its edit type 1, like most of them.
pub fn new_furniture(kind: i64, center: Point, width: f64, height: f64) -> Furniture {
    Furniture { id: 0, kind, segment: 0, width, height, center, angle: 0.0, scale: 1.0, edit_type: 1, modified: 0, stored: Default::default() }
}

/// Map version for r2579a (maps.md §2.9). Models with `MAP_V2` need 3.
pub const MAP_VERSION: u8 = 2;

/// What the edit flow needs from the robot.
#[allow(async_fn_in_trait)]
pub trait MapAccess {
    /// The current value of property 6.8 (the saved-map list pointer), as text.
    async fn saved_pointer(&self) -> Result<String>;
    /// Every saved map in the list the 6.8 pointer names, with its envelope.
    async fn saved_maps(&self, pointer: &str) -> Result<SavedMaps>;
    /// Runs action 6.2 with the command and returns the action result.
    async fn update_map(&self, cmd: &EditCommand) -> Result<Value>;
    /// `Some(reason)` when the robot is busy (cleaning, paused in a task, mapping).
    async fn busy(&self) -> Result<Option<String>>;
    /// The current value of property 6.3 (the live map frame's object), as text.
    async fn live_pointer(&self) -> Result<String>;
    /// The envelope text of the live map frame, given the 6.3 pointer.
    async fn live_envelope(&self, pointer: &str) -> Result<String>;
}

/// A saved map as read from the robot, with the live frame's per-room settings on its rooms when
/// it is the robot's current map.
///
/// The saved map has no `cleanset`; the robot keeps per-room cleaning settings only in the live
/// frame (6.3), which is also where the app reads them. The live frame belongs to the current
/// map, so another floor's map has no settings.
#[derive(Clone)]
pub struct Saved {
    pub pointer: String,
    pub envelope: String,
    pub map: Map,
    /// True when this is the map the robot uses now.
    pub current: bool,
    /// The custom map name from the list; empty when the map is not named.
    pub name: String,
    /// The `map_id` of every saved map in the list, in list order.
    pub map_ids: Vec<i64>,
    /// Trailer keys the typed parser could not read. The model misses whatever they hold.
    pub unparsed: Vec<String>,
    pub live_pointer: String,
    pub live_envelope: String,
}

impl Saved {
    /// Refuses a map with unread keys: an edit built from it could drop what they hold.
    pub fn check_fully_read(&self) -> Result<()> {
        if !self.unparsed.is_empty() {
            bail!(
                "the saved map has keys the parser could not read ({:?}); fix the parser before editing",
                self.unparsed
            );
        }
        Ok(())
    }
}

/// Reads the robot's current saved map. See [`read_saved_map`].
pub async fn read_saved<R: MapAccess>(robot: &R) -> Result<Saved> {
    read_saved_map(robot, None).await
}

/// Every saved map the robot holds (one per floor), with the list's current map id.
pub async fn read_saved_maps<R: MapAccess>(robot: &R) -> Result<SavedMaps> {
    robot.saved_maps(&robot.saved_pointer().await?).await
}

/// Reads saved map `map_id`, or the robot's current map when `None`, with the live frame.
pub async fn read_saved_map<R: MapAccess>(robot: &R, map_id: Option<i64>) -> Result<Saved> {
    // The saved maps and the live frame are independent downloads; fetch them together.
    let saved = async {
        let pointer = robot.saved_pointer().await?;
        let maps = robot.saved_maps(&pointer).await?;
        anyhow::Ok((pointer, maps))
    };
    let live = async {
        let pointer = robot.live_pointer().await?;
        let envelope = robot.live_envelope(&pointer).await?;
        anyhow::Ok((pointer, envelope))
    };
    let ((pointer, maps), (live_pointer, live_envelope)) = tokio::try_join!(saved, live)?;
    let chosen = maps.pick(map_id)?;
    let frame = Frame::decode(&chosen.envelope, None, None)?;
    let mut map = Map::from_frame(&frame, MAP_VERSION, true)?;
    let live = Frame::decode(&live_envelope, None, None).context("decode the live frame (6.3)")?;
    // The live frame names the map it belongs to (`curid`); without it, trust the list.
    let current_id = live.trailer.curid.as_ref().map(|c| c.value).or(maps.current_map().map(|m| m.map_id));
    let current = current_id == Some(chosen.map_id);
    if let (true, Some(cleanset)) = (current, &live.trailer.cleanset) {
        for room in map.rooms.values_mut() {
            if let Some(s) = cleanset.value.get(&room.id) {
                room.settings = Some(s.clone());
            }
        }
    }
    Ok(Saved {
        pointer,
        envelope: chosen.envelope.clone(),
        map,
        current,
        name: chosen.name.clone(),
        map_ids: maps.ids(),
        unparsed: frame.trailer.unparsed.clone(),
        live_pointer,
        live_envelope,
    })
}

/// One edit, as given on the command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    RenameRoom { id: u32, name: String },
    AddNoGo(Rect),
    RemoveNoGo(usize),
    AddNoMop(Rect),
    RemoveNoMop(usize),
    AddWall(Line),
    RemoveWall(usize),
    /// Adds a rectangular carpet (`cpt`), corners snapped to cell corners as the app does, with
    /// the lowest free id and the generic type 7.
    AddCarpet(Rect),
    RemoveCarpet(usize),
    /// Adds an area where detected carpet is ignored (`cpt` `nocpt`), snapped like a carpet.
    AddIgnoredCarpet(Rect),
    RemoveIgnoredCarpet(usize),
    /// Adds a virtual threshold (`vws` `vwsl`).
    AddThreshold(Line),
    RemoveThreshold(usize),
    /// Adds an impassable threshold (`vws` `npthrsd`).
    AddImpassableThreshold(Line),
    RemoveImpassableThreshold(usize),
    /// Adds a curtain line (`curtain`, stored as `ct`).
    AddCurtain(Line),
    RemoveCurtain(usize),
    /// Replaces furniture item `index` (`mapfurniture_v2`, the full list). Its room is set from
    /// the room under its new centre.
    SetFurniture(usize, Box<Furniture>),
    /// Adds a furniture item; its room is set from the room under its centre.
    AddFurniture(Box<Furniture>),
    RemoveFurniture(usize),
    /// Adds a ramp (`vws` `ramp`): a rectangle and an angle in degrees, as stored.
    AddRamp(Rect, i64),
    RemoveRamp(usize),
    /// Adds a round carpet (`cpt`, ellipse flag 1) inscribed in the rectangle: centre snapped
    /// to a cell centre, sizes to whole cells. Whether r2579a reads the sizes as widths or radii
    /// is not known; this sends the full widths.
    AddRoundCarpet(Rect),
    /// Changes one room's settings; every other room is resent unchanged.
    RoomSettings {
        id: u32,
        suction: Option<i64>,
        water: Option<i64>,
        times: Option<i64>,
        clean_type: Option<i64>,
        mop_effect: Option<i64>,
    },
    /// Sets every room's name and type back to what a snapshot of the same map holds. This is
    /// the undo for `RenameRoom` that never needs the old name on the command line.
    RestoreNames(Box<Map>),
    /// Sends every room's cleaning settings (`customeClean`) as a snapshot of the live frame
    /// holds them. A map restore drops them, and this puts them back.
    RestoreSettings(Box<Map>),
    /// Sets the full list of hidden rooms (`delsr`); an empty list shows every room.
    HideRooms(Vec<u32>),
    /// Sets the cleaning order of the visible rooms (`cleanareaorder`). Hidden rooms the stored
    /// order lists are kept at the end, as the app does.
    CleaningOrder(Vec<u32>),
    /// Merges two neighbour rooms (`msr`). Room ids change afterwards.
    MergeRooms(u32, u32),
    /// Splits a room along a line in world mm (`dsrid`). Both ends must lie outside the room's
    /// floor, and the line must cross no other room.
    SplitRoom { id: u32, line: Line },
    /// Moves the boundary between neighbour rooms `a` and `b` to `line`: merge them, split the
    /// merged room along the line, then give each half back the name, type, floor, settings and
    /// cleaning-order place of the room it mostly overlaps. Several writes; see [`apply`].
    MoveBoundary { a: u32, b: u32, line: Line },
}

/// Reads a snapshot written by `map snapshot` or `map edit --apply`.
pub fn read_snapshot(path: &Path) -> Result<Map> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let frame = Frame::decode(&text, None, None)?;
    Ok(Map::from_frame(&frame, MAP_VERSION, true)?)
}

/// A copy of a command for printing, with room names hidden (CLAUDE.md: never print them).
pub fn redacted(cmd: &EditCommand) -> Value {
    let mut v = cmd.value().clone();
    if let Some(nsr) = v.get_mut("nsr").and_then(Value::as_object_mut) {
        for entry in nsr.values_mut() {
            if let Some(name) = entry.get_mut("name") {
                *name = Value::from("<hidden>");
            }
        }
    }
    v
}

fn take<T>(list: &mut Vec<T>, index: usize, what: &str) -> Result<()> {
    if index >= list.len() {
        bail!("there is no {what} {index}; the map has {}", list.len());
    }
    list.remove(index);
    Ok(())
}

/// Builds the command for an edit from the saved map.
pub fn build(map: &Map, e: &Edit) -> Result<EditCommand> {
    let mut m = map.clone();
    let r = &mut m.restrictions;
    let cmd = match e {
        Edit::RenameRoom { id, name } => edit::rename_room(map, *id, name)?,
        Edit::AddNoGo(rect) => {
            r.no_go.push(*rect);
            edit::restrictions(&m)?
        }
        Edit::RemoveNoGo(i) => {
            take(&mut r.no_go, *i, "no-go zone")?;
            edit::restrictions(&m)?
        }
        Edit::AddNoMop(rect) => {
            r.no_mop.push(*rect);
            edit::restrictions(&m)?
        }
        Edit::RemoveNoMop(i) => {
            take(&mut r.no_mop, *i, "no-mop zone")?;
            edit::restrictions(&m)?
        }
        Edit::AddWall(line) => {
            r.virtual_walls.push(*line);
            edit::restrictions(&m)?
        }
        Edit::RemoveWall(i) => {
            take(&mut r.virtual_walls, *i, "virtual wall")?;
            edit::restrictions(&m)?
        }
        Edit::AddThreshold(line) => {
            r.thresholds.push(*line);
            edit::thresholds(&m)?
        }
        Edit::RemoveThreshold(i) => {
            take(&mut r.thresholds, *i, "threshold")?;
            edit::thresholds(&m)?
        }
        Edit::AddImpassableThreshold(line) => {
            r.impassable_thresholds.push(*line);
            edit::thresholds(&m)?
        }
        Edit::RemoveImpassableThreshold(i) => {
            take(&mut r.impassable_thresholds, *i, "impassable threshold")?;
            edit::thresholds(&m)?
        }
        Edit::AddCurtain(line) => {
            r.curtains.push(*line);
            edit::curtains(&m)?
        }
        Edit::RemoveCurtain(i) => {
            take(&mut r.curtains, *i, "curtain")?;
            edit::curtains(&m)?
        }
        Edit::AddCarpet(rect) => {
            let shape = CarpetShape::Rect(m.snap_to_cells(rect));
            let id = edit::next_carpet_id(&m.carpet_areas);
            m.carpet_areas.push(CarpetArea { id, shape, kind: Some(NEW_CARPET_TYPE), extra: Vec::new() });
            edit::carpets(&m)?
        }
        Edit::RemoveCarpet(i) => {
            take(&mut m.carpet_areas, *i, "carpet")?;
            edit::carpets(&m)?
        }
        Edit::AddIgnoredCarpet(rect) => {
            let snapped = m.snap_to_cells(rect);
            m.ignored_carpet.push(snapped);
            edit::carpets(&m)?
        }
        Edit::RemoveIgnoredCarpet(i) => {
            take(&mut m.ignored_carpet, *i, "ignored-carpet area")?;
            edit::carpets(&m)?
        }
        Edit::SetFurniture(i, f) => {
            let mut f = (**f).clone();
            f.segment = m.room_at(f.center).map_or(f.segment, i64::from);
            let slot = m.furniture.get_mut(*i).ok_or_else(|| anyhow!("there is no furniture {i}"))?;
            *slot = f;
            edit::furniture(&m)?
        }
        Edit::AddFurniture(f) => {
            let mut f = (**f).clone();
            f.segment = m.room_at(f.center).map_or(0, i64::from);
            m.furniture.push(f);
            edit::furniture(&m)?
        }
        Edit::RemoveFurniture(i) => {
            take(&mut m.furniture, *i, "furniture")?;
            edit::furniture(&m)?
        }
        Edit::AddRamp(rect, angle) => {
            m.restrictions.ramps.push(Ramp { rect: *rect, angle: *angle });
            edit::thresholds(&m)?
        }
        Edit::RemoveRamp(i) => {
            take(&mut m.restrictions.ramps, *i, "ramp")?;
            edit::thresholds(&m)?
        }
        Edit::AddRoundCarpet(rect) => {
            let center = Point::new((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
            let (center, size) = m.snap_ellipse(center, (rect.width(), rect.height()));
            let id = edit::next_carpet_id(&m.carpet_areas);
            let shape = CarpetShape::Ellipse { center, size };
            m.carpet_areas.push(CarpetArea { id, shape, kind: Some(NEW_CARPET_TYPE), extra: Vec::new() });
            edit::carpets(&m)?
        }
        Edit::RoomSettings { id, suction, water, times, clean_type, mop_effect } => {
            let mut all = RoomSettings::current(map);
            let s = all
                .iter_mut()
                .find(|s| s.id == *id)
                .ok_or_else(|| anyhow!("room {id} has no stored settings (the robot keeps them only for its current map)"))?;
            s.suction = suction.unwrap_or(s.suction);
            s.water = water.unwrap_or(s.water);
            s.times = times.unwrap_or(s.times);
            if clean_type.is_some() {
                s.clean_type = *clean_type;
            }
            if mop_effect.is_some() {
                s.mop_effect = *mop_effect;
            }
            edit::room_settings(map, &all)?
        }
        Edit::RestoreNames(snap) => {
            if snap.map_id != map.map_id {
                bail!("the snapshot is of map {}, the robot has map {}", snap.map_id, map.map_id);
            }
            let ids = |m: &Map| m.rooms.keys().copied().collect::<Vec<_>>();
            if ids(snap) != ids(map) {
                bail!("the rooms changed since the snapshot ({:?} now {:?})", ids(snap), ids(map));
            }
            edit::rename_rooms(snap, &[])?
        }
        Edit::RestoreSettings(snap) => {
            // The live frame's map id counts map sessions, so it differs from the saved map's;
            // match by room instead.
            let all = RoomSettings::current(snap);
            if all.is_empty() {
                bail!("the snapshot has no per-room settings; use a live-frame snapshot (*-live.txt)");
            }
            if let Some(s) = all.iter().find(|s| !map.rooms.contains_key(&s.id)) {
                bail!("room {} of the snapshot is not on the map; the rooms changed since", s.id);
            }
            edit::room_settings(map, &all)?
        }
        Edit::HideRooms(ids) => edit::hide_rooms(map, ids)?,
        Edit::CleaningOrder(ids) => edit::cleaning_order(map, ids)?,
        Edit::MergeRooms(a, b) => edit::merge_rooms(map, *a, *b)?,
        Edit::SplitRoom { id, line } => edit::split_room(map, *id, *line)?,
        Edit::MoveBoundary { .. } => bail!("a boundary move takes several commands; use steps()"),
    };
    Ok(cmd)
}

/// The commands an edit sends up front, with a short label each. A boundary move has two (merge,
/// then split); the commands that restore names, settings and order are built only after the
/// split, from the map the robot returns (see [`restore_after_move`]).
pub fn steps(map: &Map, e: &Edit) -> Result<Vec<(&'static str, EditCommand)>> {
    Ok(match e {
        Edit::MoveBoundary { a, b, line } => {
            let [merge, split] = edit::move_boundary(map, *a, *b, *line)?;
            vec![("merge", merge), ("split", split)]
        }
        _ => vec![("edit", build(map, e)?)],
    })
}

/// Which room ids hold the two halves after a boundary move, from the map before the move and
/// the map after the split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovedRooms {
    /// The half that overlaps old room `a` most; it takes `a`'s metadata.
    pub like_a: u32,
    /// The half that overlaps old room `b` most; it takes `b`'s metadata.
    pub like_b: u32,
}

/// Finds the two halves after merge and split. The merged room keeps id `a`, and the split adds
/// one room whose id is new (or `b` again). Each half is matched to the old room whose floor it
/// overlaps most.
pub fn find_moved_rooms(before: &Map, after: &Map, a: u32, b: u32) -> Result<MovedRooms> {
    if !after.rooms.contains_key(&a) {
        bail!("room {a} is gone after the split; the robot did not keep the merged room's id");
    }
    let new: Vec<u32> = after.rooms.keys().copied().filter(|id| *id != a && (*id == b || !before.rooms.contains_key(id))).collect();
    let [n] = new[..] else {
        bail!("expected one new room after the split, found {new:?}");
    };
    // The half with more of old b's floor plays b. Ties go to the usual case, where a stays a.
    let (oa, on) = (after.floor_overlap(a, before, b), after.floor_overlap(n, before, b));
    Ok(if oa > on { MovedRooms { like_a: n, like_b: a } } else { MovedRooms { like_a: a, like_b: n } })
}

/// The commands that give the halves back their old metadata, each with a label: names and
/// types (`nsr`), floor material (`nsm`), cleaning settings (`customeClean`) and the cleaning
/// order (`cleanareaorder`). A command is left out when nothing in it would change.
pub fn restore_after_move(before: &Map, after: &Map, a: u32, b: u32, ids: MovedRooms) -> Result<Vec<(&'static str, EditCommand)>> {
    let pairs = [(a, ids.like_a), (b, ids.like_b)];
    let mut target = after.clone();
    for (old, new) in pairs {
        let (src, dst) = (&before.rooms[&old], target.rooms.get_mut(&new).expect("found above"));
        dst.name = src.name.clone();
        dst.room_type = src.room_type;
        dst.index = src.index;
        dst.material = src.material;
        dst.direction = src.direction;
    }
    let mut out = Vec::new();
    let names = |m: &Map, id: u32| m.rooms.get(&id).map(|r| (r.name.clone(), r.room_type, r.index));
    if pairs.iter().any(|&(_, n)| names(&target, n) != names(after, n)) {
        out.push(("names", edit::rename_rooms(&target, &[])?));
    }
    let floor = |m: &Map, id: u32| m.rooms.get(&id).map(|r| (r.material, r.direction));
    let floors: Vec<(u32, i64, i64)> = pairs
        .iter()
        .filter(|&&(_, n)| floor(&target, n) != floor(after, n))
        .filter_map(|&(_, n)| Some((n, target.rooms[&n].material?, target.rooms[&n].direction.unwrap_or(0))))
        .collect();
    if !floors.is_empty() {
        out.push(("floor", edit::floor_material(after, &floors)?));
    }
    let mut settings = RoomSettings::current(after);
    let mut changed = false;
    for (old, new) in pairs {
        let Some(src) = before.rooms[&old].settings.as_deref().and_then(|s| RoomSettings::from_cleanset(new, s)) else {
            continue;
        };
        match settings.iter_mut().find(|s| s.id == new) {
            Some(s) if *s == src => {}
            Some(s) => {
                *s = src;
                changed = true;
            }
            None => {
                settings.push(src);
                changed = true;
            }
        }
    }
    if changed {
        out.push(("settings", edit::room_settings(after, &settings)?));
    }
    let rename = |id: u32| pairs.iter().find(|p| p.0 == id).map_or(id, |p| p.1);
    let visible = |m: &Map, id: &u32| m.rooms.get(id).is_some_and(|r| !r.hidden);
    let mut want: Vec<u32> = before.cleaning_order().into_iter().map(rename).filter(|id| visible(after, id)).collect();
    // Visible rooms the old order did not list go last, as the editor shows them.
    for id in after.rooms.keys() {
        if visible(after, id) && !want.contains(id) {
            want.push(*id);
        }
    }
    let have: Vec<u32> = after.cleaning_order().into_iter().filter(|id| visible(after, id)).collect();
    if want != have && !want.is_empty() {
        out.push(("order", edit::cleaning_order(after, &want)?));
    }
    Ok(out)
}

/// The result code of action 6.2: the `out` entry with piid 6 (0 means success).
pub fn result_code(result: &Value) -> Option<i64> {
    let out = result.get("out").unwrap_or(result).as_array()?;
    out.iter()
        .find(|o| o.get("piid").and_then(Value::as_i64) == Some(6))
        .and_then(|o| o.get("value"))
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

/// What changed between two saved maps, one line per change. Room names are not printed.
pub fn diff(before: &Map, after: &Map) -> Vec<String> {
    let mut out = Vec::new();
    if before.map_id != after.map_id {
        out.push(format!("map id {} -> {}", before.map_id, after.map_id));
    }
    for id in before.rooms.keys().chain(after.rooms.keys()).collect::<std::collections::BTreeSet<_>>() {
        match (before.rooms.get(id), after.rooms.get(id)) {
            (Some(_), None) => out.push(format!("room {id} removed")),
            (None, Some(_)) => out.push(format!("room {id} added")),
            (Some(a), Some(b)) => {
                if a.name != b.name {
                    out.push(format!("room {id}: name changed"));
                }
                if (a.room_type, a.index) != (b.room_type, b.index) {
                    out.push(format!("room {id}: type {:?}/{:?} -> {:?}/{:?}", a.room_type, a.index, b.room_type, b.index));
                }
                if a.hidden != b.hidden {
                    out.push(format!("room {id}: hidden {} -> {}", a.hidden, b.hidden));
                }
                if a.order != b.order {
                    out.push(format!("room {id}: order {:?} -> {:?}", a.order, b.order));
                }
                if a.settings != b.settings {
                    out.push(format!("room {id}: settings {:?} -> {:?}", a.settings, b.settings));
                }
                if a.pixel_count != b.pixel_count {
                    out.push(format!("room {id}: {} -> {} pixels", a.pixel_count, b.pixel_count));
                }
            }
            (None, None) => {}
        }
    }
    let (ra, rb) = (&before.restrictions, &after.restrictions);
    list_diff(&mut out, "no-go zone", &quads(&ra.no_go), &quads(&rb.no_go));
    list_diff(&mut out, "no-mop zone", &quads(&ra.no_mop), &quads(&rb.no_mop));
    let walls = |v: &[Line]| v.iter().map(Line::to_quad).collect::<Vec<_>>();
    list_diff(&mut out, "virtual wall", &walls(&ra.virtual_walls), &walls(&rb.virtual_walls));
    let carpets = |m: &Map| m.carpet_areas.iter().map(|c| (c.id, c.shape.to_quad())).collect::<Vec<_>>();
    list_diff(&mut out, "carpet area", &carpets(before), &carpets(after));
    list_diff(&mut out, "ignored-carpet area", &quads(&before.ignored_carpet), &quads(&after.ignored_carpet));
    list_diff(&mut out, "threshold", &walls(&ra.thresholds), &walls(&rb.thresholds));
    list_diff(&mut out, "impassable threshold", &walls(&ra.impassable_thresholds), &walls(&rb.impassable_thresholds));
    list_diff(&mut out, "curtain", &walls(&ra.curtains), &walls(&rb.curtains));
    let furn = |m: &Map| m.furniture.iter().map(|f| Raw(Value::Array(f.to_entry().to_values()))).collect::<Vec<_>>();
    list_diff(&mut out, "furniture", &furn(before), &furn(after));
    let ramps = |m: &Map| m.restrictions.ramps.iter().map(|r| (r.rect.to_quad(), r.angle)).collect::<Vec<_>>();
    list_diff(&mut out, "ramp", &ramps(before), &ramps(after));
    // Raw vw objects: catches stored fields the model drops, such as trailing flags.
    let vw = |m: &Map| m.stored_vw.as_ref().and_then(|v| serde_json::to_value(v).ok());
    if vw(before) != vw(after) {
        out.push(format!(
            "stored vw: {} -> {}",
            vw(before).map_or("none".into(), |v| v.to_string()),
            vw(after).map_or("none".into(), |v| v.to_string())
        ));
    }
    out
}

fn quads(v: &[Rect]) -> Vec<[i64; 4]> {
    v.iter().map(Rect::to_quad).collect()
}

/// A JSON value that prints as JSON in [`list_diff`] output.
#[derive(PartialEq)]
struct Raw(Value);

impl std::fmt::Debug for Raw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn list_diff<T: PartialEq + std::fmt::Debug>(out: &mut Vec<String>, what: &str, a: &[T], b: &[T]) {
    for x in a.iter().filter(|x| !b.contains(x)) {
        out.push(format!("{what} removed: {x:?}"));
    }
    for x in b.iter().filter(|x| !a.contains(x)) {
        out.push(format!("{what} added: {x:?}"));
    }
}

/// How long to wait for 6.8 or 6.3 to change after an edit.
#[derive(Debug, Clone, Copy)]
pub struct Wait {
    pub interval: Duration,
    pub attempts: u32,
}

impl Default for Wait {
    fn default() -> Self {
        Self { interval: Duration::from_secs(3), attempts: 20 }
    }
}

/// The outcome of an applied edit.
#[derive(Debug)]
pub struct Report {
    pub code: Option<i64>,
    pub result: Value,
    /// Whether 6.8 (saved map) changed within the wait.
    pub pointer_changed: bool,
    /// Whether 6.3 (live frame) changed within the wait.
    pub live_changed: bool,
    pub changes: Vec<String>,
    pub before_file: PathBuf,
    pub after_file: PathBuf,
    /// One line per command sent, for edits that take several (a boundary move).
    pub steps: Vec<String>,
}

/// Writes `<dir>/<stamp>-<label>.txt` and returns its path.
fn snapshot(dir: &Path, stamp: &str, label: &str, text: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let p = dir.join(format!("{stamp}-{label}.txt"));
    std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))?;
    Ok(p)
}

/// Sends the edit to the robot's current map and reads the result back. See [`apply_to`].
pub async fn apply<R: MapAccess>(robot: &R, e: &Edit, dir: &Path, stamp: &str, wait: Wait) -> Result<Report> {
    apply_to(robot, None, e, dir, stamp, wait).await
}

/// Sends the edit to saved map `map_id` (the current map when `None`) and reads the result back.
/// See the module comment for the steps. Edits carry the map id (`mapid`); per-room settings
/// exist only for the current map.
pub async fn apply_to<R: MapAccess>(robot: &R, map_id: Option<i64>, e: &Edit, dir: &Path, stamp: &str, wait: Wait) -> Result<Report> {
    if let Some(why) = robot.busy().await? {
        bail!("the robot is busy ({why}); edit it while it is idle on the dock");
    }
    let before = read_saved_map(robot, map_id).await?;
    before.check_fully_read()?;
    let first = steps(&before.map, e)?;
    check_target(&before, e, &first)?;
    let before_file = snapshot(dir, stamp, "before", &before.envelope)?;
    snapshot(dir, stamp, "before-live", &before.live_envelope)?;
    let Edit::MoveBoundary { a, b, .. } = *e else {
        let (_, cmd) = first.into_iter().next().ok_or_else(|| anyhow!("no command"))?;
        snapshot(dir, stamp, "command", &cmd.to_json())?;
        let s = send_step(robot, &before, &cmd, wait).await?;
        return finish(&before, s, dir, stamp, before_file, Vec::new());
    };

    // A boundary move: merge, split, then restore what the two rooms had. Each step works on
    // the map the previous step left, and stops at the first failure; the snapshots of every
    // step stay in `dir`.
    let mut log = Vec::new();
    let mut current = before.clone();
    let mut last = None;
    for (i, (label, cmd)) in first.iter().enumerate() {
        let s = run_step(robot, &current, cmd, dir, stamp, i + 1, label, wait, &mut log).await?;
        current = s.after.clone();
        last = Some(s);
    }
    let ids = find_moved_rooms(&before.map, &current.map, a, b)
        .with_context(|| format!("after the split; steps so far: {log:?}"))?;
    log.push(format!("room {} now plays room {a}, room {} plays room {b}", ids.like_a, ids.like_b));
    let restore = restore_after_move(&before.map, &current.map, a, b, ids)?;
    for (i, (label, cmd)) in restore.iter().enumerate() {
        let s = run_step(robot, &current, cmd, dir, stamp, first.len() + i + 1, label, wait, &mut log).await?;
        current = s.after.clone();
        last = Some(s);
    }
    let s = last.ok_or_else(|| anyhow!("no step ran"))?;
    finish(&before, s, dir, stamp, before_file, log)
}

/// Refuses an edit of a map that is not the robot's current one (another floor) when a command
/// has no `mapid`: the robot applies such commands to its current map. Zones, walls, carpets,
/// curtains, furniture, the cleaning order and per-room settings are of that kind, and so is a
/// boundary move, which restores settings and order at the end. Room names, types, merges,
/// splits, hiding and floor materials carry `mapid`.
pub fn check_target(saved: &Saved, e: &Edit, cmds: &[(&'static str, EditCommand)]) -> Result<()> {
    if saved.current {
        return Ok(());
    }
    let without = cmds.iter().find(|(_, c)| c.value().get("mapid").is_none());
    if matches!(e, Edit::MoveBoundary { .. }) || without.is_some() {
        bail!(
            "map {} is not the robot's current map, and this edit does not name its map, so the robot \
             would apply it to the current map; select map {} on the robot first",
            saved.map.map_id,
            saved.map.map_id
        );
    }
    Ok(())
}

/// What one sent command led to.
struct Step {
    code: Option<i64>,
    result: Value,
    pointer_changed: bool,
    live_changed: bool,
    after: Saved,
}

/// Sends one command, checks its result code, waits until 6.8 or 6.3 changes, and reads the
/// map back.
async fn send_step<R: MapAccess>(robot: &R, before: &Saved, cmd: &EditCommand, wait: Wait) -> Result<Step> {
    let result = robot.update_map(cmd).await?;
    let code = result_code(&result);
    if code != Some(0) {
        bail!("the robot did not accept the edit: result code {code:?}, result {result}");
    }
    // Map edits show up in the saved map (6.8); per-room settings in the live frame (6.3).
    let (mut pointer_changed, mut live_changed) = (false, false);
    for _ in 0..wait.attempts {
        tokio::time::sleep(wait.interval).await;
        pointer_changed = robot.saved_pointer().await? != before.pointer;
        live_changed = robot.live_pointer().await? != before.live_pointer;
        if pointer_changed || live_changed {
            break;
        }
    }
    // Read back the same map, also when it is not the current one.
    let after = read_saved_map(robot, Some(i64::from(before.map.map_id))).await?;
    Ok(Step { code, result, pointer_changed, live_changed, after })
}

/// One numbered step of a multi-step edit, with its own command and after snapshots.
#[allow(clippy::too_many_arguments)]
async fn run_step<R: MapAccess>(
    robot: &R,
    current: &Saved,
    cmd: &EditCommand,
    dir: &Path,
    stamp: &str,
    n: usize,
    label: &str,
    wait: Wait,
    log: &mut Vec<String>,
) -> Result<Step> {
    let name = format!("step{n}-{label}");
    snapshot(dir, stamp, &format!("{name}-command"), &cmd.to_json())?;
    let s = send_step(robot, current, cmd, wait)
        .await
        .with_context(|| format!("step {n} ({label}) failed; steps before it: {log:?}"))?;
    snapshot(dir, stamp, &format!("{name}-after"), &s.after.envelope)?;
    snapshot(dir, stamp, &format!("{name}-after-live"), &s.after.live_envelope)?;
    log.push(format!(
        "step {n} {label}: {}; saved map changed: {}; live frame changed: {}",
        redacted(cmd),
        s.pointer_changed,
        s.live_changed
    ));
    if !s.after.unparsed.is_empty() {
        bail!("after step {n} ({label}) the map has keys the parser could not read: {:?}", s.after.unparsed);
    }
    Ok(s)
}

/// Saves the final snapshots and compares the end state with the start.
fn finish(before: &Saved, s: Step, dir: &Path, stamp: &str, before_file: PathBuf, steps: Vec<String>) -> Result<Report> {
    let after_file = snapshot(dir, stamp, "after", &s.after.envelope)?;
    snapshot(dir, stamp, "after-live", &s.after.live_envelope)?;
    let mut changes = diff(&before.map, &s.after.map);
    if !s.after.unparsed.is_empty() {
        changes.push(format!("WARNING: keys the parser could not read after the edit: {:?}", s.after.unparsed));
    }
    Ok(Report {
        code: s.code,
        result: s.result,
        pointer_changed: s.pointer_changed,
        live_changed: s.live_changed,
        changes,
        before_file,
        after_file,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_map::header::{FrameType, Header, Pose};
    use dreame_map::trailer::{encode_name, Trailer};
    use serde_json::json;
    use crate::maplist::SavedMap;
    use std::cell::RefCell;

    /// A 4 x 2 saved map: room 1 left, room 2 right; encoding E (room id in the low 6 bits).
    fn envelope(name: &str, no_go: &[[i64; 4]]) -> String {
        frame(json!({
            "seg_inf": {"1": {"name": encode_name(name), "type": 0, "index": 0, "nei_id": [2]},
                        "2": {"type": 1, "index": 0, "nei_id": [1]}},
            "cleanareaorder": [{"1": 1}, {"2": 2}],
            "vw": {"line": [], "rect": no_go, "mop": [], "cliff": [], "addcpt": [], "nocpt": []},
        }))
    }

    /// The live frame: same grid, with the per-room settings (`cleanset`), as on r2579a.
    fn live_envelope(cleanset: &Value) -> String {
        frame(json!({"cleanset": cleanset.to_string()}))
    }

    fn frame(trailer: Value) -> String {
        let trailer = Trailer::from_value(trailer).unwrap();
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width: 4,
            height: 2,
            left: 25,
            top: 25,
        };
        Frame { header, pixels: vec![1, 1, 2, 2, 1, 1, 2, 2], trailer, aes: None }.encode().unwrap()
    }

    /// A fake robot with a real grid, for boundary moves. It merges (`msr`) by relabelling pixels,
    /// splits (`dsrid`) by giving the floor left of the line's direction to a new room with the
    /// next free id, and stores names (`nsr`), floors (`nsm`), settings (`customeClean`) and the
    /// cleaning order (`cleanareaorder`). Pixels are encoding E: room id in the low 6 bits, 0x80
    /// for walls.
    struct GridFake {
        pixels: RefCell<Vec<u8>>,
        seg: RefCell<Value>,
        order: RefCell<Vec<u32>>,
        cleanset: RefCell<Value>,
        version: RefCell<u32>,
        sent: RefCell<Vec<Value>>,
        /// The highest room id handed out so far; a split takes the next one. With `reuse`, a
        /// split takes the lowest free id instead, which after a merge is the merged-away room's.
        top: RefCell<u8>,
        reuse: bool,
    }

    const W: usize = 6;

    impl GridFake {
        /// Room 1 (custom name "A") in columns 1-2, room 2 (type 2) in columns 3-4, walls round.
        fn new() -> Self {
            #[rustfmt::skip]
            let pixels = vec![
                0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
                0x81, 0x01, 0x01, 0x02, 0x02, 0x82,
                0x81, 0x01, 0x01, 0x02, 0x02, 0x82,
                0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
            ];
            Self {
                pixels: RefCell::new(pixels),
                seg: RefCell::new(json!({
                    "1": {"name": encode_name("A"), "type": 0, "index": 3, "material": 1, "direction": 90, "nei_id": [2]},
                    "2": {"type": 2, "index": 0, "material": 2, "direction": 0, "nei_id": [1]}
                })),
                order: RefCell::new(vec![2, 1]),
                cleanset: RefCell::new(json!({"1": [1, 3, 1, 2, 2, 546], "2": [3, 2, 2, 1, 2, 546]})),
                version: RefCell::new(0),
                sent: RefCell::new(vec![]),
                top: RefCell::new(2),
                reuse: false,
            }
        }

        fn envelope(&self) -> String {
            let order: Vec<Value> = self.order.borrow().iter().enumerate().map(|(i, id)| json!({id.to_string(): i + 1})).collect();
            let trailer = Trailer::from_value(json!({"seg_inf": *self.seg.borrow(), "cleanareaorder": order, "delsr": []})).unwrap();
            self.encode(trailer)
        }

        fn encode(&self, trailer: Trailer) -> String {
            let header = Header {
                map_id: 1,
                frame_id: 0,
                frame_type: FrameType::I,
                robot: Pose::new(0, 0, 0),
                charger: Pose::new(0, 0, 32767),
                grid_size: 50,
                width: W as i16,
                height: 4,
                left: 25,
                top: 25,
            };
            Frame { header, pixels: self.pixels.borrow().clone(), trailer, aes: None }.encode().unwrap()
        }

        fn apply(&self, v: &Value) {
            if let Some([a, b]) = v["msr"].as_array().map(|l| [l[0].as_u64().unwrap() as u8, l[1].as_u64().unwrap() as u8]) {
                for p in self.pixels.borrow_mut().iter_mut() {
                    if *p & 0x3f == b {
                        *p = (*p & 0xc0) | a;
                    }
                }
                self.seg.borrow_mut().as_object_mut().unwrap().remove(&b.to_string());
                self.order.borrow_mut().retain(|&id| id != u32::from(b));
                self.cleanset.borrow_mut().as_object_mut().unwrap().remove(&b.to_string());
            }
            if let Some(d) = v["dsrid"].as_array() {
                let q: Vec<f64> = d.iter().map(|x| x.as_f64().unwrap()).collect();
                let id = q[4] as u8;
                let new = if self.reuse {
                    (1..).find(|i: &u8| self.seg.borrow().get(i.to_string()).is_none()).unwrap()
                } else {
                    *self.top.borrow_mut() += 1;
                    *self.top.borrow()
                };
                let tr = Map::decode(&self.envelope(), MAP_VERSION, true).unwrap().transform();
                for (i, p) in self.pixels.borrow_mut().iter_mut().enumerate() {
                    let (x, y) = tr.cell_center(i % W, i / W);
                    let left = (q[2] - q[0]) * (y - q[1]) - (q[3] - q[1]) * (x - q[0]) > 0.0;
                    if *p == id && left {
                        *p = new;
                    }
                }
                self.seg.borrow_mut()[new.to_string()] = json!({"nei_id": [id]});
                self.order.borrow_mut().push(u32::from(new));
                self.cleanset.borrow_mut()[new.to_string()] = json!([1, 2, 1, self.order.borrow().len(), 2, 0]);
            }
            if let Some(nsr) = v["nsr"].as_object() {
                for (id, e) in nsr {
                    let mut seg = self.seg.borrow_mut();
                    let s = seg[id].as_object_mut().unwrap();
                    s.remove("name");
                    for (k, x) in e.as_object().unwrap() {
                        s.insert(k.clone(), x.clone());
                    }
                }
            }
            if let Some(nsm) = v["nsm"].as_object() {
                for (id, e) in nsm {
                    let mut seg = self.seg.borrow_mut();
                    seg[id]["material"] = e["material"].clone();
                    seg[id]["direction"] = e["direction"].clone();
                }
            }
            if let Some(list) = v["customeClean"].as_array() {
                let mut cs = self.cleanset.borrow_mut();
                for e in list {
                    let e: Vec<i64> = serde_json::from_value(e.clone()).unwrap();
                    let id = e[0].to_string();
                    let order = cs[&id][3].clone();
                    cs[&id] = json!([e[1], e[2], e[3], order, e[4], e[5]]);
                }
            }
            if let Some(list) = v["cleanareaorder"].as_array() {
                *self.order.borrow_mut() =
                    list.iter().map(|e| e.as_object().unwrap().keys().next().unwrap().parse().unwrap()).collect();
            }
        }
    }

    impl MapAccess for GridFake {
        async fn saved_pointer(&self) -> Result<String> {
            Ok(format!("v{}", self.version.borrow()))
        }
        async fn saved_maps(&self, _: &str) -> Result<SavedMaps> {
            Ok(one_map(self.envelope()))
        }
        async fn update_map(&self, cmd: &EditCommand) -> Result<Value> {
            self.sent.borrow_mut().push(cmd.value().clone());
            self.apply(cmd.value());
            *self.version.borrow_mut() += 1;
            Ok(json!({"out": [{"piid": 6, "value": 0}]}))
        }
        async fn busy(&self) -> Result<Option<String>> {
            Ok(None)
        }
        async fn live_pointer(&self) -> Result<String> {
            Ok(format!("l{}", self.version.borrow()))
        }
        async fn live_envelope(&self, _: &str) -> Result<String> {
            Ok(live_envelope(&self.cleanset.borrow()))
        }
    }

    #[tokio::test]
    async fn boundary_move_restores_metadata_when_ids_swap() {
        let robot = GridFake::new();
        let dir = tmp("boundary");
        let before = read_saved(&robot).await.unwrap().map;
        // The new boundary runs through column 2. The fake gives the left part (column 1) the new
        // id 3, so room 1 keeps the bigger, mostly-old-room-2 side and the roles swap.
        let line = Line::new(125.0, 10.0, 125.0, 190.0);
        let e = Edit::MoveBoundary { a: 1, b: 2, line };
        let first = steps(&before, &e).unwrap();
        assert_eq!(first.iter().map(|s| s.0).collect::<Vec<_>>(), ["merge", "split"]);
        let r = apply(&robot, &e, &dir, "m", fast()).await.unwrap();
        let sent: Vec<String> = robot.sent.borrow().iter().map(|v| v.as_object().unwrap().keys().next().unwrap().clone()).collect();
        // The order needs no fix: the new room is appended, which is where old room 1 was.
        assert_eq!(sent, ["msr", "dsrid", "nsr", "nsm", "customeClean"], "steps: {:?}", r.steps);
        assert!(r.steps[2].contains("room 3 now plays room 1, room 1 plays room 2"), "{:?}", r.steps);

        let after = read_saved(&robot).await.unwrap().map;
        let (old_a, old_b) = (&before.rooms[&1], &before.rooms[&2]);
        let (like_a, like_b) = (&after.rooms[&3], &after.rooms[&1]);
        assert_eq!((like_a.pixel_count, like_b.pixel_count), (2, 6));
        assert_eq!((&like_a.name, like_a.room_type, like_a.index), (&old_a.name, old_a.room_type, old_a.index));
        assert_eq!((like_b.room_type, like_b.index, like_b.name.is_none()), (old_b.room_type, old_b.index, true));
        assert_eq!((like_a.material, like_b.material), (old_a.material, old_b.material));
        let s = |r: &dreame_map::model::Room| r.settings.as_ref().map(|v| [v[0], v[1], v[2]]);
        assert_eq!((s(like_a), s(like_b)), (s(old_a), s(old_b)));
        assert_eq!(after.cleaning_order(), [1, 3], "room 2's first place goes to its successor, room 1");
        for f in ["m-before.txt", "m-step1-merge-command.txt", "m-step2-split-after.txt", "m-step5-settings-command.txt", "m-after.txt"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn boundary_move_when_the_robot_reuses_the_merged_id() {
        // Through column 3 with id reuse: the split gives columns 1-2 (all of old room 1) the
        // freed id 2, and room 1 keeps columns 3-4 (all of old room 2). The shapes are unchanged
        // but the ids swap, so the metadata must swap too.
        let robot = GridFake { reuse: true, ..GridFake::new() };
        let dir = tmp("boundary-same");
        let before = read_saved(&robot).await.unwrap().map;
        let e = Edit::MoveBoundary { a: 1, b: 2, line: Line::new(175.0, 10.0, 175.0, 190.0) };
        let r = apply(&robot, &e, &dir, "m", fast()).await.unwrap();
        let after = read_saved(&robot).await.unwrap().map;
        assert!(r.steps[2].contains("room 2 now plays room 1, room 1 plays room 2"), "{:?}", r.steps);
        assert_eq!(after.rooms[&2].pixel_count, before.rooms[&1].pixel_count);
        assert_eq!(after.rooms[&2].name, before.rooms[&1].name);
        assert_eq!(after.rooms[&1].room_type, before.rooms[&2].room_type);
        assert_eq!(after.cleaning_order(), [1, 2], "old room 2 was first; it is room 1 now");
        // Refused before anything is sent: the line ends inside the merged room.
        let robot = GridFake::new();
        let e = Edit::MoveBoundary { a: 1, b: 2, line: Line::new(125.0, 10.0, 125.0, 100.0) };
        assert!(apply(&robot, &e, &dir, "x", fast()).await.is_err());
        assert!(robot.sent.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The list of a robot with multi-floor off: one map, no `curr_id`.
    fn one_map(envelope: String) -> SavedMaps {
        SavedMaps { current: None, maps: vec![SavedMap { map_id: 1, name: String::new(), angle: 0, envelope }] }
    }

    /// A fake robot that applies `vw` and `nsr` edits to its stored map.
    struct Fake {
        /// With multi-floor on: map 2's envelope. Map 1 stays the current map.
        other_floor: Option<RefCell<String>>,
        name: RefCell<String>,
        no_go: RefCell<Vec<[i64; 4]>>,
        version: RefCell<u32>,
        cleanset: RefCell<Value>,
        live_version: RefCell<u32>,
        busy: Option<String>,
        code: i64,
        sent: RefCell<Vec<Value>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                other_floor: None,
                name: RefCell::new("A".into()),
                no_go: RefCell::new(vec![]),
                version: RefCell::new(0),
                cleanset: RefCell::new(json!({"1": [1, 3, 1, 1, 2, 546], "2": [2, 3, 1, 2, 2, 546]})),
                live_version: RefCell::new(0),
                busy: None,
                code: 0,
                sent: RefCell::new(vec![]),
            }
        }
    }

    impl MapAccess for Fake {
        async fn saved_pointer(&self) -> Result<String> {
            Ok(format!("v{}", self.version.borrow()))
        }
        async fn saved_maps(&self, _: &str) -> Result<SavedMaps> {
            let first = envelope(&self.name.borrow(), &self.no_go.borrow());
            Ok(match &self.other_floor {
                Some(other) => SavedMaps {
                    current: Some(1),
                    maps: vec![
                        SavedMap { map_id: 1, name: String::new(), angle: 0, envelope: first },
                        SavedMap { map_id: 2, name: "Upstairs".into(), angle: 0, envelope: other.borrow().clone() },
                    ],
                },
                None => one_map(first),
            })
        }
        async fn update_map(&self, cmd: &EditCommand) -> Result<Value> {
            let v = cmd.value().clone();
            self.sent.borrow_mut().push(v.clone());
            if self.code == 0 {
                if let Some(rect) = v["vw"]["rect"].as_array() {
                    *self.no_go.borrow_mut() = serde_json::from_value(Value::Array(rect.clone()))?;
                }
                if let Some(n) = v["nsr"]["1"]["name"].as_str() {
                    let bytes = base64_decode(n);
                    *self.name.borrow_mut() = String::from_utf8(bytes)?;
                }
                // Settings land in the live frame only; the order field (index 3) is kept.
                if let Some(list) = v["customeClean"].as_array() {
                    let mut cs = self.cleanset.borrow_mut();
                    for e in list {
                        let e: Vec<i64> = serde_json::from_value(e.clone())?;
                        let id = e[0].to_string();
                        let order = cs[&id][3].clone();
                        cs[&id] = json!([e[1], e[2], e[3], order, e[4], e[5]]);
                    }
                    *self.live_version.borrow_mut() += 1;
                    return Ok(json!({"code": 0, "out": [{"piid": 6, "value": 0}]}));
                }
                *self.version.borrow_mut() += 1;
            }
            Ok(json!({"siid": 6, "aiid": 2, "code": 0, "out": [{"piid": 6, "value": self.code}]}))
        }
        async fn busy(&self) -> Result<Option<String>> {
            Ok(self.busy.clone())
        }
        async fn live_pointer(&self) -> Result<String> {
            Ok(format!("l{}", self.live_version.borrow()))
        }
        async fn live_envelope(&self, _: &str) -> Result<String> {
            Ok(live_envelope(&self.cleanset.borrow()))
        }
    }

    fn base64_decode(s: &str) -> Vec<u8> {
        dreame_map::envelope::base64_decode(s).unwrap()
    }

    fn fast() -> Wait {
        Wait { interval: Duration::ZERO, attempts: 3 }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dreame-cli-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[tokio::test]
    async fn dry_run_builds_payloads() {
        let s = read_saved(&Fake::new()).await.unwrap();
        assert_eq!(build(&s.map, &Edit::HideRooms(vec![2])).unwrap().to_json(), r#"{"delsr":[2],"mapid":1}"#);
        assert_eq!(build(&s.map, &Edit::HideRooms(vec![])).unwrap().to_json(), r#"{"delsr":[],"mapid":1}"#);
        assert!(build(&s.map, &Edit::HideRooms(vec![1, 2])).is_err(), "cannot hide every room");
        assert_eq!(
            build(&s.map, &Edit::CleaningOrder(vec![2, 1])).unwrap().to_json(),
            r#"{"cleanareaorder":[{"2":1},{"1":2}]}"#
        );
        assert!(build(&s.map, &Edit::CleaningOrder(vec![2])).is_err(), "every visible room must be listed");
        let c = build(&s.map, &Edit::AddNoGo(Rect::new(0.0, 0.0, 100.0, 50.0))).unwrap();
        assert_eq!(
            c.to_json(),
            r#"{"vw":{"line":[],"rect":[[0,0,100,50]],"mop":[],"cliff":[],"addcpt":[],"nocpt":[]}}"#
        );
        assert!(build(&s.map, &Edit::RemoveNoGo(0)).is_err());
        let c = build(
            &s.map,
            &Edit::RoomSettings { id: 2, suction: Some(3), water: None, times: None, clean_type: None, mop_effect: None },
        )
        .unwrap();
        assert_eq!(c.to_json(), r#"{"customeClean":[[1,1,3,1,2,546],[2,3,3,1,2,546]]}"#);
        // Settings come back from a snapshot that has them onto a map that lost them.
        let mut lost = s.map.clone();
        for r in lost.rooms.values_mut() {
            r.settings = None;
        }
        let c = build(&lost, &Edit::RestoreSettings(Box::new(s.map.clone()))).unwrap();
        assert_eq!(c.to_json(), r#"{"customeClean":[[1,1,3,1,2,546],[2,2,3,1,2,546]]}"#);
        assert!(build(&s.map, &Edit::RestoreSettings(Box::new(lost.clone()))).is_err(), "no settings to restore");
        let mut fewer = lost.clone();
        fewer.rooms.remove(&2);
        assert!(build(&fewer, &Edit::RestoreSettings(Box::new(s.map.clone()))).is_err(), "rooms changed");
        assert!(build(&s.map, &Edit::RenameRoom { id: 9, name: "x".into() }).is_err());
        // A new carpet snaps to cell corners and gets id 1 and type 7.
        assert_eq!(
            build(&s.map, &Edit::AddCarpet(Rect::new(10.0, 10.0, 140.0, 90.0))).unwrap().to_json(),
            r#"{"cpt":{"addcpt":[[0,0,150,100,1,0,7]],"nocpt":[]}}"#
        );
        assert_eq!(
            build(&s.map, &Edit::AddIgnoredCarpet(Rect::new(60.0, 10.0, 140.0, 90.0))).unwrap().to_json(),
            r#"{"cpt":{"addcpt":[],"nocpt":[[50,0,150,100]]}}"#
        );
        assert!(build(&s.map, &Edit::RemoveCarpet(0)).is_err());
        assert_eq!(
            build(&s.map, &Edit::AddThreshold(Line::new(100.0, 0.0, 100.0, 100.0))).unwrap().to_json(),
            r#"{"vws":{"vwsl":[[100,0,100,100]],"npthrsd":[],"ramp":[]}}"#
        );
        assert_eq!(
            build(&s.map, &Edit::AddImpassableThreshold(Line::new(100.0, 0.0, 100.0, 100.0))).unwrap().to_json(),
            r#"{"vws":{"vwsl":[],"npthrsd":[[100,0,100,100]],"ramp":[]}}"#
        );
        assert_eq!(
            build(&s.map, &Edit::AddCurtain(Line::new(0.0, 50.0, 200.0, 50.0))).unwrap().to_json(),
            r#"{"curtain":{"line":[[0,50,200,50]]}}"#
        );
        assert!(build(&s.map, &Edit::RemoveCurtain(0)).is_err());
        // New furniture takes the room under its centre (room 1 at x 0..100) and the stored
        // wire form: floats for fields 6 to 12.
        let f = new_furniture(7, Point::new(50.0, 50.0), 100.0, 100.0);
        assert_eq!(
            build(&s.map, &Edit::AddFurniture(Box::new(f))).unwrap().to_json(),
            r#"{"mapfurniture_v2":[[0,7,1,100,100,0,50.0,50.0,0.0,0.0,0.0,0.0,1.0,1,0]]}"#
        );
        assert!(build(&s.map, &Edit::RemoveFurniture(0)).is_err());
        assert_eq!(
            build(&s.map, &Edit::AddRamp(Rect::new(0.0, 0.0, 100.0, 50.0), 90)).unwrap().to_json(),
            r#"{"vws":{"vwsl":[],"npthrsd":[],"ramp":[[0,0,100,50,90]]}}"#
        );
        // A round carpet from a 130 x 80 drag centred at (65, 40): centre to the nearest cell
        // centre (75, 25), sizes to whole 50 mm cells.
        assert_eq!(
            build(&s.map, &Edit::AddRoundCarpet(Rect::new(0.0, 0.0, 130.0, 80.0))).unwrap().to_json(),
            r#"{"cpt":{"addcpt":[[75,25,150,100,1,1,7]],"nocpt":[]}}"#
        );
        assert_eq!(build(&s.map, &Edit::MergeRooms(1, 2)).unwrap().to_json(), r#"{"msr":[1,2],"mapid":1}"#);
        assert!(build(&s.map, &Edit::MergeRooms(1, 1)).is_err());
        // Room 1 fills the map's full height, so no line can start and end outside it.
        let e = Edit::SplitRoom { id: 1, line: Line::new(75.0, 30.0, 75.0, 120.0) };
        assert!(build(&s.map, &e).is_err());
    }

    #[tokio::test]
    async fn apply_adds_then_removes_a_zone() {
        let robot = Fake::new();
        let dir = tmp("zone");
        let r = apply(&robot, &Edit::AddNoGo(Rect::new(0.0, 0.0, 100.0, 50.0)), &dir, "t1", fast()).await.unwrap();
        assert_eq!(r.code, Some(0));
        assert!(r.pointer_changed);
        assert_eq!(r.changes, ["no-go zone added: [0, 0, 100, 50]", "stored vw: {\"line\":[],\"rect\":[],\"mop\":[],\"cliff\":[],\"addcpt\":[],\"nocpt\":[]} -> {\"line\":[],\"rect\":[[0,0,100,50]],\"mop\":[],\"cliff\":[],\"addcpt\":[],\"nocpt\":[]}"]);
        assert!(r.before_file.is_file() && r.after_file.is_file());
        assert!(dir.join("t1-command.txt").is_file());
        let r = apply(&robot, &Edit::RemoveNoGo(0), &dir, "t2", fast()).await.unwrap();
        assert_eq!(r.changes[0], "no-go zone removed: [0, 0, 100, 50]");
        assert!(robot.no_go.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn settings_come_from_the_live_frame() {
        let robot = Fake::new();
        let dir = tmp("settings");
        let e = Edit::RoomSettings { id: 2, suction: Some(3), water: None, times: None, clean_type: None, mop_effect: None };
        let r = apply(&robot, &e, &dir, "s", fast()).await.unwrap();
        assert!(r.live_changed && !r.pointer_changed);
        assert_eq!(r.changes, ["room 2: settings Some([2, 3, 1, 2, 2, 546]) -> Some([3, 3, 1, 2, 2, 546])"]);
        assert_eq!(robot.sent.borrow()[0], json!({"customeClean": [[1, 1, 3, 1, 2, 546], [2, 3, 3, 1, 2, 546]]}));
        assert!(dir.join("s-before-live.txt").is_file() && dir.join("s-after-live.txt").is_file());
        // Restore.
        let e = Edit::RoomSettings { id: 2, suction: Some(2), water: None, times: None, clean_type: None, mop_effect: None };
        let r = apply(&robot, &e, &dir, "t", fast()).await.unwrap();
        assert_eq!(r.changes, ["room 2: settings Some([3, 3, 1, 2, 2, 546]) -> Some([2, 3, 1, 2, 2, 546])"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_renames_without_printing_names() {
        let robot = Fake::new();
        let dir = tmp("rename");
        let r = apply(&robot, &Edit::RenameRoom { id: 1, name: "Secret".into() }, &dir, "t", fast()).await.unwrap();
        assert_eq!(r.changes, ["room 1: name changed"]);
        assert_eq!(*robot.name.borrow(), "Secret");
        // The printed form hides names; the sent command keeps them.
        let sent = EditCommand(robot.sent.borrow()[0].clone());
        assert_eq!(redacted(&sent)["nsr"]["1"]["name"], "<hidden>");
        assert_ne!(sent.value()["nsr"]["1"]["name"], "<hidden>");
        // Undo from the before snapshot, without the old name on the command line.
        let snap = read_snapshot(&r.before_file).unwrap();
        let r = apply(&robot, &Edit::RestoreNames(Box::new(snap)), &dir, "u", fast()).await.unwrap();
        assert_eq!(r.changes, ["room 1: name changed"]);
        assert_eq!(*robot.name.borrow(), "A");
        // A snapshot of a different map is refused.
        let mut other = read_snapshot(&r.before_file).unwrap();
        other.map_id = 2;
        let now = read_saved(&robot).await.unwrap().map;
        assert!(build(&now, &Edit::RestoreNames(Box::new(other))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_refuses_when_busy_and_reports_rejection() {
        let dir = tmp("refuse");
        let busy = Fake { busy: Some("segment cleaning".into()), ..Fake::new() };
        let e = apply(&busy, &Edit::RemoveNoGo(0), &dir, "t", fast()).await.unwrap_err();
        assert!(e.to_string().contains("busy"));
        assert!(busy.sent.borrow().is_empty());
        assert!(!dir.exists(), "no snapshot before the busy check");

        let rejecting = Fake { code: 5, ..Fake::new() };
        let e = apply(&rejecting, &Edit::AddNoGo(Rect::new(0.0, 0.0, 100.0, 50.0)), &dir, "t", fast()).await.unwrap_err();
        assert!(e.to_string().contains("Some(5)"));
        assert!(dir.join("t-before.txt").is_file(), "the before snapshot is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Map 2 for the multi-floor fake: the fake's map 1 envelope with header `map_id` 2 and
    /// another room name.
    fn upstairs() -> String {
        let mut f = Frame::decode(&envelope("Up", &[]), None, None).unwrap();
        f.header.map_id = 2;
        f.encode().unwrap()
    }

    #[tokio::test]
    async fn multi_floor_reads_each_map_and_settings_only_for_the_current_one() {
        let robot = Fake { other_floor: Some(RefCell::new(upstairs())), ..Fake::new() };
        let maps = read_saved_maps(&robot).await.unwrap();
        assert_eq!((maps.current, maps.ids()), (Some(1), vec![1, 2]));

        let cur = read_saved(&robot).await.unwrap();
        assert_eq!((cur.map.map_id, cur.current, cur.map_ids.clone()), (1, true, vec![1, 2]));
        assert!(cur.map.rooms[&1].settings.is_some(), "the live frame's settings belong to the current map");

        let up = read_saved_map(&robot, Some(2)).await.unwrap();
        assert_eq!((up.map.map_id, up.current, up.name.as_str()), (2, false, "Upstairs"));
        assert_eq!(up.map.rooms[&1].name.as_deref(), Some("Up"));
        assert!(up.map.rooms.values().all(|r| r.settings.is_none()), "another floor has no settings");
        assert!(read_saved_map(&robot, Some(3)).await.err().unwrap().to_string().contains("[1, 2]"));
    }

    #[tokio::test]
    async fn edits_of_another_floor_need_a_map_id() {
        let robot = Fake { other_floor: Some(RefCell::new(upstairs())), ..Fake::new() };
        let dir = tmp("floors");
        // A no-go zone (`vw`) names no map: refused before anything is sent.
        let e = apply_to(&robot, Some(2), &Edit::AddNoGo(Rect::new(0.0, 0.0, 100.0, 50.0)), &dir, "f", fast()).await.unwrap_err();
        assert!(e.to_string().contains("not the robot's current map"), "{e}");
        let e = Edit::MoveBoundary { a: 1, b: 2, line: Line::new(125.0, 10.0, 125.0, 190.0) };
        assert!(apply_to(&robot, Some(2), &e, &dir, "f", fast()).await.is_err());
        assert!(robot.sent.borrow().is_empty());
        // Hiding a room carries `mapid` 2.
        apply_to(&robot, Some(2), &Edit::HideRooms(vec![2]), &dir, "f", fast()).await.unwrap();
        assert_eq!(robot.sent.borrow()[0], json!({"delsr": [2], "mapid": 2}));
        // The current map takes zones as before.
        apply_to(&robot, Some(1), &Edit::AddNoGo(Rect::new(0.0, 0.0, 100.0, 50.0)), &dir, "g", fast()).await.unwrap();
        assert_eq!(robot.no_go.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn result_code_forms() {
        assert_eq!(result_code(&json!({"out": [{"piid": 6, "value": 0}]})), Some(0));
        assert_eq!(result_code(&json!([{"piid": 6, "value": "5"}])), Some(5));
        assert_eq!(result_code(&json!({"out": []})), None);
        assert_eq!(result_code(&Value::Null), None);
    }
}
