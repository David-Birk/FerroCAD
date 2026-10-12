//! Drag handles in the viewport, like Fusion's distance arrows on Extrude
//! and Press Pull. Dragging rewrites the dialog's distance expression.

use crate::app::{FerroApp, Mode};
use crate::theme;
use egui::{Color32, Pos2, Stroke};
use ferrocad_model::geom::{add, dot, normalize, scale, sub, Vec3};
use ferrocad_sketch::find_profiles;
use ferrocad_sketch::profile::profile_at;
use ferrocad_view::Viewport;

/// Parameter along the line `base + t·dir` closest to the pointer ray.
fn line_param(app: &FerroApp, pos: Pos2, vp: &Viewport, base: Vec3, dir: Vec3) -> Option<f64> {
    let (o, d) = app.camera.ray([pos.x as f64, pos.y as f64], vp);
    let (n, d) = (normalize(dir), normalize(d));
    let w0 = sub(base, o);
    let b = dot(n, d);
    let denom = 1.0 - b * b;
    if denom < 1e-9 {
        return None; // looking along the arrow
    }
    Some((b * dot(d, w0) - dot(n, w0)) / denom)
}

impl FerroApp {
    /// An arrow from `base` along `dir` at distance `value`; returns the new
    /// value while it is dragged.
    #[allow(clippy::too_many_arguments)]
    fn distance_handle(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        vp: &Viewport,
        id: &str,
        base: Vec3,
        dir: Vec3,
        value: f64,
    ) -> Option<f64> {
        let n = normalize(dir);
        let start = add(base, scale(n, value));
        let len = self.camera.distance * 0.12 * if value < 0.0 { -1.0 } else { 1.0 };
        let tip = add(start, scale(n, len));
        let (Some(s), Some(t)) = (self.to_screen(start, vp), self.to_screen(tip, vp)) else { return None };
        if (t - s).length() < 4.0 {
            return None;
        }
        let rect = egui::Rect::from_center_size(t, egui::vec2(26.0, 26.0));
        let resp = ui.interact(rect, ui.id().with(("handle", id)), egui::Sense::drag());
        let hot = resp.hovered() || resp.dragged();
        let color = if hot { Color32::from_rgb(120, 200, 255) } else { theme::ACCENT };
        painter.line_segment([s, t], Stroke::new(2.5, color));
        let u = (t - s).normalized();
        let side = egui::vec2(-u.y, u.x);
        let head = vec![t + u * 4.0, t - u * 10.0 + side * 6.0, t - u * 10.0 - side * 6.0];
        painter.add(egui::Shape::convex_polygon(head, color, Stroke::new(1.0, Color32::from_black_alpha(140))));
        painter.circle_filled(s, 3.5, color);
        if hot {
            let label = format!("{value:.2} mm");
            painter.text(t + side * 14.0, egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), theme::TEXT);
        }
        if !resp.dragged() {
            return None;
        }
        let pos = resp.interact_pointer_pos()?;
        // The pointer grabs the tip, so subtract the arrow length.
        line_param(self, pos, vp, base, n).map(|t| t - len)
    }

    /// Handles for the active command.
    pub fn manipulators(&mut self, ui: &egui::Ui, painter: &egui::Painter, vp: &Viewport) {
        let params = self.doc.params.clone();
        match &self.mode {
            Mode::Extrude(sess) => {
                let (Some(sk), Some(pp)) = (sess.sketch, sess.profile_point) else { return };
                if sess.extent != crate::dialogs::Extent::Distance {
                    return;
                }
                let Some(sf) = self.doc.sketch(sk) else { return };
                // Arrow from the profile's centre.
                let profiles = find_profiles(&sf.sketch);
                let centre = profile_at(&profiles, pp)
                    .map(|i| {
                        let poly = profiles[i].outer.polygon();
                        let n = poly.len().max(1) as f64;
                        let c = poly.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
                        if profiles[i].contains(c) { c } else { pp }
                    })
                    .unwrap_or(pp);
                let base = sf.plane.to_world(centre);
                let normal = sf.plane.normal();
                let Ok(d) = params.eval(&sess.distance) else { return };
                let signed = if sess.flip { -d } else { d };
                let shown = if sess.symmetric { signed / 2.0 } else { signed };
                if let Some(v) = self.distance_handle(ui, painter, vp, "extrude", base, normal, shown) {
                    let v = if sess.symmetric { v * 2.0 } else { v };
                    if let Mode::Extrude(sess) = &mut self.mode {
                        sess.distance = format!("{:.2} mm", v.abs().max(0.01));
                        if !sess.symmetric {
                            sess.flip = v < 0.0;
                        }
                    }
                }
            }
            Mode::Feature(sess) if sess.ty == crate::features_ui::FeatureType::PressPull => {
                let Some(crate::features_ui::Pick::Face(face)) = sess.sels[0].picks.first() else { return };
                let (base, normal) = (face.point, face.normal);
                let Ok(d) = params.eval(&sess.exprs[0].text) else { return };
                if let Some(v) = self.distance_handle(ui, painter, vp, "presspull", base, normal, d) {
                    if let Mode::Feature(sess) = &mut self.mode {
                        sess.exprs[0].text = format!("{v:.2} mm");
                    }
                }
            }
            _ => {}
        }
    }
}
