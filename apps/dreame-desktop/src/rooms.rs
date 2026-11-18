//! The room settings page: each room's suction, water, passes and mode, the cleaning order,
//! and which rooms show. Like the official app's "Customize room cleaning".
//!
//! A room setting, a hidden room or a new order is a map edit ([`Edit`]); the page asks the home
//! screen to save it and shows which room is saving.

use dreame::mapedit::Edit;
use dreame_i18n::{tr, trf};
use dreame_map::Map;
use eframe::egui::{self, RichText};

/// What the page asks the home screen to do.
pub enum RoomAction {
    /// Save a map edit; `what` names it for the status line.
    Save { edit: Edit, what: String },
    /// Turn per-room settings on or off (4.26).
    Customized(bool),
}

pub struct RoomsPage {
    /// Rooms in the order shown, which is the cleaning order once saved.
    order: Vec<u32>,
    order_changed: bool,
    /// The room whose change is saving, so its row shows a spinner.
    pub saving: Option<u32>,
    /// A wetness value while the slider moves: room and value.
    dragging: Option<(u32, i64)>,
}

/// Suction levels as the app names them.
pub fn suction_names() -> [&'static str; 4] {
    [tr("Quiet"), tr("Standard"), tr("Strong"), tr("Max")]
}
/// Water levels on robots without the 1 to 32 wetness scale.
fn water_names() -> [&'static str; 3] {
    [tr("Low"), tr("Medium"), tr("High")]
}
/// `cleanType` values.
pub fn mode_names() -> [&'static str; 3] {
    [tr("Vacuum"), tr("Mop"), tr("Vacuum and mop")]
}

impl RoomsPage {
    pub fn new(map: &Map) -> Self {
        Self { order: dreame::shapes::initial_order(map), order_changed: false, saving: None, dragging: None }
    }

    /// Takes a new map after a save: the order follows it unless the user is reordering.
    pub fn set_map(&mut self, map: &Map) {
        if !self.order_changed {
            self.order = dreame::shapes::initial_order(map);
        }
        self.saving = None;
    }

    /// Draws the page. `customized` is 4.26 (`None` when not known), `wetness` whether water is
    /// the 1 to 32 scale, and `busy` true while anything saves.
    pub fn ui(&mut self, ui: &mut egui::Ui, map: &Map, customized: Option<bool>, wetness: bool, busy: bool) -> Option<RoomAction> {
        let mut action = None;
        let (suction_names, water_names, mode_names) = (suction_names(), water_names(), mode_names());
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(on) = customized {
                let mut v = on;
                if ui.add_enabled(!busy, egui::Checkbox::new(&mut v, RichText::new(tr("Clean each room with its own settings")).size(15.0))).changed() {
                    action = Some(RoomAction::Customized(v));
                }
                ui.label(
                    RichText::new(if on {
                        tr("On: each room uses the settings below. The mode, suction and water on the home screen apply to zones.")
                    } else {
                        tr("Off: every room uses the mode, suction and water chosen on the home screen. Passes and order below still apply.")
                    })
                    .weak(),
                );
                ui.add_space(10.0);
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("rooms").striped(true).spacing([16.0, 10.0]).min_col_width(40.0).show(ui, |ui| {
                    for h in [tr("Order"), tr("Room"), tr("Suction"), tr("Water"), tr("Passes"), tr("Mode"), ""] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    let mut moves = None;
                    let n = self.order.len();
                    for (i, id) in self.order.clone().into_iter().enumerate() {
                        let Some(room) = map.rooms.get(&id) else { continue };
                        ui.horizontal(|ui| {
                            ui.label(format!("{}", i + 1));
                            if ui.add_enabled(!busy && i > 0, egui::Button::new("▲").small()).on_hover_text(tr("Clean earlier")).clicked() {
                                moves = Some((i, true));
                            }
                            if ui.add_enabled(!busy && i + 1 < n, egui::Button::new("▼").small()).on_hover_text(tr("Clean later")).clicked() {
                                moves = Some((i, false));
                            }
                        });
                        ui.label(RichText::new(room.display_name()).size(15.0));
                        let s = room.settings.clone().unwrap_or_default();
                        if s.len() < 3 {
                            ui.label(RichText::new(tr("No stored settings")).weak());
                            ui.end_row();
                            continue;
                        }
                        let rname = room.display_name();
                        let save = |what: String, edit: Edit| RoomAction::Save { edit, what };
                        let blank = Edit::RoomSettings { id, suction: None, water: None, times: None, clean_type: None, mop_effect: None };
                        let with = |f: &dyn Fn(&mut Edit)| {
                            let mut e = blank.clone();
                            f(&mut e);
                            e
                        };
                        ui.add_enabled_ui(!busy, |ui| {
                            // Suction.
                            let cur = s[0].clamp(0, 3) as usize;
                            let mut v = cur;
                            egui::ComboBox::from_id_salt(("suction", id)).selected_text(suction_names[cur]).show_ui(ui, |ui| {
                                for (k, name) in suction_names.iter().enumerate() {
                                    ui.selectable_value(&mut v, k, *name);
                                }
                            });
                            if v != cur {
                                action = Some(save(trf("Saving the suction for {room}", &[("room", &rname)]), with(&|e| set(e, Some(v as i64), None, None, None))));
                            }
                        });
                        ui.add_enabled_ui(!busy, |ui| {
                            // Water: wetness 1 to 32, or three steps. r2579a stores a 1 to 3
                            // level here although it has the wetness scale (device-model.md
                            // §4.10, note B), so a stored 1 to 3 shows as three steps.
                            if wetness && s[1] > 3 {
                                let mut v = self.dragging.filter(|d| d.0 == id).map_or(s[1], |d| d.1).clamp(1, 32);
                                let r = ui.add(egui::Slider::new(&mut v, 1..=32));
                                if r.dragged() {
                                    self.dragging = Some((id, v));
                                }
                                if (r.drag_stopped() || (r.changed() && !r.dragged())) && v != s[1] {
                                    self.dragging = None;
                                    action = Some(save(trf("Saving the water for {room}", &[("room", &rname)]), with(&|e| set(e, None, Some(v), None, None))));
                                }
                            } else {
                                let cur = (s[1].clamp(1, 3) - 1) as usize;
                                let mut v = cur;
                                egui::ComboBox::from_id_salt(("water", id)).selected_text(water_names[cur]).show_ui(ui, |ui| {
                                    for (k, name) in water_names.iter().enumerate() {
                                        ui.selectable_value(&mut v, k, *name);
                                    }
                                });
                                if v != cur {
                                    action = Some(save(trf("Saving the water for {room}", &[("room", &rname)]), with(&|e| set(e, None, Some(v as i64 + 1), None, None))));
                                }
                            }
                        });
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.horizontal(|ui| {
                                for t in 1..=3i64 {
                                    if ui.selectable_label(s[2] == t, format!("{t}×")).clicked() && s[2] != t {
                                        action = Some(save(trf("Saving the passes for {room}", &[("room", &rname)]), with(&|e| set(e, None, None, Some(t), None))));
                                    }
                                }
                            });
                        });
                        ui.add_enabled_ui(!busy, |ui| match s.get(4).copied() {
                            Some(m) if (0..3).contains(&m) => {
                                let cur = m as usize;
                                let mut v = cur;
                                egui::ComboBox::from_id_salt(("mode", id)).selected_text(mode_names[cur]).show_ui(ui, |ui| {
                                    for (k, name) in mode_names.iter().enumerate() {
                                        ui.selectable_value(&mut v, k, *name);
                                    }
                                });
                                if v != cur {
                                    action = Some(save(trf("Saving the mode for {room}", &[("room", &rname)]), with(&|e| set(e, None, None, None, Some(v as i64)))));
                                }
                            }
                            _ => {
                                ui.label("");
                            }
                        });
                        if self.saving == Some(id) {
                            ui.spinner();
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                    if let Some((i, up)) = moves {
                        dreame::shapes::move_entry(&mut self.order, i, up);
                        self.order_changed = true;
                    }
                });
                if self.order_changed {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let b = egui::Button::new(RichText::new(tr("Save the order")).color(egui::Color32::WHITE)).fill(crate::ACCENT);
                        if ui.add_enabled(!busy, b).clicked() {
                            self.order_changed = false;
                            action = Some(RoomAction::Save { edit: Edit::CleaningOrder(self.order.clone()), what: tr("Saving the cleaning order").into() });
                        }
                        if ui.add_enabled(!busy, egui::Button::new(tr("Undo"))).clicked() {
                            self.order_changed = false;
                            self.order = dreame::shapes::initial_order(map);
                        }
                    });
                }

                // Hidden rooms: the robot skips them and the app does not show them.
                let hidden: Vec<u32> = map.hidden_rooms();
                ui.add_space(16.0);
                ui.label(RichText::new(tr("Shown rooms")).strong());
                ui.label(RichText::new(tr("A hidden room is not cleaned and does not show on the map.")).weak());
                ui.horizontal_wrapped(|ui| {
                    for room in map.rooms.values() {
                        let mut shown = !hidden.contains(&room.id);
                        if ui.add_enabled(!busy, egui::Checkbox::new(&mut shown, room.display_name())).changed() {
                            let ids = dreame::shapes::toggled_hidden(map, room.id, !shown);
                            let what = if shown {
                                trf("Showing {room}", &[("room", &room.display_name())])
                            } else {
                                trf("Hiding {room}", &[("room", &room.display_name())])
                            };
                            action = Some(RoomAction::Save { edit: Edit::HideRooms(ids), what });
                        }
                    }
                });
            });
        });
        if let Some(RoomAction::Save { edit: Edit::RoomSettings { id, .. }, .. }) = &action {
            self.saving = Some(*id);
        }
        action
    }
}

/// Sets the given fields of a room settings edit.
fn set(e: &mut Edit, suction: Option<i64>, water: Option<i64>, times: Option<i64>, clean_type: Option<i64>) {
    if let Edit::RoomSettings { suction: s, water: w, times: t, clean_type: c, .. } = e {
        *s = suction.or(*s);
        *w = water.or(*w);
        *t = times.or(*t);
        *c = clean_type.or(*c);
    }
}
