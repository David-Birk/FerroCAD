//! The timeline at the bottom: one chip per feature plus the rollback marker.

use crate::app::{FerroApp, Mode};
use egui::{Color32, RichText};
use ferrocad_model::{FeatureId, FeatureKind, FeatureStatus};

enum Action {
    Edit(FeatureId),
    ToggleSuppress(FeatureId),
    Delete(FeatureId),
    Move(FeatureId, usize),
    Marker(usize),
}

impl FerroApp {
    pub fn timeline(&mut self, ui: &mut egui::Ui) {
        let marker = self.doc.marker();
        let count = self.doc.features().len();
        let editing = !matches!(self.mode, Mode::Model);
        let mut action: Option<Action> = None;

        ui.horizontal(|ui| {
            ui.add_enabled_ui(!editing, |ui| {
                if ui.small_button("⏮").on_hover_text("Roll back to start").clicked() {
                    action = Some(Action::Marker(0));
                }
                if ui.small_button("◀").clicked() && marker > 0 {
                    action = Some(Action::Marker(marker - 1));
                }
                if ui.small_button("▶").clicked() && marker < count {
                    action = Some(Action::Marker(marker + 1));
                }
                if ui.small_button("⏭").on_hover_text("Roll forward to end").clicked() {
                    action = Some(Action::Marker(count));
                }
            });
            ui.separator();

            egui::ScrollArea::horizontal().id_salt("timeline_scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, f) in self.doc.features().iter().enumerate() {
                        if i == marker {
                            ui.colored_label(Color32::from_rgb(255, 200, 60), "▌");
                        }
                        let icon = match &f.kind {
                            FeatureKind::Sketch(_) => "✏",
                            FeatureKind::Extrude(_) => "⬆",
                            FeatureKind::Fillet(fl) if fl.chamfer => "◢",
                            FeatureKind::Fillet(_) => "◔",
                        };
                        let status = self.model.status(f.id);
                        let mut text = RichText::new(format!("{icon} {}", f.name));
                        text = match status {
                            FeatureStatus::Ok => text,
                            FeatureStatus::Error(_) => text.color(Color32::from_rgb(255, 110, 100)),
                            FeatureStatus::Suppressed => text.strikethrough().weak(),
                            FeatureStatus::RolledBack => text.weak(),
                        };
                        let hover = match status {
                            FeatureStatus::Error(e) => format!("{}: {e}", f.kind.type_name()),
                            _ => format!("{} – double-click to edit, right-click for more", f.kind.type_name()),
                        };
                        let resp = ui.add_enabled(!editing, egui::Button::new(text)).on_hover_text(hover);
                        if resp.double_clicked() {
                            action = Some(Action::Edit(f.id));
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Edit").clicked() {
                                action = Some(Action::Edit(f.id));
                                ui.close();
                            }
                            let label = if f.suppressed { "Unsuppress" } else { "Suppress" };
                            if ui.button(label).clicked() {
                                action = Some(Action::ToggleSuppress(f.id));
                                ui.close();
                            }
                            if ui.button("Roll back to here").clicked() {
                                action = Some(Action::Marker(i + 1));
                                ui.close();
                            }
                            if i > 0 && ui.button("Move left").clicked() {
                                action = Some(Action::Move(f.id, i - 1));
                                ui.close();
                            }
                            if i + 1 < count && ui.button("Move right").clicked() {
                                action = Some(Action::Move(f.id, i + 1));
                                ui.close();
                            }
                            if ui.button("Delete").clicked() {
                                action = Some(Action::Delete(f.id));
                                ui.close();
                            }
                        });
                    }
                    if marker == count {
                        ui.colored_label(Color32::from_rgb(255, 200, 60), "▌");
                    }
                    if count == 0 {
                        ui.weak("Timeline is empty – start with ✏ Sketch");
                    }
                });
            });
        });

        let Some(action) = action else { return };
        match action {
            Action::Edit(id) => self.edit_feature(id),
            Action::ToggleSuppress(id) => {
                if let Some(f) = self.doc.feature_mut(id) {
                    f.suppressed = !f.suppressed;
                }
                self.message = None;
                self.rebuild();
            }
            Action::Delete(id) => match self.doc.remove(id) {
                Ok(()) => {
                    self.message = None;
                    self.rebuild();
                }
                Err(e) => self.error(format!("cannot delete: {e}")),
            },
            Action::Move(id, to) => match self.doc.move_feature(id, to) {
                Ok(()) => {
                    self.message = None;
                    self.rebuild();
                }
                Err(e) => self.error(e),
            },
            Action::Marker(m) => {
                self.doc.set_marker(m);
                self.message = None;
                self.rebuild();
            }
        }
    }
}
