//! The history page: past cleaning jobs, newest first, and the map of the one picked, with the
//! path the robot took.

use crate::mapview::{Drag, MapView, Overlay};
use dreame::history::{self, Job};
use dreame_i18n::{tr, trf, trn, Language};
use dreame_map::Map;
use eframe::egui::{self, RichText};

/// What the page asks the home screen to load.
pub enum HistoryAction {
    /// Load the newest `limit` jobs.
    Load(u32),
    /// Load the map of this job.
    Map(Box<Job>),
}

/// How many jobs to load at first, and how many more each time.
pub const PAGE: u32 = 50;

pub struct HistoryPage {
    pub jobs: Option<Result<Vec<Job>, String>>,
    limit: u32,
    selected: Option<usize>,
    /// The map of the selected job: its start time (to match the job) and the map or why not.
    map: Option<(Option<i64>, Result<Map, String>)>,
    view: MapView,
}

impl HistoryPage {
    pub fn new() -> Self {
        Self { jobs: None, limit: PAGE, selected: None, map: None, view: MapView::default() }
    }

    /// Takes a job's map. Ignored when another job was picked meanwhile.
    pub fn set_map(&mut self, started: Option<i64>, map: Result<Map, String>) {
        let current = self.selected.and_then(|i| self.jobs.as_ref()?.as_ref().ok()?.get(i)).map(Job::started);
        if current == Some(started) {
            if let Ok(m) = &map {
                self.view.set_map(m);
            }
            self.map = Some((started, map));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<HistoryAction> {
        let mut action = None;
        egui::Panel::left("jobs").resizable(false).exact_size(330.0).show(ui, |ui| {
            match &self.jobs {
                None => {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Loading the history"));
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, trf("The history could not be loaded: {error}", &[("error", e)]));
                    if ui.button(tr("Try again")).clicked() {
                        action = Some(HistoryAction::Load(self.limit));
                    }
                }
                Some(Ok(jobs)) if jobs.is_empty() => {
                    ui.label(tr("No cleaning jobs yet."));
                }
                Some(Ok(jobs)) => {
                    let total_area: i64 = jobs.iter().filter_map(|j| j.record.cleaned_area_m2).sum();
                    let total_min: i64 = jobs.iter().filter_map(|j| j.record.cleaning_time_min).sum();
                    ui.add_space(4.0);
                    let total = trn(
                        "{n} job: {area} m² in {hours} h {minutes} min",
                        "{n} jobs: {area} m² in {hours} h {minutes} min",
                        jobs.len() as i64,
                        &[("area", &total_area), ("hours", &(total_min / 60)), ("minutes", &(total_min % 60))],
                    );
                    ui.label(RichText::new(total).weak());
                    ui.add_space(4.0);
                    let mut picked = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, j) in jobs.iter().enumerate() {
                            let r = &j.record;
                            let when = j.started().map(local_time).unwrap_or_else(|| tr("Unknown time").into());
                            let what = format!(
                                "{}  ·  {}  ·  {}",
                                r.cleaning_time_min.map(|m| format!("{m} min")).unwrap_or_default(),
                                r.cleaned_area_m2.map(|a| format!("{a} m²")).unwrap_or_default(),
                                crate::phase::capitalise(tr(history::outcome(r)))
                            );
                            let text = RichText::new(format!("{when}\n{what}")).size(14.0);
                            let b = egui::Button::selectable(self.selected == Some(i), text).min_size(egui::vec2(310.0, 44.0));
                            if ui.add(b).clicked() {
                                picked = Some(i);
                            }
                        }
                        if jobs.len() as u32 >= self.limit {
                            ui.add_space(6.0);
                            if ui.button(tr("Show older jobs")).clicked() {
                                self.limit += PAGE;
                                action = Some(HistoryAction::Load(self.limit));
                            }
                        }
                    });
                    if let Some(i) = picked {
                        self.selected = Some(i);
                        self.map = None;
                        action = Some(HistoryAction::Map(Box::new(jobs[i].clone())));
                    }
                }
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| match &self.map {
            Some((_, Ok(map))) => {
                let overlay = Overlay {
                    path: &map.path,
                    pose: None,
                    picked: &[],
                    zone: None,
                    drag: Drag::Pan,
                    marks: Vec::new(),
                    hidden_labels: false,
                };
                self.view.show(ui, map, &map.active_rooms, &overlay);
            }
            Some((_, Err(e))) => {
                ui.centered_and_justified(|ui| ui.label(trf("This job's map could not be loaded: {error}", &[("error", e)])));
            }
            None if self.selected.is_some() => {
                ui.centered_and_justified(|ui| ui.spinner());
            }
            None => {
                ui.centered_and_justified(|ui| ui.label(RichText::new(tr("Pick a job to see where the robot went.")).weak()));
            }
        });
        action
    }
}

/// A Unix time as a local date and time in the form the language uses: "Mon 17 Nov 2026,
/// 16:58" in English, "17/11/2026 16:58" in Brazilian Portuguese.
pub fn local_time(t: i64) -> String {
    use chrono::TimeZone;
    let format = match dreame_i18n::language() {
        Language::English => "%a %e %b %Y, %H:%M",
        Language::PortugueseBrazil => "%d/%m/%Y %H:%M",
    };
    match chrono::Local.timestamp_opt(t, 0).single() {
        Some(d) => d.format(format).to_string(),
        None => history::utc(t),
    }
}
