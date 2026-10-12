//! Command dialogs (extrude, revolve, fillet/chamfer, parameters) and their
//! viewport picking. Dialogs float at the right of the viewport like Fusion's,
//! with OK (Enter) / Cancel (Esc); extrude and revolve show a live preview.

use crate::app::{FerroApp, Mode};
use crate::features_ui::{Pick, SelKind};
use crate::icons::{self, Icon};
use crate::theme;
use egui::{Color32, RichText, Stroke, Ui};
use ferrocad_model::kernel::polyline_midpoint;
use ferrocad_model::{
    Document, EdgeRef, ExtrudeExtent, ExtrudeFeature, FeatureId, FeatureKind, FilletFeature, Operation,
    ParamTable, PlaneRef, RevolveAxis, RevolveFeature, SketchFeature, ThinSide, ThinWall,
};
use ferrocad_sketch::find_profiles;
use ferrocad_sketch::profile::profile_at;
use ferrocad_view::{polyline_screen_distance, Camera, Viewport};

/// An axis as (point, direction).
type AxisLine = ([f64; 3], [f64; 3]);

const COL_PROFILE: Color32 = Color32::from_rgb(110, 175, 255);
const COL_PICKED: Color32 = Color32::from_rgb(255, 215, 70);

pub struct ExtrudeSession {
    pub editing: Option<FeatureId>,
    pub sketch: Option<FeatureId>,
    pub profile_point: Option<[f64; 2]>,
    pub distance: String,
    /// Extrude to the other side (stored as a negated distance).
    pub flip: bool,
    pub symmetric: bool,
    /// Second extent against the sketch normal (Fusion's "Two sides").
    pub two_sides: bool,
    pub distance2: String,
    /// Taper angle in degrees; positive narrows the profile.
    pub taper: String,
    pub extent: Extent,
    /// Target of "To object".
    pub to: Option<PlaneRef>,
    /// Clicks in the viewport pick the "To" target instead of a profile.
    pub picking_to: bool,
    pub thin: bool,
    pub thin_wall: ThinWall,
    pub operation: Operation,
}

/// The extent choice in the Extrude dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    Distance,
    To,
    All,
}

impl ExtrudeSession {
    pub fn new(doc: &Document) -> Self {
        // Default to the newest sketch before the marker.
        let sketch = doc.features()[..doc.marker()]
            .iter()
            .rev()
            .find(|f| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|f| f.id);
        // A sketch on a body face adds to that body by default, like Fusion.
        let on_face = sketch
            .and_then(|id| doc.sketch(id))
            .is_some_and(|s| matches!(s.attach, Some(ferrocad_model::PlaneRef::Face(_))));
        Self {
            editing: None,
            sketch,
            profile_point: None,
            distance: "10 mm".into(),
            flip: false,
            symmetric: false,
            two_sides: false,
            distance2: "10 mm".into(),
            taper: "0°".into(),
            extent: Extent::Distance,
            to: None,
            picking_to: false,
            thin: false,
            thin_wall: ThinWall { thickness: "2 mm".into(), side: ThinSide::Inside },
            operation: if on_face { Operation::Join } else { Operation::NewBody },
        }
    }

    pub fn edit(id: FeatureId, e: &ExtrudeFeature) -> Self {
        let d = e.distance.trim();
        let (distance, flip) = match d.strip_prefix("-(").and_then(|r| r.strip_suffix(')')) {
            Some(inner) => (inner.to_string(), true),
            None => (e.distance.clone(), false),
        };
        Self {
            editing: Some(id),
            sketch: Some(e.sketch),
            profile_point: Some(e.profile_point),
            distance,
            flip,
            symmetric: e.symmetric,
            two_sides: e.distance2.is_some(),
            distance2: match &e.distance2 {
                Some(d2) if flip => d2.trim().strip_prefix("-(").and_then(|r| r.strip_suffix(')')).unwrap_or(d2).to_string(),
                Some(d2) => d2.clone(),
                None => "10 mm".into(),
            },
            taper: e.taper.clone(),
            extent: match e.extent {
                ExtrudeExtent::Distance => Extent::Distance,
                ExtrudeExtent::To(_) => Extent::To,
                ExtrudeExtent::All => Extent::All,
            },
            to: match e.extent {
                ExtrudeExtent::To(p) => Some(p),
                _ => None,
            },
            picking_to: false,
            thin: e.thin.is_some(),
            thin_wall: e.thin.clone().unwrap_or(ThinWall { thickness: "2 mm".into(), side: ThinSide::Inside }),
            operation: e.operation,
        }
    }

    fn distance_expr(&self) -> String {
        if self.flip {
            format!("-({})", self.distance.trim())
        } else {
            self.distance.clone()
        }
    }
}

pub struct FilletSession {
    pub editing: Option<FeatureId>,
    pub edges: Vec<EdgeRef>,
    pub radius: String,
    pub chamfer: bool,
    /// Chamfer with a second distance on the other face.
    pub two_distances: bool,
    pub distance2: String,
}

impl FilletSession {
    pub fn new(chamfer: bool) -> Self {
        Self {
            editing: None,
            edges: Vec::new(),
            radius: "1 mm".into(),
            chamfer,
            two_distances: false,
            distance2: "2 mm".into(),
        }
    }

    pub fn edit(id: FeatureId, f: &FilletFeature) -> Self {
        Self {
            editing: Some(id),
            edges: f.edges.clone(),
            radius: f.radius.clone(),
            chamfer: f.chamfer,
            two_distances: f.distance2.is_some(),
            distance2: f.distance2.clone().unwrap_or_else(|| "2 mm".into()),
        }
    }

    fn feature(&self) -> FilletFeature {
        FilletFeature {
            edges: self.edges.clone(),
            radius: self.radius.clone(),
            chamfer: self.chamfer,
            distance2: (self.chamfer && self.two_distances).then(|| self.distance2.clone()),
        }
    }
}

/// Replace the edited feature, or add a new one at the marker.
pub(crate) fn apply_feature(doc: &mut Document, editing: Option<FeatureId>, prefix: &str, kind: FeatureKind) {
    match editing.and_then(|id| doc.feature_mut(id)) {
        Some(f) => f.kind = kind,
        None => {
            let name = doc.next_name(prefix);
            doc.add(name, kind);
        }
    }
}

#[derive(PartialEq)]
pub(crate) enum DialogAction {
    None,
    Ok,
    Cancel,
}

/// Fusion-style command dialog: header with icon and title, a two-column
/// form, OK/Cancel footer. Enter confirms, Esc cancels.
pub(crate) fn command_dialog(
    ctx: &egui::Context,
    viewport: egui::Rect,
    title: &str,
    icon: Icon,
    ok_enabled: bool,
    body: impl FnOnce(&mut Ui),
) -> DialogAction {
    let mut action = DialogAction::None;
    egui::Window::new(title)
        .id(egui::Id::new(("command_dialog", title)))
        .title_bar(false)
        .auto_sized()
        .collapsible(false)
        .frame(theme::dialog_frame())
        .pivot(egui::Align2::RIGHT_TOP)
        .default_pos(viewport.right_top() + egui::vec2(-16.0, 168.0))
        .show(ctx, |ui| {
            ui.set_width(300.0);
            egui::Frame::new()
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(10, 7))
                .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, icon, theme::ICON, true);
                        ui.label(RichText::new(title.to_uppercase()).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::icon_button(ui, Icon::Cancel, 18.0, "Cancel (Esc)", true, false).clicked() {
                                action = DialogAction::Cancel;
                            }
                        });
                    });
                });
            egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                egui::Grid::new(("dialog_form", title)).num_columns(2).spacing([12.0, 8.0]).show(ui, body);
            });
            ui.separator();
            egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 2, bottom: 10 }).show(ui, |ui| {
                // `Align::Min`: a centred cross axis would claim the full window height.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if theme::secondary_button(ui, "Cancel").clicked() {
                        action = DialogAction::Cancel;
                    }
                    if theme::primary_button(ui, "OK", ok_enabled).clicked() {
                        action = DialogAction::Ok;
                    }
                });
            });
        });
    let (enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if action == DialogAction::None {
        if enter && ok_enabled {
            action = DialogAction::Ok;
        } else if esc {
            action = DialogAction::Cancel;
        }
    }
    action
}

pub(crate) fn form_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_WEAK));
}

/// `v` with up to three decimals (trailing zeros dropped) and its unit:
/// `fmt_value(10.0, "mm")` = "10 mm", `fmt_value(45.0, "°")` = "45°".
pub(crate) fn fmt_value(v: f64, unit: &str) -> String {
    let mut n = format!("{v:.3}");
    if n.contains('.') {
        n = n.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if n == "-0" {
        n = "0".into();
    }
    match unit {
        "" => n,
        "°" => format!("{n}°"),
        u => format!("{n} {u}"),
    }
}

/// A plain number gets its unit ("10" → "10 mm"); expressions stay as typed.
pub(crate) fn with_unit(text: &str, unit: &str) -> String {
    match text.trim().parse::<f64>() {
        Ok(v) if !unit.is_empty() && v.is_finite() => fmt_value(v, unit),
        _ => text.trim().to_string(),
    }
}

/// Expression field with its evaluated value or error underneath. A plain
/// number gets its unit appended when the field loses focus.
pub(crate) fn expression_field(ui: &mut Ui, params: &ParamTable, text: &mut String, unit: &str) {
    ui.vertical(|ui| {
        if ui.add(egui::TextEdit::singleline(text).desired_width(150.0)).lost_focus() {
            *text = with_unit(text, unit);
        }
        match params.eval(text) {
            Ok(v) => ui.weak(format!("= {}", fmt_value(v, unit))),
            Err(e) => ui.colored_label(theme::ERROR, e.to_string()),
        };
    });
}

fn selection_status(ui: &mut Ui, done: bool, done_text: &str, todo_text: &str) {
    if done {
        ui.label(RichText::new(format!("✔ {done_text}")).color(theme::TEXT));
    } else {
        egui::Frame::new()
            .fill(theme::ACCENT_DIM)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| ui.label(RichText::new(todo_text).color(Color32::WHITE)));
    }
}

fn sketch_combo(ui: &mut Ui, id: &str, sketches: &[(FeatureId, String)], current: &mut Option<FeatureId>) -> bool {
    let mut changed = false;
    let text = sketches
        .iter()
        .find(|(id, _)| Some(*id) == *current)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| "–".into());
    egui::ComboBox::from_id_salt(id).selected_text(text).width(150.0).show_ui(ui, |ui| {
        for (sid, name) in sketches {
            changed |= ui.selectable_value(current, Some(*sid), name.as_str()).changed();
        }
    });
    changed
}

fn operation_combo(ui: &mut Ui, id: &str, op: &mut Operation) {
    egui::ComboBox::from_id_salt(id).selected_text(op.label()).width(150.0).show_ui(ui, |ui| {
        for o in Operation::ALL {
            ui.selectable_value(op, o, o.label());
        }
    });
}

fn preview_status(ui: &mut Ui, error: Option<&str>) {
    if let Some(e) = error {
        form_label(ui, "");
        ui.colored_label(theme::ERROR, format!("⚠ {e}"));
        ui.end_row();
    }
}

/// Draw the profiles of a sketch and let a click select one (stored as a
/// point inside it). Shared by extrude and revolve.
pub fn pick_profile(
    sf: &SketchFeature,
    profile_point: &mut Option<[f64; 2]>,
    response: &egui::Response,
    painter: &egui::Painter,
    vp: &Viewport,
    cam: &Camera,
) {
    let plane = sf.plane;
    let profiles = find_profiles(&sf.sketch);
    let to_local = |pos: egui::Pos2| {
        let (o, d) = cam.ray([pos.x as f64, pos.y as f64], vp);
        plane.intersect_ray(o, d).map(|w| plane.to_local(w))
    };
    if response.clicked() {
        if let Some(uv) = response.interact_pointer_pos().and_then(to_local) {
            if profile_at(&profiles, uv).is_some() {
                *profile_point = Some(uv);
            }
        }
    }
    let selected = profile_point.and_then(|p| profile_at(&profiles, p));
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

impl FerroApp {
    fn sketches_before_marker(&self) -> Vec<(FeatureId, String)> {
        self.doc.features()[..self.doc.marker()]
            .iter()
            .filter(|f| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|f| (f.id, f.name.clone()))
            .collect()
    }

    // ------------------------------------------------------------- extrude

    pub fn extrude_viewport(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        let cam = self.camera;
        let (picking_to, to) = match &self.mode {
            Mode::Extrude(s) => (s.picking_to, s.to),
            _ => return,
        };
        // "To object" target: hovered or picked plane/face, drawn like a construction plane.
        let hover = if picking_to {
            response.hover_pos().and_then(|p| self.hover_pick(SelKind::Plane, p, vp))
        } else {
            None
        };
        for (target, color) in [(to, COL_PICKED), (hover.as_ref().and_then(Pick::plane_ref), COL_PROFILE)] {
            if let Some((plane, centred)) = target.and_then(|t| self.plane_of(&t)) {
                let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 50);
                self.draw_plane(painter, vp, &plane, centred, fill, Stroke::new(1.5, color));
            }
        }
        let Mode::Extrude(sess) = &mut self.mode else { return };
        if picking_to {
            if response.clicked() {
                if let Some(p) = hover.as_ref().and_then(Pick::plane_ref) {
                    sess.to = Some(p);
                    sess.picking_to = false;
                }
            }
            return;
        }
        let Some(sf) = sess.sketch.and_then(|id| self.doc.sketch(id)) else { return };
        pick_profile(sf, &mut sess.profile_point, response, painter, vp, &cam);
    }

    pub fn extrude_window(&mut self, ctx: &egui::Context) {
        let sketches = self.sketches_before_marker();
        let params = self.doc.params.clone();
        let preview_error = self.preview.as_ref().and_then(|p| p.error.clone());
        let viewport = self.viewport_rect;
        let Mode::Extrude(sess) = &mut self.mode else { return };
        let by_distance = sess.extent == Extent::Distance;
        let ready = sess.sketch.is_some()
            && sess.profile_point.is_some()
            && (!by_distance || params.eval(&sess.distance).is_ok())
            && (!by_distance || !sess.two_sides || params.eval(&sess.distance2).is_ok())
            && (sess.extent != Extent::To || sess.to.is_some())
            && (!sess.thin || params.eval(&sess.thin_wall.thickness).is_ok())
            && params.eval(&sess.taper).is_ok();

        let action = command_dialog(ctx, viewport, "Extrude", Icon::Extrude, ready, |ui| {
            form_label(ui, "Sketch");
            if sketch_combo(ui, "extrude_sketch", &sketches, &mut sess.sketch) {
                sess.profile_point = None;
            }
            ui.end_row();
            form_label(ui, "Profile");
            selection_status(ui, sess.profile_point.is_some(), "1 selected", "Select a profile");
            ui.end_row();
            form_label(ui, "Extent");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut sess.extent, Extent::Distance, "Distance");
                if ui.selectable_value(&mut sess.extent, Extent::To, "To object").clicked() {
                    sess.picking_to = sess.to.is_none();
                }
                ui.selectable_value(&mut sess.extent, Extent::All, "All");
            });
            ui.end_row();
            if sess.extent != Extent::To {
                sess.picking_to = false;
                form_label(ui, "Direction");
                ui.horizontal(|ui| {
                    let mut direction = (sess.symmetric, sess.two_sides);
                    ui.selectable_value(&mut direction, (false, false), "One side");
                    ui.selectable_value(&mut direction, (false, true), "Two sides");
                    ui.selectable_value(&mut direction, (true, false), "Symmetric");
                    (sess.symmetric, sess.two_sides) = direction;
                    if !sess.symmetric {
                        ui.checkbox(&mut sess.flip, "Flip");
                    }
                });
                ui.end_row();
            }
            match sess.extent {
                Extent::Distance => {
                    form_label(ui, if sess.two_sides { "Distance 1" } else { "Distance" });
                    expression_field(ui, &params, &mut sess.distance, "mm");
                    ui.end_row();
                    if sess.two_sides {
                        form_label(ui, "Distance 2");
                        expression_field(ui, &params, &mut sess.distance2, "mm");
                        ui.end_row();
                    }
                }
                Extent::To => {
                    form_label(ui, "To");
                    let text = match (sess.to, sess.picking_to) {
                        (_, true) => "Select a plane or face",
                        (Some(_), false) => "✔ 1 selected",
                        (None, false) => "Click to select",
                    };
                    if ui.selectable_label(sess.picking_to, text).clicked() {
                        sess.picking_to = true;
                    }
                    ui.end_row();
                }
                Extent::All => {}
            }
            form_label(ui, "Taper angle");
            expression_field(ui, &params, &mut sess.taper, "°");
            ui.end_row();
            form_label(ui, "Thin");
            ui.horizontal(|ui| {
                ui.checkbox(&mut sess.thin, "");
                if sess.thin {
                    for side in ThinSide::ALL {
                        ui.selectable_value(&mut sess.thin_wall.side, side, side.label());
                    }
                }
            });
            ui.end_row();
            if sess.thin {
                form_label(ui, "Wall thickness");
                expression_field(ui, &params, &mut sess.thin_wall.thickness, "mm");
                ui.end_row();
            }
            form_label(ui, "Operation");
            operation_combo(ui, "extrude_op", &mut sess.operation);
            ui.end_row();
            preview_status(ui, preview_error.as_deref());
        });

        let feature = match (sess.sketch, sess.profile_point) {
            (Some(sketch), Some(profile_point)) => Some(ExtrudeFeature {
                sketch,
                profile_point,
                distance: sess.distance_expr(),
                symmetric: sess.symmetric,
                operation: sess.operation,
                distance2: sess.two_sides.then(|| match sess.flip {
                    true => format!("-({})", sess.distance2.trim()),
                    false => sess.distance2.clone(),
                }),
                taper: sess.taper.clone(),
                extent: match (sess.extent, sess.to) {
                    (Extent::Distance, _) => ExtrudeExtent::Distance,
                    (Extent::All, _) => ExtrudeExtent::All,
                    (Extent::To, Some(p)) => ExtrudeExtent::To(p),
                    (Extent::To, None) => ExtrudeExtent::Distance,
                },
                thin: sess.thin.then(|| sess.thin_wall.clone()),
            }),
            _ => None,
        };
        let editing = sess.editing;
        match action {
            DialogAction::Ok => {
                let Some(feature) = feature else { return };
                let sketch = feature.sketch;
                self.mode = Mode::Model;
                self.checkpoint("Extrude");
                apply_feature(&mut self.doc, editing, "Extrude", FeatureKind::Extrude(feature));
                self.hidden_sketches.insert(sketch);
                self.finish_body_command();
            }
            DialogAction::Cancel => self.cancel(),
            DialogAction::None => match feature.filter(|_| ready) {
                Some(f) => {
                    let key = format!("{editing:?} {f:?}");
                    self.update_preview(key, |doc| apply_feature(doc, editing, "Extrude", FeatureKind::Extrude(f)));
                }
                None => self.preview = None,
            },
        }
    }

    /// After OK in a body-creating dialog: rebuild, and frame the first body.
    fn finish_body_command(&mut self) {
        let first_body = self.bodies.is_empty();
        self.message = None;
        self.rebuild();
        if first_body && !self.bodies.is_empty() {
            self.set_view(ferrocad_view::ViewPreset::Home);
        }
    }

    // -------------------------------------------------------------- fillet

    pub fn fillet_viewport(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
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
                    if polyline_midpoint(e).is_some_and(|m| ferrocad_model::geom::distance(m, r.point) < 1e-3) {
                        draw(e, COL_PICKED);
                    }
                }
            }
        }
    }

    pub fn fillet_window(&mut self, ctx: &egui::Context) {
        let params = self.doc.params.clone();
        let viewport = self.viewport_rect;
        let preview_error = self.preview.as_ref().and_then(|p| p.error.clone());
        let Mode::Fillet(sess) = &mut self.mode else { return };
        let (title, icon) = if sess.chamfer { ("Chamfer", Icon::Chamfer) } else { ("Fillet", Icon::Fillet) };
        let ready = !sess.edges.is_empty()
            && params.eval(&sess.radius).is_ok()
            && (!(sess.chamfer && sess.two_distances) || params.eval(&sess.distance2).is_ok());

        let action = command_dialog(ctx, viewport, title, icon, ready, |ui| {
            form_label(ui, "Edges");
            let n = sess.edges.len();
            selection_status(ui, n > 0, &format!("{n} selected"), "Select edges");
            ui.end_row();
            form_label(ui, "Type");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut sess.chamfer, false, "Fillet");
                ui.selectable_value(&mut sess.chamfer, true, "Chamfer");
            });
            ui.end_row();
            if sess.chamfer {
                form_label(ui, "Chamfer type");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut sess.two_distances, false, "Equal distance");
                    ui.selectable_value(&mut sess.two_distances, true, "Two distances");
                });
                ui.end_row();
            }
            let two = sess.chamfer && sess.two_distances;
            form_label(ui, if two { "Distance 1" } else if sess.chamfer { "Distance" } else { "Radius" });
            expression_field(ui, &params, &mut sess.radius, "mm");
            ui.end_row();
            if two {
                form_label(ui, "Distance 2");
                expression_field(ui, &params, &mut sess.distance2, "mm");
                ui.end_row();
                form_label(ui, "");
                if ui.button("Flip").on_hover_text("Swap the distances between the two faces").clicked() {
                    std::mem::swap(&mut sess.radius, &mut sess.distance2);
                }
                ui.end_row();
            }
            preview_status(ui, preview_error.as_deref());
        });

        let preview = ready.then(|| (sess.editing, sess.feature()));
        match action {
            DialogAction::Ok => {
                let Mode::Fillet(sess) = std::mem::replace(&mut self.mode, Mode::Model) else { return };
                let feature = sess.feature();
                let prefix = if feature.chamfer { "Chamfer" } else { "Fillet" };
                self.checkpoint(prefix);
                apply_feature(&mut self.doc, sess.editing, prefix, FeatureKind::Fillet(feature));
                self.message = None;
                self.rebuild();
            }
            DialogAction::Cancel => self.cancel(),
            DialogAction::None => match preview {
                Some((editing, f)) => {
                    let key = format!("{editing:?} {f:?}");
                    let prefix = if f.chamfer { "Chamfer" } else { "Fillet" };
                    self.update_preview(key, |doc| apply_feature(doc, editing, prefix, FeatureKind::Fillet(f)));
                }
                None => self.preview = None,
            },
        }
    }

    // ---------------------------------------------------------- parameters

    pub fn parameters_window(&mut self, ctx: &egui::Context) {
        if self.param_edits.is_empty() {
            self.param_edits = self.doc.params.iter().map(|p| (p.name.clone(), p.expression.clone())).collect();
        }
        let mut open = true;
        let mut changed: Option<(String, String)> = None;
        let mut remove: Option<String> = None;
        let mut add = false;
        // Model parameters: (feature, slot, feature name, label, expression).
        // Slots ≥ SKETCH_SLOT are sketch dimensions (slot − SKETCH_SLOT = constraint id).
        const SKETCH_SLOT: usize = 1_000_000;
        let mut rows: Vec<(FeatureId, usize, String, String, String)> = Vec::new();
        for f in self.doc.features() {
            match &f.kind {
                FeatureKind::Sketch(sf) => {
                    for (cid, c) in sf.sketch.constraints().filter(|(_, c)| c.is_dimension()) {
                        let text = sf.expression_for(cid).map(str::to_string).unwrap_or_else(|| match c {
                            ferrocad_sketch::Constraint::Angle(_, _, v) => fmt_value(v.to_degrees(), "°"),
                            _ => fmt_value(c.value().unwrap_or(0.0), "mm"),
                        });
                        rows.push((f.id, SKETCH_SLOT + cid.0, f.name.clone(), c.label().to_string(), text));
                    }
                }
                kind => {
                    let mut k = kind.clone();
                    for (slot, (label, text)) in k.expressions_mut().into_iter().enumerate() {
                        rows.push((f.id, slot, f.name.clone(), label.to_string(), text.clone()));
                    }
                }
            }
        }
        let mut model_change: Option<(FeatureId, usize, String)> = None;

        egui::Window::new("Parameters")
            .id(egui::Id::new("parameters_window"))
            .title_bar(false)
            .frame(theme::dialog_frame())
            .default_pos(self.viewport_rect.center_top() + egui::vec2(-220.0, 60.0))
            .resizable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 7))
                    .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                            icons::paint(ui.painter(), r, Icon::Parameters, theme::ICON, true);
                            ui.label(RichText::new("PARAMETERS").strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icons::icon_button(ui, Icon::Cancel, 18.0, "Close", true, false).clicked() {
                                    open = false;
                                }
                            });
                        });
                    });
                egui::Frame::new().inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    theme::section(ui, "User parameters");
                    egui::Grid::new("params").num_columns(4).striped(true).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.weak("Name");
                        ui.weak("Expression");
                        ui.weak("Value");
                        ui.end_row();
                        for (name, expr) in &mut self.param_edits {
                            ui.label(RichText::new(name.as_str()).strong());
                            let r = ui.add(egui::TextEdit::singleline(expr).desired_width(150.0));
                            if r.lost_focus() {
                                changed = Some((name.clone(), expr.clone()));
                            }
                            match self.doc.params.value(name) {
                                Ok(v) => ui.label(format!("{v:.3}")),
                                Err(e) => ui.colored_label(theme::ERROR, e.to_string()),
                            };
                            if icons::icon_button(ui, Icon::Delete, 20.0, "Delete parameter", true, false).clicked() {
                                remove = Some(name.clone());
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    theme::section(ui, "Add parameter");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.new_param.0).hint_text("name").desired_width(90.0));
                        ui.add(egui::TextEdit::singleline(&mut self.new_param.1).hint_text("expression").desired_width(150.0));
                        if theme::primary_button(ui, "Add", !self.new_param.0.trim().is_empty()).clicked() {
                            add = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.weak("Units: mm cm m in · deg rad · functions: sqrt sin cos tan min max …");
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(RichText::new("MODEL PARAMETERS").small().strong().color(theme::TEXT_WEAK))
                        .id_salt("model_params")
                        .default_open(true)
                        .show(ui, |ui| {
                            if rows.is_empty() {
                                ui.weak("No dimensions or feature values yet.");
                            }
                            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                                egui::Grid::new("model_params_grid").num_columns(4).striped(true).spacing([10.0, 4.0]).show(
                                    ui,
                                    |ui| {
                                        let mut last = None;
                                        for (fid, slot, fname, label, text) in &rows {
                                            if last != Some(*fid) {
                                                ui.label(RichText::new(fname).strong());
                                                last = Some(*fid);
                                            } else {
                                                ui.label("");
                                            }
                                            ui.weak(label);
                                            let buf = self.model_param_edits.entry((*fid, *slot)).or_insert_with(|| text.clone());
                                            let r = ui.add(egui::TextEdit::singleline(buf).desired_width(130.0));
                                            if r.lost_focus() && buf.trim() != text.trim() {
                                                model_change = Some((*fid, *slot, buf.clone()));
                                            }
                                            let unit = if label.contains('°') || label.contains("Angle") || label == "Taper" {
                                                "°"
                                            } else if label.starts_with("Count") {
                                                ""
                                            } else {
                                                "mm"
                                            };
                                            match self.doc.params.eval(buf) {
                                                Ok(v) => ui.weak(fmt_value(v, unit)),
                                                Err(e) => ui.colored_label(theme::ERROR, e.to_string()),
                                            };
                                            ui.end_row();
                                        }
                                    },
                                );
                            });
                        });
                });
            });

        if let Some((fid, slot, text)) = model_change {
            self.model_param_edits.clear();
            match self.doc.params.eval(&text) {
                Err(e) => self.error(e.to_string()),
                Ok(_) => {
                    self.checkpoint("Change model parameter");
                    if let Some(f) = self.doc.feature_mut(fid) {
                        match &mut f.kind {
                            FeatureKind::Sketch(sf) if slot >= SKETCH_SLOT => {
                                sf.set_expression(ferrocad_sketch::ConstraintId(slot - SKETCH_SLOT), text.trim());
                            }
                            kind => {
                                if let Some((_, e)) = kind.expressions_mut().into_iter().nth(slot) {
                                    *e = text.trim().to_string();
                                }
                            }
                        }
                    }
                    self.message = None;
                    self.rebuild();
                }
            }
        }

        let mut needs_rebuild = false;
        let before = (changed.is_some() || remove.is_some() || add).then(|| self.doc.clone());
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
            if let Some(before) = before {
                self.record("Change parameters", before);
            }
            self.message = None;
            self.rebuild();
        }
        if !open {
            self.show_params = false;
        }
    }
}

// ------------------------------------------------------------------ revolve

pub struct RevolveSession {
    pub editing: Option<FeatureId>,
    pub sketch: Option<FeatureId>,
    pub profile_point: Option<[f64; 2]>,
    pub axis: RevolveAxis,
    pub angle: String,
    pub operation: Operation,
}

impl RevolveSession {
    pub fn new(doc: &Document) -> Self {
        Self {
            editing: None,
            sketch: ExtrudeSession::new(doc).sketch,
            profile_point: None,
            axis: RevolveAxis::SketchY,
            angle: "360°".into(),
            operation: Operation::NewBody,
        }
    }

    pub fn edit(id: FeatureId, r: &RevolveFeature) -> Self {
        Self {
            editing: Some(id),
            sketch: Some(r.sketch),
            profile_point: Some(r.profile_point),
            axis: r.axis,
            angle: r.angle.clone(),
            operation: r.operation,
        }
    }
}

/// World-space axis (point, direction) of a revolve axis choice.
fn axis_world(
    sf: &SketchFeature,
    axis: RevolveAxis,
    other: &dyn Fn(ferrocad_model::AxisRef) -> Option<([f64; 3], [f64; 3])>,
) -> Option<([f64; 3], [f64; 3])> {
    match axis {
        RevolveAxis::Axis(a) => other(a),
        RevolveAxis::SketchX => Some((sf.plane.origin, sf.plane.x_axis)),
        RevolveAxis::SketchY => Some((sf.plane.origin, sf.plane.y_axis)),
        RevolveAxis::Line(l) => match *sf.sketch.entity(l) {
            ferrocad_sketch::Entity::Line { a, b } if sf.sketch.is_alive(l) => {
                let a = sf.plane.to_world(sf.sketch.point_pos(a));
                let b = sf.plane.to_world(sf.sketch.point_pos(b));
                Some((a, ferrocad_model::geom::sub(b, a)))
            }
            _ => None,
        },
    }
}

fn axis_label(sf: &SketchFeature, axis: RevolveAxis) -> String {
    match axis {
        RevolveAxis::Axis(a) => a.label(),
        RevolveAxis::SketchX => "Sketch X axis".into(),
        RevolveAxis::SketchY => "Sketch Y axis".into(),
        RevolveAxis::Line(l) => {
            let kind = if sf.sketch.is_construction(l) { "construction line" } else { "line" };
            match axis_world(sf, axis, &|_| None) {
                Some((_, d)) => format!("{kind} #{} ({:.1} mm)", l.0, ferrocad_model::geom::length(d)),
                None => format!("{kind} #{} (deleted)", l.0),
            }
        }
    }
}

impl FerroApp {
    pub fn revolve_viewport(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        let cam = self.camera;
        let axes: Vec<(ferrocad_model::AxisRef, AxisLine)> = (0..3)
            .map(ferrocad_model::AxisRef::World)
            .chain(self.construction_axes().into_iter().map(|(id, _)| ferrocad_model::AxisRef::Construction(id)))
            .filter_map(|a| self.axis_of(&a).map(|v| (a, v)))
            .collect();
        let Mode::Revolve(sess) = &mut self.mode else { return };
        let Some(sf) = sess.sketch.and_then(|id| self.doc.sketch(id)) else { return };
        pick_profile(sf, &mut sess.profile_point, response, painter, vp, &cam);

        // Show the axis as a long dashed line.
        let other = |a| axes.iter().find(|(r, _)| *r == a).map(|(_, v)| *v);
        if let Some((o, d)) = axis_world(sf, sess.axis, &other) {
            let d = ferrocad_model::geom::normalize(d);
            let reach = cam.distance * 2.0;
            let a = ferrocad_model::geom::add(o, ferrocad_model::geom::scale(d, -reach));
            let b = ferrocad_model::geom::add(o, ferrocad_model::geom::scale(d, reach));
            if let (Some(a), Some(b)) = (cam.project(a, vp), cam.project(b, vp)) {
                let pts = [egui::pos2(a[0] as f32, a[1] as f32), egui::pos2(b[0] as f32, b[1] as f32)];
                painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.5, COL_PICKED), 8.0, 6.0));
            }
        }
    }

    pub fn revolve_window(&mut self, ctx: &egui::Context) {
        let sketches = self.sketches_before_marker();
        let params = self.doc.params.clone();
        let preview_error = self.preview.as_ref().and_then(|p| p.error.clone());
        let viewport = self.viewport_rect;
        let Mode::Revolve(sess) = &mut self.mode else { return };
        let sketch_feature = sess.sketch.and_then(|id| self.doc.sketch(id));
        // Axis choices: the sketch axes plus every line in the sketch.
        let axes: Vec<(RevolveAxis, String)> = match sketch_feature {
            Some(sf) => {
                let mut v = vec![RevolveAxis::SketchX, RevolveAxis::SketchY];
                v.extend(
                    sf.sketch
                        .entities()
                        .filter(|(_, e)| matches!(e, ferrocad_sketch::Entity::Line { .. }))
                        .map(|(id, _)| RevolveAxis::Line(id)),
                );
                v.extend((0..3).map(|i| RevolveAxis::Axis(ferrocad_model::AxisRef::World(i))));
                v.extend(
                    self.model
                        .axes
                        .keys()
                        .map(|id| RevolveAxis::Axis(ferrocad_model::AxisRef::Construction(*id))),
                );
                v.into_iter().map(|a| (a, axis_label(sf, a))).collect()
            }
            None => vec![],
        };
        let ready = sess.sketch.is_some() && sess.profile_point.is_some() && params.eval(&sess.angle).is_ok();

        let action = command_dialog(ctx, viewport, "Revolve", Icon::Revolve, ready, |ui| {
            form_label(ui, "Sketch");
            if sketch_combo(ui, "revolve_sketch", &sketches, &mut sess.sketch) {
                sess.profile_point = None;
                sess.axis = RevolveAxis::SketchY;
            }
            ui.end_row();
            form_label(ui, "Profile");
            selection_status(ui, sess.profile_point.is_some(), "1 selected", "Select a profile");
            ui.end_row();
            form_label(ui, "Axis");
            let axis_text =
                axes.iter().find(|(a, _)| *a == sess.axis).map(|(_, l)| l.clone()).unwrap_or_else(|| "–".into());
            egui::ComboBox::from_id_salt("revolve_axis").selected_text(axis_text).width(150.0).show_ui(ui, |ui| {
                for (a, label) in &axes {
                    ui.selectable_value(&mut sess.axis, *a, label.as_str());
                }
            });
            ui.end_row();
            form_label(ui, "Angle");
            expression_field(ui, &params, &mut sess.angle, "°");
            ui.end_row();
            form_label(ui, "Operation");
            operation_combo(ui, "revolve_op", &mut sess.operation);
            ui.end_row();
            preview_status(ui, preview_error.as_deref());
        });

        let feature = match (sess.sketch, sess.profile_point) {
            (Some(sketch), Some(profile_point)) => Some(RevolveFeature {
                sketch,
                profile_point,
                axis: sess.axis,
                angle: sess.angle.clone(),
                operation: sess.operation,
            }),
            _ => None,
        };
        let editing = sess.editing;
        match action {
            DialogAction::Ok => {
                let Some(feature) = feature else { return };
                let sketch = feature.sketch;
                self.mode = Mode::Model;
                self.checkpoint("Revolve");
                apply_feature(&mut self.doc, editing, "Revolve", FeatureKind::Revolve(feature));
                self.hidden_sketches.insert(sketch);
                self.finish_body_command();
            }
            DialogAction::Cancel => self.cancel(),
            DialogAction::None => match feature.filter(|_| ready) {
                Some(f) => {
                    let key = format!("{editing:?} {f:?}");
                    self.update_preview(key, |doc| apply_feature(doc, editing, "Revolve", FeatureKind::Revolve(f)));
                }
                None => self.preview = None,
            },
        }
    }
}
