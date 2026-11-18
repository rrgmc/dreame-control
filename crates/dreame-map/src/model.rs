//! The editable map model in world millimetres (maps.md §7 to §9).
//!
//! [`Map`] gathers everything an editor needs from one decoded frame: the classified grid, rooms
//! with their metadata, the floor plan, carpets, furniture, restrictions, dock, robot and path.
//! All coordinates are world mm (+x right, +y up). [`GridTransform`] converts to and from pixels.
//! The [`crate::edit`] builders turn a `Map` back into robot commands.

use crate::frame::Frame;
use crate::grid::{Encoding, Grid, GridTransform, PixelClass};
use crate::header::FrameType;
use crate::path::Path;
use crate::trailer::{self, Quad, Trailer};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

/// A world point in mm.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in mm. [`Rect::new`] orders the corners.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    /// Builds a rectangle with `x0 <= x1` and `y0 <= y1`.
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    pub fn from_quad(q: &Quad) -> Self {
        Self::new(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64)
    }

    /// Rounded `[x0, y0, x1, y1]` for commands.
    pub fn to_quad(&self) -> Quad {
        [
            self.x0.round() as i64,
            self.y0.round() as i64,
            self.x1.round() as i64,
            self.y1.round() as i64,
        ]
    }

    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    /// True when `other` lies fully inside this rectangle.
    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.x0 >= self.x0 && other.x1 <= self.x1 && other.y0 >= self.y0 && other.y1 <= self.y1
    }
}

/// A line segment in mm.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Line {
    pub a: Point,
    pub b: Point,
}

impl Line {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self {
            a: Point::new(x0, y0),
            b: Point::new(x1, y1),
        }
    }

    pub fn from_quad(q: &Quad) -> Self {
        Self::new(q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64)
    }

    pub fn to_quad(&self) -> Quad {
        [
            self.a.x.round() as i64,
            self.a.y.round() as i64,
            self.b.x.round() as i64,
            self.b.y.round() as i64,
        ]
    }

    pub fn length(&self) -> f64 {
        (self.b.x - self.a.x).hypot(self.b.y - self.a.y)
    }
}

/// A pose in world mm and degrees.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WorldPose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

/// Room type names for codes 0..15 (`T/types.py:12-29`), in English; [`Room::display_name`]
/// translates them with `dreame-i18n`.
pub const ROOM_TYPE_NAMES: [&str; 16] = [
    "Room",
    "Living Room",
    "Primary Bedroom",
    "Study",
    "Kitchen",
    "Dining Hall",
    "Bathroom",
    "Balcony",
    "Corridor",
    "Utility Room",
    "Closet",
    "Meeting Room",
    "Office",
    "Fitness Area",
    "Recreation Area",
    "Secondary Bedroom",
];

/// One room (segment).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Room {
    pub id: u32,
    /// Custom name (decoded from base64). Used when `room_type` is 0.
    pub name: Option<String>,
    /// Room type code; 0 means custom name.
    pub room_type: Option<i64>,
    /// Number suffix for typed rooms ("Bedroom 2" has index 1).
    pub index: Option<i64>,
    /// Floor material code.
    pub material: Option<i64>,
    /// Wood grain direction, degrees.
    pub direction: Option<i64>,
    /// Neighbour room ids from `seg_inf.nei_id`.
    pub neighbours: Vec<u32>,
    /// `seg_inf.roomID`, a unique room id on some models (absent on r2579a).
    pub unique_id: Option<serde_json::Value>,
    /// Position in the cleaning order (1-based), if listed.
    pub order: Option<i64>,
    /// Hidden through `delsr`.
    pub hidden: bool,
    /// `cleanset` entry: `[suction, water, times, order, cleanType, mopEffect, mopTemp]`; the last
    /// fields are absent on robots without them. See `edit::RoomSettings`.
    pub settings: Option<Vec<i64>>,
    /// Number of floor pixels in the grid.
    pub pixel_count: usize,
    /// World bounds of the room's floor and boundary pixels.
    pub bounds: Option<Rect>,
}

impl Room {
    /// Display name (`T/types.py:3896-3905`): custom name for type 0; else the type name with
    /// ` <index+1>` when `index > 0`; else `Room <id>`. Type names are in the current
    /// `dreame-i18n` language.
    pub fn display_name(&self) -> String {
        use dreame_i18n::{tr, trf};
        match self.room_type {
            Some(0) | None => match &self.name {
                Some(n) if !n.is_empty() => n.clone(),
                _ => trf("Room {id}", &[("id", &self.id)]),
            },
            Some(t) => match ROOM_TYPE_NAMES.get(t as usize) {
                Some(base) => match self.index {
                    Some(i) if i > 0 => format!("{} {}", tr(base), i + 1),
                    _ => tr(base).to_string(),
                },
                None => trf("Room {id}", &[("id", &self.id)]),
            },
        }
    }
}

/// One floor-plan wall.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Wall {
    pub line: Line,
    /// 0 probably interior, 1 exterior.
    pub kind: i64,
    /// Unit normal pointing out of the room; `(0, 0)` on zero-length junction walls.
    pub normal: (f64, f64),
}

/// A room outline from the floor plan: walls forming one closed ring.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomOutline {
    pub room_id: u32,
    pub floor_type: Option<i64>,
    pub walls: Vec<Wall>,
}

impl RoomOutline {
    /// Polygon vertices (each wall's start point).
    pub fn polygon(&self) -> Vec<Point> {
        self.walls.iter().map(|w| w.line.a).collect()
    }

    /// True when each wall ends where the next one begins, including the last to the first.
    pub fn is_closed(&self) -> bool {
        !self.walls.is_empty()
            && self
                .walls
                .iter()
                .zip(self.walls.iter().cycle().skip(1))
                .all(|(w, n)| w.line.b == n.line.a)
    }

    /// Signed area in mm² (positive for counter-clockwise).
    pub fn area(&self) -> f64 {
        polygon_area(&self.polygon())
    }

    /// Point-in-polygon test (even-odd rule).
    pub fn contains(&self, p: Point) -> bool {
        point_in_polygon(&self.polygon(), p)
    }
}

/// A floor-plan door between two rooms.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Door {
    pub rooms: (u32, u32),
    pub line: Line,
    pub kind: i64,
    pub status: i64,
}

/// The vector floor plan (`walls_info`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FloorPlan {
    pub version_flag: Option<i64>,
    pub rooms: Vec<RoomOutline>,
    pub doors: Vec<Door>,
}

/// A detected carpet polygon (`carpet_polygon`, object type 0).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Carpet {
    pub id: u32,
    pub polygon: Vec<Point>,
    pub kind: i64,
    pub hidden: bool,
}

/// A user-added carpet rectangle or ellipse (`vw.addcpt`, object type 1).
///
/// The app resends every stored entry with all its fields when it commits carpets
/// (map-plugin.md §6.3), so the model keeps the fields it does not interpret in `extra`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CarpetArea {
    /// Carpet id (field 4). New carpets take the lowest free id; see `edit::next_carpet_id`.
    pub id: i64,
    pub shape: CarpetShape,
    /// Carpet type (field 6; 7 is the generic type on r2579a). Absent in the 6-field form.
    pub kind: Option<i64>,
    /// Fields after the type, kept as stored.
    pub extra: Vec<serde_json::Value>,
}

/// Geometry of a user-added carpet. The wire form is four integers whose meaning depends on the
/// ellipse flag (field 5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CarpetShape {
    /// `[x0, y0, x1, y1]`: two corners.
    Rect(Rect),
    /// `[cx, cy, sx, sy]`: the centre and the size along x and y. The app snaps the centre to
    /// the grid and swaps the sizes when it rotates the map. Whether the sizes are radii or full
    /// widths is not confirmed, so they are kept exactly as stored.
    Ellipse { center: Point, size: (f64, f64) },
}

impl Default for CarpetShape {
    fn default() -> Self {
        Self::Rect(Rect::default())
    }
}

impl CarpetShape {
    /// Builds the shape from the four wire integers and the ellipse flag.
    pub fn from_wire(q: &Quad, ellipse: bool) -> Self {
        if ellipse {
            Self::Ellipse {
                center: Point::new(q[0] as f64, q[1] as f64),
                size: (q[2] as f64, q[3] as f64),
            }
        } else {
            Self::Rect(Rect::from_quad(q))
        }
    }

    /// The four wire integers, rounded.
    pub fn to_quad(&self) -> Quad {
        match self {
            Self::Rect(r) => r.to_quad(),
            Self::Ellipse { center, size } => [
                center.x.round() as i64,
                center.y.round() as i64,
                size.0.round() as i64,
                size.1.round() as i64,
            ],
        }
    }

    pub fn is_ellipse(&self) -> bool {
        matches!(self, Self::Ellipse { .. })
    }
}

/// One furniture item (`funiture_info`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Furniture {
    pub id: i64,
    /// Wire type code.
    pub kind: i64,
    pub segment: i64,
    pub width: f64,
    pub height: f64,
    pub center: Point,
    /// Degrees.
    pub angle: f64,
    pub scale: f64,
    pub edit_type: i64,
    pub modified: i64,
    /// The entry as stored. A commit writes the fields above over it, so the fields the model
    /// does not read (5, 8, 10, 11), their int or float form, and any 16th field stay as stored.
    pub stored: trailer::FurnitureEntry,
}

impl Furniture {
    /// The entry to send: the stored one with the model's fields written over it.
    pub fn to_entry(&self) -> trailer::FurnitureEntry {
        let mut e = self.stored.clone();
        let f = &mut e.fields;
        f[0] = self.id as f64;
        f[1] = self.kind as f64;
        f[2] = self.segment as f64;
        f[3] = self.width;
        f[4] = self.height;
        f[6] = self.center.x;
        f[7] = self.center.y;
        f[9] = self.angle;
        f[12] = self.scale;
        f[13] = self.edit_type as f64;
        f[14] = self.modified as f64;
        e
    }

    /// Corners of the rotated rectangle `width x height` around the centre.
    pub fn corners(&self) -> [Point; 4] {
        let (w, h) = (
            self.width * self.scale / 2.0,
            self.height * self.scale / 2.0,
        );
        let (s, c) = self.angle.to_radians().sin_cos();
        [(-w, -h), (w, -h), (w, h), (-w, h)].map(|(dx, dy)| {
            Point::new(
                self.center.x + dx * c - dy * s,
                self.center.y + dx * s + dy * c,
            )
        })
    }
}

/// A ramp: rectangle plus angle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ramp {
    pub rect: Rect,
    pub angle: i64,
}

/// Restrictions and zones of the saved map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Restrictions {
    /// `vw.rect`
    pub no_go: Vec<Rect>,
    /// `vw.mop`
    pub no_mop: Vec<Rect>,
    /// `vw.line`
    pub virtual_walls: Vec<Line>,
    /// `vws.vwsl`
    pub thresholds: Vec<Line>,
    /// `vws.npthrsd`
    pub impassable_thresholds: Vec<Line>,
    /// `vws.ramp`
    pub ramps: Vec<Ramp>,
    /// `ct.line` or `curtain.line`
    pub curtains: Vec<Line>,
}

/// The editable map.
#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    pub map_id: i16,
    pub frame_id: i16,
    pub frame_type: FrameType,
    /// True for a saved map (decoded with encoding E or as `rism`).
    pub saved: bool,
    pub map_version: u8,
    pub grid: Grid,
    pub rooms: BTreeMap<u32, Room>,
    pub floor_plan: Option<FloorPlan>,
    pub carpets: Vec<Carpet>,
    pub carpet_areas: Vec<CarpetArea>,
    /// `vw.nocpt`: areas where carpet detection is ignored.
    pub ignored_carpet: Vec<Rect>,
    pub furniture: Vec<Furniture>,
    pub restrictions: Restrictions,
    /// The stored `vw` object as decoded. `vw` edits resend every stored key, as the app does,
    /// so keys the model does not interpret (`cliff` and others) survive an edit.
    pub stored_vw: Option<trailer::VirtualWalls>,
    /// The stored `vws` object (thresholds, ramps), resent with its fields like `stored_vw`.
    pub stored_vws: Option<trailer::VirtualThresholds>,
    /// The stored curtain object (`ct` or `curtain`), resent with its fields like `stored_vw`.
    pub stored_ct: Option<trailer::Curtains>,
    pub dock: Option<WorldPose>,
    pub robot: Option<WorldPose>,
    pub path: Path,
    /// Display rotation in degrees (`mra`).
    pub rotation: i64,
    /// Router position (`whmp`).
    pub router: Option<Point>,
    /// Rooms the current job cleans (`sa`, live frames during a room clean); empty otherwise.
    pub active_rooms: Vec<u32>,
}

impl Map {
    /// Builds the model from a frame. `saved` marks a saved map (6.8 list, `rism`, recovery
    /// preview); see [`crate::grid::select_encoding`].
    pub fn from_frame(frame: &Frame, map_version: u8, saved: bool) -> Result<Self> {
        let enc = frame.encoding(map_version, saved);
        Self::from_frame_with_encoding(frame, map_version, enc)
    }

    /// Builds the model with an explicit pixel encoding.
    pub fn from_frame_with_encoding(
        frame: &Frame,
        map_version: u8,
        encoding: Encoding,
    ) -> Result<Self> {
        let t = &frame.trailer;
        let grid = frame.grid(encoding);
        let path = match t.tr.as_deref() {
            Some(tr) => Path::parse(tr)?,
            None => Path::default(),
        };
        let mut map = Map {
            map_id: frame.header.map_id,
            frame_id: frame.header.frame_id,
            frame_type: frame.header.frame_type,
            saved: encoding == Encoding::Saved,
            map_version,
            rooms: rooms_from(t, &grid),
            floor_plan: t.walls_info.as_ref().map(floor_plan_from),
            carpets: carpets_from(t),
            carpet_areas: Vec::new(),
            ignored_carpet: Vec::new(),
            furniture: t
                .funiture_info
                .iter()
                .flatten()
                .map(furniture_from)
                .collect(),
            restrictions: Restrictions::default(),
            stored_vw: None,
            stored_vws: t.vws.clone(),
            stored_ct: None,
            dock: None,
            robot: None,
            path,
            rotation: t.mra.as_ref().map_or(0, |m| m.value),
            router: t
                .whmp
                .as_ref()
                .filter(|v| v.len() >= 2)
                .map(|v| Point::new(v[0] as f64, v[1] as f64)),
            active_rooms: t.sa.iter().flatten().filter_map(|e| u32::try_from(*e.first()?).ok()).collect(),
            grid,
        };
        if let Some(vw) = &t.vw {
            map.stored_vw = Some(vw.clone());
            let r = &mut map.restrictions;
            r.no_go = vw.rect.iter().flatten().map(|e| Rect::from_quad(&e.quad)).collect();
            r.no_mop = vw.mop.iter().flatten().map(|e| Rect::from_quad(&e.quad)).collect();
            r.virtual_walls = vw.line.iter().flatten().map(|e| Line::from_quad(&e.quad)).collect();
            map.ignored_carpet = vw.nocpt.iter().flatten().map(|e| Rect::from_quad(&e.quad)).collect();
            map.carpet_areas = vw
                .addcpt
                .iter()
                .flatten()
                .map(|c| CarpetArea {
                    id: c.id,
                    shape: CarpetShape::from_wire(&c.rect, c.ellipse == 1),
                    kind: c.kind,
                    extra: c.rest.clone(),
                })
                .collect();
        }
        if let Some(vws) = &t.vws {
            let r = &mut map.restrictions;
            r.thresholds = vws.vwsl.iter().flatten().map(|e| Line::from_quad(&e.quad)).collect();
            r.impassable_thresholds = vws.npthrsd.iter().flatten().map(|e| Line::from_quad(&e.quad)).collect();
            r.ramps = vws
                .ramp
                .iter()
                .flatten()
                .filter(|v| v.len() >= 4)
                .map(|v| Ramp {
                    rect: Rect::new(v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64),
                    angle: v.get(4).copied().unwrap_or(0),
                })
                .collect();
        }
        if let Some(ct) =
            t.ct.as_ref()
                .and_then(trailer::CtValue::curtains)
                .or(t.curtain.as_ref())
        {
            map.restrictions.curtains = ct.line.iter().flatten().map(|e| Line::from_quad(&e.quad)).collect();
            map.stored_ct = Some(ct.clone());
        }
        let pose = |p: crate::header::Pose| WorldPose {
            x: p.x as f64,
            y: p.y as f64,
            angle: p.angle as f64,
        };
        map.dock = if t.nc == Some(1) {
            None
        } else {
            frame.header.charger.present().map(pose).or_else(|| {
                let v = &t.charger.as_ref()?.value;
                Some(WorldPose {
                    x: *v.first()? as f64,
                    y: *v.get(1)? as f64,
                    angle: v.get(2).copied().unwrap_or(0) as f64,
                })
            })
        };
        map.robot = if t.nr == Some(1) {
            None
        } else {
            frame.header.robot.present().map(pose).or_else(|| {
                t.robot_xy().map(|(x, y)| WorldPose {
                    x: x as f64,
                    y: y as f64,
                    angle: 0.0,
                })
            })
        };
        Ok(map)
    }

    /// Decodes an envelope and builds the model, choosing the encoding from the frame.
    pub fn decode(text: &str, map_version: u8, saved: bool) -> Result<Self> {
        Self::from_frame(&Frame::decode(text, None, None)?, map_version, saved)
    }

    /// The world-to-grid transform.
    pub fn transform(&self) -> GridTransform {
        self.grid.transform()
    }

    /// The rectangle with its corners moved to the nearest cell corners. The app snaps user
    /// carpets this way (`adjustPoint`), and r2579a stores them on this lattice.
    pub fn snap_to_cells(&self, r: &Rect) -> Rect {
        let tr = self.transform();
        let g = tr.grid_size;
        let sx = |x: f64| tr.left + ((x - tr.left) / g).round() * g;
        let sy = |y: f64| tr.top + ((y - tr.top) / g).round() * g;
        Rect::new(sx(r.x0), sy(r.y0), sx(r.x1), sy(r.y1))
    }

    /// A round carpet's centre moved to the nearest cell centre and its sizes to whole cells, as
    /// the app snaps ellipse carpets (centre against the origin, sizes against 0).
    pub fn snap_ellipse(&self, center: Point, size: (f64, f64)) -> (Point, (f64, f64)) {
        let tr = self.transform();
        let g = tr.grid_size;
        let (ox, oy) = (tr.left + g / 2.0, tr.top + g / 2.0);
        let c = Point::new(ox + ((center.x - ox) / g).round() * g, oy + ((center.y - oy) / g).round() * g);
        let whole = |v: f64| ((v / g).round() * g).max(g);
        (c, (whole(size.0), whole(size.1)))
    }

    /// World bounds of the grid.
    pub fn bounds(&self) -> Rect {
        let (x0, y0, x1, y1) = self.transform().bounds();
        Rect::new(x0, y0, x1, y1)
    }

    /// The room whose floor (or boundary) pixel lies under a world point.
    pub fn room_at(&self, p: Point) -> Option<u32> {
        let (c, r) = self.transform().cell_at(p.x, p.y)?;
        self.grid.get(c, r).class.segment().map(u32::from)
    }

    /// Where to put each room's label: the room cell farthest from the room's edge, so the
    /// label sits well inside L-shaped and narrow rooms too. Ties go to the cell nearest the
    /// room's centroid. Rooms without cells are left out.
    pub fn label_points(&self) -> BTreeMap<u32, Point> {
        let (w, h) = (self.grid.width, self.grid.height);
        let seg = |c: usize, r: usize| self.grid.get(c, r).class.segment();
        // Steps (4-neighbour) from each room cell to the nearest cell outside its room.
        let mut dist = vec![u32::MAX; w * h];
        let mut queue = std::collections::VecDeque::new();
        for r in 0..h {
            for c in 0..w {
                let Some(s) = seg(c, r) else { continue };
                let edge = c == 0
                    || r == 0
                    || c + 1 == w
                    || r + 1 == h
                    || [(c - 1, r), (c + 1, r), (c, r - 1), (c, r + 1)].iter().any(|&(x, y)| seg(x, y) != Some(s));
                if edge {
                    dist[r * w + c] = 1;
                    queue.push_back((c, r));
                }
            }
        }
        while let Some((c, r)) = queue.pop_front() {
            let d = dist[r * w + c];
            let s = seg(c, r);
            let mut next = Vec::with_capacity(4);
            if c > 0 {
                next.push((c - 1, r));
            }
            if r > 0 {
                next.push((c, r - 1));
            }
            if c + 1 < w {
                next.push((c + 1, r));
            }
            if r + 1 < h {
                next.push((c, r + 1));
            }
            for (x, y) in next {
                if seg(x, y) == s && dist[y * w + x] == u32::MAX {
                    dist[y * w + x] = d + 1;
                    queue.push_back((x, y));
                }
            }
        }
        // Centroids, then the deepest cell of each room.
        let mut sums: BTreeMap<u8, (f64, f64, f64)> = BTreeMap::new();
        for r in 0..h {
            for c in 0..w {
                if let Some(s) = seg(c, r) {
                    let e = sums.entry(s).or_default();
                    *e = (e.0 + c as f64, e.1 + r as f64, e.2 + 1.0);
                }
            }
        }
        let mut best: BTreeMap<u8, (u32, f64, usize, usize)> = BTreeMap::new();
        for r in 0..h {
            for c in 0..w {
                let Some(s) = seg(c, r) else { continue };
                let (sx, sy, n) = sums[&s];
                let off = (c as f64 - sx / n).hypot(r as f64 - sy / n);
                let d = dist[r * w + c];
                let better = match best.get(&s) {
                    None => true,
                    Some(&(bd, boff, _, _)) => d > bd || (d == bd && off < boff),
                };
                if better {
                    best.insert(s, (d, off, c, r));
                }
            }
        }
        let tr = self.transform();
        best.into_iter()
            .map(|(s, (_, _, c, r))| {
                let (x, y) = tr.cell_center(c, r);
                (u32::from(s), Point::new(x, y))
            })
            .collect()
    }

    /// Room ids in cleaning order (rooms without an order are left out).
    pub fn cleaning_order(&self) -> Vec<u32> {
        let mut v: Vec<(i64, u32)> = self
            .rooms
            .values()
            .filter_map(|r| r.order.map(|o| (o, r.id)))
            .collect();
        v.sort();
        v.into_iter().map(|(_, id)| id).collect()
    }

    /// Hidden room ids.
    pub fn hidden_rooms(&self) -> Vec<u32> {
        self.rooms
            .values()
            .filter(|r| r.hidden)
            .map(|r| r.id)
            .collect()
    }

    /// The number of floor cells of room `id` on this map that lie on floor of room `other_id`
    /// on `other`, compared at each cell's centre in world mm. Used to match rooms across an
    /// edit that changes room ids.
    pub fn floor_overlap(&self, id: u32, other: &Map, other_id: u32) -> usize {
        let (tr, otr) = (self.transform(), other.transform());
        let mut n = 0;
        for row in 0..self.grid.height {
            for col in 0..self.grid.width {
                if self.grid.get(col, row).class != PixelClass::Floor(id as u8) {
                    continue;
                }
                let (x, y) = tr.cell_center(col, row);
                if let Some((c, r)) = otr.cell_at(x, y) {
                    if other.grid.get(c, r).class == PixelClass::Floor(other_id as u8) {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Whether two rooms are neighbours, from `nei_id` or, failing that, from the floor-plan doors.
    pub fn are_neighbours(&self, a: u32, b: u32) -> bool {
        let by_seg = self
            .rooms
            .get(&a)
            .is_some_and(|r| r.neighbours.contains(&b))
            || self
                .rooms
                .get(&b)
                .is_some_and(|r| r.neighbours.contains(&a));
        let by_door = self.floor_plan.as_ref().is_some_and(|fp| {
            fp.doors
                .iter()
                .any(|d| d.rooms == (a, b) || d.rooms == (b, a))
        });
        by_seg || by_door
    }

    /// Copies room metadata (name, type, index, material, direction, neighbours, order, hidden
    /// state, settings) from a saved map onto rooms with the same id, and takes the saved map's
    /// floor plan, carpets, furniture and restrictions when this map has none. Tasshack does the
    /// same for live frames on a saved map (maps.md §6.2).
    pub fn apply_saved(&mut self, saved: &Map) {
        for (id, s) in &saved.rooms {
            if let Some(r) = self.rooms.get_mut(id) {
                r.name.clone_from(&s.name);
                r.room_type = s.room_type;
                r.index = s.index;
                r.material = s.material;
                r.direction = s.direction;
                r.neighbours.clone_from(&s.neighbours);
                r.unique_id.clone_from(&s.unique_id);
                r.order = r.order.or(s.order);
                r.hidden = r.hidden || s.hidden;
                if r.settings.is_none() {
                    r.settings.clone_from(&s.settings);
                }
            }
        }
        if self.floor_plan.is_none() {
            self.floor_plan.clone_from(&saved.floor_plan);
        }
        if self.carpets.is_empty() && self.carpet_areas.is_empty() {
            self.carpets.clone_from(&saved.carpets);
            self.carpet_areas.clone_from(&saved.carpet_areas);
            self.ignored_carpet.clone_from(&saved.ignored_carpet);
        }
        if self.furniture.is_empty() {
            self.furniture.clone_from(&saved.furniture);
        }
        if self.restrictions == Restrictions::default() {
            self.restrictions.clone_from(&saved.restrictions);
        }
        if self.stored_vw.is_none() {
            self.stored_vw.clone_from(&saved.stored_vw);
        }
        if self.stored_vws.is_none() {
            self.stored_vws.clone_from(&saved.stored_vws);
        }
        if self.stored_ct.is_none() {
            self.stored_ct.clone_from(&saved.stored_ct);
        }
    }

    /// Builds a live map and, when the frame embeds a saved map (`rism`), applies its metadata
    /// with [`Map::apply_saved`].
    pub fn from_live(frame: &Frame, map_version: u8) -> Result<Self> {
        let mut m = Self::from_frame(frame, map_version, false)?;
        if let Some(saved) = Self::saved_from_live(frame, map_version) {
            m.apply_saved(&saved?);
        }
        Ok(m)
    }

    /// The embedded saved map of a live frame, as a model.
    pub fn saved_from_live(frame: &Frame, map_version: u8) -> Option<Result<Self>> {
        frame
            .trailer
            .decode_rism()
            .map(|r| r.and_then(|f| Map::from_frame(&f, map_version, true)))
    }
}

fn rooms_from(t: &Trailer, grid: &Grid) -> BTreeMap<u32, Room> {
    let mut rooms: BTreeMap<u32, Room> = BTreeMap::new();
    let hist = grid.segment_histogram();
    let mut ids: BTreeSet<u32> = hist.keys().map(|&s| s as u32).collect();
    if let Some(seg) = &t.seg_inf {
        ids.extend(seg.keys().copied());
    }
    let order: BTreeMap<u32, i64> = t
        .cleanareaorder
        .iter()
        .flatten()
        .flat_map(|m| m.iter().map(|(&k, &v)| (k, v)))
        .collect();
    let hidden: BTreeSet<u32> = t.delsr.iter().flatten().copied().collect();
    let tr = grid.transform();
    for id in ids {
        let mut r = Room {
            id,
            ..Default::default()
        };
        if let Some(s) = t.seg_inf.as_ref().and_then(|m| m.get(&id)) {
            r.name = s.name_text();
            r.room_type = s.room_type;
            r.index = s.index;
            r.material = s.material;
            r.direction = s.direction;
            r.neighbours = s.nei_id.clone().unwrap_or_default();
            r.unique_id = s.room_id.clone();
        }
        r.order = order.get(&id).copied();
        r.hidden = hidden.contains(&id);
        r.settings = t.cleanset.as_ref().and_then(|c| c.value.get(&id).cloned());
        r.pixel_count = u8::try_from(id)
            .ok()
            .and_then(|s| hist.get(&s).copied())
            .unwrap_or(0);
        r.bounds = u8::try_from(id)
            .ok()
            .and_then(|s| grid.segment_bounds(s))
            .map(|(c0, r0, c1, r1)| {
                let (x0, y0) = tr.grid_to_world(c0 as f64, r0 as f64);
                let (x1, y1) = tr.grid_to_world((c1 + 1) as f64, (r1 + 1) as f64);
                Rect::new(x0, y0, x1, y1)
            });
        rooms.insert(id, r);
    }
    rooms
}

fn floor_plan_from(w: &trailer::WallsInfo) -> FloorPlan {
    let mut fp = FloorPlan {
        version_flag: w.version_flag,
        ..Default::default()
    };
    for s in &w.storeys {
        for r in &s.rooms {
            fp.rooms.push(RoomOutline {
                room_id: r.room_id,
                floor_type: r.floor_type,
                walls: r
                    .walls
                    .iter()
                    .map(|w| Wall {
                        line: Line::new(
                            w.beg_pt_x as f64,
                            w.beg_pt_y as f64,
                            w.end_pt_x as f64,
                            w.end_pt_y as f64,
                        ),
                        kind: w.kind,
                        normal: (w.normal_x, w.normal_y),
                    })
                    .collect(),
            });
        }
        for d in &s.doors {
            fp.doors.push(Door {
                rooms: (d.door_id, d.nei_door_id),
                line: Line::new(
                    d.beg_pt_x as f64,
                    d.beg_pt_y as f64,
                    d.end_pt_x as f64,
                    d.end_pt_y as f64,
                ),
                kind: d.door_type,
                status: d.door_status,
            });
        }
    }
    fp
}

fn carpets_from(t: &Trailer) -> Vec<Carpet> {
    t.carpet_polygon
        .iter()
        .flatten()
        .map(|(&id, c)| Carpet {
            id,
            polygon: c
                .vertices()
                .into_iter()
                .map(|(x, y)| Point::new(x, y))
                .collect(),
            kind: c.kind,
            hidden: c.hidden == Some(1),
        })
        .collect()
}

fn furniture_from(f: &trailer::FurnitureEntry) -> Furniture {
    let (cx, cy) = f.center();
    Furniture {
        id: f.id(),
        kind: f.kind(),
        segment: f.segment(),
        width: f.width(),
        height: f.height(),
        center: Point::new(cx, cy),
        angle: f.angle(),
        scale: f.scale(),
        edit_type: f.edit_type(),
        modified: f.modified(),
        stored: f.clone(),
    }
}

/// Signed polygon area (shoelace), positive for counter-clockwise.
pub fn polygon_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        / 2.0
}

/// Even-odd point-in-polygon test.
pub fn point_in_polygon(pts: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = pts.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (pts[i], pts[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::header::{Header, Pose};
    use crate::trailer::encode_name;
    use serde_json::json;

    /// A 6 x 4 saved map: room 1 on the left, room 2 on the right, walls around. The origin is the
    /// centre of cell (0, 0), so (25, 25) puts the grid corner at (0, 0).
    pub(crate) fn saved_frame() -> Frame {
        #[rustfmt::skip]
        let px = vec![
            0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
            0x81, 0x01, 0x41, 0x02, 0x02, 0x82,
            0x81, 0x01, 0x01, 0x02, 0x02, 0x82,
            0x81, 0x81, 0x81, 0x82, 0x82, 0x82,
        ];
        let trailer = Trailer::from_value(json!({
            "seg_inf": {
                "1": {"type": 0, "index": 0, "name": encode_name("Lounge"), "material": 1, "direction": 90, "nei_id": [2]},
                "2": {"type": 15, "index": 1, "material": 2, "direction": 0, "nei_id": [1]}
            },
            "cleanareaorder": [{"2": 1}, {"1": 2}],
            "cleanset": "{\"1\":[1,3,1,2,2,546],\"2\":[1,3,1,1,2,546]}",
            "delsr": [],
            "vw": {"line": [[0, 0, 100, 0]], "rect": [[200, 150, 100, 50]], "mop": [], "cliff": [], "addcpt": [[50, 50, 150, 150, 1, 0, 7]], "nocpt": []},
            "vws": {"vwsl": [], "npthrsd": [], "ramp": [[0, 0, 50, 50, 10]]},
            "ct": {"line": []},
            "carpet_polygon": {"2": [[50.0, 50.0, 100.0, 50.0, 100.0, 100.0], 7]},
            "funiture_info": [[0, 2, 1, 200, 100, 0, 100.0, 100.0, 0.0, 90.0, 0.0, 0.0, 1.0, 1, 0]],
            "walls_info": {"version_flag": 3, "storeys": [{"rooms": [{"room_id": 1, "floor_type": 3, "walls": [
                {"type": 1, "beg_pt_x": 25, "beg_pt_y": 25, "end_pt_x": 125, "end_pt_y": 25, "normal_x": 0.0, "normal_y": -1.0},
                {"type": 0, "beg_pt_x": 125, "beg_pt_y": 25, "end_pt_x": 125, "end_pt_y": 175, "normal_x": 1.0, "normal_y": 0.0},
                {"type": 1, "beg_pt_x": 125, "beg_pt_y": 175, "end_pt_x": 25, "end_pt_y": 175, "normal_x": 0.0, "normal_y": 1.0},
                {"type": 1, "beg_pt_x": 25, "beg_pt_y": 175, "end_pt_x": 25, "end_pt_y": 25, "normal_x": -1.0, "normal_y": 0.0}]}],
                "doors": [{"door_type": 0, "door_id": 1, "nei_door_id": 2, "door_status": 0, "beg_pt_x": 125, "beg_pt_y": 75, "end_pt_x": 125, "end_pt_y": 125}]}]},
            "robot": [0, 0],
            "charger": [150, 100, 90],
            "origin": [25, 25]
        }))
        .unwrap();
        Frame {
            header: Header {
                map_id: 1,
                frame_id: 0,
                frame_type: FrameType::I,
                robot: Pose::new(0, 0, 0),
                charger: Pose::new(150, 100, 90),
                grid_size: 50,
                width: 6,
                height: 4,
                left: 25,
                top: 25,
            },
            pixels: px,
            trailer,
            aes: None,
        }
    }

    pub(crate) fn saved_map() -> Map {
        Map::from_frame(&saved_frame(), 2, true).unwrap()
    }

    #[test]
    fn rooms_and_metadata() {
        let m = saved_map();
        assert!(m.saved);
        assert_eq!(m.rooms.len(), 2);
        let r1 = &m.rooms[&1];
        assert_eq!(r1.display_name(), "Lounge");
        assert_eq!(r1.pixel_count, 4);
        assert_eq!(r1.order, Some(2));
        assert_eq!(r1.settings.as_ref().unwrap()[5], 546);
        assert_eq!(r1.bounds, Some(Rect::new(0.0, 0.0, 150.0, 200.0)));
        assert_eq!(m.rooms[&2].display_name(), "Secondary Bedroom 2");
        assert_eq!(m.cleaning_order(), vec![2, 1]);
        assert!(m.are_neighbours(1, 2));
        assert_eq!(m.room_at(Point::new(75.0, 75.0)), Some(1));
        assert_eq!(m.room_at(Point::new(175.0, 75.0)), Some(2));
        // Each label lies inside its own room.
        let labels = m.label_points();
        assert_eq!(labels.len(), 2);
        for (id, p) in &labels {
            assert_eq!(m.room_at(*p), Some(*id), "label of room {id} at {p:?}");
        }
    }

    #[test]
    fn geometry_from_trailer() {
        let m = saved_map();
        assert_eq!(
            m.restrictions.no_go,
            vec![Rect::new(100.0, 50.0, 200.0, 150.0)]
        );
        assert_eq!(m.restrictions.virtual_walls.len(), 1);
        assert_eq!(m.restrictions.ramps[0].angle, 10);
        assert_eq!(m.carpet_areas[0].id, 1);
        assert_eq!(m.carpets[0].polygon.len(), 3);
        assert_eq!(m.furniture[0].center, Point::new(100.0, 100.0));
        let c = m.furniture[0].corners();
        assert!(
            (c[0].x - 150.0).abs() < 1e-9 && (c[0].y - 0.0).abs() < 1e-9,
            "{c:?}"
        );
        assert_eq!(
            m.dock,
            Some(WorldPose {
                x: 150.0,
                y: 100.0,
                angle: 90.0
            })
        );
        let fp = m.floor_plan.as_ref().unwrap();
        assert!(fp.rooms[0].is_closed());
        assert_eq!(fp.rooms[0].area(), 100.0 * 150.0);
        assert!(fp.rooms[0].contains(Point::new(75.0, 75.0)));
        assert!(!fp.rooms[0].contains(Point::new(175.0, 75.0)));
        assert_eq!(fp.doors[0].rooms, (1, 2));
    }

    #[test]
    fn live_takes_saved_metadata() {
        let saved = saved_frame();
        let mut live = saved_frame();
        // Same rooms in encoding B: segment << 2.
        live.pixels = saved
            .pixels
            .iter()
            .map(|&b| if b & 0x80 != 0 { 0xFC } else { (b & 0x3F) << 2 })
            .collect();
        live.trailer =
            Trailer::from_value(json!({"fsm": 1, "ris": 2, "rism": saved.encode().unwrap()}))
                .unwrap();
        let m = Map::from_live(&live, 2).unwrap();
        assert!(!m.saved);
        assert_eq!(m.grid.encoding, Encoding::FrameMap);
        assert_eq!(m.rooms[&1].display_name(), "Lounge");
        assert_eq!(m.rooms[&2].neighbours, vec![1]);
        assert_eq!(m.restrictions.no_go.len(), 1);
        assert!(m.floor_plan.is_some());
    }

    #[test]
    fn rect_normalises() {
        let r = Rect::new(10.0, 20.0, -10.0, -20.0);
        assert_eq!(r.to_quad(), [-10, -20, 10, 20]);
        assert!(r.contains(Point::new(0.0, 0.0)));
    }
}
