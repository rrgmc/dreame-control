//! The home screen: the map with the robot, a status card, and the cleaning controls.
//!
//! Like other robot vacuum apps, the user picks what to clean (the whole home, rooms tapped on
//! the map, or a zone drawn on it), sets suction and water, and presses one big button that
//! starts, pauses or resumes. Stop and Dock ask first while a job runs. "Go to" sends the robot
//! to a point clicked on the map, without cleaning.

use crate::backend::{Backend, LoadedMap, Msg, SharedRobot};
use crate::mapview::{Drag, MapInput, MapView, Mark, Overlay};
use crate::phase::{self, capitalise, Phase, Primary};
use dreame::control::{CleanOptions, ZONE_DEFAULTS};
use dreame_i18n::{tr, trf, trn};
use dreame::live::RobotPose;
use dreame_map::model::{Point, Rect};
use dreame_map::path::Path;
use dreame_model::enums::{CleaningMode, CodeEnum, Open, SuctionLevel, WaterVolume};
use dreame_model::{Feature, Status};
use eframe::egui::{self, Color32, RichText};
use std::time::{Duration, Instant};

/// What to clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Home,
    Rooms,
    Zone,
    /// Drive to a point and stop there, without cleaning.
    GoTo,
}

/// The official app refuses to send the robot to a point below this battery level.
const GO_TO_MIN_BATTERY: i64 = 15;

/// The size of the go to marker on the map, in mm.
const GO_TO_MARK_MM: f64 = 350.0;

/// A question before a command that ends the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    Stop,
    Dock,
}

/// What the app should do after this frame.
pub enum Exit {
    Stay,
    SignOut,
    SwitchRobot,
    /// The app settings changed: advanced editing, notifications, language (a tag, or `None`
    /// to follow the system).
    Prefs { advanced: bool, notify: bool, language: Option<String> },
}

/// A page other than the map.
enum Page {
    Rooms(crate::rooms::RoomsPage),
    Robot(crate::robotpage::RobotPage),
    History(Box<crate::historypage::HistoryPage>),
    Schedules(crate::schedulepage::SchedulePage),
    Photos(Box<crate::photospage::PhotosPage>),
}

pub struct Home {
    robot: SharedRobot,
    name: String,
    status: Status,
    map: Option<LoadedMap>,
    pub loading_map: bool,
    map_error: Option<String>,
    live_path: Option<Path>,
    pose: Option<RobotPose>,
    active_rooms: Vec<u32>,
    link: String,
    target: Target,
    picked: Vec<u32>,
    zone: Option<Rect>,
    /// Where to send the robot, in world mm.
    go_to: Option<Point>,
    passes: u32,
    wetness: Option<u8>,
    view: MapView,
    ask: Option<Ask>,
    /// Commands sent and not yet answered.
    in_flight: Vec<String>,
    /// Short messages over the map: text, when shown, and whether it is an error.
    notes: Vec<(String, Instant, bool)>,
    was_active: bool,
    last_refresh: Instant,
    can_switch: bool,
    /// The map editing screen, while open.
    editor: Option<crate::edit::EditMap>,
    page: Option<Page>,
    /// The drive pad, while open.
    drive: Option<crate::drive::DrivePad>,
    /// A map edit sent from the rooms page and not yet answered.
    saving_edit: Option<String>,
    advanced: bool,
    notify: bool,
    /// Whether the window has the focus; notices go to the system only when it has not.
    focused: bool,
    settings_open: bool,
    /// The language chosen in the settings; `None` follows the system.
    language: Option<String>,
}

impl Home {
    pub fn new(robot: SharedRobot, name: String, status: Status, can_switch: bool, prefs: &crate::config::Prefs) -> Self {
        let was_active = phase::job_active(&status);
        Self {
            robot,
            name,
            status,
            map: None,
            loading_map: false,
            map_error: None,
            live_path: None,
            pose: None,
            active_rooms: Vec::new(),
            link: String::new(),
            target: Target::Home,
            picked: Vec::new(),
            zone: None,
            go_to: None,
            passes: 1,
            wetness: None,
            view: MapView::default(),
            ask: None,
            in_flight: Vec::new(),
            notes: Vec::new(),
            was_active,
            last_refresh: Instant::now(),
            can_switch,
            editor: None,
            page: None,
            drive: None,
            saving_edit: None,
            advanced: prefs.advanced,
            notify: prefs.notify,
            focused: true,
            settings_open: false,
            language: prefs.language.clone(),
        }
    }

    /// Swaps in the same robot after discovery, so the controls follow what it supports.
    /// Returns false, changing nothing, when `robot` is another device (the user switched).
    pub fn set_robot(&mut self, robot: SharedRobot) -> bool {
        if robot.cloud_link().device.did != self.robot.cloud_link().device.did {
            return false;
        }
        self.robot = robot;
        true
    }

    pub fn handle(&mut self, msg: Msg, backend: &Backend) {
        match msg {
            Msg::Map(m) => {
                self.view.set_map(&m.map);
                // Rooms that are gone cannot stay picked.
                self.picked.retain(|id| m.map.rooms.contains_key(id));
                self.map = Some(*m);
                self.loading_map = false;
                self.map_error = None;
            }
            Msg::MapFailed(e) => {
                self.loading_map = false;
                self.map_error = Some(e);
            }
            Msg::Status(s) => {
                for n in crate::notify::notices(&self.status, &s) {
                    self.tell(&n);
                }
                let active = phase::job_active(&s);
                // A job ended: the saved map now has its path; read it again.
                if self.was_active && !active {
                    backend.load_map(&self.robot);
                }
                self.was_active = active;
                self.status = *s;
                self.last_refresh = Instant::now();
            }
            Msg::Live { path, pose, active_rooms } => {
                self.live_path = Some(path);
                self.pose = pose;
                self.active_rooms = active_rooms;
            }
            Msg::Link(l) => self.link = l,
            Msg::Schedules(slots) => {
                if let Some(Page::Schedules(p)) = &mut self.page {
                    p.slots = Some(slots);
                }
            }
            Msg::History(jobs) => {
                if let Some(Page::History(p)) = &mut self.page {
                    p.jobs = Some(jobs);
                }
            }
            Msg::JobMap { started, result } => {
                if let Some(Page::History(p)) = &mut self.page {
                    p.set_map(started, result.map(|m| *m));
                }
            }
            Msg::Drove(Err(e)) => self.note(trf("Driving failed: {error}", &[("error", &e)]), true),
            Msg::Drove(Ok(())) => {}
            Msg::Obstacles(list) => {
                if let Some(Page::Photos(p)) = &mut self.page {
                    p.set_obstacles(list);
                }
            }
            Msg::Photo { index, result } => {
                if let Some(Page::Photos(p)) = &mut self.page {
                    p.set_photo(backend.ctx(), index, result);
                }
            }
            Msg::Stall(line) => {
                crate::config::log_line(&backend.paths().log, &line);
                let n = crate::notify::Notice {
                    title: tr("The robot may be stuck").into(),
                    body: tr("It has hardly moved for a while. Check that nothing holds it.").into(),
                    good: false,
                };
                self.tell(&n);
            }
            Msg::Edited { before, result } if self.editor.is_none() => {
                // A save from the rooms page.
                let what = self.saving_edit.take().unwrap_or_else(|| tr("The change").into());
                match result {
                    Ok(new) => {
                        self.view.set_map(&new.map);
                        if let Some(Page::Rooms(r)) = &mut self.page {
                            r.set_map(&new.map);
                        }
                        self.map = Some(*new);
                        self.note(tr("Saved to the robot.").into(), false);
                    }
                    Err(e) => {
                        if let Some(Page::Rooms(r)) = &mut self.page {
                            r.set_map(&before);
                        }
                        self.note(trf("{what} failed: {error}", &[("what", &what), ("error", &e)]), true);
                    }
                }
            }
            Msg::Edited { .. } | Msg::Progress(_) | Msg::BackedUp(_) => {
                if let Some(new) = self.editor.as_mut().and_then(|e| e.handle(msg)) {
                    self.view.set_map(&new.map);
                    self.picked.retain(|id| new.map.rooms.contains_key(id));
                    self.map = Some(*new);
                }
            }
            Msg::Done { what, result } => {
                self.in_flight.retain(|w| w != &what);
                // A schedule change: read the list back from the robot.
                if let Some(Page::Schedules(p)) = &mut self.page {
                    p.slots = None;
                    backend.load_schedules(&self.robot);
                }
                if let Err(e) = result {
                    self.note(trf("{what} failed: {error}", &[("what", &what), ("error", &e)]), true);
                }
            }
            _ => {}
        }
    }

    /// Tells the user something that happened: in the window, and in the system's
    /// notification centre when the window is in the background.
    fn tell(&mut self, n: &crate::notify::Notice) {
        self.note(format!("{}: {}", n.title, n.body), !n.good);
        if self.notify && !self.focused {
            crate::notify::show(n);
        }
    }

    fn note(&mut self, text: String, error: bool) {
        self.notes.push((text, Instant::now(), error));
    }

    fn send<F, Fut, T>(&mut self, backend: &Backend, what: &str, f: F)
    where
        F: FnOnce(SharedRobot) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<T>> + Send,
        T: Send,
    {
        self.in_flight.push(what.to_string());
        backend.command(&self.robot, what, f);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, backend: &Backend) -> Exit {
        let mut exit = Exit::Stay;
        // Without the live connection, read the status now and then.
        let live = self.link == "connected";
        let every = if live { Duration::from_secs(300) } else { Duration::from_secs(20) };
        if self.last_refresh.elapsed() > every {
            self.last_refresh = Instant::now();
            backend.refresh(&self.robot);
        }
        ui.ctx().request_repaint_after(Duration::from_secs(5));
        self.focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
        if let Some(ed) = self.editor.as_mut() {
            let idle = !phase::job_active(&self.status);
            if let crate::edit::EditExit::Back = ed.ui(ui, backend, idle) {
                self.editor = None;
            }
            return exit;
        }
        self.notes.retain(|n| n.1.elapsed() < Duration::from_secs(if n.2 { 12 } else { 5 }));
        if self.page.is_some() {
            self.page_ui(ui, backend);
            return exit;
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.name).size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new(tr("Menu")).size(15.0), |ui| {
                        if ui.button(tr("Reload the map")).clicked() {
                            self.loading_map = true;
                            backend.load_map(&self.robot);
                            ui.close();
                        }
                        if ui.button(tr("Settings")).clicked() {
                            self.settings_open = true;
                            ui.close();
                        }
                        if self.can_switch && ui.button(tr("Switch robot")).clicked() {
                            exit = Exit::SwitchRobot;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr("Sign out")).clicked() {
                            exit = Exit::SignOut;
                            ui.close();
                        }
                    });
                    ui.add_space(4.0);
                    if let Some(l) = &self.map {
                        if ui.button(RichText::new(tr("Edit map")).size(15.0)).on_hover_text(tr("Rooms, no-go zones and walls")).clicked() {
                            self.editor = Some(crate::edit::EditMap::new(self.robot.clone(), l.map.clone(), self.advanced));
                        }
                    }
                    if let Some(l) = &self.map {
                        if ui.button(RichText::new(tr("Rooms")).size(15.0)).on_hover_text(tr("Each room's cleaning settings and order")).clicked() {
                            self.page = Some(Page::Rooms(crate::rooms::RoomsPage::new(&l.map)));
                            backend.read_settings(&self.robot);
                        }
                    }
                    if ui.button(RichText::new(tr("Schedules")).size(15.0)).on_hover_text(tr("Timed cleanings")).clicked() {
                        self.page = Some(Page::Schedules(crate::schedulepage::SchedulePage::new()));
                        backend.load_schedules(&self.robot);
                    }
                    if ui.button(RichText::new(tr("History")).size(15.0)).on_hover_text(tr("Past cleaning jobs and where the robot went")).clicked() {
                        self.page = Some(Page::History(Box::new(crate::historypage::HistoryPage::new())));
                        backend.load_history(&self.robot, crate::historypage::PAGE);
                    }
                    if ui.button(RichText::new(tr("Robot")).size(15.0)).on_hover_text(tr("Settings, station and maintenance")).clicked() {
                        self.page = Some(Page::Robot(crate::robotpage::RobotPage::new()));
                        backend.read_settings(&self.robot);
                    }
                    if ui.button(RichText::new(tr("Photos")).size(15.0)).on_hover_text(tr("Obstacles and pets the camera saw")).clicked() {
                        self.page = Some(Page::Photos(Box::default()));
                        backend.load_obstacles(&self.robot);
                    }
                    let job = phase::job_active(&self.status);
                    let drive = ui
                        .add_enabled(!job, egui::Button::new(RichText::new(tr("Drive")).size(15.0)).selected(self.drive.is_some()))
                        .on_hover_text(tr("Drive the robot by hand"))
                        .on_disabled_hover_text(tr("Not while a job runs"));
                    if drive.clicked() {
                        self.drive = if self.drive.is_some() { None } else { Some(Default::default()) };
                    }
                    ui.add_space(8.0);
                    if let Some(b) = self.status.battery {
                        let charging = phase::phase(&self.status) == Phase::Docked;
                        ui.label(RichText::new(format!("{}{b}%", if charging { "⚡" } else { "" })).size(15.0))
                            .on_hover_text(tr("Battery"));
                    }
                    ui.add_space(8.0);
                    let (dot, tip) = match self.link.as_str() {
                        "connected" => (Color32::from_rgb(60, 180, 90), tr("Live updates on")),
                        "" | "connecting" => (Color32::from_rgb(230, 170, 40), tr("Connecting for live updates")),
                        _ => (Color32::from_rgb(200, 70, 60), tr("No live updates; the status is read every 20 seconds")),
                    };
                    ui.label(RichText::new("●").color(dot)).on_hover_text(tip);
                });
            });
            ui.add_space(2.0);
        });

        egui::Panel::bottom("controls").show(ui, |ui| {
            ui.add_space(8.0);
            self.controls(ui, backend);
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            match self.map.as_ref() {
                Some(l) => {
                    // Rooms drawn in colour: the job's rooms while it runs, the picked ones
                    // while choosing, else all.
                    let coloured: Vec<u32> = if phase::job_active(&self.status) && !self.active_rooms.is_empty() {
                        self.active_rooms.clone()
                    } else if self.target == Target::Rooms {
                        self.picked.clone()
                    } else {
                        Vec::new()
                    };
                    let path = self.live_path.as_ref().filter(|p| !p.points.is_empty()).unwrap_or(&l.path);
                    let overlay = Overlay {
                        path,
                        pose: shown_pose(&self.status, self.pose, l.map.dock),
                        picked: if self.target == Target::Rooms { &self.picked } else { &[] },
                        zone: if self.target == Target::Zone { self.zone } else { None },
                        drag: if self.target == Target::Zone { Drag::Rect } else { Drag::Pan },
                        marks: match (self.target, self.go_to) {
                            (Target::GoTo, Some(center)) => vec![Mark {
                                shape: dreame::shapes::Shape::Ellipse { center, size: (GO_TO_MARK_MM, GO_TO_MARK_MM) },
                                colour: crate::ACCENT,
                            }],
                            _ => Vec::new(),
                        },
                        hidden_labels: false,
                    };
                    let input = self.view.show(ui, &l.map, &coloured, &overlay);
                    // Hidden rooms cannot be cleaned; a click on one says so.
                    let hidden = |p| l.map.room_at(p).filter(|id| l.map.rooms.get(id).is_some_and(|r| r.hidden));
                    let room = |p| l.map.room_at(p).filter(|id| l.map.rooms.get(id).is_some_and(|r| !r.hidden));
                    let mut note = None;
                    match input {
                        MapInput::Click(p) if hidden(p).is_some() => {
                            note = Some(
                                tr("This grey area is hidden, so the robot does not clean it. To show it again, \
                                    click it in Edit map or tick it on the Rooms page.")
                                .to_string(),
                            );
                        }
                        MapInput::Click(p) if self.target == Target::Rooms && room(p).is_some() => {
                            let id = room(p).expect("checked");
                            if let Some(i) = self.picked.iter().position(|r| *r == id) {
                                self.picked.remove(i);
                            } else {
                                self.picked.push(id);
                            }
                        }
                        MapInput::Click(p) if self.target == Target::Home && room(p).is_some() => {
                            // A tap on a room switches to choosing rooms, as in other apps.
                            self.target = Target::Rooms;
                            self.picked = vec![room(p).expect("checked")];
                        }
                        MapInput::Click(p) if self.target == Target::GoTo => {
                            if room(p).is_some() {
                                self.go_to = Some(p);
                            } else {
                                note = Some(tr("Click on the floor of a room.").to_string());
                            }
                        }
                        MapInput::Rect(a, b) if self.target == Target::Zone => {
                            let z = l.map.snap_to_cells(&Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)));
                            if z.width() >= 300.0 && z.height() >= 300.0 {
                                self.zone = Some(z);
                            }
                        }
                        _ => {}
                    }
                    if let Some(n) = note {
                        self.note(n, false);
                    }
                }
                None => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(rect.height() / 2.0 - 30.0);
                        if self.loading_map {
                            ui.spinner();
                            ui.label(tr("Loading the map"));
                        } else if let Some(e) = &self.map_error {
                            ui.label(tr("The map could not be loaded."));
                            ui.label(RichText::new(e).small().weak());
                            if ui.button(tr("Try again")).clicked() {
                                self.loading_map = true;
                                backend.load_map(&self.robot);
                            }
                        } else {
                            ui.label(tr("No map yet. Start a cleaning to map the home."));
                        }
                    });
                }
            }
            self.status_card(ui, rect);
        });

        if let Some(pad) = self.drive.as_mut() {
            let docked = matches!(phase::phase(&self.status), Phase::Docked | Phase::AtStation);
            let (send, open) = pad.ui(ui.ctx(), docked);
            if let Some((step, first)) = send {
                backend.drive(&self.robot, step, first);
            }
            if !open {
                self.drive = None;
            }
        }
        if let Some(ask) = self.ask {
            self.ask_dialog(ui, backend, ask);
        }
        if self.settings_open {
            if let Some((advanced, notify, language)) = self.settings_dialog(ui) {
                exit = Exit::Prefs { advanced, notify, language };
            }
        }
        exit
    }

    /// The card over the top left of the map: what the robot does, errors, and job numbers.
    fn status_card(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let area = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 12.0), egui::vec2(rect.width().min(420.0) - 24.0, rect.height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            let frame = egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(12, 8)).corner_radius(10.0);
            frame.show(ui, |ui| {
                ui.label(RichText::new(phase::headline(&self.status)).size(17.0).strong());
                if let Some(e) = phase::error_text(&self.status) {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                if phase::job_active(&self.status) {
                    let mut parts = Vec::new();
                    if let Some(a) = self.status.cleaned_area_m2 {
                        parts.push(format!("{a} m²"));
                    }
                    if let Some(t) = self.status.cleaning_time_min {
                        parts.push(format!("{t} min"));
                    }
                    if let Some(p) = self.status.cleaning_progress.filter(|p| (1..=100).contains(p)) {
                        parts.push(trf("{pct}% done", &[("pct", &p)]));
                    }
                    if !parts.is_empty() {
                        ui.label(parts.join("  ·  "));
                    }
                }
                if !self.in_flight.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(self.in_flight.join(", "));
                    });
                }
            });
            for (text, _, error) in &self.notes {
                ui.add_space(6.0);
                egui::Frame::popup(ui.style()).corner_radius(10.0).show(ui, |ui| {
                    if *error {
                        ui.colored_label(ui.visuals().error_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                });
            }
        });
    }

    fn controls(&mut self, ui: &mut egui::Ui, backend: &Backend) {
        let s = self.status.clone();
        let job = phase::job_active(&s);
        let idle = !job;

        // What to clean. Fixed while a job runs.
        ui.add_enabled_ui(idle, |ui| {
            ui.horizontal(|ui| {
                for (t, label) in
                    [(Target::Home, tr("Whole home")), (Target::Rooms, tr("Rooms")), (Target::Zone, tr("Zone")), (Target::GoTo, tr("Go to"))]
                {
                    let b = egui::Button::selectable(self.target == t, RichText::new(label).size(15.0)).min_size(egui::vec2(110.0, 30.0));
                    if ui.add(b).clicked() {
                        self.target = t;
                    }
                }
                ui.add_space(12.0);
                match self.target {
                    Target::Home => {
                        ui.label(RichText::new(tr("Cleans every room in the saved order.")).weak());
                    }
                    Target::Rooms if self.picked.is_empty() => {
                        ui.label(RichText::new(tr("Click rooms on the map to choose them, in the order to clean.")).weak());
                    }
                    Target::Rooms => {
                        let names: Vec<String> = self.picked.iter().map(|id| self.room_name(*id)).collect();
                        let text = trn("{n} room: {names}", "{n} rooms: {names}", names.len() as i64, &[("names", &names.join(", "))]);
                        ui.label(text);
                        if ui.small_button(tr("Clear")).clicked() {
                            self.picked.clear();
                        }
                    }
                    Target::Zone if self.zone.is_none() => {
                        ui.label(RichText::new(tr("Drag on the map to draw the zone. Right-drag moves the map.")).weak());
                    }
                    Target::Zone => {
                        let z = self.zone.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
                        let (w, h) = (format!("{:.1}", z.width() / 1000.0), format!("{:.1}", z.height() / 1000.0));
                        ui.label(trf("Zone of {width} × {height} m", &[("width", &w), ("height", &h)]));
                        if ui.small_button(tr("Clear")).clicked() {
                            self.zone = None;
                        }
                    }
                    Target::GoTo if self.go_to.is_none() => {
                        ui.label(RichText::new(tr("Click on the map where the robot should go. It drives there without cleaning.")).weak());
                    }
                    Target::GoTo => {
                        ui.label(tr("The robot drives to the marked point and stops there."));
                        if ui.small_button(tr("Clear")).clicked() {
                            self.go_to = None;
                        }
                    }
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| self.cleaning_settings(ui, backend));
        ui.add_space(8.0);

        // The buttons.
        ui.horizontal(|ui| {
            let big = |t: &str| RichText::new(t).size(17.0);
            let primary = phase::primary(&s);
            let low_battery = s.battery.is_some_and(|b| b < GO_TO_MIN_BATTERY);
            let (icon, label, ready) = match primary {
                Primary::Pause => ("⏸", tr("Pause"), true),
                Primary::Resume => ("▶", tr("Resume"), true),
                Primary::Start => match self.target {
                    Target::Home => ("▶", tr("Start cleaning"), true),
                    Target::Rooms => ("▶", tr("Clean rooms"), !self.picked.is_empty() && self.map.is_some()),
                    Target::Zone => ("▶", tr("Clean zone"), self.zone.is_some() && self.map.is_some()),
                    Target::GoTo => ("➡", tr("Go here"), self.go_to.is_some() && self.map.is_some() && !low_battery),
                },
            };
            let label = format!("{icon}  {label}");
            let fill = crate::ACCENT;
            let b = egui::Button::new(big(&label).color(Color32::WHITE)).fill(fill).min_size(egui::vec2(190.0, 42.0));
            let b = ui.add_enabled(ready, b);
            let b = if low_battery && primary == Primary::Start && self.target == Target::GoTo {
                b.on_disabled_hover_text(trf("The battery must be at {percent}% or more.", &[("percent", &GO_TO_MIN_BATTERY)]))
            } else {
                b
            };
            if b.clicked() {
                self.primary(backend, primary);
            }
            let side = |t: &str| egui::Button::new(big(t)).min_size(egui::vec2(110.0, 42.0));
            if ui.add_enabled(job, side(&format!("⏹  {}", tr("Stop")))).on_hover_text(tr("End the current job")).clicked() {
                self.ask = Some(Ask::Stop);
            }
            let docked = matches!(phase::phase(&s), Phase::Docked | Phase::Returning | Phase::AtStation);
            if ui.add_enabled(!docked, side(&format!("⌂  {}", tr("Dock")))).on_hover_text(tr("Send the robot back to its dock")).clicked() {
                if job {
                    self.ask = Some(Ask::Dock);
                } else {
                    self.send(backend, tr("Docking"), |r| async move { r.dock().await });
                }
            }
            if ui.add(side(&format!("🔊  {}", tr("Find")))).on_hover_text(tr("The robot says where it is")).clicked() {
                self.send(backend, tr("Finding the robot"), |r| async move { r.locate().await });
            }
        });
    }

    fn primary(&mut self, backend: &Backend, p: Primary) {
        match p {
            Primary::Pause => self.send(backend, tr("Pausing"), |r| async move { r.pause().await }),
            Primary::Resume => self.send(backend, tr("Resuming"), |r| async move { r.start().await }),
            Primary::Start => {
                let Some(map) = self.map.as_ref().map(|l| l.map.clone()) else {
                    if self.target == Target::Home {
                        self.send(backend, tr("Starting"), |r| async move { r.start().await });
                    }
                    return;
                };
                let passes = Some(self.passes);
                match self.target {
                    Target::Home => self.send(backend, tr("Starting"), |r| async move { r.start().await }),
                    Target::Rooms => {
                        let rooms = self.picked.clone();
                        let opts = CleanOptions { passes, ..Default::default() };
                        self.send(backend, tr("Starting the rooms"), move |r| async move { r.clean_rooms(&map, &rooms, opts).await });
                    }
                    Target::Zone => {
                        let Some(zone) = self.zone else { return };
                        let suction = self.status.suction.map(|s| s.code()).or(Some(ZONE_DEFAULTS.1));
                        let opts = CleanOptions { passes, suction, water: None };
                        self.send(backend, tr("Starting the zone"), move |r| async move { r.clean_zone(&map, zone, opts).await });
                    }
                    Target::GoTo => {
                        let Some(p) = self.go_to else { return };
                        self.send(backend, tr("Sending the robot"), move |r| async move { r.go_to(&map, p).await });
                    }
                }
            }
        }
    }

    /// Mode, suction, water and passes. A change is sent to the robot at once, as in the
    /// official app.
    fn cleaning_settings(&mut self, ui: &mut egui::Ui, backend: &Backend) {
        let s = &self.status;
        // Vacuum and mop mode.
        if let Some(Open::Known(current)) = s.logical_cleaning_mode() {
            let label = |m: CleaningMode| match m {
                CleaningMode::Sweeping => tr("Vacuum"),
                CleaningMode::Mopping => tr("Mop"),
                CleaningMode::SweepingAndMopping => tr("Vacuum and mop"),
                CleaningMode::MoppingAfterSweeping => tr("Vacuum, then mop"),
            };
            let mut chosen = current;
            ui.label(tr("Mode"));
            egui::ComboBox::from_id_salt("mode").selected_text(label(current)).show_ui(ui, |ui| {
                for m in CleaningMode::ALL {
                    ui.selectable_value(&mut chosen, *m, label(*m));
                }
            });
            if chosen != current {
                self.send(backend, tr("Changing the mode"), move |r| async move { r.set_mop_mode(chosen).await });
            }
            ui.add_space(10.0);
        }
        // Suction.
        let s = &self.status;
        if let Some(Open::Known(current)) = s.suction {
            let label = |l: SuctionLevel| match l {
                SuctionLevel::Quiet => tr("Quiet"),
                SuctionLevel::Standard => tr("Standard"),
                SuctionLevel::Strong => tr("Strong"),
                SuctionLevel::Turbo => tr("Max"),
            };
            let mut chosen = current;
            ui.label(tr("Suction"));
            egui::ComboBox::from_id_salt("suction").selected_text(label(current)).show_ui(ui, |ui| {
                for l in SuctionLevel::ALL {
                    ui.selectable_value(&mut chosen, *l, label(*l));
                }
            });
            if chosen != current {
                self.send(backend, tr("Changing the suction"), move |r| async move { r.set_suction(chosen).await });
            }
            ui.add_space(10.0);
        }
        // Water: the 1 to 32 wetness scale where the robot has it, else three steps.
        let s = &self.status;
        if self.robot.has(Feature::Wetness) == Some(true) || s.wetness.is_some() {
            if let Some(w) = s.wetness {
                let mut v = self.wetness.unwrap_or(w.clamp(1, 32) as u8);
                ui.label(tr("Mop wetness"));
                let r = ui.add(egui::Slider::new(&mut v, 1..=32));
                self.wetness = Some(v);
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.send(backend, tr("Changing the wetness"), move |r| async move { r.set_wetness(v).await });
                }
                if !r.dragged() && !r.has_focus() && self.in_flight.iter().all(|w| w != tr("Changing the wetness")) {
                    self.wetness = None;
                }
                ui.add_space(10.0);
            }
        } else if let Some(Open::Known(current)) = s.water_volume {
            let mut chosen = current;
            ui.label(tr("Water"));
            egui::ComboBox::from_id_salt("water").selected_text(capitalise(tr(current.label()))).show_ui(ui, |ui| {
                for l in WaterVolume::ALL {
                    ui.selectable_value(&mut chosen, *l, capitalise(tr(l.label())));
                }
            });
            if chosen != current {
                self.send(backend, tr("Changing the water"), move |r| async move { r.set_water_volume(chosen).await });
            }
            ui.add_space(10.0);
        }
        // Passes, for rooms and zones; the whole home uses the stored room settings.
        if matches!(self.target, Target::Rooms | Target::Zone) && !phase::job_active(&self.status) {
            ui.label(tr("Passes"));
            for n in 1..=3 {
                ui.selectable_value(&mut self.passes, n, format!("{n}×"));
            }
        }
    }

    fn ask_dialog(&mut self, ui: &mut egui::Ui, backend: &Backend, ask: Ask) {
        let (title, text, button) = match ask {
            Ask::Stop => (tr("End cleaning?"), tr("The current job ends. The robot stays where it is."), tr("End cleaning")),
            Ask::Dock => (
                tr("Return to the dock?"),
                tr("The current job ends and the robot goes back to its dock."),
                tr("Return to dock"),
            ),
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("ask")).show(ui.ctx(), |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(6.0);
            ui.label(text);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(button).clicked() {
                    match ask {
                        Ask::Stop => self.send(backend, tr("Stopping"), |r| async move { r.stop().await }),
                        Ask::Dock => self.send(backend, tr("Docking"), |r| async move { r.dock().await }),
                    }
                    close = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.ask = None;
        }
    }

    /// The settings dialog. Returns the settings when one of them changes.
    fn settings_dialog(&mut self, ui: &mut egui::Ui) -> Option<(bool, bool, Option<String>)> {
        let before = (self.advanced, self.notify, self.language.clone());
        let modal = egui::Modal::new(egui::Id::unique("settings")).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(tr("Settings")).size(17.0).strong());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Language"));
                crate::language_picker(ui, &mut self.language);
            });
            ui.add_space(8.0);
            ui.checkbox(&mut self.notify, tr("Notifications"));
            ui.label(
                RichText::new(tr(
                    "When the app is in the background, tell me when a job ends, when the robot reports \
                     an error, and when it seems stuck.",
                ))
                .small()
                .weak(),
            );
            ui.add_space(8.0);
            ui.checkbox(&mut self.advanced, tr("Advanced map editing"));
            ui.label(
                RichText::new(tr(
                    "Adds tools that change the robot's room layout directly, such as moving the border \
                     between two rooms. They save through a backup and restore of the map, take a few \
                     minutes, and need this computer and the robot on the same network.",
                ))
                .small()
                .weak(),
            );
            ui.add_space(12.0);
            if ui.button(tr("Close")).clicked() {
                self.settings_open = false;
            }
        });
        if modal.should_close() {
            self.settings_open = false;
        }
        let now = (self.advanced, self.notify, self.language.clone());
        (now != before).then_some(now)
    }

    /// The rooms or robot page, with a bar to go back to the map.
    fn page_ui(&mut self, ui: &mut egui::Ui, backend: &Backend) {
        let title = match &self.page {
            Some(Page::Rooms(_)) => tr("Rooms"),
            Some(Page::Robot(_)) => tr("Robot"),
            Some(Page::History(_)) => tr("History"),
            Some(Page::Schedules(_)) => tr("Schedules"),
            Some(Page::Photos(_)) => tr("Photos"),
            None => return,
        };
        let mut back = false;
        egui::Panel::top("page-top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                back = ui.button(RichText::new(format!("← {}", tr("Back to the map"))).size(15.0)).clicked();
                ui.add_space(12.0);
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(16.0);
                let busy: Vec<String> = self.in_flight.iter().cloned().chain(self.saving_edit.clone()).collect();
                if !busy.is_empty() {
                    ui.spinner();
                    ui.label(busy.join(", "));
                }
                for (text, _, error) in &self.notes {
                    if *error {
                        ui.colored_label(ui.visuals().error_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                }
            });
            ui.add_space(2.0);
        });
        if back {
            self.page = None;
            return;
        }
        let robot = self.robot.clone();
        let has = move |f: Feature| robot.has(f) == Some(true);
        let wetness = self.robot.has(Feature::Wetness) == Some(true) || self.status.wetness.is_some();
        let customized = self.status.raw.get(&dreame_core::PropId::new(4, 26)).and_then(|v| v.as_i64()).map(|v| v != 0);
        let saving = self.saving_edit.is_some();
        let sending = !self.in_flight.is_empty();
        let status = self.status.clone();
        let map = self.map.as_ref().map(|l| l.map.clone());
        let mut room_action = None;
        let mut robot_action = None;
        let mut schedule_save = None;
        match &mut self.page {
            Some(Page::Rooms(page)) => {
                if let Some(map) = &map {
                    room_action = page.ui(ui, map, customized, wetness, saving || sending);
                }
            }
            Some(Page::Robot(page)) => robot_action = page.ui(ui, &status, &has, sending),
            Some(Page::Schedules(page)) => match page.ui(ui, map.as_ref(), sending) {
                Some(crate::schedulepage::ScheduleAction::Load) => backend.load_schedules(&self.robot),
                Some(crate::schedulepage::ScheduleAction::Save { slots, what }) => {
                    schedule_save = Some((slots, what));
                }
                None => {}
            },
            Some(Page::History(page)) => match page.ui(ui) {
                Some(crate::historypage::HistoryAction::Load(limit)) => backend.load_history(&self.robot, limit),
                Some(crate::historypage::HistoryAction::Map(job)) => backend.load_job_map(&self.robot, *job),
                None => {}
            },
            Some(Page::Photos(page)) => {
                if let Some(crate::photospage::PhotosAction::Reload) = page.ui(ui) {
                    page.obstacles = None;
                    backend.load_obstacles(&self.robot);
                }
            }
            None => {}
        }
        match room_action {
            Some(crate::rooms::RoomAction::Save { edit, what }) => {
                if let Some(map) = map {
                    self.saving_edit = Some(what);
                    backend.apply_edit(&self.robot, map, edit);
                }
            }
            Some(crate::rooms::RoomAction::Customized(on)) => {
                let what = if on { tr("Turning on per-room settings") } else { tr("Turning off per-room settings") };
                self.send(backend, what, move |r| async move { r.set(&dreame::Setting::CustomizedCleaning(on)).await });
            }
            None => {}
        }
        if let Some((slots, what)) = schedule_save {
            self.send(backend, &what, move |r| async move { r.set_schedules(&slots).await });
        }
        use crate::robotpage::RobotAction as A;
        match robot_action {
            Some(A::Set(setting)) => {
                let what = changing(&setting);
                self.send(backend, &what, move |r| async move { r.set(&setting).await });
            }
            Some(A::Reset(c)) => {
                let what = trf("Resetting the {part}", &[("part", &tr(c.label()))]);
                self.send(backend, &what, move |r| async move { r.reset_consumable(c).await });
            }
            Some(A::Station(op)) => self.send(backend, tr("Sending to the station"), move |r| async move { r.station(op).await }),
            Some(A::EmptyBin) => self.send(backend, tr("Emptying the bin"), |r| async move { r.start_auto_empty().await }),
            None => {}
        }
    }

    fn room_name(&self, id: u32) -> String {
        self.map
            .as_ref()
            .and_then(|l| l.map.rooms.get(&id))
            .map(|r| r.display_name())
            .unwrap_or_else(|| trf("Room {id}", &[("id", &id)]))
    }
}

/// The status line while a setting is sent, such as "Changing the voice volume".
fn changing(setting: &dreame::Setting) -> String {
    use dreame::Setting as S;
    match setting {
        S::Volume(_) => tr("Changing the voice volume").into(),
        S::ChildLock(_) => tr("Changing the child lock").into(),
        S::CarpetBoost(_) => tr("Changing carpet boost").into(),
        S::ResumeCleaning(_) => tr("Changing resume after recharging").into(),
        S::CustomizedCleaning(_) => tr("Changing per-room settings").into(),
        S::SelfClean(_) => tr("Changing mop washing during the job").into(),
        S::DryingTimeHours(_) => tr("Changing the mop drying time").into(),
        S::CarpetSensitivity(_) => tr("Changing the carpet sensitivity").into(),
        S::AutoEmptyFrequency(_) => tr("Changing how often the bin is emptied").into(),
        S::Dnd { .. } => tr("Changing do not disturb").into(),
        other => trf("Changing {setting}", &[("setting", &tr(&other.name()))]),
    }
}

/// Where to draw the robot. On the dock it sits on the charger: the robot sends no position
/// there, so the last one from a run or from the saved map is stale.
fn shown_pose(status: &Status, live: Option<RobotPose>, dock: Option<dreame_map::model::WorldPose>) -> Option<RobotPose> {
    match (phase::phase(status), dock) {
        (Phase::Docked | Phase::AtStation, Some(d)) => Some(RobotPose { x: d.x, y: d.y, angle: d.angle }),
        _ => live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_core::{PropId, RawValue};
    use dreame_map::model::WorldPose;

    fn state(code: i64) -> Status {
        let mut s = Status::default();
        s.update(PropId::new(2, 1), &RawValue::Int(code));
        s
    }

    #[test]
    fn a_docked_robot_is_drawn_on_the_dock() {
        let live = Some(RobotPose { x: 500.0, y: -200.0, angle: 0.0 });
        let dock = Some(WorldPose { x: 10.0, y: 20.0, angle: 90.0 });
        let on_dock = Some(RobotPose { x: 10.0, y: 20.0, angle: 90.0 });
        assert_eq!(shown_pose(&state(6), live, dock), on_dock); // charging
        assert_eq!(shown_pose(&state(13), None, dock), on_dock); // charged
        assert_eq!(shown_pose(&state(1), live, dock), live); // sweeping
        assert_eq!(shown_pose(&state(6), live, None), live); // no dock on the map
    }
}
