//! Extrude / fillet / parameter dialogs and their viewport picking.

use crate::app::{FerroApp, Mode};
use egui::{Color32, Stroke};
use ferrocad_model::kernel::polyline_midpoint;
use ferrocad_model::{
    Document, EdgeRef, ExtrudeFeature, FeatureId, FeatureKind, FilletFeature, Operation,
};
use ferrocad_sketch::find_profiles;
use ferrocad_sketch::profile::profile_at;
use ferrocad_view::{polyline_screen_distance, Viewport};

const COL_PROFILE: Color32 = Color32::from_rgb(110, 175, 255);
const COL_PICKED: Color32 = Color32::from_rgb(255, 215, 70);

pub struct ExtrudeSession {
    pub editing: Option<FeatureId>,
    pub sketch: Option<FeatureId>,
    pub profile_point: Option<[f64; 2]>,
    pub distance: String,
    pub symmetric: bool,
    pub operation: Operation,
}

impl ExtrudeSession {
    pub fn new(doc: &Document) -> Self {
        // Default to the newest sketch before the marker.
        let sketch = doc.features()[..doc.marker()]
            .iter()
            .rev()
            .find(|f| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|f| f.id);
        Self {
            editing: None,
            sketch,
            profile_point: None,
            distance: "10 mm".into(),
            symmetric: false,
            operation: Operation::NewBody,
        }
    }

    pub fn edit(id: FeatureId, e: &ExtrudeFeature) -> Self {
        Self {
            editing: Some(id),
            sketch: Some(e.sketch),
            profile_point: Some(e.profile_point),
            distance: e.distance.clone(),
            symmetric: e.symmetric,
            operation: e.operation,
        }
    }
}

pub struct FilletSession {
    pub editing: Option<FeatureId>,
    pub edges: Vec<EdgeRef>,
    pub radius: String,
    pub chamfer: bool,
}

impl FilletSession {
    pub fn new(chamfer: bool) -> Self {
        Self { editing: None, edges: Vec::new(), radius: "1 mm".into(), chamfer }
    }

    pub fn edit(id: FeatureId, f: &FilletFeature) -> Self {
        Self { editing: Some(id), edges: f.edges.clone(), radius: f.radius.clone(), chamfer: f.chamfer }
    }
}

impl FerroApp {
    // ------------------------------------------------------------- extrude

    pub fn extrude_viewport(
        &mut self,
        response: &egui::Response,
        painter: &egui::Painter,
        vp: &Viewport,
    ) {
        let cam = self.camera;
        let Mode::Extrude(sess) = &mut self.mode else { return };
        let Some(sf) = sess.sketch.and_then(|id| self.doc.sketch(id)) else { return };
        let plane = sf.plane;
        let profiles = find_profiles(&sf.sketch);

        let to_local = |pos: egui::Pos2| {
            let (o, d) = cam.ray([pos.x as f64, pos.y as f64], vp);
            plane.intersect_ray(o, d).map(|w| plane.to_local(w))
        };
        if response.clicked() {
            if let Some(uv) = response.interact_pointer_pos().and_then(to_local) {
                if profile_at(&profiles, uv).is_some() {
                    sess.profile_point = Some(uv);
                }
            }
        }

        let selected = sess.profile_point.and_then(|p| profile_at(&profiles, p));
        let hovered = response.hover_pos().and_then(to_local).and_then(|p| profile_at(&profiles, p));
        for (i, pr) in profiles.iter().enumerate() {
            let (color, width) = if Some(i) == selected {
                (COL_PICKED, 2.5)
            } else if Some(i) == hovered {
                (COL_PROFILE, 2.5)
            } else {
                (COL_PROFILE, 1.0)
            };
            for lp in std::iter::once(&pr.outer).chain(&pr.holes) {
                let mut pts: Vec<egui::Pos2> = lp
                    .polygon()
                    .iter()
                    .filter_map(|uv| cam.project(plane.to_world(*uv), vp))
                    .map(|s| egui::pos2(s[0] as f32, s[1] as f32))
                    .collect();
                if let Some(first) = pts.first().copied() {
                    pts.push(first);
                }
                painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
            }
        }
    }

    pub fn extrude_window(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut commit = false;
        let sketches: Vec<(FeatureId, String)> = self.doc.features()[..self.doc.marker()]
            .iter()
            .filter(|f| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|f| (f.id, f.name.clone()))
            .collect();
        let params = self.doc.params.clone();
        let Mode::Extrude(sess) = &mut self.mode else { return };

        egui::Window::new("Extrude").default_width(260.0).show(ctx, |ui| {
            let current = sketches
                .iter()
                .find(|(id, _)| Some(*id) == sess.sketch)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "–".into());
            egui::ComboBox::from_label("Sketch").selected_text(current).show_ui(ui, |ui| {
                for (id, name) in &sketches {
                    if ui.selectable_value(&mut sess.sketch, Some(*id), name.as_str()).clicked() {
                        sess.profile_point = None;
                    }
                }
            });
            if sess.profile_point.is_some() {
                ui.label("✔ Profile selected (click another to change)");
            } else {
                ui.colored_label(COL_PICKED, "Click inside a profile in the viewport");
            }
            ui.horizontal(|ui| {
                ui.label("Distance");
                ui.text_edit_singleline(&mut sess.distance);
            });
            match params.eval(&sess.distance) {
                Ok(v) => ui.weak(format!("= {v:.3} mm")),
                Err(e) => ui.colored_label(Color32::from_rgb(255, 110, 100), e.to_string()),
            };
            ui.checkbox(&mut sess.symmetric, "Symmetric");
            egui::ComboBox::from_label("Operation").selected_text(sess.operation.label()).show_ui(
                ui,
                |ui| {
                    for op in Operation::ALL {
                        ui.selectable_value(&mut sess.operation, op, op.label());
                    }
                },
            );
            ui.separator();
            ui.horizontal(|ui| {
                let ready = sess.sketch.is_some()
                    && sess.profile_point.is_some()
                    && params.eval(&sess.distance).is_ok();
                if ui.add_enabled(ready, egui::Button::new("OK")).clicked() {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if commit {
            let Mode::Extrude(sess) = std::mem::replace(&mut self.mode, Mode::Model) else { return };
            let (Some(sketch), Some(profile_point)) = (sess.sketch, sess.profile_point) else { return };
            let feature = ExtrudeFeature {
                sketch,
                profile_point,
                distance: sess.distance,
                symmetric: sess.symmetric,
                operation: sess.operation,
            };
            match sess.editing.and_then(|id| self.doc.feature_mut(id)) {
                Some(f) => f.kind = FeatureKind::Extrude(feature),
                None => {
                    let name = self.doc.next_name("Extrude");
                    self.doc.add(name, FeatureKind::Extrude(feature));
                }
            }
            let first_body = self.bodies.is_empty();
            self.message = None;
            self.rebuild();
            if first_body {
                self.camera.set_view(ferrocad_view::ViewPreset::Home);
                self.fit_view();
            }
        } else if close {
            self.mode = Mode::Model;
        }
    }

    // -------------------------------------------------------------- fillet

    pub fn fillet_viewport(
        &mut self,
        response: &egui::Response,
        painter: &egui::Painter,
        vp: &Viewport,
    ) {
        let cam = self.camera;
        let Mode::Fillet(sess) = &mut self.mode else { return };
        let pos = response.hover_pos();

        // nearest edge under the cursor, across visible bodies
        let mut hovered: Option<(f64, Vec<[f64; 3]>)> = None;
        if let Some(p) = pos {
            for body in self.bodies.iter().filter(|b| b.visible) {
                for e in &body.edges {
                    let d = polyline_screen_distance([p.x as f64, p.y as f64], e, &cam, vp);
                    if d < 8.0 && hovered.as_ref().is_none_or(|(best, _)| d < *best) {
                        hovered = Some((d, e.clone()));
                    }
                }
            }
        }
        if response.clicked() {
            if let Some(mid) = hovered.as_ref().and_then(|(_, e)| polyline_midpoint(e)) {
                let close = |r: &EdgeRef| ferrocad_model::geom::distance(r.point, mid) < 1e-6;
                if let Some(i) = sess.edges.iter().position(close) {
                    sess.edges.remove(i);
                } else {
                    sess.edges.push(EdgeRef { point: mid });
                }
            }
        }

        let draw = |poly: &[[f64; 3]], color: Color32| {
            let pts: Vec<egui::Pos2> = poly
                .iter()
                .filter_map(|p| cam.project(*p, vp))
                .map(|s| egui::pos2(s[0] as f32, s[1] as f32))
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(3.0, color)));
        };
        if let Some((_, e)) = &hovered {
            draw(e, COL_PROFILE);
        }
        // highlight selected edges: find the current edge for each reference
        for r in &sess.edges {
            for body in &self.bodies {
                for e in &body.edges {
                    if polyline_midpoint(e)
                        .is_some_and(|m| ferrocad_model::geom::distance(m, r.point) < 1e-3)
                    {
                        draw(e, COL_PICKED);
                    }
                }
            }
        }
    }

    pub fn fillet_window(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut commit = false;
        let params = self.doc.params.clone();
        let Mode::Fillet(sess) = &mut self.mode else { return };
        let title = if sess.chamfer { "Chamfer" } else { "Fillet" };

        egui::Window::new(title).default_width(240.0).show(ctx, |ui| {
            ui.label(format!("{} edge(s) selected – click edges to toggle", sess.edges.len()));
            ui.horizontal(|ui| {
                ui.label(if sess.chamfer { "Distance" } else { "Radius" });
                ui.text_edit_singleline(&mut sess.radius);
            });
            match params.eval(&sess.radius) {
                Ok(v) => ui.weak(format!("= {v:.3} mm")),
                Err(e) => ui.colored_label(Color32::from_rgb(255, 110, 100), e.to_string()),
            };
            ui.checkbox(&mut sess.chamfer, "Chamfer instead of round");
            ui.separator();
            ui.horizontal(|ui| {
                let ready = !sess.edges.is_empty() && params.eval(&sess.radius).is_ok();
                if ui.add_enabled(ready, egui::Button::new("OK")).clicked() {
                    commit = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if commit {
            let Mode::Fillet(sess) = std::mem::replace(&mut self.mode, Mode::Model) else { return };
            let feature = FilletFeature { edges: sess.edges, radius: sess.radius, chamfer: sess.chamfer };
            match sess.editing.and_then(|id| self.doc.feature_mut(id)) {
                Some(f) => f.kind = FeatureKind::Fillet(feature),
                None => {
                    let prefix = if feature.chamfer { "Chamfer" } else { "Fillet" };
                    let name = self.doc.next_name(prefix);
                    self.doc.add(name, FeatureKind::Fillet(feature));
                }
            }
            self.message = None;
            self.rebuild();
        } else if close {
            self.mode = Mode::Model;
        }
    }

    // ---------------------------------------------------------- parameters

    pub fn parameters_window(&mut self, ctx: &egui::Context) {
        if self.param_edits.is_empty() {
            self.param_edits =
                self.doc.params.iter().map(|p| (p.name.clone(), p.expression.clone())).collect();
        }
        let mut open = true;
        let mut changed: Option<(String, String)> = None;
        let mut remove: Option<String> = None;
        let mut add = false;

        egui::Window::new("Parameters").default_width(380.0).show(ctx, |ui| {
            egui::Grid::new("params").num_columns(4).striped(true).show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Expression");
                ui.strong("Value");
                ui.end_row();
                for (name, expr) in &mut self.param_edits {
                    ui.label(name.as_str());
                    let r = ui.add(egui::TextEdit::singleline(expr).desired_width(140.0));
                    if r.lost_focus() {
                        changed = Some((name.clone(), expr.clone()));
                    }
                    let value = self.doc.params.value(name);
                    match value {
                        Ok(v) => ui.label(format!("{v:.3}")),
                        Err(e) => ui.colored_label(Color32::from_rgb(255, 110, 100), e.to_string()),
                    };
                    if ui.small_button("✕").clicked() {
                        remove = Some(name.clone());
                    }
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.new_param.0).hint_text("name").desired_width(90.0));
                ui.add(egui::TextEdit::singleline(&mut self.new_param.1).hint_text("expression").desired_width(140.0));
                if ui.button("Add").clicked() {
                    add = true;
                }
            });
            ui.weak("Units: mm cm m in · deg rad · functions: sqrt sin cos tan min max …");
            if ui.button("Close").clicked() {
                open = false;
            }
        });

        let mut needs_rebuild = false;
        if let Some((name, expr)) = changed {
            let unchanged = self.doc.params.get(&name).is_some_and(|p| p.expression == expr);
            if !unchanged {
                match self.doc.params.set(&name, &expr) {
                    Ok(()) => needs_rebuild = true,
                    Err(e) => self.error(format!("{name}: {e}")),
                }
                self.param_edits.clear();
            }
        }
        if let Some(name) = remove {
            match self.doc.params.remove(&name) {
                Ok(()) => needs_rebuild = true,
                Err(e) => self.error(e),
            }
            self.param_edits.clear();
        }
        if add {
            let (name, expr) = std::mem::take(&mut self.new_param);
            match self.doc.params.set(name.trim(), &expr) {
                Ok(()) => needs_rebuild = true,
                Err(e) => {
                    self.error(format!("{}: {e}", name.trim()));
                    self.new_param = (name, expr);
                }
            }
            self.param_edits.clear();
        }
        if needs_rebuild {
            self.message = None;
            self.rebuild();
        }
        if !open {
            self.show_params = false;
        }
    }
}
