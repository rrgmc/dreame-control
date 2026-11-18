//! The map editing screen. The user picks a tool, works on the map, sees the change outlined,
//! and saves it to the robot. One change is saved at a time, as in the official app.
//!
//! Standard tools: rename, divide and merge rooms; add no-go zones, no-mop zones and virtual
//! walls; delete one by clicking it. With advanced editing on (Settings), two tools work
//! through a backup and restore of the map: "Move a border" redraws the border between two
//! rooms, and "Delete a room" gives a room's floor to a neighbour and removes the room. Hidden
//! rooms can be deleted too.

use crate::backend::{Backend, LoadedMap, Msg, SharedRobot};
use crate::mapview::{Drag, Mark, MapInput, MapView, Overlay};
use dreame::archiveedit::GridEdit;
use dreame::mapedit::{self, Edit};
use dreame::shapes::{self, Selection, Shape, ZoneKind};
use dreame_i18n::{tr, trf};
use dreame_map::model::Line;
use dreame_map::path::Path;
use dreame_map::Map;
use eframe::egui::{self, Color32, RichText};

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
    /// Advanced: delete a room, giving its floor to a neighbour.
    Remove,
}

impl Tool {
    fn hint(self) -> &'static str {
        match self {
            Tool::Select => tr("Click a room to rename, divide or merge it. Click a zone or wall to delete it."),
            Tool::Divide => tr("Click the room to divide, then drag a line across it, from wall to wall."),
            Tool::Merge => tr("Click two rooms that touch each other."),
            Tool::Draw(ZoneKind::NoGo) => tr("Drag a rectangle. The robot will not enter it."),
            Tool::Draw(ZoneKind::NoMop) => tr("Drag a rectangle. The robot will vacuum but not mop there."),
            Tool::Draw(_) => tr("Drag a line. The robot will not cross it."),
            Tool::Border => tr(
                "Click two rooms that touch, then drag the new border across both. Room names and \
                 settings stay. Saving takes a few minutes.",
            ),
            Tool::Remove => tr(
                "Click the room to delete, hidden or not, then a room next to it. That room takes \
                 the floor and keeps its name and settings. Saving takes a few minutes.",
            ),
        }
    }

    fn drag(self) -> Drag {
        match self {
            Tool::Draw(k) if k.is_line() => Drag::Line,
            Tool::Draw(_) => Drag::Rect,
            Tool::Divide | Tool::Border => Drag::Line,
            Tool::Select | Tool::Merge | Tool::Remove => Drag::Pan,
        }
    }
}

/// A change ready to save.
enum Staged {
    Map(Edit),
    /// An archive edit; `line` is drawn on the map when the edit has one.
    Grid { edit: GridEdit, line: Option<Line> },
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
    saving: bool,
    progress: Vec<String>,
    /// The last save's outcome, shown until the next change.
    outcome: Option<(String, bool)>,
}

impl EditMap {
    pub fn new(robot: SharedRobot, map: Map, advanced: bool) -> Self {
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
            saving: false,
            progress: Vec::new(),
            outcome: None,
        }
    }

    /// Takes the outcome of a save. Returns the new map for the home screen.
    pub fn handle(&mut self, msg: Msg) -> Option<Box<LoadedMap>> {
        match msg {
            Msg::Progress(line) => {
                self.progress.push(line);
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
    }

    fn reset(&mut self) {
        self.room = None;
        self.selected = None;
        self.staged = None;
        self.problem = None;
        self.outcome = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, backend: &Backend, can_edit: bool) -> EditExit {
        let mut exit = EditExit::Stay;
        egui::Panel::top("edit-top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.saving, egui::Button::new(RichText::new(format!("← {}", tr("Back to the map"))).size(15.0))).clicked() {
                    exit = EditExit::Back;
                }
                ui.add_space(12.0);
                ui.label(RichText::new(tr("Edit map")).size(18.0).strong());
            });
            ui.add_space(2.0);
        });

        egui::Panel::left("tools").resizable(false).exact_size(230.0).show(ui, |ui| {
            ui.add_enabled_ui(!self.saving && can_edit, |ui| self.toolbox(ui));
            if !can_edit {
                ui.add_space(8.0);
                ui.colored_label(ui.visuals().warn_fg_color, tr("The map can be edited while the robot is not cleaning."));
            }
        });

        egui::Panel::bottom("edit-bottom").show(ui, |ui| {
            ui.add_space(8.0);
            self.save_bar(ui, backend, can_edit);
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let coloured: Vec<u32> = match (self.tool, self.room, &self.staged) {
                (_, _, Some(Staged::Map(Edit::MergeRooms(a, b)))) => vec![*a, *b],
                (_, _, Some(Staged::Grid { edit: GridEdit::RemoveRoom { room, into }, .. })) => {
                    vec![u32::from(*room), u32::from(*into)]
                }
                (Tool::Border, Some(a), Some(Staged::Grid { edit: GridEdit::Border { b, .. }, .. })) => vec![a, u32::from(*b)],
                (Tool::Border, Some(a), _) => match self.selected {
                    Some(Selection::Room(b)) => vec![a, b],
                    _ => vec![a],
                },
                (Tool::Divide | Tool::Merge | Tool::Remove, Some(r), _) => vec![r],
                _ => match self.selected {
                    Some(Selection::Room(r)) => vec![r],
                    _ => Vec::new(),
                },
            };
            let marks = self.marks();
            let overlay = Overlay {
                path: &Path::default(),
                pose: None,
                picked: &[],
                zone: None,
                drag: if self.saving || !can_edit { Drag::Pan } else { self.tool.drag() },
                marks,
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
        if self.advanced {
            ui.add_space(10.0);
            ui.label(RichText::new(tr("Advanced")).strong());
            button(ui, Tool::Border, tr("Move a border"));
            button(ui, Tool::Remove, tr("Delete a room"));
        }
        if tool != self.tool {
            self.tool = tool;
            self.reset();
        }
        ui.add_space(14.0);
        ui.label(RichText::new(self.tool.hint()).weak());

        // What the selection offers.
        ui.add_space(14.0);
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
                // Only the kinds this app edits can be picked.
                self.selected = match shapes::hit(&self.map, p, tolerance) {
                    Some(Selection::Zone(k, i)) if matches!(k, ZoneKind::NoGo | ZoneKind::NoMop | ZoneKind::Wall) => {
                        Some(Selection::Zone(k, i))
                    }
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
            (Tool::Merge | Tool::Border | Tool::Remove, MapInput::Click(p)) => {
                // Hidden rooms may be merged or deleted too, such as a grey area beside a room.
                let Some(r) = self.map.room_at(p) else { return };
                match self.room {
                    Some(first) if first != r && shapes::merge_partners(&self.map, first).contains(&r) => {
                        if self.tool == Tool::Merge {
                            self.stage(Staged::Map(Edit::MergeRooms(first, r)));
                        } else if self.tool == Tool::Remove {
                            match shapes::remove_room_edit(&self.map, first, r) {
                                Some(edit) => self.stage(Staged::Grid { edit, line: None }),
                                None => self.problem = Some(tr("These rooms cannot be changed this way.").into()),
                            }
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
                let (Some(ra), Some(Selection::Room(rb))) = (self.room, self.selected) else {
                    self.problem = Some(tr("Click the two rooms first.").into());
                    return;
                };
                let line = Line::new(
                    dreame_map::view::snap(a.x, SNAP_MM),
                    dreame_map::view::snap(a.y, SNAP_MM),
                    dreame_map::view::snap(b.x, SNAP_MM),
                    dreame_map::view::snap(b.y, SNAP_MM),
                );
                match dreame_map::edit::border_cells(&self.map, ra, rb, &[line.a, line.b]) {
                    Ok(_) => match (u8::try_from(ra), u8::try_from(rb)) {
                        (Ok(a8), Ok(b8)) => self.stage(Staged::Grid {
                            edit: GridEdit::Border { a: a8, b: b8, line: vec![(line.a.x, line.a.y), (line.b.x, line.b.y)] },
                            line: Some(line),
                        }),
                        _ => self.problem = Some(tr("These rooms cannot be changed this way.").into()),
                    },
                    Err(e) => {
                        self.problem = Some(trf("Drag the line across both rooms, from one outer wall to another. ({error})", &[("error", &e)]));
                    }
                }
            }
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
            Staged::Grid { .. } => None,
        };
        self.staged = Some(s);
    }

    fn save(&mut self, backend: &Backend, s: Staged) {
        self.saving = true;
        self.progress.clear();
        self.outcome = None;
        match s {
            Staged::Map(e) => backend.apply_edit(&self.robot, self.map.clone(), e),
            Staged::Grid { edit, .. } => backend.apply_grid(&self.robot, self.map.clone(), edit),
        }
    }

    fn save_bar(&mut self, ui: &mut egui::Ui, backend: &Backend, can_edit: bool) {
        if self.saving {
            ui.horizontal(|ui| {
                ui.spinner();
                let text = match self.progress.last() {
                    Some(p) => trf("Saving to the robot: {step}", &[("step", p)]),
                    None => tr("Saving to the robot. This can take a minute.").into(),
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
        if matches!(self.staged, Some(Staged::Grid { .. })) {
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
        let mut marks = Vec::new();
        if let Some(Selection::Zone(kind, i)) = self.selected {
            if let Some(shape) = kind.shapes(&self.map).get(i) {
                marks.push(Mark { shape: *shape, colour: Color32::from_rgb(230, 120, 30) });
            }
        }
        match &self.staged {
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
            Some(Staged::Grid { line: Some(line), .. }) => marks.push(Mark { shape: Shape::Line(*line), colour: crate::ACCENT }),
            Some(Staged::Grid { line: None, .. }) | None => {}
        }
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
                other => format!("{other:?}"),
            },
            Staged::Grid { edit: GridEdit::Border { a, b, .. }, .. } => {
                trf(
                    "New border between {room} and {other}",
                    &[("room", &self.room_name(u32::from(*a))), ("other", &self.room_name(u32::from(*b)))],
                )
            }
            Staged::Grid { edit: GridEdit::RemoveRoom { room, into }, .. } => trf(
                "Delete {room}; {other} takes its floor",
                &[("room", &self.room_name(u32::from(*room))), ("other", &self.room_name(u32::from(*into)))],
            ),
            Staged::Grid { .. } => tr("Room change").into(),
        }
    }

    /// Switches to "Delete a room" with `id`, hidden or not, as the room to delete.
    fn start_remove(&mut self, id: u32) {
        self.tool = Tool::Remove;
        self.reset();
        self.room = Some(id);
    }

    fn visible(&self, id: u32) -> bool {
        self.map.rooms.get(&id).is_some_and(|r| !r.hidden)
    }

    fn room_name(&self, id: u32) -> String {
        self.map.rooms.get(&id).map(|r| r.display_name()).unwrap_or_else(|| trf("Room {id}", &[("id", &id)]))
    }
}

/// The name of a kind of zone or line, as a heading.
fn zone_name(kind: ZoneKind) -> String {
    match kind {
        ZoneKind::NoGo => tr("No-go zone").into(),
        ZoneKind::NoMop => tr("No-mop zone").into(),
        ZoneKind::Wall => tr("Virtual wall").into(),
        other => crate::phase::capitalise(tr(other.label())),
    }
}

/// The button that deletes the selected zone or line.
fn delete_label(kind: ZoneKind) -> String {
    match kind {
        ZoneKind::NoGo => tr("Delete this no-go zone").into(),
        ZoneKind::NoMop => tr("Delete this no-mop zone").into(),
        ZoneKind::Wall => tr("Delete this virtual wall").into(),
        other => trf("Delete this {kind}", &[("kind", &tr(other.label()))]),
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
