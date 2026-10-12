//! The timeline at the bottom: playback buttons, one icon chip per feature
//! and the draggable rollback marker.

use crate::app::{FerroApp, Mode};
use crate::icons::{self, Icon};
use crate::theme;
use egui::{Color32, Rect, Sense, Stroke};
use ferrocad_model::{FeatureId, FeatureKind, FeatureStatus};

enum Action {
    Edit(FeatureId),
    ToggleSuppress(FeatureId),
    Delete(FeatureId),
    Move(FeatureId, usize),
    Marker(usize),
    Animate(FeatureId),
}

const CHIP: f32 = 30.0;

fn feature_icon(kind: &FeatureKind) -> Icon {
    match kind {
        FeatureKind::Sketch(_) => Icon::Sketch,
        FeatureKind::Extrude(_) => Icon::Extrude,
        FeatureKind::Revolve(_) => Icon::Revolve,
        FeatureKind::Fillet(fl) if fl.chamfer => Icon::Chamfer,
        FeatureKind::Fillet(_) => Icon::Fillet,
        FeatureKind::Plane(p) if p.base2.is_some() => Icon::Midplane,
        FeatureKind::Plane(_) => Icon::Plane,
        FeatureKind::Draft(_) => Icon::Draft,
        FeatureKind::Axis(_) => Icon::Axis,
        FeatureKind::PressPull(_) => Icon::PressPull,
        FeatureKind::Shell(_) => Icon::Shell,
        FeatureKind::Hole(_) => Icon::Hole,
        FeatureKind::Mirror(_) => Icon::Mirror,
        FeatureKind::Pattern(p) if matches!(p.kind, ferrocad_model::PatternKind::Circular { .. }) => Icon::CircPattern,
        FeatureKind::Pattern(_) => Icon::RectPattern,
        FeatureKind::Combine(_) => Icon::Combine,
        FeatureKind::Split(_) => Icon::Split,
        FeatureKind::Move(_) => Icon::Move,
        FeatureKind::Loft(_) => Icon::Loft,
        FeatureKind::Sweep(_) => Icon::Sweep,
        FeatureKind::Remove(_) => Icon::Delete,
        FeatureKind::Import(_) => Icon::Open,
        FeatureKind::Joint(_) => Icon::Joint,
        FeatureKind::Component(_) => Icon::Body,
    }
}

impl FerroApp {
    pub fn timeline(&mut self, ui: &mut egui::Ui) {
        let marker = self.doc.marker();
        let count = self.doc.features().len();
        let idle = matches!(self.mode, Mode::Model);
        let mut action: Option<Action> = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let buttons = [
                (Icon::First, "Roll back to the start", 0, marker > 0),
                (Icon::Prev, "Step back", marker.saturating_sub(1), marker > 0),
                (Icon::Next, "Step forward", (marker + 1).min(count), marker < count),
                (Icon::Last, "Roll forward to the end", count, marker < count),
            ];
            for (icon, tip, target, enabled) in buttons {
                if icons::icon_button(ui, icon, 24.0, tip, idle && enabled, false).clicked() {
                    action = Some(Action::Marker(target));
                }
            }
            ui.add_space(6.0);
            let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, CHIP), Sense::hover());
            ui.painter().rect_filled(sep, 0.0, theme::BORDER);
            ui.add_space(6.0);

            egui::ScrollArea::horizontal().id_salt("timeline_scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let mut centers: Vec<f32> = Vec::new();
                    let mut marker_drag: Option<egui::Response> = None;
                    for (i, f) in self.doc.features().iter().enumerate() {
                        if i == marker {
                            marker_drag = Some(marker_handle(ui, idle));
                        }
                        let status = self.model.status(f.id);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(CHIP, CHIP), Sense::click());
                        centers.push(rect.center().x);
                        paint_chip(ui, rect, &resp, feature_icon(&f.kind), status, idle);
                        let hover = match &status {
                            FeatureStatus::Error(e) => format!("{} ({})\n⚠ {e}", f.name, f.kind.type_name()),
                            FeatureStatus::Suppressed => format!("{} – suppressed", f.name),
                            FeatureStatus::RolledBack => format!("{} – rolled back", f.name),
                            FeatureStatus::Ok => format!("{} ({})\nDouble-click to edit · right-click for more", f.name, f.kind.type_name()),
                        };
                        let resp = resp.on_hover_text(hover);
                        if !idle {
                            continue;
                        }
                        if resp.double_clicked() {
                            action = Some(Action::Edit(f.id));
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Edit Feature").clicked() {
                                action = Some(Action::Edit(f.id));
                                ui.close();
                            }
                            let label = if f.suppressed { "Unsuppress Feature" } else { "Suppress Feature" };
                            if ui.button(label).clicked() {
                                action = Some(Action::ToggleSuppress(f.id));
                                ui.close();
                            }
                            let movable = matches!(&f.kind, FeatureKind::Joint(j) if j.kind != ferrocad_model::JointKind::Rigid);
                            if movable && ui.button("Animate").clicked() {
                                action = Some(Action::Animate(f.id));
                                ui.close();
                            }
                            if ui.button("Roll Back To Here").clicked() {
                                action = Some(Action::Marker(i + 1));
                                ui.close();
                            }
                            ui.separator();
                            if i > 0 && ui.button("Move Left").clicked() {
                                action = Some(Action::Move(f.id, i - 1));
                                ui.close();
                            }
                            if i + 1 < count && ui.button("Move Right").clicked() {
                                action = Some(Action::Move(f.id, i + 1));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(egui::RichText::new("Delete").color(theme::ERROR)).clicked() {
                                action = Some(Action::Delete(f.id));
                                ui.close();
                            }
                        });
                    }
                    if marker == count {
                        marker_drag = Some(marker_handle(ui, idle));
                    }
                    if count == 0 {
                        ui.weak("Timeline is empty – start with Create Sketch");
                    }

                    // Dragging the marker: show where it will land, apply on release.
                    if let Some(m) = marker_drag {
                        if let Some(p) = m.interact_pointer_pos().filter(|_| m.dragged() || m.drag_stopped()) {
                            let target = centers.iter().filter(|c| **c < p.x).count();
                            let x = match target {
                                0 => centers.first().map(|c| c - CHIP * 0.5 - 2.0),
                                t => centers.get(t - 1).map(|c| c + CHIP * 0.5 + 2.0),
                            };
                            if let Some(x) = x {
                                let y = m.rect.y_range();
                                ui.painter().line_segment(
                                    [egui::pos2(x, y.min), egui::pos2(x, y.max)],
                                    Stroke::new(3.0, theme::ORANGE.gamma_multiply(0.6)),
                                );
                            }
                            if m.drag_stopped() && target != marker {
                                action = Some(Action::Marker(target));
                            }
                        }
                    }
                });
            });
        });

        let Some(action) = action else { return };
        let label = match &action {
            Action::Edit(_) | Action::Animate(_) => None,
            Action::ToggleSuppress(_) => Some("Suppress"),
            Action::Delete(_) => Some("Delete feature"),
            Action::Move(..) => Some("Reorder timeline"),
            Action::Marker(_) => Some("Move timeline marker"),
        };
        if let Some(label) = label {
            self.checkpoint(label);
        }
        match action {
            Action::Edit(id) => self.edit_feature(id),
            Action::Animate(id) => self.start_animation(id),
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

fn marker_handle(ui: &mut egui::Ui, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::drag() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(10.0, CHIP), sense);
    let color = if resp.hovered() || resp.dragged() { Color32::from_rgb(255, 180, 90) } else { theme::ORANGE };
    let bar = Rect::from_center_size(rect.center(), egui::vec2(3.0, CHIP));
    ui.painter().rect_filled(bar, 1.0, color);
    let top = rect.center_top();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![top + egui::vec2(-5.0, 0.0), top + egui::vec2(5.0, 0.0), top + egui::vec2(0.0, 6.0)],
        color,
        Stroke::NONE,
    ));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal).on_hover_text("Drag to roll back the timeline")
    } else {
        resp
    }
}

fn paint_chip(ui: &egui::Ui, rect: Rect, resp: &egui::Response, icon: Icon, status: &FeatureStatus, idle: bool) {
    let painter = ui.painter();
    let (fill, stroke) = match status {
        FeatureStatus::Error(_) => (Color32::from_rgb(90, 40, 40), Stroke::new(1.0, theme::ERROR)),
        _ if resp.hovered() && idle => (theme::HOVER, Stroke::new(1.0, theme::ACCENT)),
        _ => (Color32::from_rgb(54, 58, 64), Stroke::new(1.0, theme::BORDER)),
    };
    painter.rect(rect, 4.0, fill, stroke, egui::StrokeKind::Inside);
    let dim = matches!(status, FeatureStatus::RolledBack | FeatureStatus::Suppressed);
    let color = if dim { Color32::from_gray(100) } else { theme::ICON };
    icons::paint(painter, rect.shrink(5.0), icon, color, !dim);
    if matches!(status, FeatureStatus::Suppressed) {
        painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.5, theme::ERROR));
    }
}
