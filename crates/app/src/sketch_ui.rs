//! Sketch mode: drawing tools, snapping, dragging, constraints, dimensions.

use crate::app::{FerroApp, Mode};
use egui::{Color32, Pos2, Stroke};
use ferrocad_model::{FeatureId, Plane};
use ferrocad_sketch::solver::Analysis;
use ferrocad_sketch::{Constraint, ConstraintId, Entity, EntityId, PointId, Segment, Sketch};
use ferrocad_view::{point_segment_distance, Camera, Viewport};
use std::collections::HashMap;

const SNAP_PX: f64 = 10.0;
const PICK_PX: f64 = 8.0;

const COL_FREE: Color32 = Color32::from_rgb(110, 175, 255);
const COL_FIXED: Color32 = Color32::from_rgb(235, 235, 235);
const COL_CONSTRUCTION: Color32 = Color32::from_rgb(230, 150, 60);
const COL_SELECTED: Color32 = Color32::from_rgb(255, 215, 70);
const COL_PREVIEW: Color32 = Color32::from_rgb(160, 160, 160);
const COL_DIM: Color32 = Color32::from_rgb(200, 220, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
}

impl Tool {
    const ALL: [(Tool, &'static str); 5] = [
        (Tool::Select, "⬉ Select"),
        (Tool::Line, "╱ Line"),
        (Tool::Rectangle, "▭ Rectangle"),
        (Tool::Circle, "◯ Circle"),
        (Tool::Arc, "◜ Arc (center, start, end)"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Point(PointId),
    Entity(EntityId),
}

/// A clicked location: either snapped to an existing point or free.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Existing(PointId),
    New([f64; 2]),
}

impl Anchor {
    fn pos(&self, sk: &Sketch) -> [f64; 2] {
        match *self {
            Anchor::Existing(p) => sk.point_pos(p),
            Anchor::New(uv) => uv,
        }
    }

    fn existing(&self) -> Option<PointId> {
        match *self {
            Anchor::Existing(p) => Some(p),
            Anchor::New(_) => None,
        }
    }
}

pub struct SketchSession {
    pub feature: FeatureId,
    pub tool: Tool,
    pub clicks: Vec<Anchor>,
    pub selection: Vec<Pick>,
    pub dragging: Option<PointId>,
    pub analysis: Option<Analysis>,
    pub dim_edits: HashMap<ConstraintId, String>,
}

impl SketchSession {
    pub fn new(feature: FeatureId) -> Self {
        Self {
            feature,
            tool: Tool::Line,
            clicks: Vec::new(),
            selection: Vec::new(),
            dragging: None,
            analysis: None,
            dim_edits: HashMap::new(),
        }
    }
}

// ------------------------------------------------------------------ geometry

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Polyline of an entity in sketch coordinates.
fn entity_polyline(sk: &Sketch, e: &Entity) -> Vec<[f64; 2]> {
    match *e {
        Entity::Line { a, b } => vec![sk.point_pos(a), sk.point_pos(b)],
        Entity::Circle { center, radius } => {
            let seg = Segment::Circle { center: sk.point_pos(center), radius: sk.param(radius), ccw: true };
            seg.sample(64)
        }
        Entity::Arc { center, start, end } => Segment::Arc {
            center: sk.point_pos(center),
            from: sk.point_pos(start),
            to: sk.point_pos(end),
            ccw: true,
        }
        .sample(32),
    }
}

fn line_dir(sk: &Sketch, l: EntityId) -> Option<[f64; 2]> {
    match *sk.entity(l) {
        Entity::Line { a, b } => {
            let (a, b) = (sk.point_pos(a), sk.point_pos(b));
            Some([b[0] - a[0], b[1] - a[1]])
        }
        _ => None,
    }
}

/// Screen projection helpers for one sketch plane.
struct Projector<'a> {
    cam: &'a Camera,
    plane: Plane,
    vp: &'a Viewport,
}

impl Projector<'_> {
    fn screen(&self, uv: [f64; 2]) -> Option<Pos2> {
        self.cam
            .project(self.plane.to_world(uv), self.vp)
            .map(|s| egui::pos2(s[0] as f32, s[1] as f32))
    }

    fn local(&self, pos: Pos2) -> Option<[f64; 2]> {
        let (o, d) = self.cam.ray([pos.x as f64, pos.y as f64], self.vp);
        self.plane.intersect_ray(o, d).map(|w| self.plane.to_local(w))
    }

    fn polyline(&self, pts: &[[f64; 2]]) -> Vec<Pos2> {
        pts.iter().filter_map(|p| self.screen(*p)).collect()
    }
}

fn visible_points(sk: &Sketch) -> Vec<PointId> {
    let mut pts: Vec<PointId> = sk.entities().flat_map(|(_, e)| e.points()).collect();
    pts.sort();
    pts.dedup();
    pts
}

fn pick_point(sk: &Sketch, pr: &Projector, pos: Pos2) -> Option<PointId> {
    visible_points(sk)
        .into_iter()
        .filter_map(|p| pr.screen(sk.point_pos(p)).map(|s| (p, s.distance(pos) as f64)))
        .filter(|(_, d)| *d < SNAP_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p)
}

fn pick_entity(sk: &Sketch, pr: &Projector, pos: Pos2) -> Option<EntityId> {
    let p = [pos.x as f64, pos.y as f64];
    sk.entities()
        .map(|(id, e)| {
            let pts = pr.polyline(&entity_polyline(sk, e));
            let d = pts
                .windows(2)
                .map(|w| point_segment_distance(p, [w[0].x as f64, w[0].y as f64], [w[1].x as f64, w[1].y as f64]))
                .fold(f64::INFINITY, f64::min);
            (id, d)
        })
        .filter(|(_, d)| *d < PICK_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Add a constraint only if it solves and does not over-constrain.
fn try_constrain(sk: &mut Sketch, c: Constraint) -> Result<ConstraintId, String> {
    let before = sk.analyze().redundant;
    let backup = sk.clone();
    let id = sk.constrain(c);
    if let Err(e) = sk.solve() {
        *sk = backup;
        return Err(format!("not possible: {e}"));
    }
    if sk.analyze().redundant > before {
        *sk = backup;
        return Err("would over-constrain the sketch".into());
    }
    Ok(id)
}

/// Current measured value of a dimension (angles in radians).
fn measure(sk: &Sketch, c: &Constraint) -> Option<f64> {
    Some(match *c {
        Constraint::Length(l, _) => {
            let d = line_dir(sk, l)?;
            d[0].hypot(d[1])
        }
        Constraint::Distance(a, b, _) => dist(sk.point_pos(a), sk.point_pos(b)),
        Constraint::Radius(e, _) => sk.radius(e)?,
        Constraint::Diameter(e, _) => 2.0 * sk.radius(e)?,
        Constraint::Angle(l1, l2, _) => {
            let (d1, d2) = (line_dir(sk, l1)?, line_dir(sk, l2)?);
            (d1[0] * d2[1] - d1[1] * d2[0]).atan2(d1[0] * d2[0] + d1[1] * d2[1])
        }
        Constraint::HorizontalDistance(a, b, _) => sk.point_pos(b)[0] - sk.point_pos(a)[0],
        Constraint::VerticalDistance(a, b, _) => sk.point_pos(b)[1] - sk.point_pos(a)[1],
        _ => return None,
    })
}

/// Where to draw a dimension's value.
fn dimension_anchor(sk: &Sketch, c: &Constraint) -> Option<[f64; 2]> {
    let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    match *c {
        Constraint::Length(l, _) => match *sk.entity(l) {
            Entity::Line { a, b } => Some(mid(sk.point_pos(a), sk.point_pos(b))),
            _ => None,
        },
        Constraint::Distance(a, b, _)
        | Constraint::HorizontalDistance(a, b, _)
        | Constraint::VerticalDistance(a, b, _) => Some(mid(sk.point_pos(a), sk.point_pos(b))),
        Constraint::Radius(e, _) | Constraint::Diameter(e, _) => {
            let r = sk.radius(e)?;
            let c = sk.point_pos(e_center(sk, e)?);
            Some([c[0] + r * 0.707, c[1] + r * 0.707])
        }
        Constraint::Angle(_, l2, _) => match *sk.entity(l2) {
            Entity::Line { a, b } => Some(mid(sk.point_pos(a), sk.point_pos(b))),
            _ => None,
        },
        _ => None,
    }
}

fn e_center(sk: &Sketch, e: EntityId) -> Option<PointId> {
    match *sk.entity(e) {
        Entity::Circle { center, .. } | Entity::Arc { center, .. } => Some(center),
        Entity::Line { .. } => None,
    }
}

fn format_dimension(c: &Constraint, v: f64) -> String {
    match c {
        Constraint::Angle(..) => format!("{:.1}°", v.to_degrees()),
        Constraint::Diameter(..) => format!("⌀{v:.2}"),
        Constraint::Radius(..) => format!("R{v:.2}"),
        _ => format!("{v:.2}"),
    }
}

// ------------------------------------------------------------- viewport part

impl FerroApp {
    pub fn sketch_viewport(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        painter: &egui::Painter,
        vp: &Viewport,
    ) {
        let cam = self.camera;
        let Mode::Sketch(sess) = &mut self.mode else { return };
        // (a missing sketch makes the palette finish the session)
        let Some(sf) = self.doc.sketch_mut(sess.feature) else { return };
        let pr = Projector { cam: &cam, plane: sf.plane, vp };
        let sk = &mut sf.sketch;
        let mut changed = false;
        let mut error: Option<String> = None;

        let keyboard_free = !ui.ctx().egui_wants_keyboard_input();
        if keyboard_free && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if sess.clicks.is_empty() {
                sess.tool = Tool::Select;
                sess.selection.clear();
            }
            sess.clicks.clear();
        }
        if keyboard_free && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            for pick in sess.selection.drain(..) {
                if let Pick::Entity(e) = pick {
                    sk.remove_entity(e);
                    changed = true;
                }
            }
        }

        let hover = response.hover_pos();
        let hover_snap = hover.and_then(|p| pick_point(sk, &pr, p));
        let hover_uv = hover.and_then(|p| match hover_snap {
            Some(s) => Some(sk.point_pos(s)),
            None => pr.local(p),
        });

        // ---- dragging points (select tool)
        if sess.tool == Tool::Select {
            if response.drag_started_by(egui::PointerButton::Primary) {
                sess.dragging = response.interact_pointer_pos().and_then(|p| pick_point(sk, &pr, p));
            }
            if let (Some(p), true) = (sess.dragging, response.dragged_by(egui::PointerButton::Primary)) {
                if let Some(uv) = response.interact_pointer_pos().and_then(|pos| pr.local(pos)) {
                    let _ = sk.drag_point(p, uv); // refuses silently if impossible
                }
            }
            if response.drag_stopped() && sess.dragging.take().is_some() {
                changed = true;
            }
        }

        // ---- clicks
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let snapped = pick_point(sk, &pr, pos);
                let uv = snapped.map(|p| sk.point_pos(p)).or_else(|| pr.local(pos));
                if let Some(uv) = uv {
                    let anchor = snapped.map(Anchor::Existing).unwrap_or(Anchor::New(uv));
                    let shift = ui.input(|i| i.modifiers.shift);
                    changed |= handle_click(sess, sk, anchor, pos, &pr, shift, &mut error);
                }
            }
        }

        if changed {
            if let Err(e) = sk.solve() {
                error = Some(format!("sketch does not solve: {e}"));
            }
            sess.analysis = None;
        }
        if sess.analysis.is_none() {
            sess.analysis = Some(sk.analyze());
        }

        // ---- drawing
        let analysis = sess.analysis.as_ref();
        for (id, e) in sk.entities() {
            let pts = pr.polyline(&entity_polyline(sk, e));
            let selected = sess.selection.contains(&Pick::Entity(id));
            let color = if selected {
                COL_SELECTED
            } else if sk.is_construction(id) {
                COL_CONSTRUCTION
            } else if analysis.is_some_and(|a| sk.is_entity_determined(a, id)) {
                COL_FIXED
            } else {
                COL_FREE
            };
            let width = if selected { 2.5 } else { 1.8 };
            painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
        }
        for p in visible_points(sk) {
            if let Some(s) = pr.screen(sk.point_pos(p)) {
                let selected = sess.selection.contains(&Pick::Point(p));
                let color = if selected {
                    COL_SELECTED
                } else if sk.is_point_fixed(p) {
                    Color32::from_rgb(255, 120, 100)
                } else if analysis.is_some_and(|a| sk.is_point_determined(a, p)) {
                    COL_FIXED
                } else {
                    COL_FREE
                };
                let r = if Some(p) == hover_snap { 5.0 } else { 3.0 };
                painter.circle_filled(s, r, color);
            }
        }
        // dimension values
        for (_, c) in sk.constraints() {
            if let (Some(v), Some(at)) = (c.value(), dimension_anchor(sk, c)) {
                if let Some(s) = pr.screen(at) {
                    painter.text(
                        s + egui::vec2(0.0, -10.0),
                        egui::Align2::CENTER_BOTTOM,
                        format_dimension(c, v),
                        egui::FontId::proportional(13.0),
                        COL_DIM,
                    );
                }
            }
        }
        // rubber band preview
        if let Some(h) = hover_uv {
            let preview = Stroke::new(1.2, COL_PREVIEW);
            let pts: Vec<[f64; 2]> = sess.clicks.iter().map(|a| a.pos(sk)).collect();
            let shape: Vec<[f64; 2]> = match (sess.tool, pts.as_slice()) {
                (Tool::Line, [a]) => vec![*a, h],
                (Tool::Rectangle, [a]) => vec![*a, [h[0], a[1]], h, [a[0], h[1]], *a],
                (Tool::Circle, [c]) => {
                    Segment::Circle { center: *c, radius: dist(*c, h), ccw: true }.sample(64)
                }
                (Tool::Arc, [c]) => vec![*c, h],
                (Tool::Arc, [c, s]) => Segment::Arc { center: *c, from: *s, to: h, ccw: true }.sample(32),
                _ => vec![],
            };
            if shape.len() > 1 {
                painter.add(egui::Shape::line(pr.polyline(&shape), preview));
            }
        }

        if changed {
            self.commit_sketch_edit("Sketch edit");
        }
        if let Some(e) = error {
            self.error(e);
        }
    }

    // --------------------------------------------------------- palette window

    pub fn sketch_palette(&mut self, ctx: &egui::Context) {
        let mut finish = false;
        egui::Window::new("Sketch")
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-330.0, 60.0))
            .default_width(300.0)
            .resizable(true)
            .show(ctx, |ui| finish = self.sketch_palette_ui(ui));
        if finish {
            self.mode = Mode::Model;
            self.rebuild();
            if self.message.as_ref().is_some_and(|(_, err)| !err) {
                self.message = None;
            }
        }
    }

    /// Returns true when the user finishes the sketch.
    fn sketch_palette_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let params = self.doc.params.clone();
        let Mode::Sketch(sess) = &mut self.mode else { return true };
        let name = self.doc.feature(sess.feature).map(|f| f.name.clone()).unwrap_or_default();
        let Some(sf) = self.doc.sketch_mut(sess.feature) else { return true };
        let mut message: Option<(String, bool)> = None;
        // Cheap per-frame copy of just this sketch to detect edits for undo.
        let snapshot = (sf.sketch.to_data(), sf.driven.clone());

        ui.horizontal(|ui| {
            ui.strong(name);
            if let Some(a) = &sess.analysis {
                if a.is_fully_constrained() {
                    ui.colored_label(COL_FIXED, "● fully constrained");
                } else {
                    ui.colored_label(COL_FREE, format!("○ {} DOF left", a.dof));
                }
            }
        });
        ui.separator();

        // ---- tools
        for (tool, label) in Tool::ALL {
            if ui.selectable_label(sess.tool == tool, label).clicked() {
                sess.tool = tool;
                sess.clicks.clear();
            }
        }
        ui.separator();

        // ---- constraints on the selection
        let sk = &mut sf.sketch;
        let points: Vec<PointId> =
            sess.selection.iter().filter_map(|p| if let Pick::Point(p) = p { Some(*p) } else { None }).collect();
        let entities: Vec<EntityId> =
            sess.selection.iter().filter_map(|p| if let Pick::Entity(e) = p { Some(*e) } else { None }).collect();
        let lines: Vec<EntityId> =
            entities.iter().copied().filter(|e| matches!(sk.entity(*e), Entity::Line { .. })).collect();
        let curves: Vec<EntityId> =
            entities.iter().copied().filter(|e| !matches!(sk.entity(*e), Entity::Line { .. })).collect();
        let (np, nl, nc) = (points.len(), lines.len(), curves.len());
        let only = |p, l, c| np == p && nl == l && nc == c;

        let mut wanted: Vec<Constraint> = Vec::new();
        let mut new_dimension: Option<Constraint> = None;
        ui.label("Constraints (select geometry first)");
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(nl > 0 && np == 0 && nc == 0, egui::Button::new("━ Horizontal")).clicked() {
                wanted.extend(lines.iter().map(|&l| Constraint::Horizontal(l)));
            }
            if ui.add_enabled(nl > 0 && np == 0 && nc == 0, egui::Button::new("┃ Vertical")).clicked() {
                wanted.extend(lines.iter().map(|&l| Constraint::Vertical(l)));
            }
            let coincident_ok = only(2, 0, 0) || only(1, 1, 0) || only(1, 0, 1);
            if ui.add_enabled(coincident_ok, egui::Button::new("● Coincident")).clicked() {
                if np == 2 {
                    wanted.push(Constraint::Coincident(points[0], points[1]));
                } else if nl == 1 {
                    wanted.push(Constraint::PointOnLine(points[0], lines[0]));
                } else {
                    wanted.push(Constraint::PointOnCircle(points[0], curves[0]));
                }
            }
            if ui.add_enabled(only(0, 2, 0), egui::Button::new("∥ Parallel")).clicked() {
                wanted.push(Constraint::Parallel(lines[0], lines[1]));
            }
            if ui.add_enabled(only(0, 2, 0), egui::Button::new("⊥ Perpendicular")).clicked() {
                wanted.push(Constraint::Perpendicular(lines[0], lines[1]));
            }
            if ui.add_enabled(only(0, 2, 0) || only(0, 0, 2), egui::Button::new("= Equal")).clicked() {
                let e = if nl == 2 { &lines } else { &curves };
                wanted.push(Constraint::Equal(e[0], e[1]));
            }
            if ui.add_enabled(only(0, 1, 1), egui::Button::new("◡ Tangent")).clicked() {
                wanted.push(Constraint::Tangent(lines[0], curves[0]));
            }
            if ui.add_enabled(only(0, 0, 2), egui::Button::new("◎ Concentric")).clicked() {
                wanted.push(Constraint::Concentric(curves[0], curves[1]));
            }
            if ui.add_enabled(only(1, 1, 0), egui::Button::new("⋯ Midpoint")).clicked() {
                wanted.push(Constraint::Midpoint(points[0], lines[0]));
            }
            if ui.add_enabled(np > 0 && nl == 0 && nc == 0, egui::Button::new("🔒 Fix/Unfix")).clicked() {
                for &p in &points {
                    let fixed = sk.is_point_fixed(p);
                    sk.set_point_fixed(p, !fixed);
                }
                sess.analysis = None;
            }
        });

        // ---- dimension
        let dim_ok = only(0, 1, 0) || only(0, 0, 1) || only(2, 0, 0) || only(0, 2, 0);
        ui.horizontal(|ui| {
            if ui.add_enabled(dim_ok, egui::Button::new("↔ Dimension")).clicked() {
                let c = if nl == 1 {
                    Constraint::Length(lines[0], 0.0)
                } else if nc == 1 {
                    match sk.entity(curves[0]) {
                        Entity::Circle { .. } => Constraint::Diameter(curves[0], 0.0),
                        _ => Constraint::Radius(curves[0], 0.0),
                    }
                } else if np == 2 {
                    Constraint::Distance(points[0], points[1], 0.0)
                } else {
                    Constraint::Angle(lines[0], lines[1], 0.0)
                };
                new_dimension = Some(c);
            }
            let any_entity = !entities.is_empty();
            if ui.add_enabled(any_entity, egui::Button::new("┄ Construction")).clicked() {
                for &e in &entities {
                    let c = sk.is_construction(e);
                    sk.set_construction(e, !c);
                }
            }
            if ui.add_enabled(any_entity, egui::Button::new("🗑 Delete")).clicked() {
                for &e in &entities {
                    sk.remove_entity(e);
                }
                sess.selection.clear();
                sess.analysis = None;
            }
        });

        for c in wanted {
            match try_constrain(sk, c) {
                Ok(_) => sess.analysis = None,
                Err(e) => message = Some((e, true)),
            }
        }
        if let Some(mut c) = new_dimension {
            // Start from the current size so nothing jumps.
            if let (Some(v), Some(slot)) = (measure(sk, &c), c.value_mut()) {
                *slot = v;
            }
            match try_constrain(sk, c.clone()) {
                Ok(id) => {
                    sess.analysis = None;
                    sess.selection.clear();
                    let shown = match c {
                        Constraint::Angle(_, _, v) => format!("{:.3}", v.to_degrees()),
                        _ => format!("{:.3}", c.value().unwrap_or(0.0)),
                    };
                    sess.dim_edits.insert(id, shown);
                    message = Some(("dimension added – type a value or parameter name below".into(), false));
                }
                Err(e) => message = Some((e, true)),
            }
        }

        ui.separator();

        // ---- dimension table
        ui.label("Dimensions (values or parameter expressions)");
        let dims: Vec<(ConstraintId, Constraint)> =
            sk.constraints().filter(|(_, c)| c.is_dimension()).map(|(id, c)| (id, c.clone())).collect();
        let mut remove: Option<ConstraintId> = None;
        egui::Grid::new("dims").num_columns(3).striped(true).show(ui, |ui| {
            for (id, c) in &dims {
                ui.label(c.label());
                let text = sess.dim_edits.entry(*id).or_insert_with(|| {
                    sf.driven
                        .iter()
                        .find(|(d, _)| d == id)
                        .map(|(_, e)| e.clone())
                        .unwrap_or_else(|| match c {
                            Constraint::Angle(_, _, v) => format!("{:.3}", v.to_degrees()),
                            _ => format!("{:.3}", c.value().unwrap_or(0.0)),
                        })
                });
                let resp = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                let commit = resp.lost_focus();
                if ui.small_button("✕").clicked() {
                    remove = Some(*id);
                }
                ui.end_row();

                if commit {
                    let expr = text.clone();
                    match params.eval(&expr) {
                        Ok(v) => {
                            let v = if matches!(c, Constraint::Angle(..)) { v.to_radians() } else { v };
                            let old = c.value().unwrap_or(0.0);
                            sf.sketch.set_dimension(*id, v);
                            match sf.sketch.solve() {
                                Ok(_) => {
                                    sf.set_expression(*id, expr);
                                    sess.analysis = None;
                                }
                                Err(e) => {
                                    sf.sketch.set_dimension(*id, old);
                                    message = Some((format!("value not possible: {e}"), true));
                                }
                            }
                        }
                        Err(e) => message = Some((e.to_string(), true)),
                    }
                }
            }
        });
        if let Some(id) = remove {
            sf.sketch.remove_constraint(id);
            sf.driven.retain(|(d, _)| *d != id);
            sess.dim_edits.remove(&id);
            sess.analysis = None;
        }

        // ---- geometric constraints list
        egui::CollapsingHeader::new("Geometric constraints").show(ui, |ui| {
            let geo: Vec<(ConstraintId, &'static str)> = sf
                .sketch
                .constraints()
                .filter(|(_, c)| !c.is_dimension())
                .map(|(id, c)| (id, c.label()))
                .collect();
            for (id, label) in geo {
                ui.horizontal(|ui| {
                    ui.label(label);
                    if ui.small_button("✕").clicked() {
                        sf.sketch.remove_constraint(id);
                        sess.analysis = None;
                    }
                });
            }
        });

        ui.separator();
        let finish = ui.button("✔ Finish sketch").clicked();
        ui.weak("Esc: cancel / select tool · Del: delete selection · drag points with Select");
        ui.weak("Ctrl+Z / Ctrl+Shift+Z: undo / redo");

        let edited = (sf.sketch.to_data(), sf.driven.clone()) != snapshot;
        if let Some(m) = message {
            self.message = Some(m);
        }
        if edited {
            self.commit_sketch_edit("Sketch edit");
        }
        finish
    }
}

/// Apply a viewport click for the active tool. Returns true if geometry changed.
fn handle_click(
    sess: &mut SketchSession,
    sk: &mut Sketch,
    anchor: Anchor,
    pos: Pos2,
    pr: &Projector,
    shift: bool,
    error: &mut Option<String>,
) -> bool {
    let resolve = |a: Anchor, sk: &mut Sketch| match a {
        Anchor::Existing(p) => p,
        Anchor::New(uv) => sk.add_point(uv),
    };
    match sess.tool {
        Tool::Select => {
            let pick = anchor
                .existing()
                .map(Pick::Point)
                .or_else(|| pick_entity(sk, pr, pos).map(Pick::Entity));
            match pick {
                Some(p) if shift => {
                    if let Some(i) = sess.selection.iter().position(|q| *q == p) {
                        sess.selection.remove(i);
                    } else {
                        sess.selection.push(p);
                    }
                }
                Some(p) => sess.selection = vec![p],
                None => sess.selection.clear(),
            }
            false
        }
        Tool::Line => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return false;
            }
            let (a0, b0) = (sess.clicks[0], sess.clicks[1]);
            if dist(a0.pos(sk), b0.pos(sk)) < 1e-6 {
                sess.clicks.pop();
                return false;
            }
            let a = resolve(a0, sk);
            let b = resolve(b0, sk);
            let line = sk.add_line_between(a, b);
            // Auto-constrain nearly horizontal / vertical lines.
            let d = line_dir(sk, line).unwrap_or([1.0, 0.0]);
            let angle = d[1].atan2(d[0]).to_degrees().rem_euclid(180.0);
            if angle < 3.0 || angle > 177.0 {
                let _ = try_constrain(sk, Constraint::Horizontal(line));
            } else if (angle - 90.0).abs() < 3.0 {
                let _ = try_constrain(sk, Constraint::Vertical(line));
            }
            // Snapping onto an existing point closes the chain.
            sess.clicks = if b0.existing().is_some() { vec![] } else { vec![Anchor::Existing(b)] };
            true
        }
        Tool::Rectangle => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return false;
            }
            let (c1, c2) = (sess.clicks[0], sess.clicks[1]);
            sess.clicks.clear();
            let (p1, p2) = (c1.pos(sk), c2.pos(sk));
            if (p1[0] - p2[0]).abs() < 1e-6 || (p1[1] - p2[1]).abs() < 1e-6 {
                *error = Some("rectangle needs width and height".into());
                return false;
            }
            let [bottom, _, top, _] = sk.add_rectangle(p1, p2);
            let corner = |sk: &Sketch, l: EntityId| match *sk.entity(l) {
                Entity::Line { a, .. } => a,
                _ => unreachable!(),
            };
            if let Some(p) = c1.existing() {
                let q = corner(sk, bottom);
                let _ = try_constrain(sk, Constraint::Coincident(q, p));
            }
            if let Some(p) = c2.existing() {
                let q = corner(sk, top);
                let _ = try_constrain(sk, Constraint::Coincident(q, p));
            }
            true
        }
        Tool::Circle => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return false;
            }
            let (c, r) = (sess.clicks[0], sess.clicks[1]);
            sess.clicks.clear();
            let radius = dist(c.pos(sk), r.pos(sk));
            if radius < 1e-6 {
                return false;
            }
            let circle = sk.add_circle(c.pos(sk), radius);
            if let (Some(p), Some(center)) = (c.existing(), e_center(sk, circle)) {
                let _ = try_constrain(sk, Constraint::Coincident(center, p));
            }
            true
        }
        Tool::Arc => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 3 {
                return false;
            }
            let (c, s, e) = (sess.clicks[0], sess.clicks[1], sess.clicks[2]);
            sess.clicks.clear();
            if dist(c.pos(sk), s.pos(sk)) < 1e-6 {
                return false;
            }
            let arc = sk.add_arc(c.pos(sk), s.pos(sk), e.pos(sk));
            let pts = sk.entity(arc).points(); // [center, start, end]
            for (anchor, p) in [(c, pts[0]), (s, pts[1]), (e, pts[2])] {
                if let Some(existing) = anchor.existing() {
                    let _ = try_constrain(sk, Constraint::Coincident(p, existing));
                }
            }
            true
        }
    }
}
