//! The schedules page: timed cleanings of chosen rooms, each room with its own passes,
//! suction, mode and mop wetness, as the official app's "Custom" schedules.
//!
//! The robot keeps all schedules in one property, so adding, changing, switching or deleting
//! one writes the whole list. Schedules of other kinds (such as CleanGenius) are listed and can
//! be switched on, off or deleted, but are edited in the official app.

use crate::rooms::{mode_names, suction_names};
use dreame_i18n::{tr, trf};
use dreame_map::Map;
use dreame_model::codec::schedule::{days_from_mask, mask_from_days, RoomTask, ScheduleSlot};
use dreame_model::codec::HhMm;
use eframe::egui::{self, Color32, RichText};

/// What the page asks the home screen to do.
pub enum ScheduleAction {
    Load,
    /// Write this list; `what` names the change.
    Save { slots: Vec<ScheduleSlot>, what: String },
}

/// Day names as the page shows them, Monday first. The robot's weekday field starts on Sunday
/// (verified on r2579a: its 5th character is Thursday); see [`days_from_mask`].
fn day_names() -> [&'static str; 7] {
    [tr("Mon"), tr("Tue"), tr("Wed"), tr("Thu"), tr("Fri"), tr("Sat"), tr("Sun")]
}

/// A schedule being added or changed.
struct Draft {
    /// The slot it replaces; `None` for a new one.
    index: Option<usize>,
    time: HhMm,
    days: [bool; 7],
    weekly: bool,
    tasks: Vec<RoomTask>,
}

pub struct SchedulePage {
    pub slots: Option<Result<Vec<ScheduleSlot>, String>>,
    draft: Option<Draft>,
    confirm_delete: Option<usize>,
}

impl SchedulePage {
    pub fn new() -> Self {
        Self { slots: None, draft: None, confirm_delete: None }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, map: Option<&Map>, busy: bool) -> Option<ScheduleAction> {
        let mut action = None;
        let name = |id: u8| {
            map.and_then(|m| m.rooms.get(&u32::from(id)))
                .map(|r| r.display_name())
                .unwrap_or_else(|| trf("Room {id}", &[("id", &id)]))
        };
        let suction = suction_names();
        egui::CentralPanel::default().show(ui, |ui| {
            let slots = match &self.slots {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Reading the schedules"));
                    });
                    return;
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, trf("The schedules could not be read: {error}", &[("error", e)]));
                    if ui.button(tr("Try again")).clicked() {
                        action = Some(ScheduleAction::Load);
                    }
                    return;
                }
                Some(Ok(s)) => s.clone(),
            };
            ui.add_enabled_ui(!busy && self.draft.is_none(), |ui| {
                let b = egui::Button::new(RichText::new(format!("+ {}", tr("Add a schedule"))).color(Color32::WHITE)).fill(crate::ACCENT);
                if ui.add(b).clicked() {
                    self.draft = Some(Draft {
                        index: None,
                        time: HhMm { hour: 10, minute: 0 },
                        days: [true; 7],
                        weekly: true,
                        tasks: Vec::new(),
                    });
                }
            });
            ui.add_space(10.0);
            if slots.is_empty() && self.draft.is_none() {
                ui.label(RichText::new(tr("No schedules yet.")).weak());
            }
            for (i, slot) in slots.iter().enumerate().filter(|(_, s)| !s.invalid()) {
                egui::Frame::group(ui.style()).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(slot.time.to_string()).size(22.0).strong());
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.label(days_text(&slot.weekdays, !slot.once()));
                            let what = match slot.room_tasks() {
                                Some(tasks) if !tasks.is_empty() => tasks
                                    .iter()
                                    .map(|t| format!("{} ({}×, {})", name(t.room), t.passes, suction[usize::from(t.suction.min(3))]))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                Some(_) => tr("No rooms").into(),
                                None => tr("Set in the official app").into(),
                            };
                            ui.label(RichText::new(what).weak());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(!busy && self.draft.is_none(), |ui| {
                                if ui.button(tr("Delete")).clicked() {
                                    self.confirm_delete = Some(i);
                                }
                                if slot.room_tasks().is_some() && ui.button(tr("Edit")).clicked() {
                                    let days = days_from_mask(&slot.weekdays);
                                    self.draft = Some(Draft {
                                        index: Some(i),
                                        time: slot.time,
                                        days,
                                        weekly: !slot.once(),
                                        tasks: slot.room_tasks().unwrap_or_default(),
                                    });
                                }
                                let mut on = slot.enabled();
                                if ui.checkbox(&mut on, tr("On")).changed() {
                                    let mut next = slots.clone();
                                    next[i].set_enabled(on);
                                    let time = slot.time.to_string();
                                    let what = if on {
                                        trf("Turning on the {time} schedule", &[("time", &time)])
                                    } else {
                                        trf("Turning off the {time} schedule", &[("time", &time)])
                                    };
                                    action = Some(ScheduleAction::Save { slots: next, what });
                                }
                            });
                        });
                    });
                });
                ui.add_space(6.0);
            }
            if let Some(a) = self.draft_ui(ui, map, &slots, busy, &name) {
                action = Some(a);
            }
        });
        if let Some(i) = self.confirm_delete {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::unique("delete-schedule")).show(ui.ctx(), |ui| {
                ui.set_width(300.0);
                ui.label(RichText::new(tr("Delete this schedule?")).size(17.0).strong());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(tr("Delete")).clicked() {
                        if let Some(Ok(slots)) = &self.slots {
                            let mut next = slots.clone();
                            if i < next.len() {
                                let gone = next.remove(i);
                                action = Some(ScheduleAction::Save { slots: next, what: trf("Deleting the {time} schedule", &[("time", &gone.time)]) });
                            }
                        }
                        close = true;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if close || modal.should_close() {
                self.confirm_delete = None;
            }
        }
        action
    }

    /// The form for a new or changed schedule.
    fn draft_ui(
        &mut self,
        ui: &mut egui::Ui,
        map: Option<&Map>,
        slots: &[ScheduleSlot],
        busy: bool,
        name: &dyn Fn(u8) -> String,
    ) -> Option<ScheduleAction> {
        let d = self.draft.as_mut()?;
        let mut action = None;
        let mut close = false;
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(if d.index.is_some() { tr("Change the schedule") } else { tr("New schedule") }).size(17.0).strong());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(tr("Start at"));
                let (mut h, mut m) = (d.time.hour, d.time.minute);
                ui.add(egui::DragValue::new(&mut h).range(0..=23).custom_formatter(|v, _| format!("{v:02}")));
                ui.label(":");
                ui.add(egui::DragValue::new(&mut m).range(0..=59).speed(0.5).custom_formatter(|v, _| format!("{v:02}")));
                d.time = HhMm { hour: h, minute: m };
            });
            ui.horizontal(|ui| {
                ui.label(tr("Days"));
                for (k, day) in day_names().iter().enumerate() {
                    ui.toggle_value(&mut d.days[k], *day);
                }
                ui.checkbox(&mut d.weekly, tr("Every week"));
            });
            ui.add_space(8.0);
            ui.label(RichText::new(tr("Rooms, in cleaning order")).strong());
            // Rooms on the map that are not in the schedule yet.
            if let Some(m) = map {
                ui.horizontal_wrapped(|ui| {
                    for r in m.rooms.values().filter(|r| !r.hidden) {
                        let Ok(id) = u8::try_from(r.id) else { continue };
                        if d.tasks.iter().any(|t| t.room == id) {
                            continue;
                        }
                        if ui.button(format!("+ {}", r.display_name())).clicked() {
                            d.tasks.push(RoomTask::new(id, 1, 1, 2, 16));
                        }
                    }
                });
            }
            let mut remove = None;
            let (suction, modes) = (suction_names(), mode_names());
            egui::Grid::new("tasks").spacing([12.0, 6.0]).show(ui, |ui| {
                for (k, t) in d.tasks.iter_mut().enumerate() {
                    ui.label(format!("{}. {}", k + 1, name(t.room)));
                    ui.horizontal(|ui| {
                        for p in 1..=3u8 {
                            ui.selectable_value(&mut t.passes, p, format!("{p}×"));
                        }
                    });
                    egui::ComboBox::from_id_salt(("s", k)).selected_text(suction[usize::from(t.suction.min(3))]).show_ui(ui, |ui| {
                        for (v, n) in suction.iter().enumerate() {
                            ui.selectable_value(&mut t.suction, v as u8, *n);
                        }
                    });
                    egui::ComboBox::from_id_salt(("m", k)).selected_text(modes[usize::from(t.mode.min(2))]).show_ui(ui, |ui| {
                        for (v, n) in modes.iter().enumerate() {
                            ui.selectable_value(&mut t.mode, v as u8, *n);
                        }
                    });
                    ui.add_enabled(t.mode != 0, egui::Slider::new(&mut t.wetness, 1..=32).text(tr("wetness")));
                    if ui.small_button(tr("Remove")).clicked() {
                        remove = Some(k);
                    }
                    ui.end_row();
                }
            });
            if let Some(k) = remove {
                d.tasks.remove(k);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ok = !busy && !d.tasks.is_empty() && d.days.iter().any(|x| *x);
                let b = egui::Button::new(RichText::new(tr("Save")).color(Color32::WHITE)).fill(crate::ACCENT);
                if ui.add_enabled(ok, b).clicked() {
                    let weekdays = mask_from_days(&d.days);
                    let mut next = slots.to_vec();
                    let what = match d.index.and_then(|i| next.get_mut(i)) {
                        Some(s) => {
                            s.time = d.time;
                            s.weekdays = weekdays;
                            s.repeat = if d.weekly { "1".into() } else { "0".into() };
                            s.set_room_tasks(&d.tasks);
                            trf("Saving the {time} schedule", &[("time", &d.time)])
                        }
                        None => {
                            let id = next.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                            let mut s = ScheduleSlot::new_rooms(id, d.time, &weekdays, &d.tasks);
                            s.repeat = if d.weekly { "1".into() } else { "0".into() };
                            next.push(s);
                            trf("Adding the {time} schedule", &[("time", &d.time)])
                        }
                    };
                    action = Some(ScheduleAction::Save { slots: next, what });
                    close = true;
                }
                if ui.button(tr("Cancel")).clicked() {
                    close = true;
                }
                if d.tasks.is_empty() {
                    ui.label(RichText::new(tr("Add at least one room.")).weak());
                }
            });
        });
        if close {
            self.draft = None;
        }
        action
    }
}

/// The days as words: "Every day", "Weekdays", "Mon, Wed", with "once" for one-off schedules.
fn days_text(mask: &str, weekly: bool) -> String {
    let d = days_from_mask(mask);
    let on: Vec<&str> = day_names().into_iter().zip(d).filter(|(_, x)| *x).map(|(n, _)| n).collect();
    let days = match d {
        [true, true, true, true, true, true, true] => tr("Every day").to_string(),
        [true, true, true, true, true, false, false] => tr("Weekdays").to_string(),
        [false, false, false, false, false, true, true] => tr("Weekends").to_string(),
        _ if on.is_empty() => tr("No day").to_string(),
        _ => on.join(", "),
    };
    if weekly {
        days
    } else {
        trf("{days}, once", &[("days", &days)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_in_words() {
        // The field starts on Sunday: the official app showed "0000100" as Thursday.
        assert_eq!(days_text("0000100", true), "Thu");
        assert_eq!(days_text("1111111", true), "Every day");
        assert_eq!(days_text("0101000", true), "Mon, Wed");
        assert_eq!(days_text("0111110", true), "Weekdays");
        assert_eq!(days_text("1000001", false), "Weekends, once");
        let fri = [false, false, false, false, true, false, false];
        assert_eq!(mask_from_days(&fri), "0000010");
        assert_eq!(days_from_mask("0000010"), fri);
    }
}
