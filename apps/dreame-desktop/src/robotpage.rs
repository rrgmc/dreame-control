//! The robot page: settings, do-not-disturb hours, station actions, and the maintenance list of
//! brushes, filters and pads with their remaining life.
//!
//! Every control writes to the robot at once, as in the official app. The page returns what to
//! send; the home screen sends it and shows the result.

use dreame::Setting;
use dreame_i18n::{tr, trf};
use dreame_model::codec::ai_obstacle::{AiFlags, AiObstacle};
use dreame_model::codec::consumables::Consumable;
use dreame_model::codec::station::StationOp;
use dreame_model::codec::HhMm;
use dreame_model::enums::{CarpetSensitivity, CodeEnum};
use dreame_model::{Feature, Status};
use eframe::egui::{self, RichText};

/// What the page asks the home screen to send.
pub enum RobotAction {
    Set(Setting),
    Reset(Consumable),
    Station(StationOp),
    EmptyBin,
}

pub struct RobotPage {
    /// Slider values while they move, by name, so a drag sends one write at the end.
    dragging: Option<(&'static str, i64)>,
    /// The do-not-disturb window being edited: on, start, end.
    dnd: Option<(bool, HhMm, HhMm)>,
    /// A consumable to reset, waiting for confirmation.
    confirm_reset: Option<Consumable>,
}

impl RobotPage {
    pub fn new() -> Self {
        Self { dragging: None, dnd: None, confirm_reset: None }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, s: &Status, has: &dyn Fn(Feature) -> bool, busy: bool) -> Option<RobotAction> {
        let mut action = None;
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    ui.columns(2, |cols| {
                        self.settings(&mut cols[0], s, has, &mut action);
                        self.maintenance(&mut cols[1], s, has, &mut action);
                    });
                });
            });
        });
        if let Some(c) = self.confirm_reset {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::unique("reset")).show(ui.ctx(), |ui| {
                ui.set_width(320.0);
                ui.label(RichText::new(trf("Reset the {part}?", &[("part", &tr(c.label()))])).size(17.0).strong());
                ui.add_space(6.0);
                ui.label(tr("Do this after replacing or cleaning it. Its remaining life goes back to 100%."));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Reset")).clicked() {
                        action = Some(RobotAction::Reset(c));
                        close = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.confirm_reset = None;
            }
        }
        action
    }

    fn settings(&mut self, ui: &mut egui::Ui, s: &Status, has: &dyn Fn(Feature) -> bool, action: &mut Option<RobotAction>) {
        let raw_int = |siid: u32, piid: u32| s.raw.get(&dreame_core::PropId::new(siid, piid)).and_then(|v| v.as_i64());

        heading(ui, tr("Cleaning"));
        if let Some(a) = toggle(ui, s.carpet_boost, tr("Carpet boost"), tr("More suction on carpets."), Setting::CarpetBoost) {
            *action = Some(a);
        }
        if let Some(a) = toggle(ui, s.resume_cleaning, tr("Resume after recharging"), tr("Finish a long job after charging."), Setting::ResumeCleaning) {
            *action = Some(a);
        }
        if let Some(a) = toggle(
            ui,
            raw_int(4, 34).map(|v| v != 0),
            tr("Wash the mop during the job"),
            tr("Return to the station to wash the mop."),
            Setting::SelfClean,
        ) {
            *action = Some(a);
        }
        if let Some(level) = raw_int(4, 28).and_then(CarpetSensitivity::from_code) {
            let mut v = level;
            ui.horizontal(|ui| {
                ui.label(tr("Carpet sensitivity"));
                egui::ComboBox::from_id_salt("carpet").selected_text(crate::phase::capitalise(tr(level.label()))).show_ui(ui, |ui| {
                    for l in CarpetSensitivity::ALL {
                        ui.selectable_value(&mut v, *l, crate::phase::capitalise(tr(l.label())));
                    }
                });
            });
            if v != level {
                *action = Some(RobotAction::Set(Setting::CarpetSensitivity(v)));
            }
        }
        if let Some(h) = raw_int(4, 40) {
            if let Some(v) = self.slider(ui, "drying", tr("Mop drying time (hours)"), h, 1..=12) {
                *action = Some(RobotAction::Set(Setting::DryingTimeHours(v as u8)));
            }
        }

        heading(ui, tr("Robot"));
        if let Some(v) = s.volume {
            if let Some(v) = self.slider(ui, "volume", tr("Voice volume"), v, 0..=100) {
                *action = Some(RobotAction::Set(Setting::Volume(v as u8)));
            }
        }
        if let Some(a) = toggle(ui, s.child_lock, tr("Child lock"), tr("The robot's buttons do nothing."), Setting::ChildLock) {
            *action = Some(a);
        }

        if has(Feature::Dnd) || s.dnd.enabled.is_some() {
            heading(ui, tr("Do not disturb"));
            if let (Some(on), Some(start), Some(end)) = (s.dnd.enabled, s.dnd.start, s.dnd.end) {
                let (mut on2, mut a, mut b) = self.dnd.unwrap_or((on, start, end));
                ui.checkbox(&mut on2, tr("No cleaning or voice prompts in these hours"));
                ui.horizontal(|ui| {
                    ui.label(tr("From"));
                    time_edit(ui, "dnd-start", &mut a);
                    ui.label(tr("to"));
                    time_edit(ui, "dnd-end", &mut b);
                });
                let changed = (on2, a, b) != (on, start, end);
                self.dnd = changed.then_some((on2, a, b));
                if changed {
                    ui.horizontal(|ui| {
                        if ui.button(tr("Save")).clicked() {
                            *action = Some(RobotAction::Set(Setting::Dnd { enabled: on2, start: a, end: b }));
                            self.dnd = None;
                        }
                        if ui.button(tr("Undo")).clicked() {
                            self.dnd = None;
                        }
                    });
                }
            }
        }

        if let Some(AiObstacle::Bits(f)) = &s.ai_obstacle {
            heading(ui, tr("Obstacle avoidance"));
            for (bit, name) in [
                (AiFlags::OBSTACLE, tr("Avoid obstacles seen by the camera")),
                (AiFlags::FURNITURE, tr("Recognise furniture")),
                (AiFlags::FLUID, tr("Detect spilled liquids")),
                (AiFlags::PET, tr("Recognise pets")),
                (AiFlags::PET_AVOIDANCE, tr("Keep away from pet waste")),
                (AiFlags::OBSTACLE_PICTURE, tr("Keep pictures of obstacles")),
            ] {
                let mut on = f.has(bit);
                if ui.checkbox(&mut on, name).changed() {
                    *action = Some(RobotAction::Set(Setting::AiFlag { bit, on }));
                }
            }
        }
    }

    fn maintenance(&mut self, ui: &mut egui::Ui, s: &Status, has: &dyn Fn(Feature) -> bool, action: &mut Option<RobotAction>) {
        let raw_int = |siid: u32, piid: u32| s.raw.get(&dreame_core::PropId::new(siid, piid)).and_then(|v| v.as_i64());
        if has(Feature::AutoEmptyBase) || has(Feature::SelfWashBase) {
            heading(ui, tr("Station"));
            ui.horizontal_wrapped(|ui| {
                if has(Feature::AutoEmptyBase) && ui.button(tr("Empty the bin")).clicked() {
                    *action = Some(RobotAction::EmptyBin);
                }
                if has(Feature::SelfWashBase) {
                    if ui.button(tr("Wash the mop")).clicked() {
                        *action = Some(RobotAction::Station(StationOp::ReturnAndWash));
                    }
                    if ui.button(tr("Dry the mop")).clicked() {
                        *action = Some(RobotAction::Station(StationOp::StartDrying));
                    }
                    if ui.button(tr("Stop drying")).clicked() {
                        *action = Some(RobotAction::Station(StationOp::StopDrying));
                    }
                }
            });
            if let Some(n) = raw_int(15, 2) {
                if let Some(v) = self.slider(ui, "autoempty", tr("Empty the bin every … cleanings"), n, 1..=10) {
                    *action = Some(RobotAction::Set(Setting::AutoEmptyFrequency(v as u8)));
                }
            }
        }

        heading(ui, tr("Maintenance"));
        if s.consumables.is_empty() {
            ui.label(RichText::new(tr("Not read yet.")).weak());
        }
        egui::Grid::new("consumables").spacing([12.0, 8.0]).show(ui, |ui| {
            for (c, life) in &s.consumables {
                ui.label(crate::phase::capitalise(tr(c.label())));
                match life.percent {
                    Some(p) => {
                        let p = p.clamp(0, 100);
                        let bar = egui::ProgressBar::new(p as f32 / 100.0).desired_width(120.0).text(format!("{p}%"));
                        let bar = if p <= 10 { bar.fill(ui.visuals().error_fg_color) } else { bar };
                        ui.add(bar);
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.label(life.time_left.map(|h| trf("{hours} h left", &[("hours", &h)])).unwrap_or_default());
                if ui.small_button(tr("Reset")).clicked() {
                    self.confirm_reset = Some(*c);
                }
                ui.end_row();
            }
        });
    }

    /// A slider that reports its value once, when the user lets go.
    fn slider(&mut self, ui: &mut egui::Ui, key: &'static str, label: &str, current: i64, range: std::ops::RangeInclusive<i64>) -> Option<i64> {
        let mut v = self.dragging.filter(|d| d.0 == key).map_or(current, |d| d.1);
        let r = ui.add(egui::Slider::new(&mut v, range).text(label));
        if r.dragged() {
            self.dragging = Some((key, v));
            return None;
        }
        if self.dragging.is_some_and(|d| d.0 == key) && !r.dragged() {
            self.dragging = None;
        }
        (r.drag_stopped() || r.changed()).then_some(v).filter(|v| *v != current)
    }
}

/// A checkbox for an on/off setting; nothing when the value was not read.
fn toggle(ui: &mut egui::Ui, value: Option<bool>, label: &str, hint: &str, make: fn(bool) -> Setting) -> Option<RobotAction> {
    let mut on = value?;
    ui.checkbox(&mut on, label).on_hover_text(hint).changed().then(|| RobotAction::Set(make(on)))
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(2.0);
}

/// Hours and minutes as two drag fields.
fn time_edit(ui: &mut egui::Ui, id: &str, t: &mut HhMm) {
    ui.push_id(id, |ui| {
        let mut h = t.hour;
        let mut m = t.minute;
        ui.add(egui::DragValue::new(&mut h).range(0..=23).custom_formatter(|v, _| format!("{v:02}")));
        ui.label(":");
        ui.add(egui::DragValue::new(&mut m).range(0..=59).speed(0.5).custom_formatter(|v, _| format!("{v:02}")));
        *t = HhMm { hour: h, minute: m };
    });
}
