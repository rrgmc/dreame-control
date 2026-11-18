//! `dreame-desktop`: the desktop app for Dreame robot vacuums, for Windows, macOS and Linux.
//!
//! It opens on the map of the home: the robot, its dock, the rooms and the path of the current
//! job. From there the user starts, pauses and stops cleaning (the whole home, chosen rooms or a
//! drawn zone), sends the robot to its dock, and sets suction and water. Map editing is a
//! separate screen.
//!
//! The developer lab with every experimental tool is `dreame-lab-editor`; this app keeps to what end
//! users need, with controls like those of other robot vacuum apps.

// A release build on Windows is a windowed program, so starting it opens no console window.
// Debug builds keep the console for their log output.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backend;
mod config;
mod drive;
mod edit;
mod historypage;
mod home;
mod login;
mod mapview;
mod notify;
mod photospage;
mod robotpage;
mod rooms;
mod schedulepage;

/// The status wording lives in the library, shared with the Android app.
use dreame::phase;

use backend::{Backend, Msg, SharedRobot};
use config::{Paths, Prefs};
use dreame_cloud::CloudClient;
use dreame_core::DeviceInfo;
use dreame_i18n::{tr, trf, Language};
use eframe::egui;
use std::sync::Arc;

/// The colour of main buttons and chosen rooms: strong enough for white text on it.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);

/// The app's own Brazilian Portuguese text. The library's text has its catalog in dreame-i18n.
pub const PT_BR: &str = include_str!("../locales/pt-BR.toml");

/// Switches to the language the user chose (a tag such as `pt-BR`), or to the system's when
/// `choice` is `None`.
pub fn apply_language(choice: Option<&str>) {
    let language = match choice {
        Some(tag) => Language::from_tag(tag),
        None => dreame_i18n::system_language(sys_locale::get_locale().as_deref()),
    };
    dreame_i18n::set_language(language);
}

/// A combo box for the language: "System" or one of the app's languages, each named in its
/// own language. Returns true when the user picked another one.
pub fn language_picker(ui: &mut egui::Ui, choice: &mut Option<String>) -> bool {
    let before = choice.clone();
    let name = |c: &Option<String>| match c {
        None => tr("System").to_string(),
        Some(tag) => Language::from_tag(tag).native_name().to_string(),
    };
    egui::ComboBox::from_id_salt("language").selected_text(name(choice)).show_ui(ui, |ui| {
        ui.selectable_value(choice, None, tr("System"));
        for l in Language::ALL {
            ui.selectable_value(choice, Some(l.tag().to_string()), l.native_name());
        }
    });
    *choice != before
}

fn main() -> eframe::Result {
    if let Err(e) = dreame_i18n::add_catalog(Language::PortugueseBrazil, PT_BR) {
        eprintln!("the pt-BR catalog does not parse: {e}");
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_min_inner_size([720.0, 560.0])
            .with_title("Dreame Control")
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "dreame-control",
        options,
        Box::new(|cc| {
            let paths = Paths::system();
            let backend = Backend::new(cc.egui_ctx.clone(), paths.clone()).map_err(|e| e.to_string())?;
            let prefs = Prefs::load(&paths);
            apply_language(prefs.language.as_deref());
            backend.resume();
            Ok(Box::new(App::new(backend, prefs)))
        }),
    )
}

/// The screen shown.
enum Screen {
    /// Resuming the saved session.
    Starting,
    Login(login::LoginForm),
    /// More than one robot on the account: pick one.
    Devices { client: Arc<CloudClient>, devices: Vec<DeviceInfo> },
    /// Connecting to the chosen robot.
    Connecting { name: String },
    /// The robot's home screen.
    Home(Box<home::Home>),
    /// A failure with a way out.
    Failed { message: String },
}

struct App {
    backend: Backend,
    prefs: Prefs,
    screen: Screen,
    /// Kept for "Switch robot".
    client: Option<Arc<CloudClient>>,
    devices: Vec<DeviceInfo>,
}

impl App {
    fn new(backend: Backend, prefs: Prefs) -> Self {
        Self { backend, prefs, screen: Screen::Starting, client: None, devices: Vec::new() }
    }

    fn login_screen(&self, error: Option<String>) -> Screen {
        Screen::Login(login::LoginForm::new(&self.prefs, error))
    }

    fn connect(&mut self, client: Arc<CloudClient>, device: DeviceInfo) {
        self.prefs.device = Some(device.did.clone());
        self.prefs.save(self.backend.paths());
        self.screen = Screen::Connecting { name: device.name.clone() };
        self.backend.connect(client, device);
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::NeedLogin(why) => self.screen = self.login_screen(why),
            Msg::LoginFailed(e) => self.screen = self.login_screen(Some(e)),
            Msg::Devices { client, devices } => {
                self.client = Some(client.clone());
                self.devices = devices.clone();
                let remembered = self.prefs.device.as_ref().and_then(|d| devices.iter().find(|x| &x.did == d)).cloned();
                match (remembered, devices.len()) {
                    (Some(d), _) => self.connect(client, d),
                    (None, 1) => self.connect(client, devices[0].clone()),
                    _ => self.screen = Screen::Devices { client, devices },
                }
            }
            Msg::Connected { robot, status, discover } => {
                let name = self.device_name(&robot);
                let mut h = home::Home::new(robot.clone(), name, *status, self.devices.len() > 1, &self.prefs);
                self.backend.load_map(&robot);
                self.backend.start_live(&robot, dreame::mapedit::MAP_VERSION);
                // The first status came from the cloud cache; ask the robot itself too.
                self.backend.refresh(&robot);
                if discover {
                    self.backend.discover(&robot);
                }
                h.loading_map = true;
                self.screen = Screen::Home(Box::new(h));
            }
            Msg::Discovered(robot) => {
                if let Screen::Home(h) = &mut self.screen {
                    if h.set_robot(robot.clone()) {
                        self.backend.start_live(&robot, dreame::mapedit::MAP_VERSION);
                        self.backend.refresh(&robot);
                    }
                }
            }
            Msg::ConnectFailed(e) => {
                self.screen = Screen::Failed { message: trf("Could not reach the robot.\n\n{error}", &[("error", &e)]) };
            }
            other => {
                if let Screen::Home(h) = &mut self.screen {
                    h.handle(other, &self.backend);
                }
            }
        }
    }

    fn device_name(&self, robot: &SharedRobot) -> String {
        let d = &robot.cloud_link().device;
        if d.name.trim().is_empty() {
            d.model.clone()
        } else {
            d.name.clone()
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Some(m) = self.backend.poll() {
            self.handle(m);
        }
        let mut next: Option<Screen> = None;
        match &mut self.screen {
            Screen::Starting => centered(ui, |ui| {
                ui.spinner();
                ui.label(tr("Signing in"));
            }),
            Screen::Connecting { name } => {
                let name = name.clone();
                centered(ui, |ui| {
                    ui.spinner();
                    ui.label(trf("Connecting to {name}", &[("name", &name)]));
                });
            }
            Screen::Login(form) => {
                // The language can be chosen before signing in; the settings have it later.
                egui::Panel::bottom("language").show_separator_line(false).show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(tr("Language"));
                        if language_picker(ui, &mut self.prefs.language) {
                            apply_language(self.prefs.language.as_deref());
                            self.prefs.save(self.backend.paths());
                        }
                    });
                    ui.add_space(4.0);
                });
                if let Some((email, password, region)) = form.ui(ui) {
                    self.prefs.email = Some(email.trim().to_string());
                    self.prefs.region = region.clone();
                    self.prefs.save(self.backend.paths());
                    self.backend.login(email, password, region);
                }
            }
            Screen::Devices { client, devices } => {
                if let Some(d) = login::device_picker(ui, devices) {
                    let (c, d) = (client.clone(), d.clone());
                    self.connect(c, d);
                }
            }
            Screen::Failed { message } => {
                let message = message.clone();
                let mut retry = false;
                let mut sign_out = false;
                centered(ui, |ui| {
                    ui.label(message);
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        retry = ui.button(tr("Try again")).clicked();
                        sign_out = ui.button(tr("Sign out")).clicked();
                    });
                });
                if retry {
                    next = Some(Screen::Starting);
                    self.backend.resume();
                }
                if sign_out {
                    self.backend.sign_out();
                    next = Some(self.login_screen(None));
                }
            }
            Screen::Home(h) => match h.ui(ui, &self.backend) {
                home::Exit::Stay => {}
                home::Exit::SignOut => {
                    self.backend.sign_out();
                    self.prefs.device = None;
                    self.prefs.save(self.backend.paths());
                    next = Some(self.login_screen(None));
                }
                home::Exit::Prefs { advanced, notify, language } => {
                    self.prefs.advanced = advanced;
                    self.prefs.notify = notify;
                    if language != self.prefs.language {
                        self.prefs.language = language;
                        apply_language(self.prefs.language.as_deref());
                    }
                    self.prefs.save(self.backend.paths());
                }
                home::Exit::SwitchRobot => {
                    self.backend.stop_live();
                    if let Some(c) = self.client.clone() {
                        next = Some(Screen::Devices { client: c, devices: self.devices.clone() });
                    }
                }
            },
        }
        if let Some(s) = next {
            self.screen = s;
        }
    }
}

/// Lays out `add` in the middle of the window.
fn centered(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 40.0).max(0.0));
            add(ui);
        });
    });
}
