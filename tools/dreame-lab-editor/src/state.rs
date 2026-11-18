//! Editor logic that does not need a window: tools, selection, hit-testing, and turning user
//! gestures into one staged [`Edit`].

use dreame::archiveedit::GridEdit;
use dreame::mapedit::Edit;
use std::collections::{BTreeMap, BTreeSet};
use dreame_map::roomreg::RoomMeta;
use dreame_map::segmap::BorderCells;
use dreame_map::model::{Map, Point, Rect};

use dreame_map::view::snap;

pub use dreame::shapes::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Click to select; drag to pan.
    Select,
    /// Drag to draw a zone or wall.
    Draw(ZoneKind),
    /// Drag a rectangle for the robot to clean now.
    CleanZone,
    /// Click to place a furniture item of the type set in the panel.
    PlaceFurniture,
    /// Drag a line across one room to split it in two.
    Split,
    /// Drag the new boundary line between rooms `a` and `b`.
    MoveBoundary { a: u32, b: u32 },
    /// Paint cells of other rooms into room `room` (a grid edit through backup and restore).
    Paint { room: u32 },
}

/// Cells staged for a paint edit: the room they move to, and map grid cells (column, row).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintCells {
    pub room: u32,
    pub cells: BTreeSet<(usize, usize)>,
    /// For a new room (`room` not on the map yet): the room whose type, floor and settings it
    /// takes.
    pub parent: Option<u32>,
    /// For a new room: its name and floor, if set.
    pub meta: RoomMeta,
}

impl PaintCells {
    /// The rooms the cells come from, with a cell count each.
    pub fn sources(&self, map: &Map) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for &(c, r) in &self.cells {
            if let Some(s) = map.grid.get(c, r).class.segment() {
                *m.entry(u32::from(s)).or_default() += 1;
            }
        }
        m
    }

    /// Rooms the paint would take every cell of. The archive edit refuses such a paint: merging
    /// the rooms is the way to remove one.
    pub fn emptied(&self, map: &Map) -> Vec<u32> {
        let mut size: BTreeMap<u32, usize> = BTreeMap::new();
        for r in 0..map.grid.height {
            for c in 0..map.grid.width {
                if let Some(s) = map.grid.get(c, r).class.segment().filter(|s| *s != 0) {
                    *size.entry(u32::from(s)).or_default() += 1;
                }
            }
        }
        self.sources(map).into_iter().filter(|(r, n)| *r != self.room && size.get(r) == Some(n)).map(|(r, _)| r).collect()
    }

    /// The edit to send: the cells' centres in world mm.
    pub fn edit(&self, map: &Map) -> Option<GridEdit> {
        let room = u8::try_from(self.room).ok()?;
        let t = map.transform();
        let points = self.cells.iter().map(|&(c, r)| t.cell_center(c, r)).collect();
        Some(match self.parent {
            Some(p) => {
                let name = self.meta.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(String::from);
                let meta = RoomMeta { name, ..self.meta.clone() };
                GridEdit::PaintNewRoom { new: room, parent: u8::try_from(p).ok()?, points, meta }
            }
            None => GridEdit::Paint { room, points },
        })
    }
}

/// The lowest room id the map does not use, neither in its room list nor in its grid, that the
/// archive's room bits can hold. `None` when all are taken.
pub fn free_room_id(map: &Map) -> Option<u32> {
    let mut used: BTreeSet<u32> = map.rooms.keys().copied().collect();
    used.extend(map.grid.pixels.iter().filter_map(|p| p.class.segment()).map(u32::from));
    (1..=u32::from(dreame_map::segmap::MAX_ROOM)).find(|id| !used.contains(id))
}

/// The cells a square brush of `radius` cells around world point `p` adds to a paint into
/// `room`: the cells in the square that belong to another room. Outside cells and walls without
/// a room are left out, as the archive edit would skip them.
pub fn brush_cells(map: &Map, room: u32, p: Point, radius: usize) -> Vec<(usize, usize)> {
    let Some((c0, r0)) = map.transform().cell_at(p.x, p.y) else { return Vec::new() };
    paintable_cells(map, room, (c0.saturating_sub(radius), r0.saturating_sub(radius)), (c0 + radius, r0 + radius))
}

/// The cells a box dragged from world point `a` to `b` adds to a paint into `room`: the cells
/// whose centre lies in the box and that belong to another room.
pub fn box_cells(map: &Map, room: u32, a: Point, b: Point) -> Vec<(usize, usize)> {
    let t = map.transform();
    let (ca, ra) = t.world_to_grid(a.x.min(b.x), a.y.min(b.y));
    let (cb, rb) = t.world_to_grid(a.x.max(b.x), a.y.max(b.y));
    // Cell (c, r) has its centre at grid (c + 0.5, r + 0.5).
    let first = |v: f64| (v - 0.5).ceil().max(0.0) as usize;
    let last = |v: f64| (v - 0.5).floor();
    let (c1, r1) = (last(cb), last(rb));
    if c1 < 0.0 || r1 < 0.0 {
        return Vec::new();
    }
    paintable_cells(map, room, (first(ca), first(ra)), (c1 as usize, r1 as usize))
}

/// The cells from `from` to `to` (column, row; both included, clipped to the grid) that belong
/// to a room other than `room`.
fn paintable_cells(map: &Map, room: u32, from: (usize, usize), to: (usize, usize)) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for r in from.1..=to.1.min(map.grid.height.saturating_sub(1)) {
        for c in from.0..=to.0.min(map.grid.width.saturating_sub(1)) {
            if map.grid.get(c, r).class.segment().is_some_and(|s| s != 0 && u32::from(s) != room) {
                out.push((c, r));
            }
        }
    }
    out
}

/// What Escape undoes, one step per press, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    /// Close the confirmation dialog without sending.
    CloseDialog,
    /// Drop the shape or line being dragged; releasing the mouse then stages nothing.
    CancelDrag,
    /// Discard the staged edit.
    DiscardStaged,
    /// Leave a drawing tool for Select / pan.
    LeaveTool,
    Nothing,
}

/// The step Escape takes, given what is open, in progress or staged.
pub fn escape(dialog: bool, dragging: bool, staged: bool, tool: Tool) -> Escape {
    match () {
        _ if dialog => Escape::CloseDialog,
        _ if dragging => Escape::CancelDrag,
        _ if staged => Escape::DiscardStaged,
        _ if tool != Tool::Select => Escape::LeaveTool,
        _ => Escape::Nothing,
    }
}

/// A border move staged for the archive edit: the drawn line, straight or bent, with its points
/// snapped, and the map cells it moves. Room ids, names and settings stay, unlike a merge and split.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderMove {
    pub a: u32,
    pub b: u32,
    pub line: Vec<Point>,
    pub cells: BorderCells,
}

impl BorderMove {
    /// The border move of rooms `a` and `b` for a line through `points`, or why there is none.
    pub fn new(map: &Map, a: u32, b: u32, points: &[Point], snap_mm: f64) -> Result<Self, String> {
        let line: Vec<Point> = points.iter().map(|p| Point::new(snap(p.x, snap_mm), snap(p.y, snap_mm))).collect();
        let cells = dreame_map::edit::border_cells(map, a, b, &line).map_err(|e| e.to_string())?;
        Ok(Self { a, b, line, cells })
    }

    /// The edit to send. The archive edit cuts its own grid along the same line.
    pub fn edit(&self) -> Option<GridEdit> {
        let (a, b) = (u8::try_from(self.a).ok()?, u8::try_from(self.b).ok()?);
        Some(GridEdit::Border { a, b, line: self.line.iter().map(|p| (p.x, p.y)).collect() })
    }
}

/// One-line description of a staged edit for the side panel.
pub fn describe(e: &Edit) -> String {
    let q = |r: &Rect| format!("{:?}", r.to_quad());
    match e {
        Edit::RenameRoom { id, .. } => format!("rename room {id}"),
        Edit::AddNoGo(r) => format!("add no-go zone {}", q(r)),
        Edit::RemoveNoGo(i) => format!("remove no-go zone {i}"),
        Edit::AddNoMop(r) => format!("add no-mop zone {}", q(r)),
        Edit::RemoveNoMop(i) => format!("remove no-mop zone {i}"),
        Edit::AddWall(l) => format!("add virtual wall {:?}", l.to_quad()),
        Edit::RemoveWall(i) => format!("remove virtual wall {i}"),
        Edit::AddCarpet(r) => format!("add carpet {} (corners snap to cells)", q(r)),
        Edit::RemoveCarpet(i) => format!("remove carpet {i}"),
        Edit::AddIgnoredCarpet(r) => format!("add ignored-carpet area {} (corners snap to cells)", q(r)),
        Edit::RemoveIgnoredCarpet(i) => format!("remove ignored-carpet area {i}"),
        Edit::AddThreshold(l) => format!("add threshold {:?}", l.to_quad()),
        Edit::RemoveThreshold(i) => format!("remove threshold {i}"),
        Edit::AddImpassableThreshold(l) => format!("add impassable threshold {:?}", l.to_quad()),
        Edit::RemoveImpassableThreshold(i) => format!("remove impassable threshold {i}"),
        Edit::AddCurtain(l) => format!("add curtain {:?}", l.to_quad()),
        Edit::RemoveCurtain(i) => format!("remove curtain {i}"),
        Edit::SetFurniture(i, f) => format!(
            "change furniture {i}: type {}, centre ({:.0}, {:.0}), {:.0} x {:.0} mm, {:.0} degrees",
            f.kind, f.center.x, f.center.y, f.width, f.height, f.angle
        ),
        Edit::AddFurniture(f) => format!("add furniture of type {} at ({:.0}, {:.0})", f.kind, f.center.x, f.center.y),
        Edit::RemoveFurniture(i) => format!("remove furniture {i}"),
        Edit::AddRamp(r, angle) => format!("add ramp {} at {angle} degrees", q(r)),
        Edit::RemoveRamp(i) => format!("remove ramp {i}"),
        Edit::AddRoundCarpet(r) => format!("add round carpet inside {} (centre and size snap to cells)", q(r)),
        Edit::RoomSettings { id, .. } => format!("change room {id} settings"),
        Edit::RestoreNames(_) => "restore room names from a snapshot".into(),
        Edit::RestoreSettings(_) => "restore room settings from a snapshot".into(),
        Edit::HideRooms(ids) if ids.is_empty() => "show every room".into(),
        Edit::HideRooms(ids) => format!("hide rooms {ids:?} (and show the others)"),
        Edit::CleaningOrder(ids) => format!("cleaning order {ids:?}"),
        Edit::MergeRooms(a, b) => format!("merge rooms {a} and {b}"),
        Edit::SplitRoom { id, line } => format!("split room {id} along {:?}", line.to_quad()),
        Edit::MoveBoundary { a, b, line } => format!("move the boundary of rooms {a} and {b} to {:?}", line.to_quad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved map from raw encoding-E pixels, 50 mm cells, column 0 centred on x = 25 mm.
    fn grid_map(width: i16, height: i16, pixels: Vec<u8>) -> Map {
        use dreame_map::header::{FrameType, Header, Pose};
        use dreame_map::{Frame, Trailer};
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width,
            height,
            left: 25,
            top: 25,
        };
        let trailer = Trailer::from_value(serde_json::json!({})).unwrap();
        Map::from_frame(&Frame { header, pixels, trailer, aes: None }, 2, true).unwrap()
    }

    #[test]
    fn brush_takes_other_rooms_only() {
        // Row 0: room 1, room 2, outside.
        let m = grid_map(3, 1, vec![1, 2, 0]);
        let t = m.transform();
        let (x, y) = t.cell_center(1, 0);
        assert_eq!(brush_cells(&m, 1, Point::new(x, y), 0), [(1, 0)]);
        assert_eq!(brush_cells(&m, 1, Point::new(x, y), 1), [(1, 0)], "own room and outside left out");
        assert_eq!(brush_cells(&m, 2, Point::new(x, y), 1), [(0, 0)]);
        assert!(brush_cells(&m, 1, Point::new(-5000.0, 0.0), 3).is_empty());
        let p = PaintCells { room: 1, cells: [(1, 0)].into(), ..Default::default() };
        assert_eq!(p.sources(&m), [(2, 1)].into());
        assert_eq!(p.edit(&m), Some(GridEdit::Paint { room: 1, points: vec![(x, y)] }));
        assert_eq!(p.emptied(&m), [2], "room 2 has one cell");
        let m = grid_map(3, 1, vec![1, 2, 2]);
        assert!(PaintCells { room: 1, cells: [(1, 0)].into(), ..Default::default() }.emptied(&m).is_empty());
    }

    /// A 6 x 4 map: room 1 left, room 2 right, walls (0x81, 0x82) around; neighbours by nei_id.
    fn two_rooms() -> Map {
        use dreame_map::header::{FrameType, Header, Pose};
        use dreame_map::{Frame, Trailer};
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
        Map::from_frame(&frame, 2, true).unwrap()
    }

    #[test]
    fn box_takes_the_cells_whose_centre_is_inside() {
        let m = two_rooms();
        let t = m.transform();
        // From the centre of cell (1, 1) to just past the centre of cell (4, 2): columns 1 to 4,
        // rows 1 and 2, less room 1's own cells.
        let (a, b) = (t.cell_center(1, 1), t.cell_center(4, 2));
        let cells = box_cells(&m, 1, Point::new(b.0 + 1.0, b.1 + 1.0), Point::new(a.0, a.1));
        assert_eq!(cells, [(3, 1), (4, 1), (3, 2), (4, 2)]);
        // A box between two centres holds no cell.
        let mid = |c: usize| (t.cell_center(c, 1).0 + t.cell_center(c + 1, 1).0) / 2.0;
        assert!(box_cells(&m, 1, Point::new(mid(3), a.1 - 1.0), Point::new(mid(3) + 1.0, a.1 + 1.0)).is_empty());
    }

    #[test]
    fn new_room_paint() {
        let m = two_rooms();
        assert_eq!(free_room_id(&m), Some(3));
        let p = PaintCells { room: 3, cells: [(4, 1)].into(), parent: Some(2), ..Default::default() };
        let (x, y) = m.transform().cell_center(4, 1);
        assert_eq!(p.edit(&m), Some(GridEdit::PaintNewRoom { new: 3, parent: 2, points: vec![(x, y)], meta: RoomMeta::default() }));
        assert_eq!(p.sources(&m), [(2, 1)].into());
        assert!(p.emptied(&m).is_empty());
    }

    #[test]
    fn border_move_takes_the_cells_past_the_line() {
        let m = two_rooms();
        let t = m.transform();
        // Room 1 holds columns 0 to 2, room 2 columns 3 to 5. A line between columns 3 and 4,
        // across all rows, gives column 3 to room 1.
        let x = (t.cell_center(3, 0).0 + t.cell_center(4, 0).0) / 2.0;
        let (y0, y1) = (t.cell_center(0, 0).1 - 30.0, t.cell_center(0, 3).1 + 30.0);
        let mv = BorderMove::new(&m, 1, 2, &[Point::new(x, y0), Point::new(x, y1)], 0.0).unwrap();
        assert_eq!(mv.cells.to_a, [(3, 0), (3, 1), (3, 2), (3, 3)]);
        assert!(mv.cells.to_b.is_empty());
        assert_eq!(mv.edit(), Some(GridEdit::Border { a: 1, b: 2, line: vec![(x, y0), (x, y1)] }));
        // A line that ends inside the rooms does not cut them in two.
        let short = [Point::new(x, y0), Point::new(x, t.cell_center(0, 1).1)];
        assert!(BorderMove::new(&m, 1, 2, &short, 0.0).is_err());
        // Bent: down between columns 3 and 4 to between rows 1 and 2, then left past the edge.
        // Room 1 keeps the top two rows and takes column 3 there; room 2 takes the bottom rows.
        let ym = (t.cell_center(0, 1).1 + t.cell_center(0, 2).1) / 2.0;
        let xl = t.cell_center(0, 0).0 - 30.0;
        let bent = [Point::new(x, y0), Point::new(x, ym), Point::new(xl, ym)];
        let mv = BorderMove::new(&m, 1, 2, &bent, 0.0).unwrap();
        assert_eq!(mv.cells.to_a, [(3, 0), (3, 1)]);
        assert_eq!(mv.cells.to_b, [(0, 2), (0, 3), (1, 2), (1, 3), (2, 2), (2, 3)]);
    }

    #[test]
    fn escape_undoes_the_newest_thing_first() {
        let t = Tool::MoveBoundary { a: 1, b: 2 };
        assert_eq!(escape(true, true, true, t), Escape::CloseDialog);
        assert_eq!(escape(false, true, true, t), Escape::CancelDrag);
        assert_eq!(escape(false, false, true, t), Escape::DiscardStaged);
        assert_eq!(escape(false, false, false, t), Escape::LeaveTool);
        assert_eq!(escape(false, false, false, Tool::Select), Escape::Nothing);
    }

    #[test]
    fn describe_names_the_edit() {
        assert_eq!(describe(&Edit::AddNoGo(Rect::new(0.0, 0.0, 50.0, 50.0))), "add no-go zone [0, 0, 50, 50]");
        assert_eq!(describe(&Edit::MergeRooms(1, 2)), "merge rooms 1 and 2");
    }
}
