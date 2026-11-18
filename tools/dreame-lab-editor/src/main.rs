//! `dreame-lab-editor`: a lab map editor for Dreame robot vacuums, for trying features before they
//! reach the apps.
//!
//! It loads the saved map from the robot (or from a snapshot file), shows it with pan and zoom,
//! and stages one edit at a time: draw or delete a no-go zone, no-mop zone, virtual wall,
//! carpet, ignored-carpet area, threshold, impassable threshold or curtain; rename, hide or
//! reorder rooms; change room settings; split a room along a line; merge two neighbour rooms;
//! move the boundary between two neighbour rooms along a straight or bent line; paint cells of
//! neighbour rooms into a room; or paint a new room out of a room. Boundary moves and paints are
//! sent as grid edits through backup and restore (`dreame map edit-archive`), so room ids, names
//! and settings stay. A staged edit shows the exact command first.
//! It is sent only after Apply and a confirmation, through the same flow as
//! `dreame map edit --apply` (busy check, snapshots, result check, read-back), and the map
//! reloads afterwards.
//!
//! Run it from the repository (or below it) so it finds `local/env`. An optional argument names
//! a snapshot file to open instead of the robot.

mod live;
mod state;
mod worker;

use dreame::control;
use dreame::mapedit::{self, Edit};
use dreame_model::commands::{self, Command};
use dreame_map::edit::RoomSettings;
use dreame_map::model::Point;
use dreame_map::render::{render, RenderOptions};
use eframe::egui::{self, Color32, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use state::{PaintCells, Selection, Tool, ZoneKind};
use dreame_map::view::{self, View, V2};
use worker::{Loaded, Reply, Request, Worker};

/// Pixels per grid cell in the map texture.
/// Width of the drawn robot path in mm, about what the app shows.
const PATH_WIDTH_MM: f32 = 40.0;
const TEXTURE_SCALE: u32 = 4;

fn main() -> eframe::Result {
    let file = std::env::args().nth(1);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 860.0]).with_title("Dreame lab editor"),
        ..Default::default()
    };
    eframe::run_native(
        "dreame-lab-editor",
        options,
        Box::new(move |cc| {
            let mut app = Editor::new(Worker::spawn(cc.egui_ctx.clone()));
            match file {
                Some(f) => app.worker.send(Request::LoadFile(f.into())),
                None => app.worker.send(Request::LoadRobot),
            }
            Ok(Box::new(app))
        }),
    )
}

/// What the confirmation dialog will send.
enum Confirm {
    /// The staged map edit.
    Edit,
    /// A control command; `zone` is drawn on the map while waiting.
    Control { cmd: Command, what: String, zone: Option<dreame_map::model::Rect> },
    /// The staged paint cells, as a grid edit through backup and restore.
    Grid,
}

struct Editor {
    worker: Worker,
    loaded: Option<Loaded>,
    texture: Option<TextureHandle>,
    view: View,
    fit_pending: bool,
    tool: Tool,
    selection: Option<Selection>,
    pending: Option<Edit>,
    /// Cells staged by the paint tool; sent apart from `pending`, through backup and restore.
    paint: Option<PaintCells>,
    /// A border move staged by the Move boundary tool; sent like `paint`.
    border: Option<state::BorderMove>,
    /// Points clicked so far for a bent border line (Move boundary tool).
    border_points: Vec<Point>,
    /// Paint brush radius in cells (0 paints one cell).
    brush: usize,
    /// The paint tool removes cells from the staged paint instead of adding them.
    erase: bool,
    /// The paint tool takes the cells of a dragged box instead of a brush.
    paint_box: bool,
    drag_from: Option<Point>,
    rename_to: String,
    /// Put the keyboard focus in the rename box on the next frame.
    focus_rename: bool,
    /// The edit form for the selected furniture item.
    furniture_form: Option<dreame_map::model::Furniture>,
    /// The wire type code the place-furniture tool uses.
    place_kind: i64,
    /// The angle the ramp tool stores, in degrees.
    ramp_angle: i64,
    /// The settings form for the selected room, filled from its stored settings.
    settings_form: Option<RoomSettings>,
    /// The cleaning order being edited: visible room ids.
    order: Vec<u32>,
    snap_mm: f64,
    show_path: bool,
    /// True while following the robot over MQTT.
    following: bool,
    live_path: Option<dreame_map::path::Path>,
    live_pose: Option<live::RobotPose>,
    /// The rooms the followed job cleans; the others are drawn grey.
    live_active: Vec<u32>,
    confirm: Option<Confirm>,
    file_path: String,
    cursor: Option<Point>,
    log: Vec<String>,
}

impl Editor {
    fn new(worker: Worker) -> Self {
        Self {
            worker,
            loaded: None,
            texture: None,
            view: View::default(),
            fit_pending: true,
            tool: Tool::Select,
            selection: None,
            pending: None,
            paint: None,
            border: None,
            border_points: Vec::new(),
            brush: 1,
            erase: false,
            paint_box: false,
            drag_from: None,
            rename_to: String::new(),
            focus_rename: false,
            furniture_form: None,
            place_kind: 7,
            ramp_angle: 0,
            settings_form: None,
            order: Vec::new(),
            snap_mm: 50.0,
            show_path: true,
            following: false,
            live_path: None,
            live_pose: None,
            live_active: Vec::new(),
            confirm: None,
            file_path: String::new(),
            cursor: None,
            log: vec!["loading the saved map...".into()],
        }
    }

    /// Renders the map texture. While following a job that cleans some rooms, the other rooms are
    /// drawn grey, as in the app.
    fn render_texture(&mut self, ctx: &egui::Context, map: &dreame_map::Map) {
        let opts = RenderOptions {
            scale: TEXTURE_SCALE,
            restrictions: false,
            path: false,
            highlight_active: true,
            ..RenderOptions::default()
        };
        let mut m = map.clone();
        m.active_rooms = if self.following { self.live_active.clone() } else { Vec::new() };
        match render(&m, &opts) {
            Ok(img) => {
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                self.texture = Some(ctx.load_texture("map", color, TextureOptions::NEAREST));
            }
            Err(e) => self.log.push(format!("render failed: {e}")),
        }
    }

    fn set_loaded(&mut self, ctx: &egui::Context, l: Loaded) {
        self.render_texture(ctx, &l.map);
        if !l.unparsed.is_empty() {
            self.log.push(format!("map has keys the parser could not read ({:?}); editing is off", l.unparsed));
        }
        self.log.push(format!(
            "loaded map {} from {}: {} rooms, {} path points",
            l.map.map_id,
            l.source,
            l.map.rooms.len(),
            l.path.points.len()
        ));
        // Indices of zones may have changed: drop what refers to the old map.
        self.selection = None;
        self.pending = None;
        self.paint = None;
        self.border = None;
        if matches!(self.tool, Tool::Paint { .. } | Tool::MoveBoundary { .. }) {
            self.tool = Tool::Select;
        }
        self.settings_form = None;
        self.order = state::initial_order(&l.map);
        self.loaded = Some(l);
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        while let Some(reply) = self.worker.poll() {
            match reply {
                Reply::Loaded(l) => {
                    self.fit_pending = self.loaded.is_none();
                    self.set_loaded(ctx, *l);
                }
                Reply::Applied { report, loaded } => {
                    self.log.push(format!(
                        "applied: result code {:?}; saved map changed: {}; live frame changed: {}",
                        report.code, report.pointer_changed, report.live_changed
                    ));
                    for st in &report.steps {
                        self.log.push(format!("  {st}"));
                    }
                    if report.changes.is_empty() {
                        self.log.push("  no change seen in the saved map".into());
                    }
                    for c in &report.changes {
                        self.log.push(format!("  {c}"));
                    }
                    self.log.push(format!("  snapshots: {}", report.before_file.display()));
                    let new_room = self.loaded.as_ref().and_then(|old| state::unnamed_new_room(&old.map, &loaded.map));
                    self.set_loaded(ctx, *loaded);
                    if let Some(id) = new_room {
                        // A split leaves the new half without a name or type: offer to name it.
                        self.selection = Some(Selection::Room(id));
                        self.settings_form = self
                            .loaded
                            .as_ref()
                            .and_then(|l| l.map.rooms.get(&id)?.settings.as_deref().map(|s| (id, s.to_vec())))
                            .and_then(|(id, s)| RoomSettings::from_cleanset(id, &s));
                        self.rename_to.clear();
                        self.focus_rename = true;
                        self.log.push(format!("room {id} is new and has no name; type one under Selection and press Rename"));
                    }
                }
                Reply::Controlled { what, result } => {
                    let ok = control::result_code(&result) == Some(0);
                    self.log.push(format!("{what}: {} ({result})", if ok { "accepted" } else { "NOT accepted" }));
                }
                Reply::Live { path, pose, frames, active_rooms } => {
                    self.live_path = Some(path);
                    self.live_pose = pose;
                    if active_rooms != self.live_active {
                        if !active_rooms.is_empty() {
                            self.log.push(format!("live: the job cleans rooms {active_rooms:?}"));
                        }
                        self.live_active = active_rooms;
                        if let Some(map) = self.loaded.as_ref().map(|l| l.map.clone()) {
                            self.render_texture(ctx, &map);
                        }
                    }
                    if frames % 20 == 1 {
                        self.log.push(format!("live: {frames} map frames merged"));
                    }
                }
                Reply::GridApplied { lines, loaded } => {
                    self.log.push("grid edit done".into());
                    self.log.extend(lines.into_iter().map(|l| format!("  {l}")));
                    self.set_loaded(ctx, *loaded);
                }
                Reply::Progress(s) => self.log.push(format!("  {s}")),
                Reply::LiveStatus(s) => self.log.push(s),
                Reply::Failed(e) => self.log.push(format!("error: {e}")),
            }
        }
    }

    fn can_edit(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.unparsed.is_empty())
    }

    fn can_apply(&self) -> bool {
        self.can_edit() && self.loaded.as_ref().is_some_and(|l| l.from_robot) && !self.worker.busy
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Map");
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.worker.busy, egui::Button::new("Load from robot")).clicked() {
                self.log.push("loading the saved map from the robot...".into());
                self.worker.send(Request::LoadRobot);
            }
            if self.worker.busy {
                ui.spinner();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.file_path).hint_text("snapshot file").desired_width(170.0));
            if ui.add_enabled(!self.worker.busy && !self.file_path.is_empty(), egui::Button::new("Open")).clicked() {
                self.worker.send(Request::LoadFile(self.file_path.clone().into()));
            }
        });
        if let Some(l) = &self.loaded {
            ui.label(format!("{}: map {}, {} rooms", l.source, l.map.map_id, l.map.rooms.len()));
        }
        ui.horizontal(|ui| {
            if ui.button("Fit to window").clicked() {
                self.fit_pending = true;
            }
            ui.checkbox(&mut self.show_path, "Show robot path");
        });

        ui.separator();
        ui.heading("Tool");
        ui.add_enabled_ui(self.can_edit(), |ui| {
            ui.radio_value(&mut self.tool, Tool::Select, "Select / pan");
            for kind in [
                ZoneKind::NoGo,
                ZoneKind::NoMop,
                ZoneKind::Wall,
                ZoneKind::Carpet,
                ZoneKind::IgnoredCarpet,
                ZoneKind::Threshold,
                ZoneKind::ImpassableThreshold,
                ZoneKind::Curtain,
                ZoneKind::RoundCarpet,
            ] {
                ui.radio_value(&mut self.tool, Tool::Draw(kind), format!("Draw {}", kind.label()));
            }
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.tool, Tool::Draw(ZoneKind::Ramp), "Draw ramp, angle");
                ui.add(egui::DragValue::new(&mut self.ramp_angle).range(0..=359));
            });
            ui.radio_value(&mut self.tool, Tool::Split, "Split a room (drag a line across it)");
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.tool, Tool::PlaceFurniture, "Place furniture (click), type");
                ui.add(egui::DragValue::new(&mut self.place_kind).range(0..=99));
            });
            ui.radio_value(&mut self.tool, Tool::CleanZone, "Draw a zone to clean now");
        });
        if let Tool::MoveBoundary { a, b } = self.tool {
            ui.colored_label(
                Color32::from_rgb(255, 200, 0),
                format!(
                    "Draw the new boundary of rooms {a} and {b} across both rooms: drag a straight line, or click \
                     the points of a bent one and double-click the last"
                ),
            );
            let mut finish = false;
            ui.horizontal(|ui| {
                ui.label(format!("{} points", self.border_points.len()));
                finish = ui.add_enabled(self.border_points.len() >= 2, egui::Button::new("Finish line")).clicked();
                if ui.small_button("Cancel").clicked() {
                    self.tool = Tool::Select;
                    self.border_points.clear();
                }
            });
            if finish {
                self.finish_border(a, b);
            }
        }
        if let Tool::Paint { room } = self.tool {
            let text = match self.paint.as_ref().and_then(|p| p.parent) {
                Some(parent) => format!("Paint the cells of new room {room}; it takes room {parent}'s type, floor and settings"),
                None => format!("Paint cells of other rooms into room {room}"),
            };
            ui.colored_label(Color32::from_rgb(255, 200, 0), text);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.paint_box, "box").on_hover_text("Drag a box instead of brushing");
                ui.add_enabled_ui(!self.paint_box, |ui| {
                    ui.label("brush radius (cells)");
                    ui.add(egui::DragValue::new(&mut self.brush).range(0..=10));
                });
                ui.checkbox(&mut self.erase, "erase");
                if ui.small_button("Done").clicked() {
                    self.tool = Tool::Select;
                }
            });
        }
        egui::ComboBox::from_label("snap")
            .selected_text(if self.snap_mm > 0.0 { format!("{} mm", self.snap_mm) } else { "off".into() })
            .show_ui(ui, |ui| {
                for s in [0.0, 10.0, 50.0, 100.0] {
                    let label = if s > 0.0 { format!("{s} mm") } else { "off".into() };
                    ui.selectable_value(&mut self.snap_mm, s, label);
                }
            });
        ui.label("Scroll to zoom. Drag with the right or middle button to pan. Escape cancels the line or shape being drawn, then the staged edit, then the tool.");

        ui.separator();
        ui.heading("Robot");
        self.robot_ui(ui);

        ui.separator();
        ui.heading("Selection");
        self.selection_ui(ui);

        ui.separator();
        egui::CollapsingHeader::new("Cleaning order").show(ui, |ui| self.order_ui(ui));

        ui.separator();
        ui.heading("Staged edit");
        self.pending_ui(ui);

        ui.separator();
        ui.heading("Log");
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            for line in &self.log {
                ui.label(egui::RichText::new(line).monospace().size(11.0));
            }
        });
    }

    fn selection_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(sel), Some(l)) = (self.selection, &self.loaded) else {
            ui.label("Nothing selected. Click a room or a zone.");
            return;
        };
        match sel {
            Selection::Room(id) => {
                let Some(room) = l.map.rooms.get(&id) else { return };
                ui.label(format!("Room {id}: {}", room.display_name()));
                ui.label(format!(
                    "type {:?}{}{}",
                    room.room_type,
                    room.order.map(|o| format!(", order {o}")).unwrap_or_default(),
                    if room.hidden { ", hidden" } else { "" }
                ));
                let can_edit = self.can_edit();
                let hidden_now = room.hidden;
                let hide_list = |hide| state::toggled_hidden(&l.map, id, hide);
                ui.horizontal(|ui| {
                    let field = ui.text_edit_singleline(&mut self.rename_to);
                    if std::mem::take(&mut self.focus_rename) {
                        field.request_focus();
                    }
                    let ok = can_edit && !self.rename_to.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Rename")).clicked() {
                        self.pending = Some(Edit::RenameRoom { id, name: self.rename_to.trim().to_string() });
                    }
                });
                let mut hidden = hidden_now;
                if ui.add_enabled(can_edit, egui::Checkbox::new(&mut hidden, "Hidden")).changed() {
                    self.pending = Some(Edit::HideRooms(hide_list(hidden)));
                }
                let partners = state::merge_partners(&l.map, id);
                if partners.is_empty() {
                    ui.label("No neighbour rooms to merge with.");
                } else {
                    let label = |other: u32| {
                        let name = l.map.rooms.get(&other).map(|r| r.display_name()).unwrap_or_default();
                        format!("{other}: {name}")
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Merge with");
                        for &other in &partners {
                            if ui.add_enabled(can_edit, egui::Button::new(label(other))).clicked() {
                                self.pending = Some(Edit::MergeRooms(id, other));
                            }
                        }
                    });
                    let paintable = can_edit && u8::try_from(id).is_ok_and(|i| i <= dreame_map::segmap::MAX_ROOM);
                    let b = egui::Button::new("Paint cells into this room");
                    let tip = "Then drag on the map over cells of neighbour rooms. Sent through a backup and restore of the map";
                    if ui.add_enabled(paintable, b).on_hover_text(tip).clicked() {
                        self.tool = Tool::Paint { room: id };
                        self.border = None;
                        if self.paint.as_ref().is_none_or(|p| p.room != id) {
                            self.paint = Some(PaintCells { room: id, ..Default::default() });
                        }
                    }
                    let free = state::free_room_id(&l.map);
                    let b = egui::Button::new("Paint a new room out of this room");
                    let tip = "Then drag over the cells of the new room. It gets an unused id and this room's type, floor and \
                               settings. Sent through a backup and restore of the map (W29)";
                    if ui.add_enabled(paintable && free.is_some(), b).on_hover_text(tip).clicked() {
                        if let Some(new) = free {
                            self.tool = Tool::Paint { room: new };
                            self.border = None;
                            self.paint = Some(PaintCells { room: new, parent: Some(id), ..Default::default() });
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Move boundary with");
                        for &other in &partners {
                            let b = egui::Button::new(label(other));
                            let tip = "Then drag the new boundary line across both rooms. Sent through a backup and restore of the map";
                            if ui.add_enabled(paintable, b).on_hover_text(tip).clicked() {
                                self.tool = Tool::MoveBoundary { a: id, b: other };
                            }
                        }
                    });
                }

                ui.label(egui::RichText::new("Cleaning settings").strong());
                match &mut self.settings_form {
                    None => {
                        ui.label("No stored settings for this room.");
                    }
                    Some(f) => {
                        const SUCTION: [&str; 4] = ["quiet", "standard", "strong", "turbo"];
                        const CLEAN_TYPE: [&str; 3] = ["sweep", "mop", "sweep and mop"];
                        egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                            ui.label("suction");
                            egui::ComboBox::from_id_salt("suction")
                                .selected_text(SUCTION.get(f.suction as usize).copied().unwrap_or("?"))
                                .show_ui(ui, |ui| {
                                    for (i, s) in SUCTION.iter().enumerate() {
                                        ui.selectable_value(&mut f.suction, i as i64, *s);
                                    }
                                });
                            ui.end_row();
                            ui.label("water");
                            ui.add(egui::DragValue::new(&mut f.water).range(1..=32));
                            ui.end_row();
                            ui.label("passes");
                            ui.add(egui::DragValue::new(&mut f.times).range(1..=3));
                            ui.end_row();
                            if let Some(t) = &mut f.clean_type {
                                ui.label("clean type");
                                egui::ComboBox::from_id_salt("clean_type")
                                    .selected_text(CLEAN_TYPE.get(*t as usize).copied().unwrap_or("?"))
                                    .show_ui(ui, |ui| {
                                        for (i, s) in CLEAN_TYPE.iter().enumerate() {
                                            ui.selectable_value(t, i as i64, *s);
                                        }
                                    });
                                ui.end_row();
                            }
                        });
                        let f = *f;
                        if ui.add_enabled(can_edit, egui::Button::new("Stage settings")).clicked() {
                            self.pending = Some(Edit::RoomSettings {
                                id,
                                suction: Some(f.suction),
                                water: Some(f.water),
                                times: Some(f.times),
                                clean_type: f.clean_type,
                                mop_effect: None,
                            });
                        }
                    }
                }
            }
            Selection::Zone(kind, i) => {
                let quad = kind.shapes(&l.map).get(i).map(state::Shape::quad);
                ui.label(format!("{} {i}: {}", kind.label(), quad.map(|q| format!("{q:?}")).unwrap_or_default()));
                if let (ZoneKind::Ramp, Some(rp)) = (kind, l.map.restrictions.ramps.get(i)) {
                    ui.label(format!("angle {} degrees", rp.angle));
                }
                if ui.add_enabled(self.can_edit(), egui::Button::new("Delete")).clicked() {
                    self.pending = state::delete_edit(sel);
                }
            }
            Selection::Furniture(i) => {
                let can_edit = self.can_edit();
                let Some(f) = &mut self.furniture_form else { return };
                ui.label(format!("furniture {i} (room {}, edit type {}, modified {})", f.segment, f.edit_type, f.modified));
                egui::Grid::new("furniture").num_columns(2).show(ui, |ui| {
                    ui.label("type (wire code)");
                    ui.add(egui::DragValue::new(&mut f.kind).range(0..=99));
                    ui.end_row();
                    ui.label("centre x, y (mm)");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut f.center.x).speed(10.0));
                        ui.add(egui::DragValue::new(&mut f.center.y).speed(10.0));
                    });
                    ui.end_row();
                    ui.label("width, depth (mm)");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut f.width).speed(10.0).range(10.0..=10000.0));
                        ui.add(egui::DragValue::new(&mut f.height).speed(10.0).range(10.0..=10000.0));
                    });
                    ui.end_row();
                    ui.label("angle (degrees)");
                    ui.add(egui::DragValue::new(&mut f.angle).speed(1.0).range(0.0..=359.0));
                    ui.end_row();
                });
                let f = f.clone();
                ui.horizontal(|ui| {
                    if ui.add_enabled(can_edit, egui::Button::new("Stage changes")).clicked() {
                        self.pending = Some(Edit::SetFurniture(i, Box::new(f)));
                    }
                    if ui.add_enabled(can_edit, egui::Button::new("Delete")).clicked() {
                        self.pending = state::delete_edit(sel);
                    }
                });
            }
        }
    }

    fn order_ui(&mut self, ui: &mut egui::Ui) {
        let Some(l) = &self.loaded else { return };
        let mut action: Option<(usize, bool)> = None;
        for (i, id) in self.order.iter().enumerate() {
            let name = l.map.rooms.get(id).map(|r| r.display_name()).unwrap_or_default();
            ui.horizontal(|ui| {
                if ui.small_button("^").clicked() {
                    action = Some((i, true));
                }
                if ui.small_button("v").clicked() {
                    action = Some((i, false));
                }
                ui.label(format!("{}. room {id}: {name}", i + 1));
            });
        }
        if let Some((i, up)) = action {
            state::move_entry(&mut self.order, i, up);
        }
        let changed = self.order != state::initial_order(&l.map);
        let can_edit = self.can_edit();
        ui.horizontal(|ui| {
            if ui.add_enabled(can_edit && changed, egui::Button::new("Stage order")).clicked() {
                self.pending = Some(Edit::CleaningOrder(self.order.clone()));
            }
            if ui.add_enabled(changed, egui::Button::new("Reset")).clicked() {
                self.order = state::initial_order(&l.map);
            }
        });
    }

    /// The staged grid edit (a border move or paint cells), if any. Returns true when it showed one.
    fn paint_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut ok = true;
        if let Some(m) = &self.border {
            let c = &m.cells;
            ui.label(format!(
                "Move the border of rooms {} and {}: {} cells to room {}, {} cells to room {}",
                m.a,
                m.b,
                c.to_a.len(),
                m.a,
                c.to_b.len(),
                m.b
            ));
        } else {
            let (Some(p), Some(l)) = (&self.paint, &self.loaded) else { return false };
            if p.cells.is_empty() {
                return false;
            }
            let from: Vec<String> = p.sources(&l.map).iter().map(|(r, n)| format!("{n} from room {r}")).collect();
            match p.parent {
                Some(parent) => ui.label(format!(
                    "New room {} with room {parent}'s type, floor and settings: {} cells, {}",
                    p.room,
                    p.cells.len(),
                    from.join(", ")
                )),
                None => ui.label(format!("Paint {} cells into room {}: {}", p.cells.len(), p.room, from.join(", "))),
            };
            let emptied = p.emptied(&l.map);
            if !emptied.is_empty() {
                ui.colored_label(Color32::RED, format!("This takes every cell of room {emptied:?}. Erase some cells, or merge the rooms instead."));
                ok = false;
            }
        }
        // A new room's name and floor, written into the archive with it.
        if let (None, Some(p)) = (&self.border, self.paint.as_mut()) {
            if let Some(parent) = p.parent {
                use dreame_map::roomreg::Floor;
                ui.horizontal(|ui| {
                    ui.label("name");
                    let mut name = p.meta.name.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut name).on_hover_text("Empty: room parent's type, numbered").changed() {
                        p.meta.name = (!name.is_empty()).then_some(name);
                    }
                });
                let label = |f: Option<Floor>| match f {
                    None => format!("as room {parent}"),
                    Some(Floor::Tile) => "tile".to_string(),
                    Some(Floor::Wood { direction }) => format!("wood, {direction}°"),
                };
                egui::ComboBox::from_label("floor").selected_text(label(p.meta.floor)).show_ui(ui, |ui| {
                    for f in [None, Some(Floor::Tile), Some(Floor::Wood { direction: 0 }), Some(Floor::Wood { direction: 90 })] {
                        ui.selectable_value(&mut p.meta.floor, f, label(f));
                    }
                });
            }
        }
        ui.colored_label(
            Color32::from_rgb(255, 200, 0),
            "Sent as a grid edit: a fresh backup, the archive edited, the original map and then the edited archive \
             restored, and the room settings put back. Takes about two minutes. This computer serves the archive \
             on port 8770, so the firewall must let the robot in. The backup is kept as a revert file.",
        );
        let mut discard = false;
        let can_apply = self.can_apply();
        ui.horizontal(|ui| {
            discard = ui.button("Discard").clicked();
            if ui.add_enabled(can_apply && ok, egui::Button::new("Apply to robot...")).clicked() {
                self.confirm = Some(Confirm::Grid);
            }
        });
        if discard {
            self.paint = None;
            self.border = None;
        }
        true
    }

    fn pending_ui(&mut self, ui: &mut egui::Ui) {
        if self.paint_ui(ui) {
            return;
        }
        let (Some(edit), Some(l)) = (&self.pending, &self.loaded) else {
            ui.label("None. Draw a shape or split line, delete a zone, or select a room.");
            return;
        };
        ui.label(state::describe(edit));
        let warn = Color32::from_rgb(255, 200, 0);
        if matches!(edit, Edit::MergeRooms(..) | Edit::SplitRoom { .. }) {
            ui.colored_label(
                warn,
                "Room ids change after this edit. Names, settings and the cleaning order of the \
                 rooms involved may need setting again.",
            );
        }
        let built = mapedit::steps(&l.map, edit);
        let from_robot = l.from_robot;
        let can_apply = self.can_apply();
        let mut discard = false;
        match built {
            Ok(steps) => {
                let json = steps
                    .iter()
                    .map(|(label, cmd)| format!("{label}: {}", mapedit::redacted(cmd)))
                    .collect::<Vec<_>>()
                    .join("
");
                egui::ScrollArea::vertical().id_salt("cmd").max_height(140.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(json).monospace().size(11.0));
                });
                ui.horizontal(|ui| {
                    discard = ui.button("Discard").clicked();
                    let tip = if from_robot { "Send this edit to the robot" } else { "Load the map from the robot to apply edits" };
                    if ui.add_enabled(can_apply, egui::Button::new("Apply to robot...")).on_hover_text(tip).clicked() {
                        self.confirm = Some(Confirm::Edit);
                    }
                });
            }
            Err(e) => {
                ui.colored_label(Color32::RED, format!("invalid: {e:#}"));
                discard = ui.button("Discard").clicked();
            }
        }
        if discard {
            self.pending = None;
        }
    }

    fn robot_ui(&mut self, ui: &mut egui::Ui) {
        let ok = self.loaded.as_ref().is_some_and(|l| l.from_robot) && !self.worker.busy;
        ui.horizontal(|ui| {
            for (label, cmd) in [
                ("Start", commands::start()),
                ("Pause", commands::pause()),
                ("Stop", commands::stop()),
                ("Dock", commands::dock()),
            ] {
                if ui.add_enabled(ok, egui::Button::new(label)).clicked() {
                    let what = format!("{} ({})", label.to_lowercase(), control::describe(&cmd)["action"]);
                    self.confirm = Some(Confirm::Control { cmd, what, zone: None });
                }
            }
        });
        if let (Some(Selection::Room(id)), Some(l)) = (self.selection, &self.loaded) {
            if ui.add_enabled(ok, egui::Button::new(format!("Clean room {id} now"))).clicked() {
                match control::clean_rooms(&l.map, &[id], control::CleanOptions::default()) {
                    Ok(cmd) => {
                        let what = format!("clean room {id} with its stored settings");
                        self.confirm = Some(Confirm::Control { cmd, what, zone: None });
                    }
                    Err(e) => self.log.push(format!("cannot clean room {id}: {e:#}")),
                }
            }
        }
        if !ok {
            ui.label("Load the map from the robot to control it.");
        }
        let can_follow = self.loaded.as_ref().is_some_and(|l| l.from_robot);
        ui.horizontal(|ui| {
            if !self.following {
                if ui.add_enabled(can_follow, egui::Button::new("Follow live")).on_hover_text("Listen to the robot's MQTT push and draw its track as it cleans").clicked() {
                    self.following = true;
                    self.live_path = None;
                    self.worker.send_quiet(Request::StartLive);
                }
            } else if ui.button("Stop following").clicked() {
                self.following = false;
                self.worker.send_quiet(Request::StopLive);
                if !std::mem::take(&mut self.live_active).is_empty() {
                    if let Some(map) = self.loaded.as_ref().map(|l| l.map.clone()) {
                        self.render_texture(ui.ctx(), &map);
                    }
                }
            }
            if self.following {
                ui.label(format!("{} live points", self.live_path.as_ref().map_or(0, |p| p.points.len())));
            }
        });
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(confirm) = &self.confirm else { return };
        let (title, lines, detail) = match confirm {
            Confirm::Edit => {
                let what = self.pending.as_ref().map(state::describe).unwrap_or_default();
                (
                    "Send this map edit?",
                    format!("This writes to the robot: {what}."),
                    "The robot must be idle on its dock. Snapshots go to local/out/edits.".to_string(),
                )
            }
            Confirm::Grid => {
                let what = match (&self.border, &self.paint) {
                    (Some(m), _) => format!(
                        "the border of rooms {} and {} moves ({} cells change rooms)",
                        m.a,
                        m.b,
                        m.cells.to_a.len() + m.cells.to_b.len()
                    ),
                    (None, Some(p)) if p.parent.is_some() => format!("{} cells become the new room {}", p.cells.len(), p.room),
                    (None, Some(p)) => format!("{} cells move to room {}", p.cells.len(), p.room),
                    (None, None) => "nothing".into(),
                };
                (
                    "Send this grid edit?",
                    format!("This rewrites the saved map: {what}."),
                    "The robot must be idle on its dock. It backs up the map, restores the original and then \
                     the edited archive, and gets its room settings back. Files go to local/out/recovery."
                        .to_string(),
                )
            }
            Confirm::Control { cmd, what, .. } => (
                "Send this command?",
                format!("The robot will act now: {what}."),
                control::describe(cmd).to_string(),
            ),
        };
        let mut decision: Option<bool> = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(lines);
                ui.label(egui::RichText::new(detail).monospace().size(11.0));
                ui.horizontal(|ui| {
                    if ui.button("Send").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match (decision, self.confirm.take()) {
            (Some(true), Some(Confirm::Edit)) => {
                if let Some(e) = self.pending.take() {
                    self.log.push(format!("applying: {}...", state::describe(&e)));
                    self.worker.send(Request::Apply(e));
                }
            }
            (Some(true), Some(Confirm::Grid)) => {
                let edit = match &self.border {
                    Some(m) => m.edit(),
                    None => self.paint.as_ref().zip(self.loaded.as_ref()).and_then(|(p, l)| p.edit(&l.map)),
                };
                match edit {
                    Some(e) => {
                        self.log.push("applying the grid edit (about two minutes)...".into());
                        self.worker.send(Request::ApplyGrid(e));
                        self.tool = Tool::Select;
                    }
                    None => self.log.push("nothing to paint".into()),
                }
            }
            (Some(true), Some(Confirm::Control { cmd, what, .. })) => {
                self.log.push(format!("sending: {what}..."));
                self.worker.send(Request::Control { cmd, what });
            }
            (Some(_), _) => {}
            (None, c) => self.confirm = c,
        }
    }

    /// Escape takes back one thing per press: the confirmation dialog, then the shape being
    /// dragged, then the staged edit, then the drawing tool. While a text field has focus, Escape
    /// only leaves the field.
    fn handle_escape(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        let painted = self.paint.as_ref().is_some_and(|p| !p.cells.is_empty()) || self.border.is_some();
        let drawing = self.drag_from.is_some() || !self.border_points.is_empty();
        match state::escape(self.confirm.is_some(), drawing, self.pending.is_some() || painted, self.tool) {
            state::Escape::CloseDialog => self.confirm = None,
            state::Escape::CancelDrag => {
                self.drag_from = None;
                self.border_points.clear();
            }
            state::Escape::DiscardStaged => {
                self.pending = None;
                self.paint = None;
                self.border = None;
            }
            state::Escape::LeaveTool => self.tool = Tool::Select,
            state::Escape::Nothing => {}
        }
    }

    /// Stages the border move for the points clicked so far (Move boundary tool), or logs why
    /// there is none.
    fn finish_border(&mut self, a: u32, b: u32) {
        let points = std::mem::take(&mut self.border_points);
        let Some(l) = &self.loaded else { return };
        let staged = state::BorderMove::new(&l.map, a, b, &points, self.snap_mm);
        self.stage_border(staged);
    }

    fn stage_border(&mut self, staged: Result<state::BorderMove, String>) {
        match staged {
            Ok(m) => {
                self.border = Some(m);
                self.paint = None;
                self.tool = Tool::Select;
            }
            Err(e) => self.log.push(format!("boundary not moved: {e}")),
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0, Color32::from_gray(30));
        let Some(l) = &self.loaded else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "no map yet", egui::FontId::default(), Color32::GRAY);
            return;
        };
        let map = &l.map;
        let tr = map.transform();
        if self.fit_pending {
            self.view = View::fit(map.grid.width, map.grid.height, V2::new(rect.width() as f64, rect.height() as f64));
            self.fit_pending = false;
        }

        // Input: zoom, pan, select, draw.
        let origin = rect.min;
        let to_v2 = |p: Pos2| V2::new((p.x - origin.x) as f64, (p.y - origin.y) as f64);
        let hover = response.hover_pos();
        self.cursor = hover.map(|p| self.view.screen_to_world(&tr, to_v2(p)));
        if let Some(p) = hover {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.view.zoom_at(to_v2(p), (scroll as f64 / 200.0).exp());
            }
        }
        let pan_drag = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || (self.tool == Tool::Select && response.dragged_by(egui::PointerButton::Primary));
        if pan_drag {
            let d = response.drag_delta();
            self.view.pan(V2::new(d.x as f64, d.y as f64));
        }
        if let Tool::Draw(kind) = self.tool {
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.drag_from = self.cursor;
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let (Some(a), Some(b)) = (self.drag_from.take(), self.cursor) {
                    self.pending = match state::draw_edit(kind, a, b, self.snap_mm) {
                        Some(Edit::AddRamp(r, _)) => Some(Edit::AddRamp(r, self.ramp_angle)),
                        other => other.or(self.pending.take()),
                    };
                }
            }
        }
        if self.tool == Tool::CleanZone {
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.drag_from = self.cursor;
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let (Some(a), Some(b)) = (self.drag_from.take(), self.cursor) {
                    if let Some(z) = state::drawn_rect(a, b, self.snap_mm) {
                        match control::clean_zone(map, z, control::CleanOptions::default()) {
                            Ok(cmd) => {
                                let what = format!("clean zone {:?}", z.to_quad());
                                self.confirm = Some(Confirm::Control { cmd, what, zone: Some(z) });
                            }
                            Err(e) => self.log.push(format!("cannot clean that zone: {e:#}")),
                        }
                    }
                }
            }
        }
        if self.tool == Tool::Split {
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.drag_from = self.cursor;
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let (Some(a), Some(b)) = (self.drag_from.take(), self.cursor) {
                    let selected = match self.selection {
                        Some(Selection::Room(id)) => Some(id),
                        _ => None,
                    };
                    match state::split_edit(map, a, b, self.snap_mm, selected) {
                        Some(e) => self.pending = Some(e),
                        None => self.log.push("the split line crosses no room".into()),
                    }
                }
            }
        }
        if let Tool::MoveBoundary { a, b } = self.tool {
            // A drag draws a straight line; clicks add the points of a bent one, and a
            // double-click ends it.
            if response.drag_started_by(egui::PointerButton::Primary) && self.border_points.is_empty() {
                self.drag_from = self.cursor;
            }
            let mut line = None;
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let (Some(p), Some(q)) = (self.drag_from.take(), self.cursor) {
                    line = Some(vec![p, q]);
                }
            }
            if response.clicked() || response.double_clicked() {
                self.border_points.extend(self.cursor);
            }
            if response.double_clicked() {
                line = Some(std::mem::take(&mut self.border_points));
            }
            // Fields one by one: `map` borrows `self.loaded`.
            match line.map(|line| state::BorderMove::new(map, a, b, &line, self.snap_mm)) {
                Some(Ok(m)) => {
                    self.border = Some(m);
                    self.paint = None;
                    self.tool = Tool::Select;
                }
                Some(Err(e)) => self.log.push(format!("boundary not moved: {e}")),
                None => {}
            }
        }
        if let Tool::Paint { room } = self.tool {
            // A brush paints while dragging; a box paints its cells when the drag ends.
            let mut cells = None;
            if self.paint_box {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.drag_from = self.cursor;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.drag_from.take(), self.cursor) {
                        cells = Some(state::box_cells(map, room, a, b));
                    }
                }
            } else if response.dragged_by(egui::PointerButton::Primary) || response.clicked() {
                cells = self.cursor.map(|p| state::brush_cells(map, room, p, self.brush));
            }
            if let Some(cells) = cells {
                let staged = self.paint.get_or_insert_with(|| PaintCells { room, ..Default::default() });
                if staged.room != room {
                    *staged = PaintCells { room, ..Default::default() };
                }
                if self.erase {
                    for c in &cells {
                        staged.cells.remove(c);
                    }
                } else {
                    staged.cells.extend(cells);
                }
            }
        }
        if self.tool == Tool::PlaceFurniture && response.clicked() {
            if let Some(p) = self.cursor {
                let c = Point::new(view::snap(p.x, self.snap_mm), view::snap(p.y, self.snap_mm));
                self.pending = Some(Edit::AddFurniture(Box::new(mapedit::new_furniture(self.place_kind, c, 600.0, 600.0))));
            }
        }
        if self.tool == Tool::Select && response.clicked() {
            if let Some(p) = self.cursor {
                let tolerance = 6.0 / self.view.zoom * tr.grid_size;
                self.selection = state::hit(map, p, tolerance);
                if let Some(Selection::Furniture(i)) = self.selection {
                    self.furniture_form = map.furniture.get(i).cloned();
                }
                if let Some(Selection::Room(id)) = self.selection {
                    let room = map.rooms.get(&id);
                    self.rename_to = room.and_then(|r| r.name.clone()).unwrap_or_default();
                    self.settings_form = room
                        .and_then(|r| r.settings.as_deref())
                        .and_then(|s| RoomSettings::from_cleanset(id, s));
                }
            }
        }

        // Drawing.
        let view = self.view;
        let to_screen = |p: Point| {
            let v = view.world_to_screen(&tr, p);
            Pos2::new(origin.x + v.x as f32, origin.y + v.y as f32)
        };
        let painter = painter.with_clip_rect(rect);
        if let Some(tex) = &self.texture {
            let a = view.cell_to_screen(V2::new(0.0, 0.0));
            let b = view.cell_to_screen(V2::new(map.grid.width as f64, map.grid.height as f64));
            let r = ERect::from_min_max(
                Pos2::new(origin.x + a.x as f32, origin.y + a.y as f32),
                Pos2::new(origin.x + b.x as f32, origin.y + b.y as f32),
            );
            painter.image(tex.id(), r, ERect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let zone_rect = |z: &dreame_map::model::Rect| ERect::from_two_pos(to_screen(Point::new(z.x0, z.y0)), to_screen(Point::new(z.x1, z.y1)));
        let selected = |k: ZoneKind, i: usize| self.selection == Some(Selection::Zone(k, i));
        // One shape in a fill and stroke colour. Ellipses are drawn as polygons in world space,
        // so the map's flipped y axis applies to them too.
        let paint = |s: &state::Shape, fill: Color32, stroke: Stroke| match s {
            state::Shape::Rect(z) => {
                painter.rect(zone_rect(z), 0.0, fill, stroke, egui::StrokeKind::Inside);
            }
            state::Shape::Line(w) => {
                painter.line_segment([to_screen(w.a), to_screen(w.b)], stroke);
            }
            state::Shape::Ellipse { center, size } => {
                let pts = (0..36)
                    .map(|k| {
                        let t = f64::from(k) * std::f64::consts::TAU / 36.0;
                        to_screen(Point::new(center.x + size.0 / 2.0 * t.cos(), center.y + size.1 / 2.0 * t.sin()))
                    })
                    .collect();
                painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
            }
        };
        // Furniture outlines, under the zones.
        for (i, f) in map.furniture.iter().enumerate() {
            let pts: Vec<Pos2> = f.corners().iter().map(|c| to_screen(*c)).collect();
            let sel = self.selection == Some(Selection::Furniture(i));
            let stroke = if sel { Stroke::new(3.0, Color32::YELLOW) } else { Stroke::new(1.5, Color32::from_rgb(190, 140, 90)) };
            painter.add(egui::Shape::convex_polygon(pts, Color32::from_rgba_unmultiplied(190, 140, 90, 40), stroke));
        }
        // Areas first, carpets lowest; lines on top.
        for kind in ZoneKind::ALL.iter().rev() {
            let (rgb, width) = zone_style(*kind);
            let [r, g, b] = rgb;
            for (i, s) in kind.shapes(map).iter().enumerate() {
                let stroke = if selected(*kind, i) {
                    Stroke::new(width + 1.5, Color32::YELLOW)
                } else {
                    Stroke::new(width, Color32::from_rgb(r, g, b))
                };
                paint(s, Color32::from_rgba_unmultiplied(r, g, b, 60), stroke);
            }
        }
        for room in map.rooms.values() {
            let Some(b) = room.bounds else { continue };
            let c = to_screen(Point::new((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0));
            let sel = self.selection == Some(Selection::Room(room.id));
            let color = if sel { Color32::YELLOW } else if room.hidden { Color32::GRAY } else { Color32::WHITE };
            painter.text(c, egui::Align2::CENTER_CENTER, room.display_name(), egui::FontId::proportional(13.0), color);
        }
        // The robot's path: one polyline per connected stroke. While following, the live path
        // replaces the one loaded with the map.
        let path = self.live_path.as_ref().filter(|p| !p.points.is_empty()).unwrap_or(&l.path);
        if self.show_path {
            // White, PATH_WIDTH_MM wide at the current zoom, as the app draws it; at least 1.5 px.
            let px_per_mm = (to_screen(Point::new(1000.0, 0.0)).x - to_screen(Point::new(0.0, 0.0)).x).abs() / 1000.0;
            let stroke = Stroke::new((PATH_WIDTH_MM * px_per_mm).max(1.5), Color32::from_rgba_unmultiplied(255, 255, 255, 230));
            let mut line: Vec<Pos2> = Vec::new();
            for p in &path.points {
                let s = to_screen(Point::new(p.x as f64, p.y as f64));
                if !p.connected && line.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(&mut line), stroke));
                }
                if !p.connected {
                    line.clear();
                }
                line.push(s);
            }
            if line.len() > 1 {
                painter.add(egui::Shape::line(line, stroke));
            }
        }
        // The robot, while following.
        if let (true, Some(p)) = (self.following, self.live_pose) {
            let c = to_screen(Point::new(p.x, p.y));
            let r = (150.0 / tr.grid_size * self.view.zoom).max(4.0) as f32;
            painter.circle(c, r, Color32::from_rgb(255, 255, 255), Stroke::new(2.0, Color32::BLACK));
            // Heading: the map's +y is up on screen, so screen y is negated.
            let a = p.angle.to_radians();
            let tip = c + Vec2::new(a.cos() as f32, -a.sin() as f32) * r * 1.6;
            painter.line_segment([c, tip], Stroke::new(2.0, Color32::BLACK));
        }
        // The staged shape, and the one being drawn.
        let pending_color = Color32::from_rgb(255, 200, 0);
        let draw_shape = |e: &Edit| match e {
            _ if state::added_shape(e).is_some() => {
                // Carpets snap to cell corners when sent; show the snapped rectangle.
                let shape = match e {
                    Edit::AddCarpet(z) | Edit::AddIgnoredCarpet(z) => state::Shape::Rect(map.snap_to_cells(z)),
                    Edit::AddRoundCarpet(z) => {
                        let c = Point::new((z.x0 + z.x1) / 2.0, (z.y0 + z.y1) / 2.0);
                        let (center, size) = map.snap_ellipse(c, (z.width(), z.height()));
                        state::Shape::Ellipse { center, size }
                    }
                    _ => state::added_shape(e).expect("checked by the guard"),
                };
                paint(&shape, Color32::from_rgba_unmultiplied(255, 200, 0, 50), Stroke::new(2.5, pending_color));
            }
            Edit::SetFurniture(_, f) | Edit::AddFurniture(f) => {
                let pts: Vec<Pos2> = f.corners().iter().map(|c| to_screen(*c)).collect();
                painter.add(egui::Shape::convex_polygon(pts, Color32::from_rgba_unmultiplied(255, 200, 0, 50), Stroke::new(2.5, pending_color)));
            }
            Edit::SplitRoom { line, .. } => {
                // Red when the line is invalid. When it is valid, a thin line shows what was
                // drawn and a thick orange one the part that is sent: trimmed to the room's edge.
                let sent = mapedit::steps(map, e).ok().and_then(|s| {
                    let q = s.iter().find_map(|(_, c)| c.value()["dsrid"].as_array().cloned())?;
                    let v: Vec<f64> = q.iter().filter_map(serde_json::Value::as_f64).collect();
                    Some((Point::new(v[0], v[1]), Point::new(v[2], v[3])))
                });
                match sent {
                    Some((a, b)) => {
                        painter.line_segment([to_screen(line.a), to_screen(line.b)], Stroke::new(1.0, pending_color));
                        painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(3.5, pending_color));
                    }
                    None => {
                        painter.line_segment([to_screen(line.a), to_screen(line.b)], Stroke::new(3.0, Color32::RED));
                    }
                }
            }
            _ => {}
        };
        if let Some(e) = &self.pending {
            draw_shape(e);
        }
        if let (Tool::Draw(kind), Some(a), Some(b)) = (self.tool, self.drag_from, self.cursor) {
            if let Some(e) = state::draw_edit(kind, a, b, self.snap_mm) {
                draw_shape(&e);
            }
        }
        if let (Tool::MoveBoundary { .. }, Some(p), Some(q)) = (self.tool, self.drag_from, self.cursor) {
            painter.line_segment([to_screen(p), to_screen(q)], Stroke::new(2.5, pending_color));
        }
        if let Tool::MoveBoundary { .. } = self.tool {
            let pts: Vec<Pos2> = self.border_points.iter().chain(self.cursor.iter().filter(|_| !self.border_points.is_empty())).map(|p| to_screen(*p)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, pending_color)));
        }
        if let (Tool::Split, Some(a), Some(b)) = (self.tool, self.drag_from, self.cursor) {
            let selected = match self.selection {
                Some(Selection::Room(id)) => Some(id),
                _ => None,
            };
            match state::split_edit(map, a, b, self.snap_mm, selected) {
                Some(e) => draw_shape(&e),
                None => {
                    painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(2.0, Color32::RED));
                }
            }
        }
        // Painted cells, and the brush under the cursor.
        let cell_rect = |c: f64, r: f64, n: f64| {
            let (x0, y0) = tr.grid_to_world(c, r);
            let (x1, y1) = tr.grid_to_world(c + n, r + n);
            ERect::from_two_pos(to_screen(Point::new(x0, y0)), to_screen(Point::new(x1, y1)))
        };
        // Painted cells take the target room's colour, as the map will show them, with an
        // orange tint so they stay visible as staged.
        let theme = dreame_map::render::Theme::default();
        let colors = dreame_map::render::room_colors(map, theme.rooms.len());
        let fill_cells = |cells: &mut dyn Iterator<Item = &(usize, usize)>, room: u32| {
            let [r, g, b, _] = colors.get(&room).and_then(|&i| theme.rooms.get(i).copied()).unwrap_or([255, 140, 0, 255]);
            let fill = Color32::from_rgb(r, g, b);
            for &(c, row) in cells {
                let cell = cell_rect(c as f64, row as f64, 1.0);
                painter.rect_filled(cell, 0.0, fill);
                painter.rect_filled(cell, 0.0, Color32::from_rgba_unmultiplied(255, 140, 0, 60));
            }
        };
        if let Some(p) = &self.paint {
            fill_cells(&mut p.cells.iter(), p.room);
        }
        if let Some(m) = &self.border {
            fill_cells(&mut m.cells.to_a.iter(), m.a);
            fill_cells(&mut m.cells.to_b.iter(), m.b);
            painter.add(egui::Shape::line(m.line.iter().map(|p| to_screen(*p)).collect(), Stroke::new(2.5, pending_color)));
        }
        if let (Tool::Paint { .. }, Some(p)) = (self.tool, self.cursor) {
            let color = if self.erase { Color32::from_rgb(220, 50, 50) } else { Color32::from_rgb(255, 140, 0) };
            if !self.paint_box {
                if let Some((c, r)) = tr.cell_at(p.x, p.y) {
                    let k = self.brush as f64;
                    let b = cell_rect(c as f64 - k, r as f64 - k, 2.0 * k + 1.0);
                    painter.rect_stroke(b, 0.0, Stroke::new(1.5, color), egui::StrokeKind::Inside);
                }
            } else if let Some(a) = self.drag_from {
                let b = ERect::from_two_pos(to_screen(a), to_screen(p));
                painter.rect_stroke(b, 0.0, Stroke::new(1.5, color), egui::StrokeKind::Inside);
            }
        }
        // A clean zone being drawn or waiting for confirmation, in green.
        let clean_zone = match (&self.confirm, self.tool, self.drag_from, self.cursor) {
            (Some(Confirm::Control { zone: Some(z), .. }), _, _, _) => Some(*z),
            (_, Tool::CleanZone, Some(a), Some(b)) => state::drawn_rect(a, b, self.snap_mm),
            _ => None,
        };
        if let Some(z) = clean_zone {
            let green = Color32::from_rgb(60, 200, 90);
            painter.rect(zone_rect(&z), 0.0, Color32::from_rgba_unmultiplied(60, 200, 90, 50), Stroke::new(2.0, green), egui::StrokeKind::Inside);
        }
        if let Some(p) = self.cursor {
            let text = format!("x {:.0}  y {:.0} mm", p.x, p.y);
            painter.text(rect.left_bottom() + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::monospace(12.0), Color32::LIGHT_GRAY);
        }
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_worker(&ctx);
        self.handle_escape(&ctx);
        egui::Panel::left("side").resizable(true).default_size(320.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui));
        });
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
        self.confirm_window(&ctx);
    }
}

/// The colour and stroke width of a map shape kind.
fn zone_style(kind: ZoneKind) -> ([u8; 3], f32) {
    match kind {
        ZoneKind::NoGo => ([220, 40, 40], 1.5),
        ZoneKind::NoMop => ([40, 120, 220], 1.5),
        ZoneKind::Carpet => ([150, 90, 200], 1.5),
        ZoneKind::IgnoredCarpet => ([140, 140, 140], 1.5),
        ZoneKind::Wall => ([220, 40, 40], 2.5),
        ZoneKind::Threshold => ([240, 150, 40], 2.5),
        ZoneKind::ImpassableThreshold => ([200, 60, 160], 2.5),
        ZoneKind::Curtain => ([60, 190, 120], 2.5),
        ZoneKind::Ramp => ([120, 200, 220], 1.5),
        ZoneKind::RoundCarpet => ([150, 90, 200], 1.5),
    }
}
