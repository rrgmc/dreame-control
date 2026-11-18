//! The room grid inside a recovery archive, and edits to it.
//!
//! `divide/1/segmented_map_with_obstacle_large.png` is the room grid the robot loads on a restore:
//! it equals the saved map's pixels in encoding E (bits 5 to 0 the room id, bit 6 carpet, bit 7
//! wall), cropped to `map_roi_large.json`. Three plain copies (`segmented_map_large.png`,
//! `dec_map_large.png`, `last_seg_map_large.png`) hold bare room ids with walls as 0. The robot
//! rebuilds the grid from the plain copies after a cleaning run, so an edit changes all four
//! (`docs/research/maps.md` §12.2, experiments W18 to W20).
//!
//! Cells are addressed in image coordinates: column `x` and row `y` of the grid image. Add the
//! region's `x_min` and `y_min` for big-grid coordinates.

use crate::archive::Archive;
use crate::{decode_err, Result};
use dreame_core::Error;
use std::io::Write;

pub const GRID: &str = "divide/1/segmented_map_with_obstacle_large.png";
pub const PLAIN: [&str; 3] = ["divide/1/segmented_map_large.png", "divide/1/dec_map_large.png", "divide/1/last_seg_map_large.png"];
pub const REGION: &str = "divide/1/map_roi_large.json";

const ROOM_BITS: u8 = 0x3f;
const WALL_BIT: u8 = 0x80;
/// Cell size of the big grid, in mm.
const CELL_MM: f64 = 50.0;
/// Big-grid cell whose low edge is at 0 mm.
const GRID_ORIGIN: i64 = 1800;
/// Highest room id the 6 room bits hold.
pub const MAX_ROOM: u8 = ROOM_BITS;

/// A direction on the grid image; `Up` is towards row 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn step(self) -> (i64, i64) {
        match self {
            Self::Up => (0, -1),
            Self::Down => (0, 1),
            Self::Left => (-1, 0),
            Self::Right => (1, 0),
        }
    }
}

impl std::str::FromStr for Direction {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            _ => Err(format!("unknown direction {s:?}; use up, down, left or right")),
        }
    }
}

/// An axis of the grid image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl std::str::FromStr for Axis {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "x" => Ok(Self::X),
            "y" => Ok(Self::Y),
            _ => Err(format!("unknown axis {s:?}; use x or y")),
        }
    }
}

/// The four segmentation images of an archive, decoded.
#[derive(Debug, Clone)]
pub struct Segmentation {
    pub width: u32,
    pub height: u32,
    /// Big-grid column of image column 0.
    pub x_min: i64,
    /// Big-grid row of image row 0.
    pub y_min: i64,
    grid: Vec<u8>,
    plain: [Vec<u8>; 3],
    dirty: bool,
}

/// The result of an edit: which cells changed in each image.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Cells changed in the grid image, in image coordinates.
    pub cells: Vec<(u32, u32)>,
    /// Cells changed in each plain image, in [`PLAIN`] order.
    pub plain: [usize; 3],
}

/// The cells that change rooms when the border of rooms `a` and `b` moves to a drawn line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BorderCells {
    /// Cells of `b` that move to `a`, sorted.
    pub to_a: Vec<(usize, usize)>,
    /// Cells of `a` that move to `b`, sorted.
    pub to_b: Vec<(usize, usize)>,
}

/// Whether the step from cell centre `p` to cell centre `q` crosses the segment `from`-`to`. The
/// centres must lie on opposite sides of the segment's line, and the crossing point on the
/// segment, ends included. A centre exactly on the line counts as the left side, so a line through
/// cell centres never isolates a cell.
fn crosses(p: (f64, f64), q: (f64, f64), from: (f64, f64), to: (f64, f64)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len2 = dx * dx + dy * dy;
    let cross = |p: (f64, f64)| dx * (p.1 - from.1) - dy * (p.0 - from.0);
    let (cp, cq) = (cross(p), cross(q));
    if len2 == 0.0 || (cp >= 0.0) == (cq >= 0.0) {
        return false;
    }
    let t = cp / (cp - cq);
    let x = (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
    let u = ((x.0 - from.0) * dx + (x.1 - from.1) * dy) / len2;
    (0.0..=1.0).contains(&u)
}

/// Cuts rooms `a` and `b` in two along a drawn line, straight or bent (`line` holds its points in
/// world mm, at least two), and gives each part to the room whose cells it holds most of. The part
/// with more of `b` goes to `b`.
///
/// The grid is `width` by `height` cells; `room_at` gives a cell's room and `centre` its centre in
/// world mm. Two side-by-side cells of the pair stay joined unless a segment of the line passes
/// between their centres ([`crosses`]). The line must cut the pair into exactly two parts: it
/// must run across both rooms from edge to edge.
pub fn border_cells(
    width: usize,
    height: usize,
    room_at: impl Fn(usize, usize) -> Option<u8>,
    centre: impl Fn(usize, usize) -> (f64, f64),
    (a, b): (u8, u8),
    line: &[(f64, f64)],
) -> Result<BorderCells> {
    if a == b || a == 0 || b == 0 {
        return Err(Error::Unsupported(format!("rooms {a} and {b} must be two different rooms")));
    }
    let segments: Vec<_> = line.windows(2).map(|w| (w[0], w[1])).filter(|(p, q)| p != q).collect();
    if segments.is_empty() {
        return Err(Error::Unsupported("the line has no length".into()));
    }
    let cut = |p: (f64, f64), q: (f64, f64)| segments.iter().any(|&(f, t)| crosses(p, q, f, t));
    let in_pair = |c: usize, r: usize| room_at(c, r).is_some_and(|s| s == a || s == b);
    let mut part = vec![usize::MAX; width * height];
    let mut parts: Vec<Vec<(usize, usize)>> = Vec::new();
    for r0 in 0..height {
        for c0 in 0..width {
            if part[r0 * width + c0] != usize::MAX || !in_pair(c0, r0) {
                continue;
            }
            let k = parts.len();
            let mut cells = vec![(c0, r0)];
            part[r0 * width + c0] = k;
            let mut i = 0;
            while i < cells.len() {
                let (c, r) = cells[i];
                i += 1;
                let next = [(c.wrapping_sub(1), r), (c + 1, r), (c, r.wrapping_sub(1)), (c, r + 1)];
                for (nc, nr) in next {
                    if nc >= width || nr >= height || part[nr * width + nc] != usize::MAX || !in_pair(nc, nr) {
                        continue;
                    }
                    if cut(centre(c, r), centre(nc, nr)) {
                        continue;
                    }
                    part[nr * width + nc] = k;
                    cells.push((nc, nr));
                }
            }
            parts.push(cells);
        }
    }
    let [p, q] = &parts[..] else {
        return Err(Error::Unsupported(format!(
            "the line cuts rooms {a} and {b} into {} parts, not two; draw it across both rooms from edge to edge",
            parts.len()
        )));
    };
    let count = |cells: &[(usize, usize)], room: u8| cells.iter().filter(|&&(c, r)| room_at(c, r) == Some(room)).count();
    let (side_a, side_b) = if count(p, b) > count(q, b) { (q, p) } else { (p, q) };
    let pick = |cells: &[(usize, usize)], room: u8| {
        let mut v: Vec<_> = cells.iter().copied().filter(|&(c, r)| room_at(c, r) == Some(room)).collect();
        v.sort_unstable();
        v
    };
    let out = BorderCells { to_a: pick(side_a, b), to_b: pick(side_b, a) };
    if out.to_a.is_empty() && out.to_b.is_empty() {
        return Err(Error::Unsupported(format!("the line leaves the border of rooms {a} and {b} where it is")));
    }
    let (size_a, size_b) = (count(p, a) + count(q, a), count(p, b) + count(q, b));
    if out.to_a.len() == size_b || out.to_b.len() == size_a {
        return Err(Error::Unsupported("the line leaves one room no cells; merge the rooms instead".into()));
    }
    Ok(out)
}

fn decode_gray(name: &str, png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).map_err(|e| decode_err(format!("{name}: {e}")))?;
    if img.color() != image::ColorType::L8 {
        return Err(Error::Unsupported(format!("{name} is {:?}, not 8-bit grey", img.color())));
    }
    let g = img.into_luma8();
    Ok((g.width(), g.height(), g.into_raw()))
}

/// Encodes an 8-bit grey PNG the way the experiments did: one IDAT, filter 0 on every row, zlib
/// level 9. The robot loaded these files.
pub fn encode_gray(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut raw = Vec::with_capacity(pixels.len() + height as usize);
    for row in pixels.chunks(width as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&raw).expect("writing to a Vec");
    let idat = z.finish().expect("writing to a Vec");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in bytes {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

impl Segmentation {
    /// Reads the four images and the region from an archive. All four must have the same size.
    pub fn read(a: &Archive) -> Result<Self> {
        let get = |name: &str| a.get(name).ok_or_else(|| decode_err(format!("the archive has no {name}")));
        let region: serde_json::Value =
            serde_json::from_slice(get(REGION)?).map_err(|e| decode_err(format!("{REGION}: {e}")))?;
        let corner = |k: &str| region[k].as_i64().ok_or_else(|| decode_err(format!("{REGION} has no {k}")));
        let (width, height, grid) = decode_gray(GRID, get(GRID)?)?;
        let mut plain: [Vec<u8>; 3] = Default::default();
        for (i, name) in PLAIN.iter().enumerate() {
            let (w, h, p) = decode_gray(name, get(name)?)?;
            if (w, h) != (width, height) {
                return Err(decode_err(format!("{name} is {w}x{h}, the grid is {width}x{height}")));
            }
            plain[i] = p;
        }
        Ok(Self { width, height, x_min: corner("x_min")?, y_min: corner("y_min")?, grid, plain, dirty: false })
    }

    /// Writes the four images back if an edit changed them. An unedited archive stays as it was.
    pub fn write(&self, a: &mut Archive) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        a.set(GRID, encode_gray(self.width, self.height, &self.grid))?;
        for (name, p) in PLAIN.iter().zip(&self.plain) {
            a.set(name, encode_gray(self.width, self.height, p))?;
        }
        Ok(())
    }

    fn index(&self, x: i64, y: i64) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.width as i64 && y < self.height as i64).then(|| y as usize * self.width as usize + x as usize)
    }

    /// The raw grid byte of a cell (encoding E).
    pub fn grid_byte(&self, x: u32, y: u32) -> Option<u8> {
        self.index(x as i64, y as i64).map(|i| self.grid[i])
    }

    /// The room id of a cell in the grid image (0 for none).
    pub fn room_at(&self, x: u32, y: u32) -> Option<u8> {
        self.grid_byte(x, y).map(|b| b & ROOM_BITS)
    }

    /// The rooms that share a cell side with `room` in the grid image, room 0 left out.
    pub fn neighbours(&self, room: u8) -> std::collections::BTreeSet<u8> {
        let mut out = std::collections::BTreeSet::new();
        for y in 0..self.height as i64 {
            for x in 0..self.width as i64 {
                if self.index(x, y).map(|i| self.grid[i] & ROOM_BITS) != Some(room) {
                    continue;
                }
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    if let Some(r) = self.index(x + dx, y + dy).map(|i| self.grid[i] & ROOM_BITS) {
                        if r != 0 && r != room {
                            out.insert(r);
                        }
                    }
                }
            }
        }
        out
    }

    /// Cell count per room id in the grid image, room 0 left out.
    pub fn room_sizes(&self) -> std::collections::BTreeMap<u8, usize> {
        let mut m = std::collections::BTreeMap::new();
        for b in &self.grid {
            let r = b & ROOM_BITS;
            if r != 0 {
                *m.entry(r).or_default() += 1;
            }
        }
        m
    }

    /// Moves cells to `room`. In the grid image only the room bits change, so carpet and wall bits
    /// stay. In each plain image a cell changes only where it holds the cell's old room id, so
    /// walls (0 there) stay walls. Cells outside the image, or already in `room`, are skipped.
    pub fn assign(&mut self, cells: &[(u32, u32)], room: u8) -> Result<Changes> {
        if room == 0 || room > MAX_ROOM {
            return Err(Error::Unsupported(format!("room id {room} is outside 1..={MAX_ROOM}")));
        }
        let mut ch = Changes::default();
        for &(x, y) in cells {
            let Some(i) = self.index(x as i64, y as i64) else { continue };
            let old = self.grid[i] & ROOM_BITS;
            if old == room {
                continue;
            }
            self.grid[i] = (self.grid[i] & !ROOM_BITS) | room;
            ch.cells.push((x, y));
            for (k, p) in self.plain.iter_mut().enumerate() {
                if old != 0 && p[i] == old {
                    p[i] = room;
                    ch.plain[k] += 1;
                }
            }
        }
        self.dirty |= !ch.cells.is_empty();
        Ok(ch)
    }

    /// Grows `room` into its neighbour `from` by up to `cells` cells in `direction`, along their
    /// whole shared border. A border cell is a cell of `from` whose neighbour against `direction`
    /// is in `room`; from each one the run continues while it stays in `from`. So with
    /// `Direction::Up` the border between `from` above and `room` below moves up.
    pub fn grow(&mut self, room: u8, from: u8, direction: Direction, cells: u32) -> Result<Changes> {
        if room == from {
            return Err(Error::Unsupported("a room cannot grow into itself".into()));
        }
        let (dx, dy) = direction.step();
        let mut take = Vec::new();
        for y in 0..self.height as i64 {
            for x in 0..self.width as i64 {
                let at = |x: i64, y: i64| self.index(x, y).map(|i| self.grid[i] & ROOM_BITS);
                if at(x, y) != Some(from) || at(x - dx, y - dy) != Some(room) {
                    continue;
                }
                for k in 0..cells as i64 {
                    let (cx, cy) = (x + k * dx, y + k * dy);
                    if at(cx, cy) != Some(from) {
                        break;
                    }
                    take.push((cx as u32, cy as u32));
                }
            }
        }
        if take.is_empty() {
            return Err(Error::Unsupported(format!("rooms {room} and {from} share no border in that direction")));
        }
        self.assign(&take, room)
    }

    /// Moves the cells of `room` at or beyond image column (`Axis::X`) or row (`Axis::Y`) `at`
    /// to `new`, a room id the grid does not use yet. Only the grid images change: the room
    /// metadata elsewhere in the archive (`seg_info` in `1.dat2`) has no entry for `new`.
    pub fn split(&mut self, room: u8, new: u8, axis: Axis, at: i64) -> Result<Changes> {
        if self.room_sizes().contains_key(&new) {
            return Err(Error::Unsupported(format!("room id {new} is already on the map")));
        }
        let mut take = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let c = match axis {
                    Axis::X => x as i64,
                    Axis::Y => y as i64,
                };
                if c >= at && self.room_at(x, y) == Some(room) {
                    take.push((x, y));
                }
            }
        }
        if take.is_empty() {
            return Err(Error::Unsupported(format!("room {room} has no cells beyond that line")));
        }
        if take.len() == self.room_sizes().get(&room).copied().unwrap_or(0) {
            return Err(Error::Unsupported(format!("the line leaves nothing of room {room}")));
        }
        self.assign(&take, new)
    }

    /// Paints cells into `room`, an existing room: the cells that belong to another room move to
    /// it. Cells outside every room (room id 0) stay outside, so painting cannot grow the floor.
    pub fn paint(&mut self, room: u8, cells: &[(u32, u32)]) -> Result<Changes> {
        if !self.room_sizes().contains_key(&room) {
            return Err(Error::Unsupported(format!("room {room} is not on the map")));
        }
        self.take_cells(room, cells)
    }

    /// Paints cells into `new`, a room id the grid does not use yet, like [`Segmentation::paint`].
    /// Only the grid images change; [`crate::roomreg::add_room`] registers the new room.
    pub fn paint_new(&mut self, new: u8, cells: &[(u32, u32)]) -> Result<Changes> {
        if self.room_sizes().contains_key(&new) {
            return Err(Error::Unsupported(format!("room id {new} is already on the map")));
        }
        let ch = self.take_cells(new, cells)?;
        if ch.cells.is_empty() {
            return Err(Error::Unsupported("the paint holds no room cells".into()));
        }
        Ok(ch)
    }

    /// Moves every cell of `room` to its neighbour `into`, so `room` leaves the grid.
    /// [`crate::roomreg::remove_room`] takes it out of the room lists.
    pub fn remove(&mut self, room: u8, into: u8) -> Result<Changes> {
        if !self.neighbours(room).contains(&into) {
            return Err(Error::Unsupported(format!("room {into} is not a neighbour of room {room} in the grid")));
        }
        let mut take = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if self.room_at(x, y) == Some(room) {
                    take.push((x, y));
                }
            }
        }
        self.assign(&take, into)
    }

    /// Erases floor cells: each cell that belongs to a room becomes outside (0) in all four
    /// images, with its carpet and wall bits cleared. Cells already outside, or outside the
    /// image, are skipped. A room may lose every cell; [`crate::roomreg::forget_room`] then takes
    /// it out of the room lists. Refused when no room would keep a cell.
    pub fn erase(&mut self, cells: &[(u32, u32)]) -> Result<Changes> {
        let mut take = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for &(x, y) in cells {
            if matches!(self.room_at(x, y), Some(r) if r != 0) && seen.insert((x, y)) {
                take.push((x, y));
            }
        }
        if take.is_empty() {
            return Err(Error::Unsupported("the area holds no floor cells".into()));
        }
        if take.len() == self.room_sizes().values().sum::<usize>() {
            return Err(Error::Unsupported("the area holds the whole floor; erase less".into()));
        }
        let mut ch = Changes::default();
        for (x, y) in take {
            let i = self.index(x as i64, y as i64).expect("checked by room_at");
            self.grid[i] = 0;
            ch.cells.push((x, y));
            for (k, p) in self.plain.iter_mut().enumerate() {
                if p[i] != 0 {
                    p[i] = 0;
                    ch.plain[k] += 1;
                }
            }
        }
        self.dirty = true;
        Ok(ch)
    }

    /// Erases every cell of `room` ([`Segmentation::erase`]), so `room` leaves the grid.
    pub fn erase_room(&mut self, room: u8) -> Result<Changes> {
        let mut cells = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if room != 0 && self.room_at(x, y) == Some(room) {
                    cells.push((x, y));
                }
            }
        }
        if cells.is_empty() {
            return Err(Error::Unsupported(format!("room {room} is not on the map")));
        }
        self.erase(&cells)
    }

    /// The cells of other rooms move to `room`; room-0 cells stay. Refused when a room would lose
    /// every cell.
    fn take_cells(&mut self, room: u8, cells: &[(u32, u32)]) -> Result<Changes> {
        let take: Vec<(u32, u32)> = cells.iter().copied().filter(|&(x, y)| matches!(self.room_at(x, y), Some(r) if r != 0)).collect();
        let mut taken = std::collections::BTreeMap::<u8, usize>::new();
        let mut seen = std::collections::HashSet::new();
        for &(x, y) in &take {
            if seen.insert((x, y)) {
                *taken.entry(self.room_at(x, y).unwrap_or(0)).or_default() += 1;
            }
        }
        let sizes = self.room_sizes();
        if let Some((r, _)) = taken.iter().find(|&(r, n)| *r != room && sizes.get(r) == Some(n)) {
            return Err(Error::Unsupported(format!("the paint takes every cell of room {r}; merge the rooms instead")));
        }
        self.assign(&take, room)
    }

    /// Moves the border of rooms `a` and `b` to a drawn line, straight or bent (points in world
    /// mm), computed on this grid with [`border_cells`]. Cells of `b` on `a`'s side move to `a`,
    /// and the other way.
    pub fn move_border(&mut self, a: u8, b: u8, line: &[(f64, f64)]) -> Result<Changes> {
        let mm = |cell: usize, min: i64| (cell as i64 + min - GRID_ORIGIN) as f64 * CELL_MM + CELL_MM / 2.0;
        let (w, h) = (self.width as usize, self.height as usize);
        let room_at = |c: usize, r: usize| self.room_at(c as u32, r as u32);
        let centre = |c: usize, r: usize| (mm(c, self.x_min), mm(r, self.y_min));
        let cells = border_cells(w, h, room_at, centre, (a, b), line)?;
        let img = |v: &[(usize, usize)]| v.iter().map(|&(c, r)| (c as u32, r as u32)).collect::<Vec<_>>();
        let mut ch = self.assign(&img(&cells.to_a), a)?;
        let more = self.assign(&img(&cells.to_b), b)?;
        ch.cells.extend(more.cells);
        for (k, n) in more.plain.iter().enumerate() {
            ch.plain[k] += n;
        }
        Ok(ch)
    }

    /// The image cells that hold points in world millimetres, without repeats, in first-seen
    /// order. Points outside the image are dropped.
    pub fn cells_at_mm(&self, points: &[(f64, f64)]) -> Vec<(u32, u32)> {
        let mut seen = std::collections::HashSet::new();
        points
            .iter()
            .filter_map(|&(x, y)| {
                let (cx, cy) = self.cell_at_mm(x, y);
                self.index(cx, cy).map(|_| (cx as u32, cy as u32))
            })
            .filter(|c| seen.insert(*c))
            .collect()
    }

    /// The image cell that holds a point in world millimetres. Big-grid cell `g` spans
    /// `(g - 1800) * 50` to `(g - 1799) * 50` mm on each axis. The cell may lie outside the image.
    pub fn cell_at_mm(&self, x_mm: f64, y_mm: f64) -> (i64, i64) {
        let g = |mm: f64| (mm / CELL_MM).floor() as i64 + GRID_ORIGIN;
        (g(x_mm) - self.x_min, g(y_mm) - self.y_min)
    }

    /// The cells of a straight line between two image cells, ends included, clipped to the image.
    /// Diagonal steps make the line 8-connected.
    pub fn line(&self, from: (i64, i64), to: (i64, i64)) -> Vec<(u32, u32)> {
        let (mut x, mut y) = from;
        let (dx, dy) = ((to.0 - x).abs(), -(to.1 - y).abs());
        let (sx, sy) = ((to.0 - x).signum(), (to.1 - y).signum());
        let mut err = dx + dy;
        let mut out = Vec::new();
        loop {
            if self.index(x, y).is_some() {
                out.push((x as u32, y as u32));
            }
            if (x, y) == to {
                return out;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Sets (`wall` true) or clears the wall bit of cells. In the grid image the room bits and the
    /// carpet bit stay. The plain images mark walls as 0, so a new wall cell becomes 0 there and a
    /// cleared one takes its room id back from the grid image. Cells outside the image, or
    /// already in the wanted state, are skipped.
    pub fn set_wall(&mut self, cells: &[(u32, u32)], wall: bool) -> Changes {
        let mut ch = Changes::default();
        for &(x, y) in cells {
            let Some(i) = self.index(x as i64, y as i64) else { continue };
            if (self.grid[i] & WALL_BIT != 0) == wall {
                continue;
            }
            self.grid[i] ^= WALL_BIT;
            ch.cells.push((x, y));
            let plain = if wall { 0 } else { self.grid[i] & ROOM_BITS };
            for (k, p) in self.plain.iter_mut().enumerate() {
                if p[i] != plain {
                    p[i] = plain;
                    ch.plain[k] += 1;
                }
            }
        }
        self.dirty |= !ch.cells.is_empty();
        ch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_to_cell() {
        let s = seg(10, 10, vec![0; 100]);
        let s = Segmentation { x_min: 1681, y_min: 1662, ..s };
        // The saved map's left edge, -5950 mm, is big-grid 1681.
        assert_eq!(s.cell_at_mm(-5950.0, -6900.0), (0, 0));
        assert_eq!(s.cell_at_mm(-5901.0, -6851.0), (0, 0));
        assert_eq!(s.cell_at_mm(-5900.0, -6850.0), (1, 1));
    }

    #[test]
    fn line_is_clipped_and_connected() {
        let s = seg(5, 5, vec![0; 25]);
        assert_eq!(s.line((-2, 1), (2, 1)), [(0, 1), (1, 1), (2, 1)]);
        assert_eq!(s.line((0, 0), (2, 2)), [(0, 0), (1, 1), (2, 2)]);
        assert_eq!(s.line((3, 4), (3, 2)).len(), 3);
    }

    #[test]
    fn paint_moves_only_room_cells() {
        // Room 1, outside, room 2, room 2.
        let mut s = seg(4, 1, vec![1, 0, 2, 2]);
        let ch = s.paint(1, &[(1, 0), (2, 0), (2, 0)]).unwrap();
        assert_eq!(ch.cells, [(2, 0)]);
        assert_eq!(s.room_at(1, 0), Some(0), "outside stays outside");
        assert!(s.paint(5, &[(0, 0)]).is_err(), "room 5 is not on the map");
        // One cell of room 1, two of room 3: painting room 1's only cell would remove room 1.
        let mut s = seg(3, 1, vec![1, 3, 3]);
        assert!(s.paint(3, &[(0, 0)]).is_err(), "would remove room 1");
        assert!(s.paint(1, &[(1, 0)]).is_ok(), "room 3 keeps a cell");
        let s = Segmentation { x_min: 1800, y_min: 1800, ..seg(3, 1, vec![0; 3]) };
        assert_eq!(s.cells_at_mm(&[(25.0, 25.0), (30.0, 40.0), (75.0, 25.0), (-25.0, 25.0)]), [(0, 0), (1, 0)]);
    }

    #[test]
    fn border_moves_to_the_line() {
        // Rooms 1 and 2 side by side, two columns each, between outside columns:
        // 0 1 1 2 2 0, three rows. Image x 0 is big-grid 1800, so column c is centred at 50c+25 mm.
        let row = [0, 1, 1, 2, 2, 0];
        let mut s = Segmentation { x_min: 1800, y_min: 1800, ..seg(6, 3, row.repeat(3)) };
        // A vertical line at x = 200 mm: columns 1 to 3 on one side, 4 on the other.
        let ch = s.move_border(1, 2, &[(200.0, -10.0), (200.0, 160.0)]).unwrap();
        assert_eq!(ch.cells, [(3, 0), (3, 1), (3, 2)]);
        assert_eq!(ch.plain, [3, 3, 3]);
        assert_eq!((s.room_at(3, 1), s.room_at(4, 1)), (Some(1), Some(2)));
        // Back the other way: room 2 takes columns 2 and 3.
        let ch = s.move_border(1, 2, &[(100.0, 160.0), (100.0, -10.0)]).unwrap();
        assert_eq!(ch.cells, [(2, 0), (2, 1), (2, 2), (3, 0), (3, 1), (3, 2)]);
        assert_eq!(s.room_sizes()[&1], 3);
        // On the existing border: nothing to do.
        assert!(s.move_border(1, 2, &[(100.0, -10.0), (100.0, 160.0)]).is_err());
        // A line that ends inside the rooms does not cut them in two.
        assert!(s.move_border(1, 2, &[(150.0, -10.0), (150.0, 60.0)]).is_err());
        // Past the last column of room 2: room 2 would have nothing left.
        assert!(s.move_border(1, 2, &[(250.0, -10.0), (250.0, 160.0)]).is_err());
    }

    #[test]
    fn border_cut_needs_the_segment_between_the_centres() {
        // One row 1 1 2 2 at unit spacing; a short line between columns 1 and 2 cuts the row.
        let rooms = [1u8, 1, 2, 2];
        let at = |c: usize, _r: usize| rooms.get(c).copied();
        let centre = |c: usize, r: usize| (c as f64, r as f64);
        let cells = border_cells(4, 1, at, centre, (1, 2), &[(1.5, -1.0), (1.5, 1.0)]);
        assert!(cells.is_err(), "the line is on the existing border");
        let cells = border_cells(4, 1, at, centre, (1, 2), &[(0.5, -1.0), (0.5, 1.0)]).unwrap();
        assert_eq!(cells, BorderCells { to_a: vec![], to_b: vec![(1, 0)] });
        // The same line, moved off the row, cuts nothing.
        assert!(border_cells(4, 1, at, centre, (1, 2), &[(0.5, 1.0), (0.5, 3.0)]).is_err());
        // A line through a centre puts that cell on one side; it is not cut off on its own.
        let cells = border_cells(4, 1, at, centre, (1, 2), &[(2.0, -1.0), (2.0, 1.0)]).unwrap();
        assert_eq!(cells.to_a.len() + cells.to_b.len(), 1);
    }

    #[test]
    fn border_follows_a_bent_line() {
        // Four rows of 1 1 2 2 at unit spacing. An L-shaped line comes down between columns 2
        // and 3, then turns left between rows 1 and 2: the top-left block becomes room 1 and the
        // rest room 2.
        let at = |c: usize, r: usize| (c < 4 && r < 4).then_some(if c < 2 { 1 } else { 2 });
        let centre = |c: usize, r: usize| (c as f64, r as f64);
        let line = [(2.5, -1.0), (2.5, 1.5), (-1.0, 1.5)];
        let cells = border_cells(4, 4, at, centre, (1, 2), &line).unwrap();
        assert_eq!(cells.to_a, [(2, 0), (2, 1)]);
        assert_eq!(cells.to_b, [(0, 2), (0, 3), (1, 2), (1, 3)]);
        // Only the first leg: it ends inside the rooms, so it cuts nothing.
        assert!(border_cells(4, 4, at, centre, (1, 2), &line[..2]).is_err());
        assert!(border_cells(4, 4, at, centre, (1, 2), &[(0.0, 0.0), (0.0, 0.0)]).is_err(), "no length");
    }

    #[test]
    fn paint_new_and_remove() {
        // Room 1, outside, room 2, room 2.
        let mut s = seg(4, 1, vec![1, 0, 2, 2]);
        let ch = s.paint_new(5, &[(1, 0), (3, 0)]).unwrap();
        assert_eq!(ch.cells, [(3, 0)], "outside stays outside");
        assert_eq!(s.room_at(3, 0), Some(5));
        assert!(s.paint_new(2, &[(0, 0)]).is_err(), "room 2 exists");
        assert!(s.paint_new(6, &[(1, 0)]).is_err(), "no room cells");
        assert!(s.paint_new(6, &[(0, 0)]).is_err(), "would take all of room 1");
        // Room 5 touches room 2 only.
        assert!(s.remove(5, 1).is_err(), "not neighbours");
        let ch = s.remove(5, 2).unwrap();
        assert_eq!(ch.cells, [(3, 0)]);
        assert_eq!(s.room_sizes(), [(1, 1), (2, 2)].into());
        assert_eq!(s.plain[0], [1, 0, 2, 2], "plain images follow");
    }

    #[test]
    fn erase_makes_cells_outside() {
        // Room 1 (one carpet cell, one wall cell), outside, room 2, room 2.
        let mut s = seg(5, 1, vec![1 | 0x40, 1 | 0x80, 0, 2, 2]);
        let ch = s.erase(&[(0, 0), (1, 0), (2, 0), (3, 0), (3, 0), (9, 9)]).unwrap();
        assert_eq!(ch.cells, [(0, 0), (1, 0), (3, 0)], "outside and off-image cells skipped, repeats once");
        // The wall cell is already 0 in the plain images.
        assert_eq!(ch.plain, [2, 2, 2]);
        assert_eq!(s.grid, [0, 0, 0, 0, 2]);
        assert_eq!(s.plain[2], [0, 0, 0, 0, 2]);
        assert_eq!(s.room_sizes(), [(2, 1)].into(), "room 1 has no cells left");
        assert!(s.dirty);
        assert!(s.erase(&[(0, 0), (2, 0)]).is_err(), "no floor cells");
        assert!(s.erase(&[(4, 0)]).is_err(), "the whole floor");
        let mut s = seg(3, 1, vec![1, 2, 2 | 0x80]);
        assert_eq!(s.erase_room(2).unwrap().cells, [(1, 0), (2, 0)]);
        assert!(s.erase_room(2).is_err(), "gone");
        assert!(s.erase_room(1).is_err(), "the whole floor");
    }

    #[test]
    fn split_makes_a_new_room() {
        // Two rows of room 3, columns 0 to 3.
        let mut s = seg(4, 2, vec![3; 8]);
        let ch = s.split(3, 9, Axis::X, 2).unwrap();
        assert_eq!(ch.cells.len(), 4);
        assert_eq!(s.room_sizes(), [(3, 4), (9, 4)].into());
        assert!(s.split(3, 9, Axis::Y, 1).is_err(), "9 is taken now");
        assert!(s.split(3, 5, Axis::X, 0).is_err(), "nothing left of room 3");
        assert!(s.split(3, 5, Axis::Y, 7).is_err(), "no cells beyond the line");
    }

    #[test]
    fn set_and_clear_walls() {
        let mut s = seg(3, 1, vec![7, 7 | 0x40, 7 | 0x80]);
        let ch = s.set_wall(&[(0, 0), (1, 0), (2, 0)], true);
        assert_eq!(ch.cells, [(0, 0), (1, 0)]);
        assert_eq!(ch.plain, [2, 2, 2]);
        assert_eq!(s.grid_byte(1, 0), Some(7 | 0x40 | 0x80), "carpet and room kept");
        assert_eq!(s.plain[0], [0, 0, 0]);
        let ch = s.set_wall(&[(0, 0)], false);
        assert_eq!(ch.cells, [(0, 0)]);
        assert_eq!(s.grid_byte(0, 0), Some(7));
        assert_eq!(s.plain[1][0], 7);
    }

    fn seg(w: u32, h: u32, grid: Vec<u8>) -> Segmentation {
        let plain = grid.iter().map(|b| if b & 0x80 != 0 { 0 } else { b & ROOM_BITS }).collect::<Vec<_>>();
        Segmentation { width: w, height: h, x_min: 0, y_min: 0, grid, plain: [plain.clone(), plain.clone(), plain], dirty: false }
    }

    #[test]
    fn png_round_trip() {
        let px: Vec<u8> = (0..=255).cycle().take(7 * 5).collect();
        let png = encode_gray(7, 5, &px);
        assert_eq!(decode_gray("t", &png).unwrap(), (7, 5, px));
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn grow_up_moves_horizontal_border() {
        // Rows 0 to 2 room 12, rows 3 to 4 room 10; one carpet cell and one wall cell in room 12.
        let mut g = vec![12u8; 3 * 3];
        g.extend(vec![10u8; 2 * 3]);
        g[3] |= 0x40;
        g[7] |= 0x80;
        let mut s = seg(3, 5, g);
        let ch = s.grow(10, 12, Direction::Up, 2).unwrap();
        assert_eq!(ch.cells.len(), 6);
        assert_eq!(s.room_sizes()[&10], 12);
        assert_eq!(s.grid_byte(0, 1), Some(0x40 | 10), "carpet bit kept");
        assert_eq!(s.grid_byte(1, 2), Some(0x80 | 10), "wall bit kept");
        // The wall cell is 0 in the plain images, so it stays 0 there.
        assert_eq!(ch.plain, [5, 5, 5]);
        assert_eq!(s.room_at(0, 0), Some(12));
        assert!(s.dirty);
    }

    #[test]
    fn grow_stops_at_other_rooms_and_edges() {
        // Column: 5, 12, 10. Growing 10 up by 3 takes only the one 12 cell.
        let mut s = seg(1, 3, vec![5, 12, 10]);
        let ch = s.grow(10, 12, Direction::Up, 3).unwrap();
        assert_eq!(ch.cells, [(0, 1)]);
        assert_eq!(s.room_at(0, 0), Some(5));
        assert!(s.grow(10, 7, Direction::Left, 1).is_err());
    }

    #[test]
    fn assign_checks_room_id() {
        let mut s = seg(1, 1, vec![1]);
        assert!(s.assign(&[(0, 0)], 0).is_err());
        assert!(s.assign(&[(0, 0)], 64).is_err());
        assert_eq!(s.assign(&[(0, 0), (5, 5)], 2).unwrap().cells, [(0, 0)]);
    }

    #[test]
    fn unedited_write_leaves_archive() {
        let s = seg(1, 1, vec![1]);
        // No archive members are touched, so an empty archive is fine.
        let mut a = Archive::decode_tar(&[0u8; 1024]).unwrap();
        s.write(&mut a).unwrap();
    }
}
