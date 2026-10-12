//! Inspect tools: measure, section analysis, body properties.

use crate::app::{FerroApp, Mode};
use crate::dialogs::{command_dialog, form_label, DialogAction};
use crate::icons::Icon;
use crate::picking::PICK_PX;
use crate::theme;
use egui::{Pos2, RichText, Stroke};
use ferrocad_model::document::polyline_axis;
use ferrocad_model::geom::{cross, distance, dot, length, normalize, sub, Vec3};
use ferrocad_model::Kernel;
use ferrocad_view::Viewport;

#[derive(Clone, Debug)]
pub struct MeasurePick {
    pub point: Vec3,
    /// The picked edge, if the pick was an edge.
    pub edge: Option<Vec<Vec3>>,
    /// Outward normal of a picked flat face.
    pub normal: Option<Vec3>,
    /// Area of a picked face.
    pub area: Option<f64>,
    pub what: &'static str,
}

impl MeasurePick {
    /// Direction of a straight edge.
    fn line_dir(&self) -> Option<Vec3> {
        let e = self.edge.as_ref()?;
        let (a, b) = (*e.first()?, *e.last()?);
        let d = sub(b, a);
        let straight = e.iter().all(|p| length(cross(sub(*p, a), d)) <= 1e-6 * dot(d, d).max(1e-12));
        (straight && length(d) > 1e-9).then(|| normalize(d))
    }
}

/// Angle in degrees between two picks: face normals (0° = parallel faces),
/// two straight edges (acute), or an edge against a face (0° = in plane).
fn pick_angle(a: &MeasurePick, b: &MeasurePick) -> Option<f64> {
    let deg = |c: f64| c.clamp(-1.0, 1.0).acos().to_degrees();
    match (a.normal, b.normal, a.line_dir(), b.line_dir()) {
        (Some(n), Some(m), _, _) => Some(deg(dot(n, m))),
        (_, _, Some(u), Some(w)) => Some(deg(dot(u, w).abs())),
        (Some(n), _, _, Some(u)) | (_, Some(n), Some(u), _) => Some(90.0 - deg(dot(n, u).abs())),
        _ => None,
    }
}

#[derive(Default)]
pub struct MeasureSession {
    pub picks: Vec<MeasurePick>,
}

/// Section analysis: everything on the positive side of the plane is cut away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    /// 0 = YZ (cut along X), 1 = XZ (along Y), 2 = XY (along Z).
    pub axis: usize,
    pub offset: f64,
    pub flip: bool,
}

impl Section {
    pub fn plane(&self) -> (Vec3, Vec3) {
        let mut n = [0.0; 3];
        n[self.axis.min(2)] = if self.flip { -1.0 } else { 1.0 };
        let mut o = [0.0; 3];
        o[self.axis.min(2)] = self.offset;
        (o, n)
    }
}

fn polyline_length(p: &[Vec3]) -> f64 {
    p.windows(2).map(|w| distance(w[0], w[1])).sum()
}

impl FerroApp {
    /// What a measure click at `pos` would pick: a vertex, an edge or a face point.
    fn measure_target(&self, pos: Pos2, vp: &Viewport) -> Option<MeasurePick> {
        let p = [pos.x as f64, pos.y as f64];
        // Vertices (edge end points) first.
        let mut best: Option<(f64, Vec3)> = None;
        for body in self.bodies.iter().filter(|b| b.visible) {
            for e in &body.edges {
                for v in [e.first(), e.last()].into_iter().flatten() {
                    if let Some(s) = self.camera.project(*v, vp) {
                        let d = (s[0] - p[0]).hypot(s[1] - p[1]);
                        if d < PICK_PX && best.is_none_or(|b| d < b.0) {
                            best = Some((d, *v));
                        }
                    }
                }
            }
        }
        if let Some((_, v)) = best {
            return Some(MeasurePick { point: v, edge: None, normal: None, area: None, what: "vertex" });
        }
        if let Some((_, poly)) = self.pick_edge(pos, vp) {
            let point = poly
                .iter()
                .filter_map(|q| self.camera.project(*q, vp).map(|s| ((s[0] - p[0]).hypot(s[1] - p[1]), *q)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, q)| q)?;
            return Some(MeasurePick { point, edge: Some(poly), normal: None, area: None, what: "edge" });
        }
        let f = self.pick_face(pos, vp)?;
        let name = &self.bodies[f.body].name;
        let area = self.model.bodies.iter().find(|b| b.name == *name).and_then(|b| self.kernel.face_area(&b.solid, &f.face));
        let normal = f.planar.then(|| normalize(f.face.normal));
        Some(MeasurePick { point: f.face.point, edge: None, normal, area, what: "face" })
    }

    pub fn measure_viewport(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        let hover = response.hover_pos().and_then(|p| self.measure_target(p, vp));
        let Mode::Measure(sess) = &mut self.mode else { return };
        if response.clicked() {
            if let Some(h) = hover.clone() {
                if sess.picks.len() >= 2 {
                    sess.picks.clear();
                }
                sess.picks.push(h);
            }
        }
        let sess_picks = sess.picks.clone();
        let draw_target = |m: &MeasurePick, color: egui::Color32| {
            if let Some(e) = &m.edge {
                let pts: Vec<Pos2> = e.iter().filter_map(|q| self.to_screen(*q, vp)).collect();
                painter.add(egui::Shape::line(pts, Stroke::new(3.0, color)));
            }
            if let Some(s) = self.to_screen(m.point, vp) {
                painter.circle(s, 4.5, color, Stroke::new(1.0, egui::Color32::BLACK));
            }
        };
        for m in &sess_picks {
            draw_target(m, crate::sketch_ui::COL_SELECTED);
        }
        if let Some(h) = &hover {
            draw_target(h, theme::ACCENT);
        }
        if let [a, b] = sess_picks.as_slice() {
            if let (Some(p), Some(q)) = (self.to_screen(a.point, vp), self.to_screen(b.point, vp)) {
                painter.extend(egui::Shape::dashed_line(&[p, q], Stroke::new(1.5, crate::sketch_ui::COL_SELECTED), 6.0, 4.0));
                let mid = p + (q - p) * 0.5;
                let text = format!("{:.3} mm", distance(a.point, b.point));
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), theme::TEXT);
                let rect = egui::Rect::from_center_size(mid, galley.size() + egui::vec2(10.0, 6.0));
                painter.rect_filled(rect, 3.0, theme::WINDOW);
                painter.galley(rect.min + egui::vec2(5.0, 3.0), galley, theme::TEXT);
            }
        }
    }

    pub fn measure_window(&mut self, ctx: &egui::Context) {
        let viewport = self.viewport_rect;
        let Mode::Measure(sess) = &mut self.mode else { return };
        let picks = sess.picks.clone();
        let mut clear = false;
        let action = command_dialog(ctx, viewport, "Measure", Icon::Measure, true, |ui| {
            let fmt = |v: Vec3| format!("{:.3}, {:.3}, {:.3}", v[0], v[1], v[2]);
            if picks.is_empty() {
                form_label(ui, "");
                ui.weak("Click a vertex, edge or face");
                ui.end_row();
            }
            for (i, m) in picks.iter().enumerate() {
                form_label(ui, &format!("Pick {} ({})", i + 1, m.what));
                ui.label(fmt(m.point));
                ui.end_row();
                if let (Some(e), 1) = (&m.edge, picks.len()) {
                    form_label(ui, "Edge length");
                    ui.label(RichText::new(format!("{:.3} mm", polyline_length(e))).strong());
                    ui.end_row();
                    if let Some((c, _)) = polyline_axis(e).filter(|_| e.len() > 2) {
                        let r = distance(c, e[0]);
                        if r < 1e6 {
                            form_label(ui, "Diameter");
                            ui.label(RichText::new(format!("{:.3} mm  (R {:.3})", 2.0 * r, r)).strong());
                            ui.end_row();
                        }
                    }
                }
                if let (Some(area), 1) = (m.area, picks.len()) {
                    form_label(ui, "Area");
                    ui.label(RichText::new(format!("{area:.3} mm²")).strong());
                    ui.end_row();
                }
            }
            if let Some(angle) = picks.first().zip(picks.get(1)).and_then(|(a, b)| pick_angle(a, b)) {
                form_label(ui, "Angle");
                ui.label(RichText::new(format!("{angle:.3}°")).strong());
                ui.end_row();
            }
            if let [a, b] = picks.as_slice() {
                let d = sub(b.point, a.point);
                form_label(ui, "Distance");
                ui.label(RichText::new(format!("{:.3} mm", distance(a.point, b.point))).strong());
                ui.end_row();
                form_label(ui, "ΔX ΔY ΔZ");
                ui.label(fmt(d));
                ui.end_row();
            }
            form_label(ui, "");
            if ui.button("Clear").clicked() {
                clear = true;
            }
            ui.end_row();
        });
        if clear {
            if let Mode::Measure(sess) = &mut self.mode {
                sess.picks.clear();
            }
        }
        if action != DialogAction::None {
            self.mode = Mode::Model;
        }
    }

    pub fn section_window(&mut self, ctx: &egui::Context) {
        let Some(mut section) = self.section else { return };
        let (min, max) = ferrocad_view::bounds(self.bodies.iter().filter(|b| b.visible).map(|b| &b.mesh))
            .unwrap_or(([-50.0; 3], [50.0; 3]));
        let mut open = true;
        egui::Window::new("Section Analysis")
            .id(egui::Id::new("section_window"))
            .default_pos(self.viewport_rect.left_top() + egui::vec2(16.0, 16.0))
            .resizable(false)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (i, label) in ["YZ", "XZ", "XY"].iter().enumerate() {
                        if ui.selectable_label(section.axis == i, *label).clicked() && section.axis != i {
                            section.axis = i;
                            section.offset = 0.5 * (min[i] + max[i]);
                        }
                    }
                    ui.checkbox(&mut section.flip, "Flip");
                });
                let i = section.axis;
                let range = (min[i] - 1.0)..=(max[i] + 1.0);
                ui.add(egui::Slider::new(&mut section.offset, range).text("offset (mm)"));
            });
        self.section = open.then_some(section);
    }

    /// Pairwise overlaps of the visible bodies; the overlaps are shown in red.
    pub fn check_interference(&mut self) {
        use ferrocad_model::BooleanOp;
        let visible: Vec<usize> = (0..self.model.bodies.len()).filter(|&i| self.bodies.get(i).is_some_and(|b| b.visible)).collect();
        let mut found = Vec::new();
        self.interference.clear();
        for (n, &i) in visible.iter().enumerate() {
            for &j in &visible[n + 1..] {
                let (a, b) = (&self.model.bodies[i], &self.model.bodies[j]);
                let Ok(common) = self.kernel.boolean(BooleanOp::Intersect, &a.solid, &b.solid) else { continue };
                if self.kernel.is_empty(&common) {
                    continue;
                }
                let volume = self.kernel.properties(&common).map(|p| p.volume).unwrap_or(0.0);
                if volume > 1e-6 {
                    found.push(format!("{} ∩ {}: {volume:.2} mm³", a.name, b.name));
                    if let Ok(mesh) = self.kernel.tessellate(&common, 0.05) {
                        self.interference.push(mesh);
                    }
                }
            }
        }
        if found.is_empty() {
            self.info("no interference between the visible bodies");
        } else {
            self.error(format!("interference: {}", found.join(" · ")));
        }
    }

    /// Volume, area and centre of mass of the selected (or all visible) bodies.
    pub fn show_properties(&mut self, names: &[String]) {
        // Component names stand for their bodies.
        let names: Vec<String> = names
            .iter()
            .flat_map(|n| match self.model.components.iter().find(|(c, _)| c == n) {
                Some((_, list)) => list.clone(),
                None => vec![n.clone()],
            })
            .collect();
        let mut lines = Vec::new();
        for (body, display) in self.model.bodies.iter().zip(&self.bodies) {
            let wanted = if names.is_empty() { display.visible } else { names.contains(&body.name) };
            if !wanted {
                continue;
            }
            if let Some(p) = self.kernel.properties(&body.solid) {
                let material = self
                    .doc
                    .view
                    .appearances
                    .iter()
                    .find(|(b, _)| *b == body.name)
                    .map(|(_, m)| m.as_str())
                    .unwrap_or(crate::theme::MATERIALS[0].0);
                // mm³ → cm³ times g/cm³.
                let mass = p.volume / 1000.0 * crate::theme::material_density(material);
                lines.push(format!(
                    "{}: {material}, mass {mass:.1} g · volume {:.2} mm³ · area {:.2} mm² · centre ({:.2}, {:.2}, {:.2})",
                    body.name, p.volume, p.area, p.center[0], p.center[1], p.center[2]
                ));
            }
        }
        if lines.is_empty() {
            self.error("no body to inspect");
        } else {
            self.info(lines.join("   |   "));
        }
    }
}
