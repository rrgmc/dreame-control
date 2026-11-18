//! Raster rendering to an RGBA buffer and PNG (maps.md §10).
//!
//! The image has +y up: grid row 0 (minimum y) is the bottom image row. Each grid cell becomes a
//! `scale x scale` block. Rooms get one of four colours by greedy graph colouring over the
//! neighbour lists, walls are dark, carpets are hatched, and vector overlays (floor plan,
//! restrictions, furniture, path, dock, robot) are drawn on top in world coordinates.

use crate::grid::{GridTransform, PixelClass};
use crate::model::{CarpetShape, Map, Point};
use crate::{decode_err, Result};
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;

pub type Color = [u8; 4];

/// Colours, defaulting to Tasshack's "Dreame Light" palette.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub floor: Color,
    pub wall: Color,
    pub new_segment: Color,
    pub unknown: Color,
    /// Room fills, indexed by colour index.
    pub rooms: Vec<Color>,
    pub hidden_room: Color,
    /// Rooms the current job does not clean, while it cleans some (see
    /// [`RenderOptions::highlight_active`]).
    pub inactive_room: Color,
    pub carpet_hatch: Color,
    pub path: Color,
    pub mop_path: Color,
    pub floor_plan: Color,
    pub no_go_fill: Color,
    pub no_go_outline: Color,
    pub no_mop_fill: Color,
    pub no_mop_outline: Color,
    pub virtual_wall: Color,
    pub threshold: Color,
    pub furniture: Color,
    pub dock: Color,
    pub robot: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: [0, 0, 0, 0],
            floor: [221, 221, 221, 255],
            wall: [96, 96, 96, 255],
            new_segment: [153, 191, 255, 255],
            unknown: [200, 200, 200, 255],
            rooms: vec![
                [171, 199, 248, 255],
                [249, 224, 125, 255],
                [184, 227, 255, 255],
                [184, 217, 141, 255],
            ],
            hidden_room: [200, 200, 200, 255],
            inactive_room: [218, 218, 218, 255],
            carpet_hatch: [0, 0, 0, 60],
            path: [255, 255, 255, 255],
            mop_path: [120, 200, 255, 255],
            floor_plan: [60, 60, 60, 200],
            no_go_fill: [177, 0, 0, 50],
            no_go_outline: [199, 0, 0, 200],
            no_mop_fill: [170, 47, 255, 50],
            no_mop_outline: [153, 0, 210, 200],
            virtual_wall: [199, 0, 0, 230],
            threshold: [230, 140, 0, 230],
            furniture: [90, 70, 50, 200],
            dock: [40, 160, 40, 255],
            robot: [30, 90, 200, 255],
        }
    }
}

/// What to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    /// Image pixels per grid cell.
    pub scale: u32,
    pub carpets: bool,
    pub floor_plan: bool,
    pub restrictions: bool,
    pub furniture: bool,
    pub path: bool,
    /// Pen width of the path in mm; 0 draws it one pixel wide.
    pub path_width_mm: f64,
    /// While a job cleans some rooms ([`Map::active_rooms`]), draw the others in
    /// [`Theme::inactive_room`], as the app does.
    pub highlight_active: bool,
    pub markers: bool,
    pub theme: Theme,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            scale: 4,
            carpets: true,
            floor_plan: false,
            restrictions: true,
            furniture: true,
            path: true,
            path_width_mm: 0.0,
            highlight_active: false,
            markers: true,
            theme: Theme::default(),
        }
    }
}

/// Greedy four-colour graph colouring over room neighbours, largest room first
/// (`T/map.py:6617-6686`). Returns a colour index per room id.
pub fn room_colors(map: &Map, colors: usize) -> BTreeMap<u32, usize> {
    let colors = colors.max(1);
    let mut order: Vec<_> = map.rooms.values().collect();
    order.sort_by(|a, b| b.pixel_count.cmp(&a.pixel_count).then(a.id.cmp(&b.id)));
    let near = map.grid.adjacency(3);
    let touching = |a: u32, b: u32| match (u8::try_from(a), u8::try_from(b)) {
        (Ok(a), Ok(b)) => near.contains(&(a.min(b), a.max(b))),
        _ => false,
    };
    let mut out: BTreeMap<u32, usize> = BTreeMap::new();
    for r in order {
        let used: Vec<usize> = map
            .rooms
            .keys()
            .filter(|&&o| o != r.id && (map.are_neighbours(r.id, o) || touching(r.id, o)))
            .filter_map(|o| out.get(o).copied())
            .collect();
        let c = (0..colors)
            .find(|c| !used.contains(c))
            .unwrap_or(r.id as usize % colors);
        out.insert(r.id, c);
    }
    out
}

struct Canvas {
    img: RgbaImage,
    tr: GridTransform,
    scale: f64,
}

impl Canvas {
    fn blend(&mut self, x: i64, y: i64, c: Color) {
        if x < 0 || y < 0 || x >= self.img.width() as i64 || y >= self.img.height() as i64 {
            return;
        }
        let p = self.img.get_pixel_mut(x as u32, y as u32);
        let a = c[3] as u32;
        if a == 255 {
            *p = Rgba(c);
            return;
        }
        let ia = 255 - a;
        for (dst, &src) in p.0.iter_mut().zip(&c).take(3) {
            *dst = ((src as u32 * a + *dst as u32 * ia) / 255) as u8;
        }
        p.0[3] = (a + p.0[3] as u32 * ia / 255).min(255) as u8;
    }

    fn to_img(&self, p: Point) -> (f64, f64) {
        self.tr.world_to_image(p.x, p.y, self.scale)
    }

    fn dot(&mut self, x: f64, y: f64, r: i64, c: Color) {
        let (cx, cy) = (x.floor() as i64, y.floor() as i64);
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    self.blend(cx + dx, cy + dy, c);
                }
            }
        }
    }

    /// Draws a line in world coordinates with a pen of radius `r` image pixels.
    fn line(&mut self, a: Point, b: Point, r: i64, c: Color) {
        let (ax, ay) = self.to_img(a);
        let (bx, by) = self.to_img(b);
        let steps = (bx - ax).abs().max((by - ay).abs()).ceil().max(1.0) as i64;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
            if r == 0 {
                self.blend(x.floor() as i64, y.floor() as i64, c);
            } else {
                self.dot(x, y, r, c);
            }
        }
    }

    fn polyline(&mut self, pts: &[Point], closed: bool, r: i64, c: Color) {
        for w in pts.windows(2) {
            self.line(w[0], w[1], r, c);
        }
        if closed && pts.len() > 2 {
            self.line(pts[pts.len() - 1], pts[0], r, c);
        }
    }

    /// Fills an axis-aligned world rectangle.
    fn fill_rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: Color) {
        let (ax, ay) = self.to_img(Point::new(x0, y0));
        let (bx, by) = self.to_img(Point::new(x1, y1));
        let (lx, hx) = (ax.min(bx).floor() as i64, ax.max(bx).ceil() as i64);
        let (ly, hy) = (ay.min(by).floor() as i64, ay.max(by).ceil() as i64);
        for y in ly..hy {
            for x in lx..hx {
                self.blend(x, y, c);
            }
        }
    }
}

/// Renders the map to an RGBA image.
pub fn render(map: &Map, opts: &RenderOptions) -> Result<RgbaImage> {
    let g = &map.grid;
    if g.width == 0 || g.height == 0 {
        return Err(decode_err("cannot render an empty grid"));
    }
    let s = opts.scale.max(1);
    let (w, h) = (g.width as u32 * s, g.height as u32 * s);
    let th = &opts.theme;
    let mut cv = Canvas {
        img: RgbaImage::from_pixel(w, h, Rgba(th.background)),
        tr: g.transform(),
        scale: s as f64,
    };
    let colors = room_colors(map, th.rooms.len());
    let dim = |id: u32| opts.highlight_active && !map.active_rooms.is_empty() && !map.active_rooms.contains(&id);

    for row in 0..g.height {
        for col in 0..g.width {
            let p = g.get(col, row);
            let c = match p.class {
                PixelClass::Outside => {
                    if !p.carpet {
                        continue;
                    }
                    th.floor
                }
                PixelClass::Floor(seg) => {
                    let id = seg as u32;
                    if map.rooms.get(&id).is_some_and(|r| r.hidden) {
                        th.hidden_room
                    } else if dim(id) {
                        th.inactive_room
                    } else {
                        th.rooms[colors.get(&id).copied().unwrap_or(0) % th.rooms.len()]
                    }
                }
                PixelClass::Wall(_) | PixelClass::ObstacleWall => th.wall,
                PixelClass::HiddenWall(seg) => {
                    th.rooms[colors.get(&(seg as u32)).copied().unwrap_or(0) % th.rooms.len()]
                }
                PixelClass::NewSegment => th.new_segment,
                PixelClass::FloorNoRoom | PixelClass::Cleaned => th.floor,
                PixelClass::Unknown | PixelClass::Dirty => th.unknown,
                PixelClass::Wifi(c) => match c {
                    2 => th.wall,
                    10 => [200, 200, 200, 255],
                    11 => [230, 80, 60, 255],
                    12 => [240, 180, 60, 255],
                    13 => [150, 210, 90, 255],
                    _ => [60, 170, 80, 255],
                },
            };
            let x0 = col as u32 * s;
            let y0 = (g.height - 1 - row) as u32 * s;
            for dy in 0..s {
                for dx in 0..s {
                    let (x, y) = (x0 + dx, y0 + dy);
                    let mut px = c;
                    if opts.carpets && p.carpet && (x + y) % 4 == 0 {
                        let hc = th.carpet_hatch;
                        let a = hc[3] as u32;
                        for i in 0..3 {
                            px[i] = ((hc[i] as u32 * a + px[i] as u32 * (255 - a)) / 255) as u8;
                        }
                    }
                    cv.img.put_pixel(x, y, Rgba(px));
                }
            }
        }
    }

    let pen = (s as i64 / 4).max(0);
    if opts.carpets {
        for c in &map.carpets {
            cv.polyline(&c.polygon, true, 0, th.carpet_hatch);
        }
        for a in &map.carpet_areas {
            let outline: Vec<Point> = match a.shape {
                CarpetShape::Rect(r) => vec![
                    Point::new(r.x0, r.y0),
                    Point::new(r.x1, r.y0),
                    Point::new(r.x1, r.y1),
                    Point::new(r.x0, r.y1),
                ],
                // Drawn with the sizes as semi-axes; radius versus width is not confirmed.
                CarpetShape::Ellipse { center, size } => (0..32)
                    .map(|i| {
                        let t = i as f64 * std::f64::consts::TAU / 32.0;
                        Point::new(center.x + size.0 * t.cos(), center.y + size.1 * t.sin())
                    })
                    .collect(),
            };
            cv.polyline(&outline, true, 0, [0, 0, 0, 120]);
        }
    }
    if opts.floor_plan {
        if let Some(fp) = &map.floor_plan {
            for r in &fp.rooms {
                for wl in &r.walls {
                    cv.line(wl.line.a, wl.line.b, 0, th.floor_plan);
                }
            }
            for d in &fp.doors {
                cv.line(d.line.a, d.line.b, pen, [255, 255, 255, 200]);
            }
        }
    }
    if opts.path {
        // Pen radius in image pixels for the width in mm.
        let pen_px = (opts.path_width_mm / f64::from(map.grid.grid_size) * s as f64 / 2.0).round() as i64;
        for stroke in map.path.strokes() {
            for w in stroke.windows(2) {
                let c = if w[1].mode == crate::path::PathMode::Mop {
                    th.mop_path
                } else {
                    th.path
                };
                cv.line(Point::new(w[0].x as f64, w[0].y as f64), Point::new(w[1].x as f64, w[1].y as f64), pen_px, c);
            }
        }
    }
    if opts.restrictions {
        let r = &map.restrictions;
        for (rects, fill, outline) in [
            (&r.no_mop, th.no_mop_fill, th.no_mop_outline),
            (&r.no_go, th.no_go_fill, th.no_go_outline),
        ] {
            for q in rects {
                cv.fill_rect(q.x0, q.y0, q.x1, q.y1, fill);
                cv.polyline(
                    &[
                        Point::new(q.x0, q.y0),
                        Point::new(q.x1, q.y0),
                        Point::new(q.x1, q.y1),
                        Point::new(q.x0, q.y1),
                    ],
                    true,
                    0,
                    outline,
                );
            }
        }
        for l in &r.virtual_walls {
            cv.line(l.a, l.b, pen, th.virtual_wall);
        }
        for l in r
            .thresholds
            .iter()
            .chain(&r.impassable_thresholds)
            .chain(&r.curtains)
        {
            cv.line(l.a, l.b, pen, th.threshold);
        }
    }
    if opts.furniture {
        for f in &map.furniture {
            cv.polyline(&f.corners(), true, 0, th.furniture);
        }
    }
    if opts.markers {
        let rad = (s as i64 * 2).max(2);
        if let Some(d) = map.dock {
            let (x, y) = cv.to_img(Point::new(d.x, d.y));
            cv.dot(x, y, rad, th.dock);
        }
        if let Some(r) = map.robot {
            let (x, y) = cv.to_img(Point::new(r.x, r.y));
            cv.dot(x, y, rad, th.robot);
            let (sn, cs) = r.angle.to_radians().sin_cos();
            let tip = Point::new(
                r.x + cs * map.grid.grid_size as f64 * 3.0,
                r.y + sn * map.grid.grid_size as f64 * 3.0,
            );
            cv.line(Point::new(r.x, r.y), tip, 0, [255, 255, 255, 255]);
        }
    }
    Ok(cv.img)
}

/// Renders the map and encodes it as PNG bytes.
pub fn render_png(map: &Map, opts: &RenderOptions) -> Result<Vec<u8>> {
    let img = render(map, opts)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| decode_err(format!("png: {e}")))?;
    Ok(out.into_inner())
}

/// Renders the map to a PNG file.
pub fn save_png(map: &Map, opts: &RenderOptions, path: &std::path::Path) -> Result<()> {
    let bytes = render_png(map, opts)?;
    std::fs::write(path, bytes).map_err(|e| decode_err(format!("write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::saved_map;

    #[test]
    fn renders_with_y_flip() {
        let m = saved_map();
        let opts = RenderOptions {
            scale: 1,
            markers: false,
            path: false,
            restrictions: false,
            furniture: false,
            carpets: false,
            ..Default::default()
        };
        let img = render(&m, &opts).unwrap();
        assert_eq!((img.width(), img.height()), (6, 4));
        // Grid row 1 (y 50..100) is image row 2; row 0 (minimum y) is the bottom image row.
        let th = Theme::default();
        assert_eq!(img.get_pixel(0, 3).0, th.wall);
        let colors = room_colors(&m, 4);
        assert_ne!(colors[&1], colors[&2]);
        assert_eq!(img.get_pixel(1, 2).0, th.rooms[colors[&1]]);
        assert_eq!(img.get_pixel(4, 1).0, th.rooms[colors[&2]]);
    }

    #[test]
    fn highlights_the_rooms_being_cleaned() {
        let mut m = saved_map();
        let opts = RenderOptions { scale: 1, markers: false, path: false, restrictions: false, furniture: false, carpets: false, ..Default::default() };
        let th = Theme::default();
        let colors = room_colors(&m, 4);
        m.active_rooms = vec![2];
        let plain = render(&m, &opts).unwrap();
        assert_eq!(plain.get_pixel(1, 2).0, th.rooms[colors[&1]], "off by default");
        let img = render(&m, &RenderOptions { highlight_active: true, ..opts.clone() }).unwrap();
        assert_eq!(img.get_pixel(1, 2).0, th.inactive_room);
        assert_eq!(img.get_pixel(4, 1).0, th.rooms[colors[&2]]);
        m.active_rooms.clear();
        let img = render(&m, &RenderOptions { highlight_active: true, ..opts }).unwrap();
        assert_eq!(img.get_pixel(1, 2).0, th.rooms[colors[&1]], "no job: every room in colour");
    }

    #[test]
    fn path_width_widens_the_pen() {
        let mut m = saved_map();
        // A sweep stroke from the centre of cell (1, 1) to the centre of cell (4, 1).
        let (x, y) = m.transform().cell_center(1, 1);
        let dx = m.transform().cell_center(4, 1).0 - x;
        m.path = crate::path::Path::parse(&format!("S{x},{y}L{dx},0")).unwrap();
        let opts = RenderOptions { scale: 10, markers: false, restrictions: false, furniture: false, carpets: false, ..Default::default() };
        let white = |img: &RgbaImage| img.pixels().filter(|p| p.0 == Theme::default().path).count();
        let thin = white(&render(&m, &opts).unwrap());
        let wide = white(&render(&m, &RenderOptions { path_width_mm: 30.0, ..opts }).unwrap());
        assert!(thin > 0 && wide > 3 * thin, "thin {thin}, wide {wide}");
    }

    #[test]
    fn png_bytes() {
        let png = render_png(&saved_map(), &RenderOptions::default()).unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
