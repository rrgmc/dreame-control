//! Map shapes for editing apps, with no GUI code: the kinds of zones and lines, hit-testing a
//! click, and turning a drawn rectangle or line into one [`Edit`]. The desktop app and the
//! editor lab share it.

use crate::archiveedit::GridEdit;
use crate::mapedit::Edit;
use dreame_map::edit::validate;
use dreame_map::model::{point_in_polygon, CarpetShape, Line, Map, Point, Rect};
use dreame_map::view::snap;

/// The kind of map shape a drawing tool adds or a selection names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    NoGo,
    NoMop,
    Wall,
    Carpet,
    IgnoredCarpet,
    Threshold,
    ImpassableThreshold,
    Curtain,
    Ramp,
    /// Drawing only: a round carpet inscribed in the dragged rectangle. Existing round carpets
    /// are listed under [`ZoneKind::Carpet`].
    RoundCarpet,
}

/// A shape as drawn on the map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Rect(Rect),
    Line(Line),
    /// A carpet ellipse: centre and size along x and y, taken as full widths.
    Ellipse { center: Point, size: (f64, f64) },
}

impl Shape {
    pub fn quad(&self) -> [i64; 4] {
        match self {
            Self::Rect(r) => r.to_quad(),
            Self::Line(l) => l.to_quad(),
            Self::Ellipse { center, size } => [center.x, center.y, size.0, size.1].map(|v| v.round() as i64),
        }
    }
}

impl ZoneKind {
    /// Every kind, in hit-test order: lines before areas, as lines are drawn on top.
    pub const ALL: [ZoneKind; 9] = [
        Self::Wall,
        Self::Threshold,
        Self::ImpassableThreshold,
        Self::Curtain,
        Self::NoGo,
        Self::NoMop,
        Self::IgnoredCarpet,
        Self::Ramp,
        Self::Carpet,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NoGo => "no-go zone",
            Self::NoMop => "no-mop zone",
            Self::Wall => "virtual wall",
            Self::Carpet => "carpet",
            Self::IgnoredCarpet => "ignored-carpet area",
            Self::Threshold => "threshold",
            Self::ImpassableThreshold => "impassable threshold",
            Self::Curtain => "curtain",
            Self::Ramp => "ramp",
            Self::RoundCarpet => "round carpet",
        }
    }

    /// True for kinds drawn as a line; the others are areas.
    pub fn is_line(self) -> bool {
        matches!(self, Self::Wall | Self::Threshold | Self::ImpassableThreshold | Self::Curtain)
    }

    /// The map's shapes of this kind, in stored order (the index a removal names).
    pub fn shapes(self, map: &Map) -> Vec<Shape> {
        let r = &map.restrictions;
        let lines = |v: &[Line]| v.iter().map(|l| Shape::Line(*l)).collect();
        let rects = |v: &[Rect]| v.iter().map(|z| Shape::Rect(*z)).collect();
        match self {
            Self::NoGo => rects(&r.no_go),
            Self::NoMop => rects(&r.no_mop),
            Self::IgnoredCarpet => rects(&map.ignored_carpet),
            Self::Wall => lines(&r.virtual_walls),
            Self::Threshold => lines(&r.thresholds),
            Self::ImpassableThreshold => lines(&r.impassable_thresholds),
            Self::Curtain => lines(&r.curtains),
            Self::Ramp => r.ramps.iter().map(|rp| Shape::Rect(rp.rect)).collect(),
            Self::RoundCarpet => Vec::new(),
            Self::Carpet => map
                .carpet_areas
                .iter()
                .map(|c| match c.shape {
                    CarpetShape::Rect(z) => Shape::Rect(z),
                    CarpetShape::Ellipse { center, size } => Shape::Ellipse { center, size },
                })
                .collect(),
        }
    }

    fn add_line(self, l: Line) -> Edit {
        match self {
            Self::Threshold => Edit::AddThreshold(l),
            Self::ImpassableThreshold => Edit::AddImpassableThreshold(l),
            Self::Curtain => Edit::AddCurtain(l),
            _ => Edit::AddWall(l),
        }
    }

    fn add_rect(self, r: Rect) -> Edit {
        match self {
            Self::NoMop => Edit::AddNoMop(r),
            Self::Carpet => Edit::AddCarpet(r),
            Self::IgnoredCarpet => Edit::AddIgnoredCarpet(r),
            Self::Ramp => Edit::AddRamp(r, 0),
            Self::RoundCarpet => Edit::AddRoundCarpet(r),
            _ => Edit::AddNoGo(r),
        }
    }

    fn remove(self, i: usize) -> Edit {
        match self {
            Self::NoGo => Edit::RemoveNoGo(i),
            Self::NoMop => Edit::RemoveNoMop(i),
            Self::Wall => Edit::RemoveWall(i),
            Self::Carpet | Self::RoundCarpet => Edit::RemoveCarpet(i),
            Self::Ramp => Edit::RemoveRamp(i),
            Self::IgnoredCarpet => Edit::RemoveIgnoredCarpet(i),
            Self::Threshold => Edit::RemoveThreshold(i),
            Self::ImpassableThreshold => Edit::RemoveImpassableThreshold(i),
            Self::Curtain => Edit::RemoveCurtain(i),
        }
    }
}

/// The shape an edit adds, for drawing it while it is staged.
pub fn added_shape(e: &Edit) -> Option<Shape> {
    match e {
        Edit::AddNoGo(r) | Edit::AddNoMop(r) | Edit::AddCarpet(r) | Edit::AddIgnoredCarpet(r) | Edit::AddRamp(r, _) => {
            Some(Shape::Rect(*r))
        }
        Edit::AddRoundCarpet(r) => Some(Shape::Ellipse {
            center: Point::new((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0),
            size: (r.width(), r.height()),
        }),
        Edit::AddWall(l) | Edit::AddThreshold(l) | Edit::AddImpassableThreshold(l) | Edit::AddCurtain(l) => {
            Some(Shape::Line(*l))
        }
        _ => None,
    }
}

/// The rectangle between two drag points, snapped. `None` when it has no area.
pub fn drawn_rect(a: Point, b: Point, snap_mm: f64) -> Option<Rect> {
    let r = Rect::new(snap(a.x, snap_mm), snap(a.y, snap_mm), snap(b.x, snap_mm), snap(b.y, snap_mm));
    (r.width() > 0.0 && r.height() > 0.0).then_some(r)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Zone(ZoneKind, usize),
    Furniture(usize),
    Room(u32),
}

/// Distance from `p` to the segment `l`, in mm.
pub fn distance_to_line(p: Point, l: &Line) -> f64 {
    let (dx, dy) = (l.b.x - l.a.x, l.b.y - l.a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.x - l.a.x) * dx + (p.y - l.a.y) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (l.a.x + t * dx, l.a.y + t * dy);
    (p.x - cx).hypot(p.y - cy)
}

fn rect_contains(r: &Rect, p: Point) -> bool {
    p.x >= r.x0 && p.x <= r.x1 && p.y >= r.y0 && p.y <= r.y1
}

fn shape_hit(s: &Shape, p: Point, tolerance: f64) -> bool {
    match s {
        Shape::Line(l) => distance_to_line(p, l) <= tolerance,
        Shape::Rect(r) => rect_contains(r, p),
        Shape::Ellipse { center, size } => {
            let (rx, ry) = (size.0 / 2.0, size.1 / 2.0);
            rx > 0.0 && ry > 0.0 && ((p.x - center.x) / rx).powi(2) + ((p.y - center.y) / ry).powi(2) <= 1.0
        }
    }
}

/// What is under a world point: a line within `tolerance` mm first, then an area, then a room;
/// kinds in [`ZoneKind::ALL`] order. Within a kind, later entries win, as they are drawn on top.
pub fn hit(map: &Map, p: Point, tolerance: f64) -> Option<Selection> {
    for kind in ZoneKind::ALL {
        if let Some(i) = kind.shapes(map).iter().rposition(|s| shape_hit(s, p, tolerance)) {
            return Some(Selection::Zone(kind, i));
        }
    }
    if let Some(i) = map.furniture.iter().rposition(|f| point_in_polygon(&f.corners(), p)) {
        return Some(Selection::Furniture(i));
    }
    map.room_at(p).map(Selection::Room)
}

/// The edit that adds the drawn shape, with both ends snapped to `snap_mm`. `None` when the
/// shape has no size after snapping.
pub fn draw_edit(kind: ZoneKind, a: Point, b: Point, snap_mm: f64) -> Option<Edit> {
    let (x0, y0, x1, y1) = (snap(a.x, snap_mm), snap(a.y, snap_mm), snap(b.x, snap_mm), snap(b.y, snap_mm));
    if kind.is_line() {
        let l = Line::new(x0, y0, x1, y1);
        (l.length() > 0.0).then(|| kind.add_line(l))
    } else {
        Some(kind.add_rect(drawn_rect(a, b, snap_mm)?))
    }
}

/// The edit that deletes a selected zone or wall.
pub fn delete_edit(sel: Selection) -> Option<Edit> {
    match sel {
        Selection::Zone(kind, i) => Some(kind.remove(i)),
        Selection::Furniture(i) => Some(Edit::RemoveFurniture(i)),
        Selection::Room(_) => None,
    }
}

/// The split edit for a drawn line, ends snapped. The room is the selected one when the line
/// crosses it, else the lowest-numbered room the line crosses. `None` when the line has no
/// length or crosses no room. The edit may still be invalid; `mapedit::build` says why.
pub fn split_edit(map: &Map, a: Point, b: Point, snap_mm: f64, selected: Option<u32>) -> Option<Edit> {
    let line = Line::new(snap(a.x, snap_mm), snap(a.y, snap_mm), snap(b.x, snap_mm), snap(b.y, snap_mm));
    if line.length() <= 0.0 {
        return None;
    }
    let crossed = validate::rooms_crossed(map, &line);
    let id = selected.filter(|id| crossed.contains(id)).or_else(|| crossed.first().copied())?;
    Some(Edit::SplitRoom { id, line })
}

/// The one room that `after` has and `before` lacks, when it has neither a name nor a type: the
/// new half after a split. `None` when no room or more than one room is new.
pub fn unnamed_new_room(before: &Map, after: &Map) -> Option<u32> {
    let new: Vec<_> = after.rooms.values().filter(|r| !before.rooms.contains_key(&r.id)).collect();
    match new[..] {
        [r] if r.name.is_none() && r.room_type.is_none_or(|t| t == 0) => Some(r.id),
        _ => None,
    }
}

/// The rooms room `id` can merge with: its neighbours that exist on the map, sorted.
pub fn merge_partners(map: &Map, id: u32) -> Vec<u32> {
    map.rooms.keys().copied().filter(|&o| o != id && map.are_neighbours(id, o)).collect()
}

/// The archive edit that deletes room `room` and gives its floor to `into`: `None` unless the two
/// rooms touch and both ids fit the grid image. Hidden rooms may be deleted, and may take a floor.
pub fn remove_room_edit(map: &Map, room: u32, into: u32) -> Option<GridEdit> {
    if !merge_partners(map, room).contains(&into) {
        return None;
    }
    Some(GridEdit::RemoveRoom { room: u8::try_from(room).ok()?, into: u8::try_from(into).ok()? })
}

/// An erase built from a drawn rectangle by [`erase_edit`].
#[derive(Debug, Clone, PartialEq)]
pub struct Erase {
    pub edit: GridEdit,
    /// The rectangle, snapped; `None` for a whole room.
    pub rect: Option<Rect>,
    /// Floor cells erased per room.
    pub cells: std::collections::BTreeMap<u32, usize>,
    /// Rooms left without floor; the edit deletes them.
    pub emptied: Vec<u32>,
}

/// The erase of the saved map's floor cells whose centre lies in the drawn rectangle, ends
/// snapped to `snap_mm`. `None` when the rectangle has no size or holds no floor cell. The edit
/// is refused later if it holds the whole floor.
pub fn erase_edit(map: &Map, a: Point, b: Point, snap_mm: f64) -> Option<Erase> {
    let rect = drawn_rect(a, b, snap_mm)?;
    let t = map.transform();
    let mut cells = std::collections::BTreeMap::<u32, usize>::new();
    let mut sizes = std::collections::BTreeMap::<u32, usize>::new();
    let mut points = Vec::new();
    for r in 0..map.grid.height {
        for c in 0..map.grid.width {
            let Some(room) = map.grid.get(c, r).class.segment().map(u32::from) else { continue };
            *sizes.entry(room).or_default() += 1;
            let (x, y) = t.cell_center(c, r);
            if rect_contains(&rect, Point::new(x, y)) {
                *cells.entry(room).or_default() += 1;
                points.push((x, y));
            }
        }
    }
    if points.is_empty() {
        return None;
    }
    let emptied = cells.iter().filter(|(r, n)| sizes.get(r) == Some(n)).map(|(r, _)| *r).collect();
    Some(Erase { edit: GridEdit::Erase { points }, rect: Some(rect), cells, emptied })
}

/// The erase of every floor cell of `room`, hidden or not. `None` when the room has no cells.
pub fn erase_room_edit(map: &Map, room: u32) -> Option<Erase> {
    let id = u8::try_from(room).ok()?;
    let g = &map.grid;
    let n = (0..g.height).flat_map(|r| (0..g.width).map(move |c| (c, r))).filter(|&(c, r)| g.get(c, r).class.segment() == Some(id)).count();
    (n > 0).then(|| Erase { edit: GridEdit::EraseRoom { room: id }, rect: None, cells: [(room, n)].into(), emptied: vec![room] })
}

/// The full hidden-room list after hiding or showing room `id`, sorted.
pub fn toggled_hidden(map: &Map, id: u32, hide: bool) -> Vec<u32> {
    let mut ids: Vec<u32> = map.hidden_rooms().into_iter().filter(|&r| r != id).collect();
    if hide {
        ids.push(id);
    }
    ids.sort_unstable();
    ids
}

/// The visible rooms in their stored cleaning order, then visible rooms without an order by id.
pub fn initial_order(map: &Map) -> Vec<u32> {
    let mut order: Vec<u32> = map.cleaning_order().into_iter().filter(|id| !map.rooms[id].hidden).collect();
    for r in map.rooms.values().filter(|r| !r.hidden) {
        if !order.contains(&r.id) {
            order.push(r.id);
        }
    }
    order
}

/// Moves entry `i` one place up (towards the start) or down. Out-of-range moves do nothing.
pub fn move_entry<T>(v: &mut [T], i: usize, up: bool) {
    match (up, i) {
        (true, i) if i > 0 && i < v.len() => v.swap(i, i - 1),
        (false, i) if i + 1 < v.len() => v.swap(i, i + 1),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_map::model::Room;

    /// A 2 x 2 saved map of room 1 at (0, 0)..(100, 100), with zones added on top.
    fn map() -> Map {
        use dreame_map::header::{FrameType, Header, Pose};
        use dreame_map::{Frame, Trailer};
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width: 2,
            height: 2,
            left: 25,
            top: 25,
        };
        let trailer = Trailer::from_value(serde_json::json!({})).unwrap();
        let frame = Frame { header, pixels: vec![1, 1, 1, 1], trailer, aes: None };
        let mut m = Map::from_frame(&frame, 2, true).unwrap();
        m.restrictions.no_go = vec![Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(50.0, 50.0, 150.0, 150.0)];
        m.restrictions.no_mop = vec![Rect::new(500.0, 500.0, 600.0, 600.0)];
        m.restrictions.virtual_walls = vec![Line::new(0.0, 300.0, 400.0, 300.0)];
        m
    }

    #[test]
    fn hit_prefers_walls_then_topmost_zone() {
        let m = map();
        assert_eq!(hit(&m, Point::new(200.0, 310.0), 20.0), Some(Selection::Zone(ZoneKind::Wall, 0)));
        assert_eq!(hit(&m, Point::new(75.0, 75.0), 20.0), Some(Selection::Zone(ZoneKind::NoGo, 1)));
        assert_eq!(hit(&m, Point::new(10.0, 10.0), 20.0), Some(Selection::Zone(ZoneKind::NoGo, 0)));
        assert_eq!(hit(&m, Point::new(550.0, 550.0), 20.0), Some(Selection::Zone(ZoneKind::NoMop, 0)));
        assert_eq!(hit(&m, Point::new(1000.0, 1000.0), 20.0), None);
        // Outside every zone but on room 1's floor.
        let mut m = m;
        m.restrictions.no_go.clear();
        assert_eq!(hit(&m, Point::new(30.0, 30.0), 20.0), Some(Selection::Room(1)));
    }

    #[test]
    fn drawing_snaps_and_rejects_empty_shapes() {
        let e = draw_edit(ZoneKind::NoGo, Point::new(12.0, 488.0), Point::new(260.0, 24.0), 50.0);
        assert_eq!(e, Some(Edit::AddNoGo(Rect::new(0.0, 0.0, 250.0, 500.0))));
        assert_eq!(draw_edit(ZoneKind::NoMop, Point::new(0.0, 0.0), Point::new(10.0, 300.0), 50.0), None);
        assert_eq!(
            draw_edit(ZoneKind::Wall, Point::new(0.0, 0.0), Point::new(0.0, 300.0), 50.0),
            Some(Edit::AddWall(Line::new(0.0, 0.0, 0.0, 300.0)))
        );
        assert_eq!(draw_edit(ZoneKind::Wall, Point::new(1.0, 1.0), Point::new(2.0, 2.0), 50.0), None);
        // The clean-zone tool uses the same snapped rectangle, in either drag direction.
        assert_eq!(
            drawn_rect(Point::new(510.0, 990.0), Point::new(-12.0, 240.0), 50.0),
            Some(Rect::new(0.0, 250.0, 500.0, 1000.0))
        );
        assert_eq!(drawn_rect(Point::new(0.0, 0.0), Point::new(20.0, 400.0), 50.0), None);
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
    fn split_picks_the_crossed_room() {
        let m = two_rooms();
        // From the bottom wall to the top wall through room 1, snapped to 50 mm.
        let e = split_edit(&m, Point::new(73.0, 12.0), Point::new(77.0, 188.0), 50.0, None);
        let Some(Edit::SplitRoom { id: 1, line }) = e else { panic!("expected a split of room 1: {e:?}") };
        assert_eq!(line.to_quad(), [50, 0, 100, 200]);
        assert!(crate::mapedit::build(&m, &Edit::SplitRoom { id: 1, line: Line::new(75.0, 10.0, 75.0, 190.0) }).is_ok());
        // Across both rooms: the selected one is split, with the line trimmed to its edge.
        let e = split_edit(&m, Point::new(10.0, 100.0), Point::new(290.0, 100.0), 0.0, Some(2)).unwrap();
        assert!(matches!(e, Edit::SplitRoom { id: 2, .. }));
        assert_eq!(crate::mapedit::build(&m, &e).unwrap().value()["dsrid"], serde_json::json!([135, 100, 265, 100, 2]));
        // An end inside the room is refused.
        let e = split_edit(&m, Point::new(10.0, 100.0), Point::new(200.0, 100.0), 0.0, Some(2)).unwrap();
        assert!(crate::mapedit::build(&m, &e).is_err());
        // Only over walls, or with no length.
        assert_eq!(split_edit(&m, Point::new(10.0, 10.0), Point::new(290.0, 10.0), 0.0, None), None);
        assert_eq!(split_edit(&m, Point::new(75.0, 75.0), Point::new(76.0, 76.0), 50.0, None), None);
    }

    #[test]
    fn furniture_is_hit_inside_its_rotated_outline() {
        let mut m = map();
        m.restrictions = Default::default();
        // 400 x 100 mm at (1000, 1000), turned 90 degrees: it covers x 950..1050, y 800..1200.
        let mut f = crate::mapedit::new_furniture(7, Point::new(1000.0, 1000.0), 400.0, 100.0);
        f.angle = 90.0;
        m.furniture = vec![f];
        assert_eq!(hit(&m, Point::new(1000.0, 1150.0), 20.0), Some(Selection::Furniture(0)));
        assert_eq!(hit(&m, Point::new(1150.0, 1000.0), 20.0), None, "inside the unrotated box only");
        assert_eq!(delete_edit(Selection::Furniture(0)), Some(Edit::RemoveFurniture(0)));
    }

    #[test]
    fn a_split_offers_to_name_the_new_room() {
        let before = two_rooms();
        let mut after = before.clone();
        let mut r3 = after.rooms[&1].clone();
        r3.id = 3;
        r3.name = None;
        r3.room_type = None;
        after.rooms.insert(3, r3.clone());
        assert_eq!(unnamed_new_room(&before, &after), Some(3));
        assert_eq!(unnamed_new_room(&before, &before), None, "nothing new");
        let mut named = after.clone();
        named.rooms.get_mut(&3).unwrap().room_type = Some(2);
        assert_eq!(unnamed_new_room(&before, &named), None, "the new room already has a type");
        let mut two = after.clone();
        two.rooms.insert(4, Room { id: 4, ..r3 });
        assert_eq!(unnamed_new_room(&before, &two), None, "more than one new room");
    }

    #[test]
    fn every_shape_kind_draws_hits_and_deletes() {
        use dreame_map::model::CarpetArea;
        let mut m = map();
        m.restrictions = Default::default();
        m.restrictions.thresholds = vec![Line::new(0.0, 500.0, 400.0, 500.0)];
        m.restrictions.impassable_thresholds = vec![Line::new(0.0, 600.0, 400.0, 600.0)];
        m.restrictions.curtains = vec![Line::new(0.0, 700.0, 400.0, 700.0)];
        m.ignored_carpet = vec![Rect::new(1000.0, 0.0, 1100.0, 100.0)];
        m.carpet_areas = vec![
            CarpetArea { id: 1, shape: CarpetShape::Rect(Rect::new(1200.0, 0.0, 1300.0, 100.0)), kind: Some(7), extra: vec![] },
            CarpetArea {
                id: 2,
                shape: CarpetShape::Ellipse { center: Point::new(2000.0, 0.0), size: (200.0, 100.0) },
                kind: Some(7),
                extra: vec![],
            },
        ];
        let at = |x, y| hit(&m, Point::new(x, y), 20.0);
        assert_eq!(at(200.0, 510.0), Some(Selection::Zone(ZoneKind::Threshold, 0)));
        assert_eq!(at(200.0, 590.0), Some(Selection::Zone(ZoneKind::ImpassableThreshold, 0)));
        assert_eq!(at(200.0, 700.0), Some(Selection::Zone(ZoneKind::Curtain, 0)));
        assert_eq!(at(1050.0, 50.0), Some(Selection::Zone(ZoneKind::IgnoredCarpet, 0)));
        assert_eq!(at(1250.0, 50.0), Some(Selection::Zone(ZoneKind::Carpet, 0)));
        assert_eq!(at(2090.0, 0.0), Some(Selection::Zone(ZoneKind::Carpet, 1)));
        assert_eq!(at(2090.0, 45.0), None, "outside the ellipse, inside its box");
        assert_eq!(delete_edit(Selection::Zone(ZoneKind::Carpet, 1)), Some(Edit::RemoveCarpet(1)));
        assert_eq!(delete_edit(Selection::Zone(ZoneKind::Curtain, 0)), Some(Edit::RemoveCurtain(0)));
        let (a, b) = (Point::new(0.0, 0.0), Point::new(300.0, 200.0));
        assert_eq!(draw_edit(ZoneKind::Carpet, a, b, 50.0), Some(Edit::AddCarpet(Rect::new(0.0, 0.0, 300.0, 200.0))));
        assert_eq!(draw_edit(ZoneKind::ImpassableThreshold, a, b, 50.0), Some(Edit::AddImpassableThreshold(Line::new(0.0, 0.0, 300.0, 200.0))));
        assert_eq!(ZoneKind::Carpet.shapes(&m)[1].quad(), [2000, 0, 200, 100]);
    }

    #[test]
    fn room_helpers() {
        let mut m = map();
        // Room 1 comes from the grid; add a second, hidden room and an unordered visible one.
        let mut r2 = m.rooms[&1].clone();
        r2.id = 2;
        r2.hidden = true;
        r2.order = Some(1);
        let mut r3 = m.rooms[&1].clone();
        r3.id = 3;
        r3.order = None;
        m.rooms.get_mut(&1).unwrap().order = Some(2);
        m.rooms.insert(2, r2);
        m.rooms.insert(3, r3);
        assert_eq!(toggled_hidden(&m, 3, true), [2, 3]);
        assert_eq!(toggled_hidden(&m, 2, false), Vec::<u32>::new());
        assert_eq!(initial_order(&m), [1, 3], "hidden room 2 is left out; unordered room 3 goes last");
    }

    #[test]
    fn merge_partners_are_neighbours() {
        let m = two_rooms();
        assert_eq!(merge_partners(&m, 1), [2]);
        assert_eq!(merge_partners(&m, 2), [1]);
    }

    #[test]
    fn remove_room_needs_a_neighbour() {
        let mut m = two_rooms();
        assert_eq!(remove_room_edit(&m, 2, 1), Some(GridEdit::RemoveRoom { room: 2, into: 1 }));
        assert_eq!(remove_room_edit(&m, 1, 1), None);
        assert_eq!(remove_room_edit(&m, 1, 3), None);
        // A hidden room can be deleted, and can take the floor.
        m.rooms.get_mut(&2).unwrap().hidden = true;
        assert_eq!(remove_room_edit(&m, 2, 1), Some(GridEdit::RemoveRoom { room: 2, into: 1 }));
        assert_eq!(remove_room_edit(&m, 1, 2), Some(GridEdit::RemoveRoom { room: 1, into: 2 }));
    }

    #[test]
    fn delete_and_distance() {
        assert_eq!(delete_edit(Selection::Zone(ZoneKind::NoMop, 2)), Some(Edit::RemoveNoMop(2)));
        assert_eq!(delete_edit(Selection::Room(3)), None);
        assert_eq!(distance_to_line(Point::new(0.0, 5.0), &Line::new(-10.0, 0.0, 10.0, 0.0)), 5.0);
        assert_eq!(distance_to_line(Point::new(13.0, 4.0), &Line::new(-10.0, 0.0, 10.0, 0.0)), 5.0);
    }

    #[test]
    fn erase_picks_floor_cells_in_the_rectangle() {
        let m = map();
        let (x, y) = m.transform().cell_center(0, 0);
        let one = erase_edit(&m, Point::new(x - 10.0, y - 10.0), Point::new(x + 10.0, y + 10.0), 1.0).unwrap();
        assert_eq!(one.edit, GridEdit::Erase { points: vec![(x, y)] });
        assert_eq!(one.cells, [(1, 1)].into());
        assert!(one.emptied.is_empty());
        let all = erase_edit(&m, Point::new(-500.0, -500.0), Point::new(500.0, 500.0), 50.0).unwrap();
        assert_eq!(all.cells, [(1, 4)].into());
        assert_eq!(all.emptied, [1]);
        assert!(erase_edit(&m, Point::new(1000.0, 1000.0), Point::new(2000.0, 2000.0), 50.0).is_none(), "no floor there");
        assert!(erase_edit(&m, Point::new(0.0, 0.0), Point::new(0.0, 500.0), 50.0).is_none(), "no width");
        let room = erase_room_edit(&m, 1).unwrap();
        assert_eq!((room.edit, room.emptied), (GridEdit::EraseRoom { room: 1 }, vec![1]));
        assert!(erase_room_edit(&m, 2).is_none());
    }

    #[test]
    fn move_entry_moves_one_place() {
        let mut v = vec![1, 2, 3];
        move_entry(&mut v, 0, true);
        assert_eq!(v, [1, 2, 3]);
        move_entry(&mut v, 2, true);
        assert_eq!(v, [1, 3, 2]);
        move_entry(&mut v, 0, false);
        assert_eq!(v, [3, 1, 2]);
        move_entry(&mut v, 2, false);
        assert_eq!(v, [3, 1, 2]);
    }
}
