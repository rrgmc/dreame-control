//! The map on screen: the rendered rooms, room names, the dock, the robot and its path, with
//! pan and zoom. It also takes the clicks and drags that pick rooms or draw a zone.

use dreame::live::RobotPose;
use dreame_map::model::{Point, Rect};
use dreame_map::path::Path;
use dreame_map::render::{render, RenderOptions};
use dreame_map::view::{View, V2};
use dreame_map::Map;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, TextureHandle, TextureOptions};
use std::collections::BTreeMap;

/// Texels per grid cell. Higher keeps room edges smooth when zoomed in.
const SCALE: u32 = 4;
/// Robot body radius in mm, about the real size.
const ROBOT_RADIUS_MM: f32 = 175.0;
/// Path pen width in mm, about what the official app shows.
const PATH_WIDTH_MM: f32 = 40.0;

/// What a primary-button drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Pan,
    /// Draws a rectangle; right-drag pans.
    Rect,
    /// Draws a line; right-drag pans.
    Line,
}

/// What a click or drag on the map asks for, in world mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapInput {
    None,
    Click(Point),
    /// A rectangle dragged, from corner to corner.
    Rect(Point, Point),
    /// A line dragged, from start to end.
    Line(Point, Point),
}

/// A shape to outline on the map, such as a staged edit or a selection.
#[derive(Debug, Clone, Copy)]
pub struct Mark {
    pub shape: dreame::shapes::Shape,
    pub colour: Color32,
}

/// What the canvas draws on top of the map this frame.
pub struct Overlay<'a> {
    pub path: &'a Path,
    pub pose: Option<RobotPose>,
    /// Rooms picked for a room clean, in cleaning order.
    pub picked: &'a [u32],
    pub zone: Option<Rect>,
    pub drag: Drag,
    pub marks: Vec<Mark>,
    /// Name hidden rooms too, greyed, as the official app does while editing.
    pub hidden_labels: bool,
}

#[derive(Default)]
pub struct MapView {
    view: Option<View>,
    texture: Option<TextureHandle>,
    /// What the texture shows: the map generation and the rooms drawn in colour.
    texture_key: Option<(u64, Vec<u32>)>,
    /// Counts [`MapView::set_map`] calls, so a new map always renders.
    generation: u64,
    labels: BTreeMap<u32, Point>,
    drag_from: Option<Point>,
    drag_to: Option<Point>,
    fit_pending: bool,
}

impl MapView {
    /// Call when a new map arrives: labels are recomputed and the view fits it.
    pub fn set_map(&mut self, map: &Map) {
        self.labels = map.label_points();
        self.generation += 1;
        if self.view.is_none() {
            self.fit_pending = true;
        }
    }

    /// Drops the line or rectangle being dragged, so releasing the button draws nothing.
    /// Returns whether one was being dragged.
    pub fn cancel_drag(&mut self) -> bool {
        let dragging = self.drag_from.is_some();
        self.drag_from = None;
        self.drag_to = None;
        dragging
    }

    /// World mm per screen point at the current zoom.
    pub fn mm_per_point(&self, map: &Map) -> f64 {
        let zoom = self.view.map_or(1.0, |v| v.zoom);
        f64::from(map.grid.grid_size) / zoom.max(1e-6)
    }

    /// Draws the map and returns what the user asked for. `coloured` lists the rooms drawn in
    /// colour; when it is empty, every room is.
    pub fn show(&mut self, ui: &mut egui::Ui, map: &Map, coloured: &[u32], o: &Overlay) -> MapInput {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let size = V2::new(f64::from(rect.width()), f64::from(rect.height()));
        if self.fit_pending || self.view.is_none() {
            self.view = Some(View::fit(map.grid.width, map.grid.height, size));
            self.fit_pending = false;
        }
        self.update_texture(ui.ctx(), map, coloured);
        let tr = map.transform();
        let origin = rect.min;
        let to_v2 = |p: Pos2| V2::new(f64::from(p.x - origin.x), f64::from(p.y - origin.y));
        let mut input = MapInput::None;
        {
            let view = self.view.as_mut().expect("set above");
            // Zoom: wheel, trackpad pinch, or ctrl + wheel.
            if let Some(p) = response.hover_pos() {
                let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
                let factor = (f64::from(scroll) / 200.0).exp() * f64::from(pinch);
                if (factor - 1.0).abs() > 1e-6 {
                    view.zoom_at(to_v2(p), factor);
                }
            }
            let drawing = o.drag != Drag::Pan;
            let draw = drawing && response.dragged_by(egui::PointerButton::Primary);
            if !draw
                && (response.dragged_by(egui::PointerButton::Primary)
                    || response.dragged_by(egui::PointerButton::Secondary)
                    || response.dragged_by(egui::PointerButton::Middle))
            {
                let d = response.drag_delta();
                view.pan(V2::new(f64::from(d.x), f64::from(d.y)));
            }
            let world = |p: Pos2| view.screen_to_world(&tr, to_v2(p));
            if drawing {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.drag_from = response.interact_pointer_pos().map(world);
                }
                if self.drag_from.is_some() {
                    self.drag_to = response.interact_pointer_pos().map(world).or(self.drag_to);
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.drag_from.take(), self.drag_to.take()) {
                        input = if o.drag == Drag::Line { MapInput::Line(a, b) } else { MapInput::Rect(a, b) };
                    }
                }
            } else {
                self.drag_from = None;
                self.drag_to = None;
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    input = MapInput::Click(world(p));
                }
            }
        }
        let view = *self.view.as_ref().expect("set above");
        let to_screen = |p: Point| {
            let v = view.world_to_screen(&tr, p);
            Pos2::new(origin.x + v.x as f32, origin.y + v.y as f32)
        };
        let px_per_mm = view.zoom as f32 / map.grid.grid_size.max(1) as f32;

        let painter = painter.with_clip_rect(rect);
        if let Some(tex) = &self.texture {
            let a = view.cell_to_screen(V2::new(0.0, 0.0));
            let b = view.cell_to_screen(V2::new(map.grid.width as f64, map.grid.height as f64));
            let r = egui::Rect::from_min_max(
                Pos2::new(origin.x + a.x as f32, origin.y + a.y as f32),
                Pos2::new(origin.x + b.x as f32, origin.y + b.y as f32),
            );
            painter.image(tex.id(), r, egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }

        // The path, white like the official app's.
        let stroke = Stroke::new((PATH_WIDTH_MM * px_per_mm).max(1.5), Color32::from_rgba_unmultiplied(255, 255, 255, 220));
        let mut line: Vec<Pos2> = Vec::new();
        for p in &o.path.points {
            if !p.connected && line.len() > 1 {
                painter.add(Shape::line(std::mem::take(&mut line), stroke));
            }
            if !p.connected {
                line.clear();
            }
            line.push(to_screen(Point::new(p.x as f64, p.y as f64)));
        }
        if line.len() > 1 {
            painter.add(Shape::line(line, stroke));
        }

        // Marks, the chosen zone, and what is being drawn.
        use dreame::shapes::Shape as S;
        let outline = |shape: &S, colour: Color32| match *shape {
            S::Rect(z) => {
                let r = egui::Rect::from_two_pos(to_screen(Point::new(z.x0, z.y0)), to_screen(Point::new(z.x1, z.y1)));
                painter.rect_filled(r, 2.0, colour.gamma_multiply(0.25));
                painter.rect_stroke(r, 2.0, Stroke::new(2.0, colour), egui::StrokeKind::Middle);
            }
            S::Line(l) => {
                painter.line_segment([to_screen(l.a), to_screen(l.b)], Stroke::new(3.0, colour));
            }
            S::Ellipse { center, size } => {
                let c = to_screen(center);
                let r = egui::vec2(size.0 as f32, size.1 as f32) * px_per_mm / 2.0;
                painter.add(Shape::ellipse_stroke(c, r, Stroke::new(2.0, colour)));
            }
        };
        for m in &o.marks {
            outline(&m.shape, m.colour);
        }
        let drawn = match (self.drag_from, self.drag_to) {
            (Some(a), Some(b)) if o.drag == Drag::Line => Some(S::Line(dreame_map::model::Line::new(a.x, a.y, b.x, b.y))),
            (Some(a), Some(b)) => Some(S::Rect(Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)))),
            _ => o.zone.map(S::Rect),
        };
        if let Some(d) = drawn {
            outline(&d, crate::ACCENT);
        }

        // Room names, with the cleaning order of picked rooms.
        let font = FontId::proportional(13.0);
        for (id, p) in &self.labels {
            let Some(room) = map.rooms.get(id) else { continue };
            if room.hidden && !o.hidden_labels {
                continue;
            }
            let pos = to_screen(*p);
            let name = room.display_name();
            let text = match o.picked.iter().position(|r| r == id) {
                Some(i) => format!("{}  {name}", i + 1),
                None => name,
            };
            let galley = painter.layout_no_wrap(text, font.clone(), Color32::from_gray(30));
            let bg = egui::Rect::from_center_size(pos, galley.size() + egui::vec2(12.0, 6.0));
            let picked = o.picked.contains(id);
            let fill = if picked { crate::ACCENT } else { Color32::from_white_alpha(225) };
            painter.rect_filled(bg, 10.0, fill);
            let colour = match (picked, room.hidden) {
                (true, _) => Color32::WHITE,
                (false, true) => Color32::from_gray(140),
                (false, false) => Color32::from_gray(30),
            };
            painter.galley_with_override_text_color(bg.min + egui::vec2(6.0, 3.0), galley, colour);
        }

        // The dock and the robot.
        if let Some(d) = map.dock {
            let c = to_screen(Point::new(d.x, d.y));
            let r = (120.0 * px_per_mm).max(7.0);
            painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r)), r * 0.4, Color32::from_rgb(60, 60, 60));
            painter.text(c, Align2::CENTER_CENTER, "⚡", FontId::proportional(r * 1.3), Color32::from_rgb(255, 210, 60));
        }
        let robot = o.pose.map(|p| (p.x, p.y, p.angle)).or(map.robot.map(|p| (p.x, p.y, p.angle)));
        if let Some((x, y, angle)) = robot {
            let c = to_screen(Point::new(x, y));
            let r = (ROBOT_RADIUS_MM * px_per_mm).max(8.0);
            painter.circle(c, r, Color32::WHITE, Stroke::new(2.0, Color32::from_rgb(40, 120, 230)));
            painter.circle_filled(c, r * 0.35, Color32::from_rgb(40, 120, 230));
            // Heading: world angle in degrees, counter-clockwise; screen y points down.
            let a = (angle as f32).to_radians();
            let tip = c + egui::vec2(a.cos(), -a.sin()) * r * 0.8;
            painter.line_segment([c, tip], Stroke::new(2.0, Color32::from_rgb(40, 120, 230)));
        }

        // Zoom buttons, bottom right, as in map apps.
        let ctl = egui::Rect::from_min_size(rect.right_bottom() - egui::vec2(52.0, 132.0), egui::vec2(40.0, 120.0));
        let mut zoom_by = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(ctl), |ui| {
            ui.vertical_centered(|ui| {
                let b = |t: &str| egui::Button::new(egui::RichText::new(t).size(18.0)).min_size(egui::vec2(36.0, 36.0));
                if ui.add(b("+")).on_hover_text(dreame_i18n::tr("Zoom in")).clicked() {
                    zoom_by = Some(1.4);
                }
                if ui.add(b("−")).on_hover_text(dreame_i18n::tr("Zoom out")).clicked() {
                    zoom_by = Some(1.0 / 1.4);
                }
                if ui.add(b(dreame_i18n::tr("Fit"))).on_hover_text(dreame_i18n::tr("Show the whole map")).clicked() {
                    self.fit_pending = true;
                }
            });
        });
        if let (Some(f), Some(v)) = (zoom_by, self.view.as_mut()) {
            v.zoom_at(V2::new(size.x / 2.0, size.y / 2.0), f);
        }
        input
    }

    fn update_texture(&mut self, ctx: &egui::Context, map: &Map, coloured: &[u32]) {
        let key = (self.generation, coloured.to_vec());
        if self.texture_key.as_ref() == Some(&key) && self.texture.is_some() {
            return;
        }
        let opts = RenderOptions {
            scale: SCALE,
            path: false,
            markers: false,
            highlight_active: true,
            ..RenderOptions::default()
        };
        let mut m = map.clone();
        m.active_rooms = coloured.to_vec();
        if let Ok(img) = render(&m, &opts) {
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            self.texture = Some(ctx.load_texture("map", color, TextureOptions::LINEAR));
            self.texture_key = Some(key);
        }
    }
}
