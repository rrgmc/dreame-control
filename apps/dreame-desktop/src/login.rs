//! The sign-in form and the robot picker.

use crate::config::Prefs;
use dreame_core::DeviceInfo;
use dreame_i18n::{tr, trf};
use eframe::egui::{self, RichText};

/// Cloud region codes, in the order the form lists them. `None`, "Automatic", tries each one.
const REGIONS: [Option<&str>; 8] = [None, Some("us"), Some("eu"), Some("sg"), Some("cn"), Some("ru"), Some("kr"), Some("by")];

/// A region as the user knows it. The region belongs to the account, not to where the user
/// lives.
fn region_name(code: Option<&str>) -> &'static str {
    match code {
        None => tr("Automatic"),
        Some("us") => tr("Americas"),
        Some("eu") => tr("Europe"),
        Some("sg") => tr("Asia Pacific"),
        Some("cn") => tr("China"),
        Some("ru") => tr("Russia"),
        Some("kr") => tr("Korea"),
        Some("by") => tr("Belarus"),
        Some(_) => tr("Other"),
    }
}

pub struct LoginForm {
    email: String,
    password: String,
    region: usize,
    error: Option<String>,
    busy: bool,
}

impl LoginForm {
    pub fn new(prefs: &Prefs, error: Option<String>) -> Self {
        let region = prefs
            .region
            .as_deref()
            .and_then(|r| REGIONS.iter().position(|c| *c == Some(r)))
            .unwrap_or(0);
        Self { email: prefs.email.clone().unwrap_or_default(), password: String::new(), region, error, busy: false }
    }

    /// Draws the form. Returns email, password and region code when the user signs in.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<(String, String, Option<String>)> {
        let mut submit = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 190.0).max(12.0));
                ui.label(RichText::new("Dreame Control").size(28.0).strong());
                ui.label(tr("Sign in with your Dreamehome account"));
                ui.add_space(18.0);
                let width = 320.0;
                ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                    ui.set_width(width);
                    ui.label(tr("Email"));
                    let email = ui.add(egui::TextEdit::singleline(&mut self.email).desired_width(width).hint_text(tr("name@example.com")));
                    ui.add_space(6.0);
                    ui.label(tr("Password"));
                    let pw = ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(width));
                    ui.add_space(6.0);
                    ui.label(tr("Account region"));
                    egui::ComboBox::from_id_salt("region").width(width).selected_text(region_name(REGIONS[self.region])).show_ui(ui, |ui| {
                        for (i, code) in REGIONS.iter().enumerate() {
                            ui.selectable_value(&mut self.region, i, region_name(*code));
                        }
                    });
                    ui.add_space(14.0);
                    let ready = !self.busy && self.email.contains('@') && !self.password.is_empty();
                    let enter = (email.lost_focus() || pw.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let button = egui::Button::new(RichText::new(if self.busy { tr("Signing in") } else { tr("Sign in") }).size(16.0))
                        .min_size(egui::vec2(width, 36.0));
                    if (ui.add_enabled(ready, button).clicked() || (enter && ready)) && !self.busy {
                        submit = true;
                    }
                    if self.busy {
                        ui.spinner();
                    }
                    if let Some(e) = &self.error {
                        ui.add_space(8.0);
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(tr(
                            "Your password goes only to the Dreame cloud and is not saved. The app keeps a \
                             sign-in token so you stay signed in.",
                        ))
                        .small()
                        .weak(),
                    );
                });
            });
        });
        if submit {
            self.busy = true;
            self.error = None;
            let region = REGIONS[self.region].map(str::to_string);
            return Some((self.email.clone(), std::mem::take(&mut self.password), region));
        }
        None
    }
}

/// Lists the account's robots. Returns the one clicked.
pub fn device_picker<'a>(ui: &mut egui::Ui, devices: &'a [DeviceInfo]) -> Option<&'a DeviceInfo> {
    let mut chosen = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(RichText::new(tr("Choose a robot")).size(24.0).strong());
            ui.add_space(16.0);
            for d in devices {
                let name = if d.name.trim().is_empty() { d.model.as_str() } else { d.name.as_str() };
                let text = if d.online {
                    format!("{name}\n{}", d.model)
                } else {
                    format!("{name}\n{}", trf("{model}  (offline)", &[("model", &d.model)]))
                };
                let b = egui::Button::new(RichText::new(text).size(15.0)).min_size(egui::vec2(320.0, 56.0));
                if ui.add(b).clicked() {
                    chosen = Some(d);
                }
                ui.add_space(6.0);
            }
        });
    });
    chosen
}
