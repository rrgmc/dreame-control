//! Pan and zoom over the map, and conversions between world mm, grid cells and screen points.
//! Shared by the desktop apps; it has no GUI code.
//!
//! The map image is drawn with one texel per grid cell, so "cell space" is the image's pixel
//! space: x grows right, y grows down, and the world's +y points up on screen (the grid
//! transform's `world_to_image` does the flip).

use crate::grid::GridTransform;
use crate::model::Point;

/// A 2D point or vector in screen or cell space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

impl V2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// The view: where cell (0, 0)'s corner sits on screen, relative to the canvas origin, and how
/// many screen points one cell takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub offset: V2,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        Self { offset: V2::default(), zoom: 1.0 }
    }
}

pub const MIN_ZOOM: f64 = 0.2;
pub const MAX_ZOOM: f64 = 60.0;

impl View {
    /// Fits a `w x h` cell image into a canvas of `size`, centred, with a small margin.
    pub fn fit(w: usize, h: usize, size: V2) -> Self {
        let (w, h) = (w.max(1) as f64, h.max(1) as f64);
        let zoom = (size.x / w).min(size.y / h) * 0.95;
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        Self { offset: V2::new((size.x - w * zoom) / 2.0, (size.y - h * zoom) / 2.0), zoom }
    }

    /// Canvas-relative screen point of a cell-space point.
    pub fn cell_to_screen(&self, c: V2) -> V2 {
        V2::new(self.offset.x + c.x * self.zoom, self.offset.y + c.y * self.zoom)
    }

    /// Cell-space point under a canvas-relative screen point.
    pub fn screen_to_cell(&self, s: V2) -> V2 {
        V2::new((s.x - self.offset.x) / self.zoom, (s.y - self.offset.y) / self.zoom)
    }

    pub fn world_to_screen(&self, tr: &GridTransform, p: Point) -> V2 {
        let (cx, cy) = tr.world_to_image(p.x, p.y, 1.0);
        self.cell_to_screen(V2::new(cx, cy))
    }

    pub fn screen_to_world(&self, tr: &GridTransform, s: V2) -> Point {
        let c = self.screen_to_cell(s);
        let (x, y) = tr.image_to_world(c.x, c.y, 1.0);
        Point::new(x, y)
    }

    pub fn pan(&mut self, delta: V2) {
        self.offset.x += delta.x;
        self.offset.y += delta.y;
    }

    /// Multiplies the zoom by `factor`, keeping the cell under `anchor` (canvas-relative) fixed.
    pub fn zoom_at(&mut self, anchor: V2, factor: f64) {
        let before = self.screen_to_cell(anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.offset = V2::new(anchor.x - before.x * self.zoom, anchor.y - before.y * self.zoom);
    }
}

/// Rounds a world coordinate to the nearest multiple of `step` mm (`step <= 0` leaves it).
pub fn snap(v: f64, step: f64) -> f64 {
    if step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr() -> GridTransform {
        // 4 x 2 cells of 50 mm; cell (0, 0)'s corner at world (0, 0).
        GridTransform { left: 0.0, top: 0.0, grid_size: 50.0, width: 4, height: 2 }
    }

    fn close(a: V2, b: V2) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn world_screen_round_trip_with_y_up() {
        let v = View { offset: V2::new(10.0, 20.0), zoom: 5.0 };
        let t = tr();
        // World (0, 0) is the bottom-left corner: image row 2, so screen y = 20 + 2*5.
        assert!(close(v.world_to_screen(&t, Point::new(0.0, 0.0)), V2::new(10.0, 30.0)));
        // Higher world y is higher on screen (smaller screen y).
        assert!(v.world_to_screen(&t, Point::new(0.0, 100.0)).y < 30.0);
        let p = Point::new(123.0, 77.0);
        let back = v.screen_to_world(&t, v.world_to_screen(&t, p));
        assert!((back.x - p.x).abs() < 1e-9 && (back.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn zoom_keeps_the_anchor_fixed() {
        let mut v = View { offset: V2::new(3.0, 4.0), zoom: 2.0 };
        let anchor = V2::new(50.0, 40.0);
        let cell = v.screen_to_cell(anchor);
        v.zoom_at(anchor, 3.0);
        assert!(close(v.cell_to_screen(cell), anchor));
        v.zoom_at(anchor, 1e9);
        assert_eq!(v.zoom, MAX_ZOOM);
    }

    #[test]
    fn fit_centres_the_image() {
        let v = View::fit(100, 50, V2::new(400.0, 400.0));
        let a = v.cell_to_screen(V2::new(0.0, 0.0));
        let b = v.cell_to_screen(V2::new(100.0, 50.0));
        assert!((a.x - (400.0 - b.x)).abs() < 1e-9, "horizontally centred");
        assert!((a.y - (400.0 - b.y)).abs() < 1e-9, "vertically centred");
        assert!(b.x - a.x <= 400.0);
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(1234.0, 50.0), 1250.0);
        assert_eq!(snap(-1224.0, 50.0), -1200.0);
        assert_eq!(snap(17.3, 0.0), 17.3);
    }
}
