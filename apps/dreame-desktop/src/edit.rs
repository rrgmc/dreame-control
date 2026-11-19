//! The map editing screen. The user picks a tool, works on the map, sees the change outlined,
//! and saves it to the robot. One change is saved at a time, as in the official app.
//!
//! Standard tools: rename, divide and merge rooms; add no-go zones, no-mop zones, virtual
//! walls, carpets, "not a carpet" areas, thresholds and curtains; delete one by clicking it.
//! With advanced editing on (Settings), five tools work through a backup and restore of the
//! map: "Move a border" redraws the border between two rooms along a straight or bent line,
//! "Paint into a room" gives a room the floor of other rooms, "New room from a room" paints a
//! new room out of a room, "Delete a room" takes a room and its floor off the map, and "Erase
//! floor" takes the floor in a rectangle off the map. The last two are for floor that does not
//! exist; merging joins a real room to its neighbour. Hidden rooms can be deleted too. The same
//! setting adds "Map backup": back up the map to a file, and restore it from one.

use crate::backend::{Backend, LoadedMap, Msg, SharedRobot};
use crate::mapview::{CellMark, Drag, Mark, MapInput, MapView, Overlay};
use dreame::archiveedit::GridEdit;
use dreame::mapedit::{self, Edit};
use dreame::shapes::{self, Selection, Shape, ZoneKind};
use dreame_i18n::{tr, trf, trn};
use dreame_map::model::{Line, Point};
use dreame_map::segmap::BorderCells;
use dreame_map::path::Path;
use dreame_map::Map;
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;

/// Grid of the robot's map, in mm; drawn shapes snap to it.
const SNAP_MM: f64 = 50.0;
/// How close a click must be to a line to pick it, in screen points.
const HIT_PX: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Click a room or a zone to see what it offers.
    Select,
    Divide,
    Merge,
    Draw(ZoneKind),
    /// Advanced: redraw the border of two neighbour rooms.
    Border,
    /// Advanced: give a room the floor of other rooms.
    Paint,
    /// Advanced: paint a new room out of a room.
    NewRoom,
    /// Advanced: delete a room, giving its floor to a neighbour.
    Remove,
    /// Advanced: make the floor in a dragged rectangle outside the map.
    Erase,
}

impl Tool {
    fn hint(self) -> &'static str {
        match self {
            Tool::Select => tr("Click a room to rename, divide or merge it. Click a zone or wall to delete it."),
            Tool::Divide => tr("Click the room to divide, then drag a line across it, from wall to wall."),
            Tool::Merge => tr("Click two rooms that touch each other."),
            Tool::Draw(ZoneKind::NoGo) => tr("Drag a rectangle. The robot will not enter it."),
            Tool::Draw(ZoneKind::NoMop) => tr("Drag a rectangle. The robot will vacuum but not mop there."),
            Tool::Draw(ZoneKind::Carpet) => tr("Drag a rectangle over a carpet the robot did not find."),
            Tool::Draw(ZoneKind::RoundCarpet) => tr("Drag a rectangle around a round carpet the robot did not find."),
            Tool::Draw(ZoneKind::IgnoredCarpet) => {
                tr("Drag a rectangle over something the robot takes for a carpet, such as a dark rug or mat. It will treat it as floor.")
            }
            Tool::Draw(ZoneKind::Threshold) => tr("Drag a line along a door sill. The robot will cross it with care."),
            Tool::Draw(ZoneKind::ImpassableThreshold) => tr("Drag a line along a door sill the robot cannot climb. It will not try."),
            Tool::Draw(ZoneKind::Curtain) => tr("Drag a line along a curtain. The robot will pass under it with care."),
            Tool::Draw(_) => tr("Drag a line. The robot will not cross it."),
            Tool::Border => tr(
                "Click two rooms that touch. Then drag the new border across both, or click its \
                 points and double-click the last one for a bent border. Room names and settings \
                 stay. Saving takes a few minutes.",
            ),
            Tool::Paint => tr(
                "Click the room that gets the floor, then paint over the floor of other rooms. \
                 Room names and settings stay. Saving takes a few minutes.",
            ),
            Tool::NewRoom => tr(
                "Click the room to take the new room from, then paint the new room's floor. It \
                 gets that room's type, floor and cleaning settings. Saving takes a few minutes. \
                 This is new: the robot may join the rooms again when it next maps.",
            ),
            Tool::Remove => tr(
                "Click a room, hidden or not, whose floor does not exist. The room and its floor \
                 leave the map. To join a real room to its neighbour, merge the two rooms instead. \
                 Saving takes a few minutes. This is new: the robot may map the floor again if it \
                 still sees floor there.",
            ),
            Tool::Erase => tr(
                "Drag a rectangle over floor that does not exist. The floor leaves the map, and a \
                 room left without floor is deleted. Saving takes a few minutes. This is new: the \
                 robot may map the area again if it still sees floor there.",
            ),
        }
    }

    fn drag(self) -> Drag {
        match self {
            Tool::Draw(k) if k.is_line() => Drag::Line,
            Tool::Draw(_) | Tool::Erase | Tool::Paint | Tool::NewRoom => Drag::Rect,
            Tool::Divide | Tool::Border => Drag::Line,
            Tool::Select | Tool::Merge | Tool::Remove => Drag::Pan,
        }
    }

    fn paints(self) -> bool {
        matches!(self, Tool::Paint | Tool::NewRoom)
    }
}

/// A change ready to save.
enum Staged {
    Map(Edit),
    /// A border move: rooms `a` and `b`, the line, straight or bent, and the cells it moves.
    Border { a: u32, b: u32, line: Vec<Point>, cells: BorderCells },
    /// An erase, an archive edit too.
    Erase(shapes::Erase),
    /// A paint, an archive edit too.
    Paint(shapes::Paint),
}

pub enum EditExit {
    Stay,
    Back,
}

pub struct EditMap {
    robot: SharedRobot,
    map: Map,
    advanced: bool,
    view: MapView,
    tool: Tool,
    /// A room clicked: the one to divide, the first to merge or the first of a border.
    room: Option<u32>,
    selected: Option<Selection>,
    staged: Option<Staged>,
    /// Why the drawn change cannot be saved.
    problem: Option<String>,
    /// The rename dialog: room and the name typed.
    rename: Option<(u32, String)>,
    /// A hidden area clicked: ask whether to show it again, as the official app does.
    unhide: Option<u32>,
    /// A backup file picked to restore: ask before replacing the robot's map with it.
    restore: Option<PathBuf>,
    /// The running job is a backup to a file, not a save.
    backing_up: bool,
    saving: bool,
    progress: Vec<String>,
    /// The last save's outcome, shown until the next change.
    outcome: Option<(String, bool)>,
    /// The points of a bent border clicked so far.
    border_points: Vec<Point>,
    /// The paint tools draw rectangles instead of brushing.
    paint_rect: bool,
    /// The brush's reach around the cell under the pointer, in cells.
    brush: usize,
    /// The paint tools take cells back out of the paint instead of adding them.
    eraser: bool,
    /// The name typed for a new room.
    new_room_name: String,
    /// The robot's path from the last job, shown when `show_path` is on.
    path: Path,
    show_path: bool,
}

impl EditMap {
    pub fn new(robot: SharedRobot, map: Map, path: Path, advanced: bool) -> Self {
        let mut view = MapView::default();
        view.set_map(&map);
        Self {
            robot,
            map,
            advanced,
            view,
            tool: Tool::Select,
            room: None,
            selected: None,
            staged: None,
            problem: None,
            rename: None,
            unhide: None,
            restore: None,
            backing_up: false,
            saving: false,
            progress: Vec::new(),
            outcome: None,
            border_points: Vec::new(),
            paint_rect: false,
            brush: 1,
            eraser: false,
            new_room_name: String::new(),
            path,
            show_path: false,
        }
    }

    /// Takes the outcome of a save. Returns the new map for the home screen.
    pub fn handle(&mut self, msg: Msg) -> Option<Box<LoadedMap>> {
        match msg {
            Msg::Progress(line) => {
                self.progress.push(line);
                None
            }
            Msg::BackedUp(result) => {
                self.saving = false;
                self.backing_up = false;
                self.outcome = Some(match result {
                    Ok(file) => (trf("The map was backed up to {file}.", &[("file", &file.display())]), false),
                    Err(e) => (trf("Not backed up: {error}", &[("error", &e)]), true),
                });
                None
            }
            Msg::Edited { before, result } => {
                self.saving = false;
                match result {
                    Ok(loaded) => {
                        self.set_map(loaded.map.clone());
                        self.outcome = Some((tr("Saved to the robot.").into(), false));
                        // A divided room's new half has no name yet: ask for one.
                        if let Some(id) = shapes::unnamed_new_room(&before, &self.map) {
                            self.rename = Some((id, String::new()));
                        }
                        Some(loaded)
                    }
                    Err(e) => {
                        self.outcome = Some((trf("Not saved: {error}", &[("error", &e)]), true));
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn set_map(&mut self, map: Map) {
        self.view.set_map(&map);
        self.map = map;
        self.staged = None;
        self.selected = None;
        self.room = None;
        self.problem = None;
        self.border_points.clear();
        self.new_room_name.clear();
    }

    fn reset(&mut self) {
        self.room = None;
        self.selected = None;
        self.staged = None;
        self.problem = None;
        self.outcome = None;
        self.border_points.clear();
        self.new_room_name.clear();
    }

    /// What a primary-button drag on the map does now.
    fn drag(&self) -> Drag {
        match self.tool {
            t if t.paints() && self.room.is_some() && !self.paint_rect => Drag::Brush(self.brush),
            // Once the points of a bent border are being clicked, a drag pans.
            Tool::Border if !self.border_points.is_empty() => Drag::Pan,
            t => t.drag(),
        }
    }

    /// Esc undoes one step: the drag in progress, then the change drawn, then the rooms or
    /// zone picked, then the tool, back to Select.
    fn escape(&mut self) {
        if self.view.cancel_drag() {
            return;
        }
        if !self.border_points.is_empty() {
            self.border_points.clear();
            self.problem = None;
        } else if self.staged.is_some() || self.problem.is_some() {
            self.staged = None;
            self.problem = None;
            self.outcome = None;
            // The room picked for a deletion is only the deletion itself.
            if self.tool == Tool::Remove {
                self.room = None;
            }
        } else if self.room.is_some() || self.selected.is_some() {
            self.reset();
        } else if self.tool != Tool::Select {
            self.tool = Tool::Select;
            self.reset();
        } else {
            self.outcome = None;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, backend: &Backend, can_edit: bool) -> EditExit {
        let mut exit = EditExit::Stay;
        // An open dialog takes Esc itself and closes.
        let dialog = self.unhide.is_some() || self.restore.is_some() || self.rename.is_some();
        if !self.saving && !dialog && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.escape();
        }
        egui::Panel::top("edit-top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.saving, egui::Button::new(RichText::new(format!("← {}", tr("Back to the map"))).size(15.0))).clicked() {
                    exit = EditExit::Back;
                }
                ui.add_space(12.0);
                ui.label(RichText::new(tr("Edit map")).size(18.0).strong());
                if self.advanced {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(!self.saving && can_edit, |ui| self.backup_buttons(ui, backend));
                    });
                }
            });
            ui.add_space(2.0);
        });

        egui::Panel::left("tools").resizable(false).exact_size(230.0).show(ui, |ui| {
            // What the tool and the selection offer stays in view below the list of tools.
            egui::Panel::bottom("tool-info").show(ui, |ui| {
                ui.add_enabled_ui(!self.saving && can_edit, |ui| self.tool_info(ui));
                if !can_edit {
                    ui.add_space(8.0);
                    ui.colored_label(ui.visuals().warn_fg_color, tr("The map can be edited while the robot is not cleaning."));
                }
                ui.add_space(8.0);
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!self.saving && can_edit, |ui| self.toolbox(ui));
            });
        });

        egui::Panel::bottom("edit-bottom").show(ui, |ui| {
            ui.add_space(8.0);
            self.save_bar(ui, backend, can_edit);
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let coloured: Vec<u32> = match (self.tool, self.room, &self.staged) {
                (_, _, Some(Staged::Map(Edit::MergeRooms(a, b)))) => vec![*a, *b],
                (_, _, Some(Staged::Erase(e))) => e.cells.keys().copied().collect(),
                (_, _, Some(Staged::Paint(p))) => std::iter::once(p.room).chain(p.sources(&self.map).into_keys()).collect(),
                (Tool::Paint | Tool::NewRoom, Some(r), _) => vec![r],
                (_, _, Some(Staged::Border { a, b, .. })) => vec![*a, *b],
                (Tool::Border, Some(a), _) => match self.selected {
                    Some(Selection::Room(b)) => vec![a, b],
                    _ => vec![a],
                },
                (Tool::Divide | Tool::Merge, Some(r), _) => vec![r],
                _ => match self.selected {
                    Some(Selection::Room(r)) => vec![r],
                    _ => Vec::new(),
                },
            };
            let marks = self.marks();
            let cells = match &self.staged {
                Some(Staged::Paint(p)) => vec![CellMark { cells: p.cells.iter().copied().collect(), room: p.parent.unwrap_or(p.room) }],
                Some(Staged::Border { a, b, cells, .. }) => vec![
                    CellMark { cells: cells.to_a.clone(), room: *a },
                    CellMark { cells: cells.to_b.clone(), room: *b },
                ],
                _ => Vec::new(),
            };
            let no_path = Path::default();
            let overlay = Overlay {
                path: if self.show_path { &self.path } else { &no_path },
                pose: None,
                picked: &[],
                zone: None,
                drag: if self.saving || !can_edit { Drag::Pan } else { self.drag() },
                marks,
                cells,
                hidden_labels: true,
            };
            let input = self.view.show(ui, &self.map, &coloured, &overlay);
            if !self.saving && can_edit {
                let tolerance = HIT_PX * self.view.mm_per_point(&self.map);
                self.input(input, tolerance);
            }
        });

        if let Some(id) = self.unhide {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::unique("unhide")).show(ui.ctx(), |ui| {
                ui.set_width(300.0);
                ui.label(RichText::new(tr("Hidden area")).size(17.0).strong());
                ui.add_space(6.0);
                ui.label(trf("{room} is hidden, so the robot does not clean it. Show it again?", &[("room", &self.room_name(id))]));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let b = egui::Button::new(RichText::new(tr("Show it")).color(Color32::WHITE)).fill(crate::ACCENT);
                    if ui.add(b).clicked() {
                        let ids = shapes::toggled_hidden(&self.map, id, false);
                        self.save(backend, Staged::Map(Edit::HideRooms(ids)));
                        close = true;
                    }
                    if self.advanced && ui.button(tr("Delete it")).clicked() {
                        self.start_remove(id);
                        close = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.unhide = None;
            }
        }
        if let Some(file) = self.restore.clone() {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::unique("restore")).show(ui.ctx(), |ui| {
                ui.set_width(360.0);
                ui.label(RichText::new(tr("Restore the map")).size(17.0).strong());
                ui.add_space(6.0);
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(trf(
                    "This replaces the robot's map with the one in {file}: rooms, borders, zones and walls. \
                     Rooms that are in both keep their cleaning settings. It takes a few minutes.",
                    &[("file", &name)],
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let b = egui::Button::new(RichText::new(tr("Restore")).color(Color32::WHITE)).fill(crate::ACCENT);
                    if ui.add(b).clicked() {
                        self.saving = true;
                        self.progress.clear();
                        self.reset();
                        backend.restore_from_file(&self.robot, self.map.clone(), file.clone());
                        close = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.restore = None;
            }
        }
        if let Some((id, mut name)) = self.rename.take() {
            let mut keep = true;
            let modal = egui::Modal::new(egui::Id::unique("rename")).show(ui.ctx(), |ui| {
                ui.set_width(300.0);
                let old = self.map.rooms.get(&id).map(|r| r.display_name()).unwrap_or_default();
                let title = if old.is_empty() { tr("Name the new room").to_string() } else { trf("Rename {room}", &[("room", &old)]) };
                ui.label(RichText::new(title).size(17.0).strong());
                ui.add_space(6.0);
                let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0).hint_text(tr("Room name")));
                field.request_focus();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let ok = !name.trim().is_empty() && name.trim().chars().count() <= 24;
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add_enabled(ok, egui::Button::new(tr("Save"))).clicked() || (ok && enter) {
                        self.save(backend, Staged::Map(Edit::RenameRoom { id, name: name.trim().to_string() }));
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
                if name.trim().chars().count() > 24 {
                    ui.colored_label(ui.visuals().error_fg_color, tr("Use at most 24 characters."));
                }
            });
            if keep && !modal.should_close() {
                self.rename = Some((id, name));
            }
        }
        exit
    }

    fn toolbox(&mut self, ui: &mut egui::Ui) {
        let mut tool = self.tool;
        let mut button = |ui: &mut egui::Ui, t: Tool, label: &str| {
            let b = egui::Button::selectable(tool == t, RichText::new(label).size(15.0)).min_size(egui::vec2(210.0, 30.0));
            if ui.add(b).clicked() {
                tool = t;
            }
        };
        ui.add_space(8.0);
        ui.label(RichText::new(tr("Rooms")).strong());
        button(ui, Tool::Select, tr("Select"));
        button(ui, Tool::Divide, tr("Divide a room"));
        button(ui, Tool::Merge, tr("Merge two rooms"));
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Restrictions")).strong());
        button(ui, Tool::Draw(ZoneKind::NoGo), &zone_name(ZoneKind::NoGo));
        button(ui, Tool::Draw(ZoneKind::NoMop), &zone_name(ZoneKind::NoMop));
        button(ui, Tool::Draw(ZoneKind::Wall), &zone_name(ZoneKind::Wall));
        ui.add_space(10.0);
        ui.label(RichText::new(tr("Floor")).strong());
        for kind in [
            ZoneKind::Carpet,
            ZoneKind::RoundCarpet,
            ZoneKind::IgnoredCarpet,
            ZoneKind::Threshold,
            ZoneKind::ImpassableThreshold,
            ZoneKind::Curtain,
        ] {
            button(ui, Tool::Draw(kind), &zone_name(kind));
        }
        if self.advanced {
            ui.add_space(10.0);
            ui.label(RichText::new(tr("Advanced")).strong());
            button(ui, Tool::Border, tr("Move a border"));
            button(ui, Tool::Paint, tr("Paint into a room"));
            button(ui, Tool::NewRoom, tr("New room from a room"));
            button(ui, Tool::Remove, tr("Delete a room"));
            button(ui, Tool::Erase, tr("Erase floor"));
        }
        if tool != self.tool {
            self.tool = tool;
            self.reset();
        }
        ui.add_space(8.0);
    }

    /// The map backup buttons, for a right-to-left layout: the first added is the rightmost.
    fn backup_buttons(&mut self, ui: &mut egui::Ui, backend: &Backend) {
        if ui.button(tr("Restore from a file…")).clicked() {
            if let Some(file) = backup_dialog().pick_file() {
                match std::fs::read(&file).map_err(anyhow::Error::from).and_then(|b| dreame::archiveedit::check_archive(&b)) {
                    Ok(()) => self.restore = Some(file),
                    Err(e) => self.outcome = Some((trf("This file cannot be restored: {error}", &[("error", &format!("{e:#}"))]), true)),
                }
            }
        }
        if ui.button(tr("Back up to a file…")).clicked() {
            let name = format!("dreame-map-{}.tbz2", chrono::Local::now().format("%Y-%m-%d-%H%M"));
            if let Some(file) = backup_dialog().set_file_name(name).save_file() {
                self.reset();
                self.saving = true;
                self.backing_up = true;
                self.progress.clear();
                backend.backup_to_file(&self.robot, file);
            }
        }
    }

    /// The current tool's hint and options, and what the selection offers.
    fn tool_info(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new(self.tool.hint()).weak());
        if self.tool.paints() {
            self.paint_options(ui);
        }
        if self.tool == Tool::Border && !self.border_points.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(trn("{n} point", "{n} points", self.border_points.len() as i64, &[]));
                if ui.add_enabled(self.border_points.len() >= 2, egui::Button::new(tr("Finish the line"))).clicked() {
                    self.finish_border();
                }
            });
        }
        if !self.path.points.is_empty() {
            ui.add_space(10.0);
            ui.checkbox(&mut self.show_path, tr("Show the robot's last path"));
        }

        // What the selection offers.
        if self.selected.is_some() {
            ui.add_space(14.0);
        }
        match self.selected {
            Some(Selection::Room(id)) => {
                let name = self.room_name(id);
                ui.label(RichText::new(&name).size(16.0).strong());
                if ui.button(tr("Rename")).clicked() {
                    self.rename = Some((id, self.map.rooms.get(&id).and_then(|r| r.name.clone()).unwrap_or(name)));
                }
                if ui.button(tr("Divide")).clicked() {
                    self.tool = Tool::Divide;
                    self.reset();
                    self.room = Some(id);
                }
                if ui.button(tr("Merge with a neighbour")).clicked() {
                    self.tool = Tool::Merge;
                    self.reset();
                    self.room = Some(id);
                }
                if self.advanced && ui.button(tr("Paint into this room")).clicked() {
                    self.tool = Tool::Paint;
                    self.reset();
                    self.room = Some(id);
                }
                if self.advanced && ui.button(tr("Make a new room from it")).clicked() {
                    self.tool = Tool::NewRoom;
                    self.reset();
                    self.room = Some(id);
                }
                if self.advanced && ui.button(tr("Delete this room")).clicked() {
                    self.start_remove(id);
                }
            }
            Some(Selection::Zone(kind, i)) => {
                ui.label(RichText::new(zone_name(kind)).size(16.0).strong());
                if ui.button(delete_label(kind)).clicked() {
                    self.stage(Staged::Map(shapes::delete_edit(Selection::Zone(kind, i)).expect("zones can be deleted")));
                }
            }
            _ => {}
        }
    }

    fn input(&mut self, input: MapInput, tolerance: f64) {
        match (self.tool, input) {
            (_, MapInput::None) => {}
            (Tool::Select, MapInput::Click(p)) => {
                self.staged = None;
                self.problem = None;
                self.outcome = None;
                // Zones and lines can be picked; furniture cannot.
                self.selected = match shapes::hit(&self.map, p, tolerance) {
                    Some(Selection::Zone(k, i)) => Some(Selection::Zone(k, i)),
                    _ => match self.map.room_at(p) {
                        Some(r) if !self.visible(r) => {
                            self.unhide = Some(r);
                            None
                        }
                        r => r.map(Selection::Room),
                    },
                };
            }
            (Tool::Divide, MapInput::Click(p)) => {
                self.room = self.map.room_at(p).filter(|r| self.visible(*r));
                self.staged = None;
            }
            (Tool::Divide, MapInput::Line(a, b)) => match shapes::split_edit(&self.map, a, b, SNAP_MM, self.room) {
                Some(e) => {
                    if let Edit::SplitRoom { id, .. } = e {
                        self.room = Some(id);
                    }
                    self.stage(Staged::Map(e));
                }
                None => self.problem = Some(tr("Drag the line across a room.").into()),
            },
            (Tool::Border, MapInput::Click(p)) if self.room.is_some() && matches!(self.selected, Some(Selection::Room(_))) => {
                // Both rooms are picked: clicks are the points of a bent border.
                self.add_border_point(p);
            }
            (Tool::Border, MapInput::DoubleClick(p)) if self.room.is_some() && matches!(self.selected, Some(Selection::Room(_))) => {
                self.add_border_point(p);
                self.finish_border();
            }
            (Tool::Merge | Tool::Border, MapInput::Click(p) | MapInput::DoubleClick(p)) => {
                // Hidden rooms may be merged too, such as a grey area beside a room.
                let Some(r) = self.map.room_at(p) else { return };
                match self.room {
                    Some(first) if first != r && shapes::merge_partners(&self.map, first).contains(&r) => {
                        if self.tool == Tool::Merge {
                            self.stage(Staged::Map(Edit::MergeRooms(first, r)));
                        } else {
                            self.staged = None;
                            self.problem = None;
                            self.selected = Some(Selection::Room(r));
                        }
                    }
                    Some(first) if first != r => {
                        self.problem = Some(trf(
                            "{room} does not touch {other}. Pick a room next to it.",
                            &[("room", &self.room_name(r)), ("other", &self.room_name(first))],
                        ));
                    }
                    _ => {
                        self.room = Some(r);
                        self.staged = None;
                        self.selected = None;
                        self.problem = None;
                    }
                }
            }
            (Tool::Border, MapInput::Line(a, b)) => {
                self.border_points = vec![a, b];
                self.finish_border();
            }
            (Tool::Remove, MapInput::Click(p)) => {
                // Hidden rooms too: they are often the floor that does not exist.
                if let Some(r) = self.map.room_at(p) {
                    self.start_remove(r);
                }
            }
            (t, MapInput::Click(p)) if t.paints() && self.room.is_none() => {
                // Hidden rooms too, as for a merge.
                if let Some(r) = self.map.room_at(p) {
                    self.room = Some(r);
                    self.staged = None;
                    self.problem = None;
                    self.outcome = None;
                }
            }
            (t, MapInput::Brush(p)) if t.paints() => {
                let cells = shapes::brush_cells(&self.map, p, self.brush);
                self.paint(cells);
            }
            (t, MapInput::Rect(a, b)) if t.paints() => {
                if let Some(rect) = shapes::drawn_rect(a, b, SNAP_MM) {
                    let cells = shapes::rect_cells(&self.map, &rect);
                    self.paint(cells);
                }
            }
            (Tool::Erase, MapInput::Rect(a, b)) => match shapes::erase_edit(&self.map, a, b, SNAP_MM) {
                Some(e) => self.stage(Staged::Erase(e)),
                None => self.problem = Some(tr("There is no floor in that rectangle.").into()),
            },
            (Tool::Draw(kind), MapInput::Line(a, b) | MapInput::Rect(a, b)) => match shapes::draw_edit(kind, a, b, SNAP_MM) {
                Some(e) => self.stage(Staged::Map(e)),
                None => self.problem = Some(tr("That is too small. Drag a bigger shape.").into()),
            },
            _ => {}
        }
    }

    /// Stages a change, after checking that the library can build it.
    fn stage(&mut self, s: Staged) {
        self.outcome = None;
        self.problem = match &s {
            Staged::Map(e) => mapedit::steps(&self.map, e).err().map(|e| friendly_problem(e, &s)),
            Staged::Paint(p) => {
                let emptied = p.emptied(&self.map);
                (!emptied.is_empty()).then(|| {
                    let rooms = emptied.iter().map(|r| self.room_name(*r)).collect::<Vec<_>>().join(", ");
                    trf("This takes all of {rooms}. To join two rooms, merge them instead.", &[("rooms", &rooms)])
                })
            }
            Staged::Border { .. } | Staged::Erase(_) => None,
        };
        self.staged = Some(s);
    }

    fn save(&mut self, backend: &Backend, s: Staged) {
        self.saving = true;
        self.progress.clear();
        self.outcome = None;
        match s {
            Staged::Map(e) => backend.apply_edit(&self.robot, self.map.clone(), e),
            Staged::Border { a, b, line, .. } => match (u8::try_from(a), u8::try_from(b)) {
                (Ok(a), Ok(b)) => {
                    let line = line.iter().map(|p| (p.x, p.y)).collect();
                    backend.apply_grid(&self.robot, self.map.clone(), GridEdit::Border { a, b, line });
                }
                _ => {
                    self.saving = false;
                    self.outcome = Some((tr("These rooms cannot be changed this way.").into(), true));
                }
            },
            Staged::Erase(e) => backend.apply_grid(&self.robot, self.map.clone(), e.edit),
            Staged::Paint(p) => match p.edit(&self.map, Some(&self.new_room_name)) {
                Some(edit) => backend.apply_grid(&self.robot, self.map.clone(), edit),
                None => {
                    self.saving = false;
                    self.outcome = Some((tr("These rooms cannot be changed this way.").into(), true));
                }
            },
        }
    }

    fn save_bar(&mut self, ui: &mut egui::Ui, backend: &Backend, can_edit: bool) {
        if self.saving {
            ui.horizontal(|ui| {
                ui.spinner();
                let text = match (self.backing_up, self.progress.last()) {
                    (true, Some(p)) => trf("Backing up the map: {step}", &[("step", p)]),
                    (true, None) => tr("Backing up the map. This can take a minute.").into(),
                    (false, Some(p)) => trf("Saving to the robot: {step}", &[("step", p)]),
                    (false, None) => tr("Saving to the robot. This can take a minute.").into(),
                };
                ui.label(text);
            });
            return;
        }
        ui.horizontal(|ui| {
            if let Some(s) = &self.staged {
                ui.label(RichText::new(self.describe(s)).size(15.0));
                ui.add_space(12.0);
                let ok = self.problem.is_none() && can_edit;
                let b = egui::Button::new(RichText::new(tr("Save")).size(15.0).color(Color32::WHITE))
                    .fill(crate::ACCENT)
                    .min_size(egui::vec2(100.0, 32.0));
                if ui.add_enabled(ok, b).clicked() {
                    if let Some(s) = self.staged.take() {
                        self.save(backend, s);
                    }
                }
                if ui.add(egui::Button::new(RichText::new(tr("Discard")).size(15.0)).min_size(egui::vec2(90.0, 32.0))).clicked() {
                    self.reset();
                }
            } else if let Some((text, error)) = &self.outcome {
                if *error {
                    ui.colored_label(ui.visuals().error_fg_color, text);
                } else {
                    ui.label(text);
                }
            } else {
                ui.label(RichText::new(tr("Changes are saved one at a time.")).weak());
            }
        });
        if let Some(p) = &self.problem {
            ui.colored_label(ui.visuals().error_fg_color, p);
        }
        if matches!(self.staged, Some(Staged::Border { .. } | Staged::Erase(_) | Staged::Paint(_))) {
            ui.label(
                RichText::new(tr(
                    "This saves through a backup and restore of the robot's map and takes a few minutes. \
                     Your computer may ask whether this app may accept network connections: allow it on \
                     private networks, as the robot fetches the edited map from this computer.",
                ))
                .small()
                .weak(),
            );
        }
    }

    fn marks(&self) -> Vec<Mark> {
        // The map image does not show "not a carpet" areas, so outline them here.
        let mut marks: Vec<Mark> =
            self.map.ignored_carpet.iter().map(|r| Mark { shape: Shape::Rect(*r), colour: Color32::from_gray(150) }).collect();
        if let Some(Selection::Zone(kind, i)) = self.selected {
            if let Some(shape) = kind.shapes(&self.map).get(i) {
                marks.push(Mark { shape: *shape, colour: Color32::from_rgb(230, 120, 30) });
            }
        }
        match &self.staged {
            Some(Staged::Map(Edit::SplitRoom { line, .. })) if self.problem.is_none() => {
                // A grey line for what was drawn, the accent for the part that is sent: trimmed
                // to the room's edge.
                marks.push(Mark { shape: Shape::Line(*line), colour: Color32::from_gray(150) });
                if let Some(sent) = self.sent_split_line() {
                    marks.push(Mark { shape: Shape::Line(sent), colour: crate::ACCENT });
                }
            }
            Some(Staged::Map(e)) => {
                let shape = shapes::added_shape(e).or(match e {
                    Edit::SplitRoom { line, .. } => Some(Shape::Line(*line)),
                    _ => None,
                });
                let colour = if self.problem.is_some() { Color32::from_rgb(210, 50, 50) } else { crate::ACCENT };
                if let Some(shape) = shape {
                    marks.push(Mark { shape, colour });
                }
            }
            Some(Staged::Border { line, .. }) => marks.extend(segments(line)),
            Some(Staged::Erase(e)) => {
                if let Some(rect) = e.rect {
                    marks.push(Mark { shape: Shape::Rect(rect), colour: Color32::from_rgb(210, 50, 50) });
                }
            }
            Some(Staged::Paint(_)) => {}
            None => {}
        }
        // The points of a bent border clicked so far.
        let dot = |p: &Point| Mark { shape: Shape::Ellipse { center: *p, size: (80.0, 80.0) }, colour: crate::ACCENT };
        marks.extend(self.border_points.iter().map(dot));
        marks.extend(segments(&self.border_points));
        marks
    }

    fn describe(&self, s: &Staged) -> String {
        match s {
            Staged::Map(e) => match e {
                Edit::AddNoGo(_) => tr("New no-go zone").into(),
                Edit::AddNoMop(_) => tr("New no-mop zone").into(),
                Edit::AddWall(_) => tr("New virtual wall").into(),
                Edit::RemoveNoGo(_) => tr("Delete the no-go zone").into(),
                Edit::RemoveNoMop(_) => tr("Delete the no-mop zone").into(),
                Edit::RemoveWall(_) => tr("Delete the virtual wall").into(),
                Edit::SplitRoom { id, .. } => trf("Divide {room}", &[("room", &self.room_name(*id))]),
                Edit::MergeRooms(a, b) => trf("Merge {room} and {other}", &[("room", &self.room_name(*a)), ("other", &self.room_name(*b))]),
                Edit::RenameRoom { id, name } => trf("Rename {room} to {name}", &[("room", &self.room_name(*id)), ("name", name)]),
                Edit::HideRooms(_) => tr("Show the hidden area again").into(),
                Edit::AddCarpet(_) => tr("New carpet").into(),
                Edit::AddRoundCarpet(_) => tr("New round carpet").into(),
                Edit::AddIgnoredCarpet(_) => tr("New \"not a carpet\" area").into(),
                Edit::AddThreshold(_) => tr("New threshold").into(),
                Edit::AddImpassableThreshold(_) => tr("New threshold the robot does not cross").into(),
                Edit::AddCurtain(_) => tr("New curtain").into(),
                Edit::RemoveCarpet(_) => tr("Delete the carpet").into(),
                Edit::RemoveIgnoredCarpet(_) => tr("Delete the \"not a carpet\" area").into(),
                Edit::RemoveThreshold(_) => tr("Delete the threshold").into(),
                Edit::RemoveImpassableThreshold(_) => tr("Delete the threshold the robot does not cross").into(),
                Edit::RemoveCurtain(_) => tr("Delete the curtain").into(),
                Edit::RemoveRamp(_) => tr("Delete the ramp").into(),
                other => format!("{other:?}"),
            },
            Staged::Border { a, b, cells, .. } => trf(
                "New border between {room} and {other}: {moved}",
                &[
                    ("room", &self.room_name(*a)),
                    ("other", &self.room_name(*b)),
                    ("moved", &trn("{n} cell changes room", "{n} cells change room", (cells.to_a.len() + cells.to_b.len()) as i64, &[])),
                ],
            ),
            Staged::Erase(e) if e.rect.is_none() => {
                let rooms = e.emptied.iter().map(|r| self.room_name(*r)).collect::<Vec<_>>().join(", ");
                trf("Delete {room} and its floor", &[("room", &rooms)])
            }
            Staged::Erase(e) => {
                let n: usize = e.cells.values().sum();
                let erase = trn("Erase {n} floor cell", "Erase {n} floor cells", n as i64, &[]);
                if e.emptied.is_empty() {
                    erase
                } else {
                    let rooms = e.emptied.iter().map(|r| self.room_name(*r)).collect::<Vec<_>>().join(", ");
                    trf("{erase}; {rooms} will be deleted", &[("erase", &erase), ("rooms", &rooms)])
                }
            }
            Staged::Paint(p) if p.parent.is_some() => trn(
                "New room from {n} floor cell of {room}",
                "New room from {n} floor cells of {room}",
                p.cells.len() as i64,
                &[("room", &self.room_name(p.parent.unwrap_or_default()))],
            ),
            Staged::Paint(p) => {
                let from = p.sources(&self.map).keys().map(|r| self.room_name(*r)).collect::<Vec<_>>().join(", ");
                trn(
                    "Give {n} floor cell of {from} to {room}",
                    "Give {n} floor cells of {from} to {room}",
                    p.cells.len() as i64,
                    &[("from", &from), ("room", &self.room_name(p.room))],
                )
            }
        }
    }

    /// Adds a clicked point to the bent border, snapped; a point on the last one is skipped.
    fn add_border_point(&mut self, p: Point) {
        let p = Point::new(dreame_map::view::snap(p.x, SNAP_MM), dreame_map::view::snap(p.y, SNAP_MM));
        if self.border_points.last() != Some(&p) {
            self.border_points.push(p);
        }
        self.staged = None;
        self.problem = None;
        self.outcome = None;
    }

    /// Stages the border along the points given so far, or says why it cannot be.
    fn finish_border(&mut self) {
        let line: Vec<Point> = std::mem::take(&mut self.border_points)
            .into_iter()
            .map(|p| Point::new(dreame_map::view::snap(p.x, SNAP_MM), dreame_map::view::snap(p.y, SNAP_MM)))
            .collect();
        let (Some(a), Some(Selection::Room(b))) = (self.room, self.selected) else {
            self.problem = Some(tr("Click the two rooms first.").into());
            return;
        };
        if line.len() < 2 {
            return;
        }
        match dreame_map::edit::border_cells(&self.map, a, b, &line) {
            Ok(cells) => self.stage(Staged::Border { a, b, line, cells }),
            Err(e) => {
                self.problem = Some(trf("Drag the line across both rooms, from one outer wall to another. ({error})", &[("error", &e)]));
            }
        }
    }

    /// Adds `cells` to the staged paint, or with the eraser on takes them out of it.
    fn paint(&mut self, cells: Vec<(usize, usize)>) {
        let Some(room) = self.room else { return };
        let mut paint = match self.staged.take() {
            Some(Staged::Paint(p)) => p,
            _ if self.tool == Tool::NewRoom => match shapes::Paint::new_room(&self.map, room) {
                Some(p) => p,
                None => {
                    self.problem = Some(tr("The map has no room id left for a new room.").into());
                    return;
                }
            },
            _ => shapes::Paint::into_room(room),
        };
        if self.eraser {
            paint.remove(cells);
        } else {
            paint.add(&self.map, cells);
        }
        if paint.cells.is_empty() {
            self.staged = None;
            self.problem = None;
        } else {
            self.stage(Staged::Paint(paint));
        }
    }

    /// The brush or rectangle, the brush size, the eraser, and for a new room its name.
    fn paint_options(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.paint_rect, false, tr("Brush"));
            ui.selectable_value(&mut self.paint_rect, true, tr("Rectangle"));
        });
        if !self.paint_rect {
            ui.horizontal(|ui| {
                ui.label(tr("Size"));
                ui.add(egui::Slider::new(&mut self.brush, 0..=6).show_value(false));
            });
        }
        ui.checkbox(&mut self.eraser, tr("Eraser: take cells back out"));
        if self.tool == Tool::NewRoom && self.room.is_some() {
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.new_room_name).desired_width(200.0).hint_text(tr("New room's name")));
        }
    }

    /// The divide line as it is sent, trimmed to the room's edge, when the staged divide is valid.
    fn sent_split_line(&self) -> Option<Line> {
        let Some(Staged::Map(e @ Edit::SplitRoom { .. })) = &self.staged else { return None };
        let steps = mapedit::steps(&self.map, e).ok()?;
        let q = steps.iter().find_map(|(_, c)| c.value()["dsrid"].as_array().cloned())?;
        let v: Vec<f64> = q.iter().filter_map(serde_json::Value::as_f64).collect();
        (v.len() >= 4).then(|| Line::new(v[0], v[1], v[2], v[3]))
    }

    /// Switches to "Delete a room" and stages the deletion of `id`, hidden or not: its floor
    /// leaves the map.
    fn start_remove(&mut self, id: u32) {
        self.tool = Tool::Remove;
        self.reset();
        self.room = Some(id);
        match shapes::erase_room_edit(&self.map, id) {
            Some(e) => self.stage(Staged::Erase(e)),
            None => self.problem = Some(tr("These rooms cannot be changed this way.").into()),
        }
    }

    fn visible(&self, id: u32) -> bool {
        self.map.rooms.get(&id).is_some_and(|r| !r.hidden)
    }

    fn room_name(&self, id: u32) -> String {
        self.map.rooms.get(&id).map(|r| r.display_name()).unwrap_or_else(|| trf("Room {id}", &[("id", &id)]))
    }
}

/// The segments of a line through `points`, as marks in the accent colour.
fn segments(points: &[Point]) -> impl Iterator<Item = Mark> + '_ {
    points.windows(2).map(|w| Mark { shape: Shape::Line(Line::new(w[0].x, w[0].y, w[1].x, w[1].y)), colour: crate::ACCENT })
}

/// A file dialog for map backups, which are bzip2 tar archives.
fn backup_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().set_title(tr("Map backup")).add_filter(tr("Map backup"), &["tbz2"])
}

/// The name of a kind of zone or line, as a heading.
fn zone_name(kind: ZoneKind) -> String {
    match kind {
        ZoneKind::NoGo => tr("No-go zone").into(),
        ZoneKind::NoMop => tr("No-mop zone").into(),
        ZoneKind::Wall => tr("Virtual wall").into(),
        ZoneKind::Carpet => tr("Carpet").into(),
        ZoneKind::RoundCarpet => tr("Round carpet").into(),
        ZoneKind::IgnoredCarpet => tr("Not a carpet").into(),
        ZoneKind::Threshold => tr("Threshold").into(),
        ZoneKind::ImpassableThreshold => tr("Threshold, do not cross").into(),
        ZoneKind::Curtain => tr("Curtain").into(),
        ZoneKind::Ramp => tr("Ramp").into(),
    }
}

/// The button that deletes the selected zone or line.
fn delete_label(kind: ZoneKind) -> String {
    match kind {
        ZoneKind::NoGo => tr("Delete this no-go zone").into(),
        ZoneKind::NoMop => tr("Delete this no-mop zone").into(),
        ZoneKind::Wall => tr("Delete this virtual wall").into(),
        ZoneKind::Carpet | ZoneKind::RoundCarpet => tr("Delete this carpet").into(),
        ZoneKind::IgnoredCarpet => tr("Delete this \"not a carpet\" area").into(),
        ZoneKind::Threshold | ZoneKind::ImpassableThreshold => tr("Delete this threshold").into(),
        ZoneKind::Curtain => tr("Delete this curtain").into(),
        ZoneKind::Ramp => tr("Delete this ramp").into(),
    }
}

/// Why a staged change cannot be saved, in words a user can act on; the library's reason
/// follows in brackets.
fn friendly_problem(e: anyhow::Error, s: &Staged) -> String {
    let detail = format!("{e:#}");
    match s {
        Staged::Map(Edit::SplitRoom { .. }) => {
            trf("Start and end the line outside the room, so it crosses the room from wall to wall. ({detail})", &[("detail", &detail)])
        }
        Staged::Map(Edit::MergeRooms(..)) => trf("These rooms cannot be merged. ({detail})", &[("detail", &detail)]),
        _ => trf("This change cannot be saved. ({detail})", &[("detail", &detail)]),
    }
}
