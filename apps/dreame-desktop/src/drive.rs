//! The drive pad: a small window over the map for driving the robot by hand.
//!
//! Hold a button or an arrow key to move; the step repeats every second, as in the official
//! app, and letting go sends a stop. The robot stops by itself about a second after the last
//! step and leaves remote control within 10 seconds. See docs/research/remote-and-camera.md.

use dreame_i18n::tr;
use dreame_model::commands::{RemoteMove, REMOTE_REPEAT_MS};
use eframe::egui::{self, Key, RichText};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct DrivePad {
    /// The step being held and when it was last sent.
    held: Option<(RemoteMove, Instant)>,
    /// Whether a step was sent since the pad opened; the first plays the robot's voice prompt.
    started: bool,
}

impl DrivePad {
    /// Draws the pad. Returns a step to send now and whether the window stays open.
    /// `docked` adds a warning that the first step takes the robot off its dock.
    pub fn ui(&mut self, ctx: &egui::Context, docked: bool) -> (Option<(RemoteMove, bool)>, bool) {
        let mut open = true;
        let mut pressed = None;
        let mut turn_around = false;
        egui::Window::new(tr("Drive"))
            .id(egui::Id::unique("drive-pad-window"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos([24.0, 160.0])
            .show(ctx, |ui| {
                if docked {
                    ui.label(RichText::new(tr("The robot leaves its dock on the first press.")).weak());
                    ui.add_space(4.0);
                }
                let size = egui::vec2(64.0, 48.0);
                let mut pad = |ui: &mut egui::Ui, label: &str, step: RemoteMove| {
                    // A held press becomes a drag after a moment; sensing drags keeps it on the
                    // button instead of letting the window take it to move itself.
                    let button = egui::Button::new(RichText::new(label).size(22.0)).sense(egui::Sense::click_and_drag());
                    let r = ui.add_sized(size, button);
                    if r.is_pointer_button_down_on() || r.dragged() {
                        pressed = Some(step);
                    }
                };
                egui::Grid::new("drive-pad").spacing([6.0, 6.0]).show(ui, |ui| {
                    ui.label("");
                    pad(ui, "▲", RemoteMove::FORWARD);
                    ui.label("");
                    ui.end_row();
                    pad(ui, "↺", RemoteMove::LEFT);
                    turn_around = ui
                        .add_sized(size, egui::Button::new(RichText::new("⟲").size(22.0)))
                        .on_hover_text(tr("Turn around"))
                        .clicked();
                    pad(ui, "↻", RemoteMove::RIGHT);
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label(RichText::new(tr("Hold a button or an arrow key. There is no reverse.")).small().weak());
            });
        // Arrow keys work while the window is open and no text field has focus.
        if pressed.is_none() && !ctx.egui_wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_down(Key::ArrowUp) {
                    pressed = Some(RemoteMove::FORWARD);
                } else if i.key_down(Key::ArrowLeft) {
                    pressed = Some(RemoteMove::LEFT);
                } else if i.key_down(Key::ArrowRight) {
                    pressed = Some(RemoteMove::RIGHT);
                }
            });
        }
        let send = self.step(if open { pressed } else { None }, turn_around && open, Instant::now());
        if self.held.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if !open {
            self.started = false;
        }
        (send, open)
    }

    /// Decides what to send for the step held now. Pure, so it is tested without a window.
    fn step(&mut self, pressed: Option<RemoteMove>, turn_around: bool, now: Instant) -> Option<(RemoteMove, bool)> {
        let repeat = Duration::from_millis(REMOTE_REPEAT_MS);
        let out = match (pressed, self.held) {
            (Some(p), Some((h, at))) if p == h => {
                if now.duration_since(at) < repeat {
                    return None;
                }
                Some(p)
            }
            (Some(p), _) => Some(p),
            (None, Some(_)) => Some(RemoteMove::STOP),
            (None, None) if turn_around => Some(RemoteMove::TURN_AROUND),
            (None, None) => None,
        };
        let step = out?;
        self.held = (pressed.is_some()).then_some((step, now));
        let first = !self.started;
        self.started = true;
        Some((step, first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_repeats_each_second_and_release_stops() {
        let mut p = DrivePad::default();
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(p.step(Some(RemoteMove::LEFT), false, t0), Some((RemoteMove::LEFT, true)));
        assert_eq!(p.step(Some(RemoteMove::LEFT), false, ms(500)), None);
        assert_eq!(p.step(Some(RemoteMove::LEFT), false, ms(1000)), Some((RemoteMove::LEFT, false)));
        assert_eq!(p.step(Some(RemoteMove::FORWARD), false, ms(1100)), Some((RemoteMove::FORWARD, false)));
        assert_eq!(p.step(None, false, ms(1200)), Some((RemoteMove::STOP, false)));
        assert_eq!(p.step(None, false, ms(1300)), None);
        assert_eq!(p.step(None, true, ms(1400)), Some((RemoteMove::TURN_AROUND, false)));
        assert_eq!(p.step(None, false, ms(1500)), None);
    }
}
