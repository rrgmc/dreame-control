//! Map editing for the Android app, the standard tools of the desktop app's edit screen:
//! rename, divide and merge rooms, show a hidden room, add no-go zones, no-mop zones and
//! virtual walls, and delete one.
//!
//! The app builds a [`MapChange`] from what the user drew or tapped, in cell space. The change
//! says what it does, why it cannot be saved (if so), and what to draw while it waits. The app
//! saves it with [`HomeRobot::save_change`]. One change is saved at a time, as in the official
//! app.

use crate::app::{CellPoint, HomeMap, HomeRobot};
use crate::{DreameError, Result};
use dreame::mapedit::{self, Edit};
use dreame::shapes::{self, Selection, Shape, ZoneKind};
use dreame_i18n::{tr, trf};
use dreame_map::model::Point;
use std::sync::Arc;

/// Grid of the robot's map, in mm; drawn shapes snap to it.
const SNAP_MM: f64 = 50.0;
/// The longest room name the robot takes.
const MAX_NAME: usize = 24;

/// A kind of restriction the app edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ZoneType {
    NoGo,
    NoMop,
    /// A virtual wall: a line the robot does not cross.
    Wall,
}

impl ZoneType {
    fn kind(self) -> ZoneKind {
        match self {
            Self::NoGo => ZoneKind::NoGo,
            Self::NoMop => ZoneKind::NoMop,
            Self::Wall => ZoneKind::Wall,
        }
    }

    fn from_kind(k: ZoneKind) -> Option<Self> {
        match k {
            ZoneKind::NoGo => Some(Self::NoGo),
            ZoneKind::NoMop => Some(Self::NoMop),
            ZoneKind::Wall => Some(Self::Wall),
            _ => None,
        }
    }
}

/// A zone or virtual wall on the map, in cell space. A zone runs from corner `a` to the
/// opposite corner `b`; a wall from end `a` to end `b`.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct MapZone {
    pub kind: ZoneType,
    /// Its place among the map's zones of its kind.
    pub index: u32,
    pub a: CellPoint,
    pub b: CellPoint,
}

/// A shape to draw over the map while a change waits to be saved, in cell space.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct MapMark {
    pub a: CellPoint,
    pub b: CellPoint,
    /// A line from `a` to `b`; otherwise the rectangle with corners `a` and `b`.
    pub line: bool,
}

/// One change to the map, ready to save.
#[derive(uniffi::Object)]
pub struct MapChange {
    edit: Edit,
    text: String,
    problem: Option<String>,
    mark: Option<MapMark>,
    rooms: Vec<u32>,
}

#[uniffi::export]
impl MapChange {
    /// What it does, such as "Merge Kitchen and Hall".
    pub fn text(&self) -> String {
        self.text.clone()
    }

    /// Why it cannot be saved, as a sentence; `None` when it can.
    pub fn problem(&self) -> Option<String> {
        self.problem.clone()
    }

    /// The shape it adds, or the dividing line, to draw on the map.
    pub fn mark(&self) -> Option<MapMark> {
        self.mark
    }

    /// The rooms it changes, to draw in colour.
    pub fn rooms(&self) -> Vec<u32> {
        self.rooms.clone()
    }
}

/// The outcome of a saved change.
#[derive(uniffi::Record)]
pub struct SavedChange {
    /// The map as the robot now has it.
    pub map: Arc<HomeMap>,
    /// The new half of a divided room, which has no name yet: ask the user for one.
    pub unnamed_room: Option<u32>,
}

impl HomeMap {
    fn zone(&self, kind: ZoneKind, index: usize, s: &Shape) -> Option<MapZone> {
        let (a, b) = match s {
            Shape::Rect(r) => (Point::new(r.x0, r.y0), Point::new(r.x1, r.y1)),
            Shape::Line(l) => (l.a, l.b),
            Shape::Ellipse { .. } => return None,
        };
        Some(MapZone { kind: ZoneType::from_kind(kind)?, index: index as u32, a: self.to_cell(a), b: self.to_cell(b) })
    }

    fn mark(&self, s: Shape) -> Option<MapMark> {
        let (a, b, line) = match s {
            Shape::Rect(r) => (Point::new(r.x0, r.y0), Point::new(r.x1, r.y1), false),
            Shape::Line(l) => (l.a, l.b, true),
            Shape::Ellipse { .. } => return None,
        };
        Some(MapMark { a: self.to_cell(a), b: self.to_cell(b), line })
    }

    fn name(&self, id: u32) -> String {
        self.map.rooms.get(&id).map(|r| r.display_name()).unwrap_or_else(|| trf("Room {id}", &[("id", &id)]))
    }

    /// Wraps an edit with its text, its problem as the library finds it, and its mark.
    fn change(&self, edit: Edit, rooms: Vec<u32>) -> MapChange {
        let problem = mapedit::steps(&self.map, &edit).err().map(|e| {
            let detail = format!("{e:#}");
            match edit {
                Edit::SplitRoom { .. } => trf(
                    "Start and end the line outside the room, so it crosses the room from wall to wall. ({detail})",
                    &[("detail", &detail)],
                ),
                Edit::MergeRooms(..) => trf("These rooms cannot be merged. ({detail})", &[("detail", &detail)]),
                _ => trf("This change cannot be saved. ({detail})", &[("detail", &detail)]),
            }
        });
        let shape = shapes::added_shape(&edit).or(match edit {
            Edit::SplitRoom { line, .. } => Some(Shape::Line(line)),
            _ => None,
        });
        MapChange { text: self.describe(&edit), problem, mark: shape.and_then(|s| self.mark(s)), rooms, edit }
    }

    fn describe(&self, e: &Edit) -> String {
        match e {
            Edit::AddNoGo(_) => tr("New no-go zone").into(),
            Edit::AddNoMop(_) => tr("New no-mop zone").into(),
            Edit::AddWall(_) => tr("New virtual wall").into(),
            Edit::RemoveNoGo(_) => tr("Delete the no-go zone").into(),
            Edit::RemoveNoMop(_) => tr("Delete the no-mop zone").into(),
            Edit::RemoveWall(_) => tr("Delete the virtual wall").into(),
            Edit::SplitRoom { id, .. } => trf("Divide {room}", &[("room", &self.name(*id))]),
            Edit::MergeRooms(a, b) => trf("Merge {room} and {other}", &[("room", &self.name(*a)), ("other", &self.name(*b))]),
            Edit::RenameRoom { id, name } => trf("Rename {room} to {name}", &[("room", &self.name(*id)), ("name", name)]),
            Edit::HideRooms(_) => tr("Show the hidden area again").into(),
            other => format!("{other:?}"),
        }
    }

    fn world(&self, p: CellPoint) -> Point {
        self.to_world(p.x, p.y)
    }
}

#[uniffi::export]
impl HomeMap {
    /// The no-go zones, no-mop zones and virtual walls.
    pub fn zones(&self) -> Vec<MapZone> {
        [ZoneKind::NoGo, ZoneKind::NoMop, ZoneKind::Wall]
            .into_iter()
            .flat_map(|k| k.shapes(&self.map).into_iter().enumerate().filter_map(move |(i, s)| self.zone(k, i, &s)))
            .collect()
    }

    /// The zone or wall under a cell-space point; walls count within `tolerance` cells.
    pub fn zone_at(&self, x: f64, y: f64, tolerance: f64) -> Option<MapZone> {
        let mm = tolerance * self.mm_per_cell();
        match shapes::hit(&self.map, self.to_world(x, y), mm)? {
            Selection::Zone(kind, i) => {
                let shapes = kind.shapes(&self.map);
                self.zone(kind, i, shapes.get(i)?)
            }
            _ => None,
        }
    }

    /// The rooms that room `id` touches, hidden ones included: those it can merge with.
    pub fn neighbours(&self, id: u32) -> Vec<u32> {
        shapes::merge_partners(&self.map, id)
    }

    /// A new zone dragged from `a` to `b`, or a wall drawn from `a` to `b`, snapped to 5 cm.
    /// `None` when it is too small.
    pub fn draw(&self, kind: ZoneType, a: CellPoint, b: CellPoint) -> Option<Arc<MapChange>> {
        let e = shapes::draw_edit(kind.kind(), self.world(a), self.world(b), SNAP_MM)?;
        Some(Arc::new(self.change(e, Vec::new())))
    }

    /// Divides a room along the line from `a` to `b`, which should cross it from wall to wall.
    /// The room is `room` when the line crosses it, else the first room it crosses. `None` when
    /// the line crosses no room.
    pub fn divide(&self, a: CellPoint, b: CellPoint, room: Option<u32>) -> Option<Arc<MapChange>> {
        let e = shapes::split_edit(&self.map, self.world(a), self.world(b), SNAP_MM, room)?;
        let id = match e {
            Edit::SplitRoom { id, .. } => id,
            _ => return None,
        };
        Some(Arc::new(self.change(e, vec![id])))
    }

    /// Merges two rooms. `None` unless they touch.
    pub fn merge(&self, a: u32, b: u32) -> Option<Arc<MapChange>> {
        (a != b && self.neighbours(a).contains(&b)).then(|| Arc::new(self.change(Edit::MergeRooms(a, b), vec![a, b])))
    }

    /// Renames a room. The name is trimmed; an empty name or one over 24 characters is a problem.
    pub fn rename(&self, id: u32, name: String) -> Arc<MapChange> {
        let name = name.trim().to_string();
        let too_long = name.chars().count() > MAX_NAME;
        let empty = name.is_empty();
        let mut c = self.change(Edit::RenameRoom { id, name }, vec![id]);
        if empty {
            c.problem = Some(tr("Type a name.").into());
        } else if too_long {
            c.problem = Some(tr("Use at most 24 characters.").into());
        }
        Arc::new(c)
    }

    /// Deletes a zone or wall listed by [`HomeMap::zones`].
    pub fn delete_zone(&self, zone: MapZone) -> Arc<MapChange> {
        let e = shapes::delete_edit(Selection::Zone(zone.kind.kind(), zone.index as usize)).expect("zones can be deleted");
        let mut c = self.change(e, Vec::new());
        c.mark = Some(MapMark { a: zone.a, b: zone.b, line: zone.kind == ZoneType::Wall });
        Arc::new(c)
    }

    /// Shows a hidden room again, so the robot cleans it.
    pub fn show_room(&self, id: u32) -> Arc<MapChange> {
        Arc::new(self.change(Edit::HideRooms(shapes::toggled_hidden(&self.map, id, false)), vec![id]))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl HomeRobot {
    /// Saves a change on the robot, then reads the map back. Only while the robot is not
    /// cleaning. A write to the robot.
    pub async fn save_change(&self, change: Arc<MapChange>) -> Result<SavedChange> {
        if let Some(p) = &change.problem {
            return Err(DreameError::Other { detail: p.clone() });
        }
        let before = self.inner.map().ok_or_else(|| DreameError::Other { detail: "the map is not loaded yet".into() })?;
        let map = self.inner.edit(change.edit.clone()).await?;
        let unnamed_room = shapes::unnamed_new_room(&before.map, &map.map);
        Ok(SavedChange { map, unnamed_room })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_map::header::{FrameType, Header, Pose};
    use dreame_map::model::{Line, Rect};
    use dreame_map::{Frame, Map, Trailer};

    /// A 6 x 4 map of 50 mm cells: room 1 left, room 2 right, walls around; one no-go zone and
    /// one virtual wall.
    fn map() -> HomeMap {
        #[rustfmt::skip]
        let pixels = vec![
            0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
            0x81, 0x01, 0x01, 0x02, 0x02, 0x82,
            0x81, 0x01, 0x01, 0x02, 0x02, 0x82,
            0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
        ];
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width: 6,
            height: 4,
            left: 25,
            top: 25,
        };
        let trailer = Trailer::from_value(serde_json::json!({
            "seg_inf": {"1": {"type": 1, "index": 0, "nei_id": [2]}, "2": {"type": 2, "index": 0, "nei_id": [1]}}
        }))
        .unwrap();
        let frame = Frame { header, pixels, trailer, aes: None };
        let mut m = Map::from_frame(&frame, 2, true).unwrap();
        m.restrictions.no_go = vec![Rect::new(50.0, 50.0, 150.0, 150.0)];
        m.restrictions.virtual_walls = vec![Line::new(0.0, 300.0, 400.0, 300.0)];
        HomeMap::new(m)
    }

    fn cell(m: &HomeMap, x: f64, y: f64) -> CellPoint {
        m.to_cell(Point::new(x, y))
    }

    #[test]
    fn zones_list_and_hit_in_cell_space() {
        let m = map();
        let zones = m.zones();
        assert_eq!(zones.len(), 2);
        assert_eq!((zones[0].kind, zones[0].index), (ZoneType::NoGo, 0));
        assert_eq!((zones[1].kind, zones[1].index), (ZoneType::Wall, 0));
        let inside = cell(&m, 100.0, 100.0);
        assert_eq!(m.zone_at(inside.x, inside.y, 0.5).map(|z| z.kind), Some(ZoneType::NoGo));
        let near_wall = cell(&m, 200.0, 310.0);
        assert_eq!(m.zone_at(near_wall.x, near_wall.y, 0.5).map(|z| z.kind), Some(ZoneType::Wall));
        let far = cell(&m, 250.0, 250.0);
        assert_eq!(m.zone_at(far.x, far.y, 0.5), None);
    }

    #[test]
    fn changes_say_what_they_do_and_what_is_wrong() {
        let m = map();
        let c = m.draw(ZoneType::NoMop, cell(&m, 0.0, 0.0), cell(&m, 200.0, 200.0)).unwrap();
        assert_eq!(c.text(), "New no-mop zone");
        assert!(c.mark().is_some_and(|k| !k.line));
        assert!(m.draw(ZoneType::Wall, cell(&m, 0.0, 0.0), cell(&m, 10.0, 10.0)).is_none(), "too small");

        let c = m.merge(1, 2).unwrap();
        assert_eq!(c.rooms(), vec![1, 2]);
        assert!(m.merge(1, 1).is_none());
        assert!(m.merge(1, 9).is_none());

        assert_eq!(m.rename(1, "  ".into()).problem().as_deref(), Some("Type a name."));
        assert!(m.rename(1, "x".repeat(25)).problem().is_some());
        let c = m.rename(1, " Office ".into());
        assert!(c.problem().is_none());
        assert!(c.text().ends_with("to Office"), "{}", c.text());

        // Wall to wall through room 1 can be saved; a line ending inside the room cannot.
        let c = m.divide(cell(&m, 75.0, 10.0), cell(&m, 75.0, 190.0), None).unwrap();
        assert_eq!(c.rooms(), vec![1]);
        assert!(c.problem().is_none(), "{:?}", c.problem());
        assert!(c.mark().is_some_and(|k| k.line));
        let c = m.divide(cell(&m, 10.0, 100.0), cell(&m, 200.0, 100.0), Some(2)).unwrap();
        assert!(c.problem().is_some());

        let zone = m.zones()[1];
        let c = m.delete_zone(zone);
        assert_eq!(c.text(), "Delete the virtual wall");
        assert!(c.mark().is_some_and(|k| k.line));
    }
}
