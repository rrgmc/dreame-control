//! The photos page: the obstacles and pets the robot's camera recognised, from the live map,
//! with the photo the robot took of each. Read-only.

use dreame_i18n::{tr, trf};
use dreame_map::obstacle::Obstacle;
use eframe::egui::{self, ColorImage, RichText, TextureHandle, TextureOptions};
use std::collections::BTreeMap;

/// A fetched photo, or why it could not be.
enum Photo {
    Shown(TextureHandle),
    Failed(String),
}

pub enum PhotosAction {
    Reload,
}

#[derive(Default)]
pub struct PhotosPage {
    /// `None` while loading.
    pub obstacles: Option<Result<Vec<Obstacle>, String>>,
    photos: BTreeMap<usize, Photo>,
    /// The photo shown large.
    selected: Option<usize>,
}

impl PhotosPage {
    pub fn set_obstacles(&mut self, list: Result<Vec<Obstacle>, String>) {
        self.photos.clear();
        self.selected = None;
        self.obstacles = Some(list);
    }

    pub fn set_photo(&mut self, ctx: &egui::Context, index: usize, result: Result<Vec<u8>, String>) {
        let photo = match result.and_then(|jpeg| decode(&jpeg)) {
            Ok(img) => Photo::Shown(ctx.load_texture(format!("obstacle-{index}"), img, TextureOptions::LINEAR)),
            Err(e) => Photo::Failed(e),
        };
        self.photos.insert(index, photo);
        if self.selected.is_none() && matches!(self.photos.get(&index), Some(Photo::Shown(_))) {
            self.selected = Some(index);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<PhotosAction> {
        let mut action = None;
        egui::Panel::left("photo-list").resizable(false).exact_size(300.0).show(ui, |ui| {
            ui.add_space(8.0);
            if ui.button(tr("Reload")).clicked() {
                action = Some(PhotosAction::Reload);
            }
            ui.add_space(6.0);
            match &self.obstacles {
                None => {
                    ui.spinner();
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, trf("The obstacles could not be read: {error}", &[("error", e)]));
                }
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(tr("The robot recognised no obstacles on its last run."));
                }
                Some(Ok(list)) => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, o) in list.iter().enumerate() {
                            let thumb = match self.photos.get(&i) {
                                Some(Photo::Shown(t)) => Some(t.clone()),
                                _ => None,
                            };
                            ui.horizontal(|ui| {
                                let size = egui::vec2(96.0, 72.0);
                                match &thumb {
                                    Some(t) => {
                                        let img = egui::Image::new(t).fit_to_exact_size(size).sense(egui::Sense::click());
                                        if ui.add(img).clicked() {
                                            self.selected = Some(i);
                                        }
                                    }
                                    None => {
                                        ui.allocate_ui(size, |ui| match self.photos.get(&i) {
                                            Some(Photo::Failed(_)) => {
                                                ui.label(RichText::new(tr("No photo")).weak());
                                            }
                                            _ if o.photo.is_none() => {
                                                ui.label(RichText::new(tr("No photo")).weak());
                                            }
                                            _ => {
                                                ui.spinner();
                                            }
                                        });
                                    }
                                }
                                ui.vertical(|ui| {
                                    let mut title = RichText::new(label(o)).strong();
                                    if self.selected == Some(i) {
                                        title = title.color(crate::ACCENT);
                                    }
                                    ui.label(title);
                                    ui.label(RichText::new(trf("{pct}% sure", &[("pct", &format!("{:.0}", o.confidence * 100.0))])).small().weak());
                                    if let Some(Photo::Failed(e)) = self.photos.get(&i) {
                                        ui.label(RichText::new(e).small().weak()).on_hover_text(e);
                                    }
                                });
                            });
                            ui.add_space(4.0);
                        }
                    });
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| match self.selected.and_then(|i| self.photos.get(&i)) {
            Some(Photo::Shown(t)) => {
                ui.centered_and_justified(|ui| {
                    ui.add(egui::Image::new(t).shrink_to_fit());
                });
            }
            _ => {
                ui.centered_and_justified(|ui| ui.label(RichText::new(tr("Pick an obstacle to see its photo.")).weak()));
            }
        });
        action
    }
}

fn label(o: &Obstacle) -> String {
    if o.is_pet() {
        tr("Pet").into()
    } else {
        trf("Obstacle (type {kind})", &[("kind", &o.kind)])
    }
}

fn decode(jpeg: &[u8]) -> Result<ColorImage, String> {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}
