//! Pixel encodings and the classified grid (maps.md §5).
//!
//! The wire byte of each pixel means different things depending on the map version and the
//! frame flags. [`select_encoding`] picks the encoding, [`classify`] turns a byte into a
//! [`Pixel`], and [`Grid`] holds the classified result so the renderer and the model do not depend
//! on the wire encoding.

use crate::header::FrameType;
use crate::trailer::Trailer;
use std::collections::BTreeMap;

/// Wire encoding of the pixel bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// A, map version 3 (`MAP_V2`): bit 7 carpet; bits 6-5 wall (0 none, 1-2 wall, 3 type-3);
    /// bits 4-0 segment, 31 special. `building` is true while the map is being built (`ris` 0 or
    /// 1); it turns segment 31 floor into a new segment.
    V3 { building: bool },
    /// B, live frame with `fsm == 1`: bits 7-2 segment (61 unknown, 62 floor, 63 wall), bits 1-0
    /// class; carpet when the class is 3.
    FrameMap,
    /// C, map being built (`ris` 0 or 1): `px & 0x3F` 1 or 3 new segment, 2 wall.
    Building,
    /// D, vSLAM unsaved, or `ris == 2` without `fsm`: `px & 0x3F` 2 wall, other non-zero new segment.
    Vslam,
    /// E, saved map and `rism`: bit 7 wall, bit 6 carpet, bits 5-0 segment.
    Saved,
    /// W-frame: `px & 0x0F` is the Wi-Fi class.
    Wifi,
    /// Clean overlay (`decmap`): `px & 3` 1 cleaned, 2 dirty.
    CleanMap,
}

/// Decoded meaning of one pixel. Carpet is separate, see [`Pixel::carpet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PixelClass {
    /// No data.
    #[default]
    Outside,
    /// Floor of a room (segment 1..63).
    Floor(u8),
    /// Wall. `Some(id)` is a boundary pixel of room `id`; `None` a wall without room.
    Wall(Option<u8>),
    /// Encoding A "wall type 3" pixel of a room; not drawn.
    HiddenWall(u8),
    /// Floor not yet assigned to a room.
    NewSegment,
    /// Floor without a room.
    FloorNoRoom,
    Unknown,
    ObstacleWall,
    /// Wi-Fi map class: 2 wall, 10 unreached, 11 poor, 12 low, 13 high, 14 excellent.
    Wifi(u8),
    Cleaned,
    Dirty,
}

impl PixelClass {
    /// The numeric class code of maps.md §5.1 (Tasshack's `pixel_type`).
    pub fn code(self) -> u8 {
        match self {
            Self::Outside => 0,
            Self::Floor(s) => s,
            Self::Wall(Some(s)) => 100 + s,
            Self::Wall(None) => 255,
            Self::HiddenWall(s) => 200 + s,
            Self::NewSegment => 253,
            Self::FloorNoRoom => 254,
            Self::Unknown => 252,
            Self::ObstacleWall => 251,
            Self::Wifi(c) => c,
            Self::Cleaned => 249,
            Self::Dirty => 250,
        }
    }

    /// The room this pixel belongs to (floor, boundary or hidden wall).
    pub fn segment(self) -> Option<u8> {
        match self {
            Self::Floor(s) | Self::HiddenWall(s) | Self::Wall(Some(s)) => Some(s),
            _ => None,
        }
    }

    pub fn is_wall(self) -> bool {
        matches!(self, Self::Wall(_) | Self::ObstacleWall) || self == Self::Wifi(2)
    }

    /// Any walkable floor: room floor, new segment or floor without room.
    pub fn is_floor(self) -> bool {
        matches!(self, Self::Floor(_) | Self::NewSegment | Self::FloorNoRoom)
    }
}

/// One classified pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pixel {
    pub class: PixelClass,
    pub carpet: bool,
}

impl Pixel {
    pub const OUTSIDE: Pixel = Pixel {
        class: PixelClass::Outside,
        carpet: false,
    };

    fn new(class: PixelClass, carpet: bool) -> Self {
        Self { class, carpet }
    }
}

/// Chooses the pixel encoding, following `T/map.py:4583-4686`.
///
/// `map_version` is 0 (vSLAM), 1 (lidar), 2 (`CLEANING_SEQUENCE_V2` or `cleanareaorder`) or 3
/// (`MAP_V2`); see [`map_version`]. `saved` marks a saved map (6.8 list, `rism`, recovery).
pub fn select_encoding(
    map_version: u8,
    frame_type: FrameType,
    trailer: &Trailer,
    saved: bool,
) -> Encoding {
    let ris = trailer.ris;
    let building = matches!(ris, Some(0) | Some(1));
    if frame_type == FrameType::W {
        return Encoding::Wifi;
    }
    if map_version >= 3 {
        return Encoding::V3 { building };
    }
    if trailer.fsm == Some(1) {
        return Encoding::FrameMap;
    }
    if building {
        return Encoding::Building;
    }
    let recovery = trailer.us == Some(1);
    if (map_version == 0 && !saved && !recovery) || ris == Some(2) {
        return Encoding::Vslam;
    }
    Encoding::Saved
}

/// Map version from capabilities and the trailer (maps.md §2.9). Pass `has_map_v2` for the
/// `MAP_V2` capability and `has_clean_seq_v2` for `CLEANING_SEQUENCE_V2`; `lidar` is false only
/// for vSLAM robots. r2579a is version 2.
pub fn map_version(
    has_map_v2: bool,
    has_clean_seq_v2: bool,
    lidar: bool,
    trailer: Option<&Trailer>,
) -> u8 {
    if has_map_v2 {
        3
    } else if has_clean_seq_v2 || trailer.is_some_and(Trailer::has_clean_order_v2) {
        2
    } else if lidar {
        1
    } else {
        0
    }
}

/// Classifies one byte.
pub fn classify(px: u8, enc: Encoding) -> Pixel {
    use PixelClass::*;
    match enc {
        Encoding::V3 { building } => {
            if px == 0 {
                return Pixel::OUTSIDE;
            }
            let carpet = px & 0x80 != 0;
            let seg = px & 0x1F;
            let wall = (px >> 5) & 3;
            let class = match (seg, wall) {
                (0, _) => Outside,
                (_, 3) => HiddenWall(seg),
                (31, 1 | 2) => Wall(None),
                (31, _) if building => NewSegment,
                (31, _) => FloorNoRoom,
                (_, 1 | 2) => Wall(Some(seg)),
                _ => Floor(seg),
            };
            Pixel::new(class, carpet)
        }
        Encoding::FrameMap => {
            if px == 0 {
                return Pixel::OUTSIDE;
            }
            let carpet = px & 3 == 3;
            let seg = px >> 2;
            let class = match seg {
                63 => Wall(None),
                62 => FloorNoRoom,
                61 => Unknown,
                // Tasshack treats every class of a room segment as floor.
                1..=60 => Floor(seg),
                _ => match px & 3 {
                    1 | 3 => NewSegment,
                    2 => Wall(None),
                    _ => Outside,
                },
            };
            Pixel::new(class, carpet)
        }
        Encoding::Building => {
            let carpet = px != 0 && px & 3 == 3;
            let class = match px & 0x3F {
                1 | 3 => NewSegment,
                2 => Wall(None),
                _ => Outside,
            };
            Pixel::new(class, carpet)
        }
        Encoding::Vslam => {
            let carpet = px != 0 && px & 3 == 3;
            let class = match px & 0x3F {
                0 => Outside,
                2 => Wall(None),
                _ => NewSegment,
            };
            Pixel::new(class, carpet)
        }
        Encoding::Saved => {
            if px == 0 {
                return Pixel::OUTSIDE;
            }
            let carpet = px & 0x40 != 0;
            let seg = px & 0x3F;
            let class = if px & 0x80 != 0 {
                Wall((seg != 0).then_some(seg))
            } else if seg > 0 {
                Floor(seg)
            } else {
                Outside
            };
            Pixel::new(class, carpet)
        }
        Encoding::Wifi => {
            let c = px & 0x0F;
            Pixel::new(if c == 0 { Outside } else { Wifi(c) }, false)
        }
        Encoding::CleanMap => Pixel::new(
            match px & 3 {
                1 => Cleaned,
                2 => Dirty,
                _ => Outside,
            },
            false,
        ),
    }
}

/// Converts between world mm and grid cells (maps.md §9).
///
/// `left` and `top` here are the world coordinates of the outer corner of cell (0, 0). Column `c`
/// covers world x in `[left + c*g, left + (c+1)*g)`; row `r` covers y in `[top + r*g,
/// top + (r+1)*g)`. Row 0 is the minimum y. [`Grid::transform`] derives the corner from the frame
/// origin, which is a cell centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridTransform {
    pub left: f64,
    pub top: f64,
    pub grid_size: f64,
    pub width: usize,
    pub height: usize,
}

impl GridTransform {
    /// World coordinates of the centre of cell `(col, row)`.
    pub fn cell_center(&self, col: usize, row: usize) -> (f64, f64) {
        let g = self.grid_size;
        (
            self.left + col as f64 * g + g / 2.0,
            self.top + row as f64 * g + g / 2.0,
        )
    }

    /// Continuous grid coordinates `(col, row)` of a world point.
    pub fn world_to_grid(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.left) / self.grid_size,
            (y - self.top) / self.grid_size,
        )
    }

    /// World point of continuous grid coordinates.
    pub fn grid_to_world(&self, col: f64, row: f64) -> (f64, f64) {
        (
            self.left + col * self.grid_size,
            self.top + row * self.grid_size,
        )
    }

    /// The cell containing a world point, if it lies inside the grid.
    pub fn cell_at(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let (c, r) = self.world_to_grid(x, y);
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.width && (r as usize) < self.height)
            .then_some((c as usize, r as usize))
    }

    /// Continuous image coordinates with +y up on screen (image row 0 at the maximum y).
    pub fn world_to_image(&self, x: f64, y: f64, scale: f64) -> (f64, f64) {
        let (c, r) = self.world_to_grid(x, y);
        (c * scale, (self.height as f64 - r) * scale)
    }

    /// Inverse of [`GridTransform::world_to_image`].
    pub fn image_to_world(&self, ix: f64, iy: f64, scale: f64) -> (f64, f64) {
        self.grid_to_world(ix / scale, self.height as f64 - iy / scale)
    }

    /// World bounds `(x0, y0, x1, y1)` of the whole grid.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let (x1, y1) = self.grid_to_world(self.width as f64, self.height as f64);
        (self.left, self.top, x1, y1)
    }
}

/// A classified pixel grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// World x of the centre of column 0, mm (the frame's `left`).
    pub left: i32,
    /// World y of the centre of row 0, mm (the frame's `top`).
    pub top: i32,
    /// mm per pixel.
    pub grid_size: i32,
    pub encoding: Encoding,
    /// Row-major, row 0 at `top`.
    pub pixels: Vec<Pixel>,
}

impl Grid {
    /// Classifies `bytes` (row-major, `width * height`). Missing bytes count as outside.
    pub fn decode(
        bytes: &[u8],
        width: usize,
        height: usize,
        left: i32,
        top: i32,
        grid_size: i32,
        encoding: Encoding,
    ) -> Self {
        let n = width * height;
        let mut pixels: Vec<Pixel> = bytes
            .iter()
            .take(n)
            .map(|&b| classify(b, encoding))
            .collect();
        pixels.resize(n, Pixel::OUTSIDE);
        Self {
            width,
            height,
            left,
            top,
            grid_size,
            encoding,
            pixels,
        }
    }

    pub fn get(&self, col: usize, row: usize) -> Pixel {
        if col < self.width && row < self.height {
            self.pixels[row * self.width + col]
        } else {
            Pixel::OUTSIDE
        }
    }

    /// The world transform. The frame origin is the centre of cell (0, 0): the app shifts it by
    /// half a cell to get the corner (map-plugin.md §4.1), and user carpet corners on r2579a lie
    /// on that shifted lattice.
    pub fn transform(&self) -> GridTransform {
        let g = self.grid_size.max(1) as f64;
        GridTransform {
            left: self.left as f64 - g / 2.0,
            top: self.top as f64 - g / 2.0,
            grid_size: g,
            width: self.width,
            height: self.height,
        }
    }

    /// True when the grid has no non-outside pixel (this covers the 2 x 2 empty map).
    pub fn is_empty(&self) -> bool {
        self.pixels.iter().all(|p| p.class == PixelClass::Outside)
    }

    /// Floor pixel count per room id (boundary pixels not counted).
    pub fn segment_histogram(&self) -> BTreeMap<u8, usize> {
        let mut m = BTreeMap::new();
        for p in &self.pixels {
            if let PixelClass::Floor(s) = p.class {
                *m.entry(s).or_insert(0) += 1;
            }
        }
        m
    }

    /// Pixel count per class code (maps.md §5.1).
    pub fn class_histogram(&self) -> BTreeMap<u8, usize> {
        let mut m = BTreeMap::new();
        for p in &self.pixels {
            *m.entry(p.class.code()).or_insert(0) += 1;
        }
        m
    }

    /// Pairs of rooms `(a, b)` with `a < b` whose floor pixels lie within `radius` cells of each
    /// other. A radius of 2 bridges a one-pixel wall. Used when `nei_id` is missing (live frames).
    pub fn adjacency(&self, radius: usize) -> std::collections::BTreeSet<(u8, u8)> {
        let mut out = std::collections::BTreeSet::new();
        let r = radius as isize;
        for row in 0..self.height {
            for col in 0..self.width {
                let PixelClass::Floor(a) = self.get(col, row).class else {
                    continue;
                };
                // Look forward only (right and down half-plane) to visit each pair once.
                for dy in 0..=r {
                    for dx in -r..=r {
                        if dy == 0 && dx <= 0 {
                            continue;
                        }
                        let (c, rr) = (col as isize + dx, row as isize + dy);
                        if c < 0 {
                            continue;
                        }
                        if let PixelClass::Floor(b) = self.get(c as usize, rr as usize).class {
                            if a != b {
                                out.insert((a.min(b), a.max(b)));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Cell bounds `(col0, row0, col1, row1)` (inclusive) of a room's floor and boundary pixels.
    pub fn segment_bounds(&self, seg: u8) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for (i, p) in self.pixels.iter().enumerate() {
            if p.class.segment() == Some(seg) {
                let (c, r) = (i % self.width, i / self.width);
                b = Some(match b {
                    None => (c, r, c, r),
                    Some((c0, r0, c1, r1)) => (c0.min(c), r0.min(r), c1.max(c), r1.max(r)),
                });
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use PixelClass::*;

    fn t(v: serde_json::Value) -> Trailer {
        Trailer::from_value(v).unwrap()
    }

    #[test]
    fn encoding_selection() {
        let live = t(json!({"fsm": 1, "ris": 2}));
        assert_eq!(
            select_encoding(2, FrameType::I, &live, false),
            Encoding::FrameMap
        );
        assert_eq!(
            select_encoding(3, FrameType::I, &live, false),
            Encoding::V3 { building: false }
        );
        assert_eq!(
            select_encoding(2, FrameType::W, &live, false),
            Encoding::Wifi
        );
        assert_eq!(
            select_encoding(2, FrameType::I, &t(json!({"ris": 1})), false),
            Encoding::Building
        );
        assert_eq!(
            select_encoding(3, FrameType::I, &t(json!({"ris": 0})), false),
            Encoding::V3 { building: true }
        );
        assert_eq!(
            select_encoding(2, FrameType::I, &t(json!({"ris": 2})), false),
            Encoding::Vslam
        );
        assert_eq!(
            select_encoding(0, FrameType::I, &Trailer::default(), false),
            Encoding::Vslam
        );
        assert_eq!(
            select_encoding(0, FrameType::I, &Trailer::default(), true),
            Encoding::Saved
        );
        assert_eq!(
            select_encoding(2, FrameType::I, &t(json!({"cleanareaorder": []})), true),
            Encoding::Saved
        );
        assert_eq!(
            map_version(false, false, true, Some(&t(json!({"cleanareaorder": []})))),
            2
        );
        assert_eq!(map_version(true, true, true, None), 3);
        assert_eq!(map_version(false, false, true, None), 1);
    }

    #[test]
    fn encoding_b_frame_map() {
        let e = Encoding::FrameMap;
        assert_eq!(classify(0x00, e), Pixel::OUTSIDE);
        assert_eq!(
            classify(0x04, e),
            Pixel {
                class: Floor(1),
                carpet: false
            }
        );
        assert_eq!(
            classify(0x30, e),
            Pixel {
                class: Floor(12),
                carpet: false
            }
        );
        assert_eq!(
            classify(0x07, e),
            Pixel {
                class: Floor(1),
                carpet: true
            }
        );
        assert_eq!(
            classify(0x2B, e),
            Pixel {
                class: Floor(10),
                carpet: true
            }
        );
        assert_eq!(classify(0xFC, e).class, Wall(None));
        assert_eq!(classify(0xF8, e).class, FloorNoRoom);
        assert_eq!(classify(0xF4, e).class, Unknown);
        assert_eq!(classify(0x01, e).class, NewSegment);
        assert_eq!(
            classify(0x03, e),
            Pixel {
                class: NewSegment,
                carpet: true
            }
        );
        assert_eq!(classify(0x02, e).class, Wall(None));
    }

    #[test]
    fn encoding_e_saved() {
        let e = Encoding::Saved;
        assert_eq!(classify(0x01, e).class, Floor(1));
        assert_eq!(classify(0x0D, e).class, Floor(13));
        assert_eq!(
            classify(0x41, e),
            Pixel {
                class: Floor(1),
                carpet: true
            }
        );
        assert_eq!(classify(0x81, e).class, Wall(Some(1)));
        assert_eq!(classify(0x8D, e).class.code(), 113);
        assert_eq!(classify(0x80, e).class, Wall(None));
        assert_eq!(
            classify(0x40, e),
            Pixel {
                class: Outside,
                carpet: true
            }
        );
    }

    #[test]
    fn encoding_a_v3() {
        let e = Encoding::V3 { building: false };
        assert_eq!(classify(0x05, e).class, Floor(5));
        assert_eq!(
            classify(0x85, e),
            Pixel {
                class: Floor(5),
                carpet: true
            }
        );
        assert_eq!(classify(0x25, e).class, Wall(Some(5)));
        assert_eq!(classify(0x65, e).class, HiddenWall(5));
        assert_eq!(classify(0x65, e).class.code(), 205);
        assert_eq!(classify(0x1F, e).class, FloorNoRoom);
        assert_eq!(
            classify(0x1F, Encoding::V3 { building: true }).class,
            NewSegment
        );
        assert_eq!(classify(0x3F, e).class, Wall(None));
        assert_eq!(classify(0x80, e).class, Outside);
    }

    #[test]
    fn encodings_c_d_wifi_clean() {
        assert_eq!(classify(0x01, Encoding::Building).class, NewSegment);
        assert_eq!(
            classify(0x03, Encoding::Building),
            Pixel {
                class: NewSegment,
                carpet: true
            }
        );
        assert_eq!(classify(0x02, Encoding::Building).class, Wall(None));
        assert_eq!(classify(0x04, Encoding::Building).class, Outside);
        assert_eq!(classify(0x02, Encoding::Vslam).class, Wall(None));
        assert_eq!(classify(0x05, Encoding::Vslam).class, NewSegment);
        assert_eq!(classify(0xFC, Encoding::Wifi).class, Wifi(12));
        assert_eq!(classify(0xF0, Encoding::Wifi).class, Outside);
        assert_eq!(classify(0x01, Encoding::CleanMap).class, Cleaned);
        assert_eq!(classify(0x02, Encoding::CleanMap).class, Dirty);
    }

    #[test]
    fn grid_helpers() {
        let g = Grid::decode(
            &[0, 1, 1, 0x81, 0, 0x41, 2, 0x82],
            4,
            2,
            -100,
            -50,
            50,
            Encoding::Saved,
        );
        assert_eq!(g.segment_histogram(), BTreeMap::from([(1, 3), (2, 1)]));
        assert_eq!(g.segment_bounds(1), Some((1, 0, 3, 1)));
        assert_eq!(g.get(3, 0).class, Wall(Some(1)));
        assert_eq!(g.get(9, 9), Pixel::OUTSIDE);
        // The origin (-100, -50) is the centre of cell (0, 0).
        let tr = g.transform();
        assert_eq!(tr.cell_center(0, 0), (-100.0, -50.0));
        assert_eq!(tr.cell_at(-100.0, -50.0), Some((0, 0)));
        assert_eq!(tr.cell_at(-124.0, -74.0), Some((0, 0)));
        assert_eq!(tr.cell_at(-126.0, 0.0), None);
        assert_eq!(tr.cell_at(-74.0, 0.0), Some((1, 1)));
        // Y flip: the bottom-left world corner maps to the bottom-left image corner.
        assert_eq!(tr.world_to_image(-125.0, -75.0, 1.0), (0.0, 2.0));
        assert_eq!(tr.image_to_world(0.0, 2.0, 1.0), (-125.0, -75.0));
        assert_eq!(tr.bounds(), (-125.0, -75.0, 75.0, 25.0));
        // Rooms 1 and 2 touch across one wall pixel.
        let g = Grid::decode(&[1, 0x80, 2, 1, 0x80, 2], 3, 2, 0, 0, 50, Encoding::Saved);
        assert!(g.adjacency(1).is_empty());
        assert_eq!(g.adjacency(2), [(1, 2)].into());
    }
}
