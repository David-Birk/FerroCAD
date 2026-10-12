//! Sketch mode: drawing tools, snapping, dragging, constraint tools and
//! badges, dimensions drawn in the viewport with an inline value editor, and
//! the docked sketch palette.

use crate::app::{FerroApp, Mode};
use crate::dialogs::{fmt_value, with_unit};
use crate::icons::{self, Icon};
use crate::theme;
use egui::{Color32, Pos2, Rect, Stroke, Vec2};
use ferrocad_model::{FeatureId, ParamTable, Plane, SketchFeature};
use ferrocad_sketch::solver::Analysis;
use ferrocad_sketch::{Constraint, ConstraintId, Entity, EntityId, PointId, Segment, Sketch};
use ferrocad_view::{point_segment_distance, Camera, Viewport};
use std::collections::HashMap;

/// A transformation of sketch points (pattern copies).
type PointMap = dyn Fn([f64; 2]) -> [f64; 2];

const SNAP_PX: f64 = 10.0;
const PICK_PX: f64 = 8.0;
const DEFAULT_DIM_OFFSET: f32 = 26.0;

pub const COL_FREE: Color32 = Color32::from_rgb(110, 175, 255);
pub const COL_FIXED: Color32 = Color32::from_rgb(240, 240, 240);
pub const COL_CONSTRUCTION: Color32 = Color32::from_rgb(240, 150, 60);
pub const COL_SELECTED: Color32 = Color32::from_rgb(255, 215, 70);
const COL_PROJECTED: Color32 = Color32::from_rgb(190, 120, 235);
const COL_HOVER: Color32 = Color32::from_rgb(170, 215, 255);
const COL_PREVIEW: Color32 = Color32::from_rgb(170, 170, 170);
const COL_DIM: Color32 = Color32::from_rgb(215, 230, 255);
const COL_DIM_BG: Color32 = Color32::from_rgba_premultiplied(30, 33, 38, 215);

// ------------------------------------------------------------------ tools

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Horizontal,
    Vertical,
    Coincident,
    Parallel,
    Perpendicular,
    Equal,
    Tangent,
    Concentric,
    Midpoint,
    Fix,
}

impl ConstraintKind {
    pub const ALL: [ConstraintKind; 10] = [
        ConstraintKind::Horizontal,
        ConstraintKind::Vertical,
        ConstraintKind::Coincident,
        ConstraintKind::Parallel,
        ConstraintKind::Perpendicular,
        ConstraintKind::Equal,
        ConstraintKind::Tangent,
        ConstraintKind::Concentric,
        ConstraintKind::Midpoint,
        ConstraintKind::Fix,
    ];

    pub fn icon(self) -> Icon {
        match self {
            ConstraintKind::Horizontal => Icon::Horizontal,
            ConstraintKind::Vertical => Icon::Vertical,
            ConstraintKind::Coincident => Icon::Coincident,
            ConstraintKind::Parallel => Icon::Parallel,
            ConstraintKind::Perpendicular => Icon::Perpendicular,
            ConstraintKind::Equal => Icon::Equal,
            ConstraintKind::Tangent => Icon::Tangent,
            ConstraintKind::Concentric => Icon::Concentric,
            ConstraintKind::Midpoint => Icon::Midpoint,
            ConstraintKind::Fix => Icon::Fix,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ConstraintKind::Horizontal => "Horizontal",
            ConstraintKind::Vertical => "Vertical",
            ConstraintKind::Coincident => "Coincident",
            ConstraintKind::Parallel => "Parallel",
            ConstraintKind::Perpendicular => "Perpendicular",
            ConstraintKind::Equal => "Equal",
            ConstraintKind::Tangent => "Tangent",
            ConstraintKind::Concentric => "Concentric",
            ConstraintKind::Midpoint => "Midpoint",
            ConstraintKind::Fix => "Fix/Unfix",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            ConstraintKind::Horizontal => "Horizontal – pick lines",
            ConstraintKind::Vertical => "Vertical – pick lines",
            ConstraintKind::Coincident => "Coincident – pick a point, then a point, line or circle",
            ConstraintKind::Parallel => "Parallel – pick two lines",
            ConstraintKind::Perpendicular => "Perpendicular – pick two lines",
            ConstraintKind::Equal => "Equal – pick two lines or two circles/arcs",
            ConstraintKind::Tangent => "Tangent – pick a line and a circle/arc",
            ConstraintKind::Concentric => "Concentric – pick two circles/arcs",
            ConstraintKind::Midpoint => "Midpoint – pick a point and a line",
            ConstraintKind::Fix => "Fix/Unfix – pick points",
        }
    }

    /// Whether picking prefers points over curves.
    fn wants_points(self) -> bool {
        matches!(self, ConstraintKind::Coincident | ConstraintKind::Midpoint | ConstraintKind::Fix)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Dimension,
    Constrain(ConstraintKind),
    CenterRectangle,
    Arc3,
    Polygon,
    Slot,
    Trim,
    Extend,
    Fillet,
    Offset,
    Mirror,
    CircPattern,
    RectPattern,
    Project,
    Spline,
}

impl Tool {
    fn is_drawing(self) -> bool {
        matches!(
            self,
            Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Arc
                | Tool::CenterRectangle
                | Tool::Arc3
                | Tool::Polygon
                | Tool::Slot
                | Tool::Spline
        )
    }

    /// Tools that work on the current selection.
    fn uses_selection(self) -> bool {
        matches!(self, Tool::Mirror | Tool::CircPattern | Tool::RectPattern)
    }
}

/// Commands from the ribbon, context menu and keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchCommand {
    Tool(Tool),
    /// Apply to the selection if it fits, otherwise start the constraint tool.
    Constrain(ConstraintKind),
    Construction,
    Delete,
    Escape,
}

pub fn sketch_shortcut(i: &egui::InputState) -> Option<SketchCommand> {
    use egui::Key;
    if i.key_pressed(Key::Escape) {
        return Some(SketchCommand::Escape);
    }
    if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
        return Some(SketchCommand::Delete);
    }
    if i.modifiers.shift {
        return None;
    }
    let tool = |t| Some(SketchCommand::Tool(t));
    if i.key_pressed(Key::L) {
        tool(Tool::Line)
    } else if i.key_pressed(Key::R) {
        tool(Tool::Rectangle)
    } else if i.key_pressed(Key::C) {
        tool(Tool::Circle)
    } else if i.key_pressed(Key::A) {
        tool(Tool::Arc)
    } else if i.key_pressed(Key::D) {
        tool(Tool::Dimension)
    } else if i.key_pressed(Key::T) {
        tool(Tool::Trim)
    } else if i.key_pressed(Key::O) {
        tool(Tool::Offset)
    } else if i.key_pressed(Key::P) {
        tool(Tool::Project)
    } else if i.key_pressed(Key::X) {
        Some(SketchCommand::Construction)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Point(PointId),
    Entity(EntityId),
    Constraint(ConstraintId),
}

/// A clicked location: either snapped to an existing point or free.
#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    Existing(PointId),
    New([f64; 2]),
    /// A new point that stays on the curve it was clicked on.
    OnCurve([f64; 2], EntityId),
}

impl Anchor {
    fn pos(&self, sk: &Sketch) -> [f64; 2] {
        match *self {
            Anchor::Existing(p) => sk.point_pos(p),
            Anchor::New(uv) | Anchor::OnCurve(uv, _) => uv,
        }
    }

    fn existing(&self) -> Option<PointId> {
        match *self {
            Anchor::Existing(p) => Some(p),
            Anchor::New(_) | Anchor::OnCurve(..) => None,
        }
    }

    fn curve(&self) -> Option<EntityId> {
        match *self {
            Anchor::OnCurve(_, e) => Some(e),
            _ => None,
        }
    }
}

/// Keep point `p` on curve `e` (lines, circles, arcs).
fn stick_to_curve(sk: &mut Sketch, p: PointId, e: EntityId) {
    let c = match sk.entity(e) {
        Entity::Line { .. } => Constraint::PointOnLine(p, e),
        Entity::Circle { .. } | Entity::Arc { .. } => Constraint::PointOnCircle(p, e),
        Entity::Spline { .. } => return,
    };
    let _ = try_constrain(sk, c);
}

/// The value editor that pops up next to a dimension.
pub struct InlineEdit {
    pub id: ConstraintId,
    pub text: String,
    /// Screen position, updated every frame from the dimension's geometry.
    pub pos: Option<Pos2>,
    pub focus: bool,
}

/// One on-screen value field while drawing (Fusion's dimension inputs).
pub struct DrawField {
    pub text: String,
    /// Current measured value, shown as hint while the field is empty.
    pub live: String,
    pub pos: Option<Pos2>,
}

/// Value fields for the shape being drawn: type a value to lock it, Tab to
/// switch fields, Enter (or a click) to create the shape.
pub struct DrawInput {
    /// First click of the shape; a new start point resets the fields.
    pub start: [f64; 2],
    pub fields: Vec<DrawField>,
    pub active: usize,
    /// Set by the overlay on Enter: create the shape this frame.
    pub commit: bool,
}

impl DrawInput {
    fn new(tool: Tool, start: [f64; 2]) -> Self {
        let n = if tool == Tool::Circle { 1 } else { 2 };
        let fields = (0..n).map(|_| DrawField { text: String::new(), live: String::new(), pos: None }).collect();
        Self { start, fields, active: 0, commit: false }
    }

    /// Typed values that evaluate (lengths must be positive).
    fn values(&self, tool: Tool, params: &ParamTable) -> [Option<f64>; 2] {
        let mut out = [None, None];
        for (i, f) in self.fields.iter().enumerate() {
            if f.text.trim().is_empty() {
                continue;
            }
            let is_angle = tool == Tool::Line && i == 1;
            out[i] = params.eval(&f.text).ok().filter(|v| is_angle || *v > 1e-9);
        }
        out
    }
}

/// Where the shape ends given the mouse and any typed values.
fn draw_target(tool: Tool, start: [f64; 2], mouse: [f64; 2], vals: [Option<f64>; 2]) -> [f64; 2] {
    let (dx, dy) = (mouse[0] - start[0], mouse[1] - start[1]);
    match tool {
        Tool::Line => {
            let len = vals[0].unwrap_or(dx.hypot(dy));
            let angle = vals[1].map(f64::to_radians).unwrap_or(dy.atan2(dx));
            [start[0] + len * angle.cos(), start[1] + len * angle.sin()]
        }
        Tool::Rectangle => {
            let sx = if dx < 0.0 { -1.0 } else { 1.0 };
            let sy = if dy < 0.0 { -1.0 } else { 1.0 };
            [start[0] + sx * vals[0].unwrap_or(dx.abs()), start[1] + sy * vals[1].unwrap_or(dy.abs())]
        }
        Tool::CenterRectangle => {
            let sx = if dx < 0.0 { -1.0 } else { 1.0 };
            let sy = if dy < 0.0 { -1.0 } else { 1.0 };
            let w = vals[0].map(|v| v * 0.5).unwrap_or(dx.abs());
            let h = vals[1].map(|v| v * 0.5).unwrap_or(dy.abs());
            [start[0] + sx * w, start[1] + sy * h]
        }
        Tool::Circle => {
            let d = dx.hypot(dy);
            let r = vals[0].map(|v| v * 0.5).unwrap_or(d);
            let (ux, uy) = if d > 1e-12 { (dx / d, dy / d) } else { (1.0, 0.0) };
            [start[0] + r * ux, start[1] + r * uy]
        }
        _ => mouse,
    }
}

/// Geometry created by a drawing click.
enum Created {
    Line(EntityId),
    /// bottom, right, top, left
    Rectangle([EntityId; 4]),
    Circle(EntityId),
    Arc,
    /// Anything without typed values (polygon, slot, …).
    Shape,
}

pub struct SketchSession {
    pub feature: FeatureId,
    pub tool: Tool,
    pub clicks: Vec<Anchor>,
    pub selection: Vec<Pick>,
    pub dragging: Option<PointId>,
    /// Start of a selection rectangle (Select tool dragged on empty space).
    pub box_start: Option<Pos2>,
    pub analysis: Option<Analysis>,
    pub dim_edits: HashMap<ConstraintId, String>,
    /// Where each dimension is drawn (screen pixels from the geometry).
    pub dim_offsets: HashMap<ConstraintId, f32>,
    pub inline: Option<InlineEdit>,
    pub draw_input: Option<DrawInput>,
    /// Last cursor position on the sketch plane (for Enter in value fields).
    pub last_mouse: Option<[f64; 2]>,
    pub show_points: bool,
    pub show_dimensions: bool,
    pub show_constraints: bool,
    pub snap_to_grid: bool,
    pub polygon_sides: usize,
    pub fillet_radius: String,
    pub offset_distance: String,
    pub pattern_count: String,
}

impl SketchSession {
    pub fn new(feature: FeatureId) -> Self {
        Self {
            feature,
            tool: Tool::Line,
            clicks: Vec::new(),
            selection: Vec::new(),
            dragging: None,
            box_start: None,
            analysis: None,
            dim_edits: HashMap::new(),
            dim_offsets: HashMap::new(),
            inline: None,
            draw_input: None,
            last_mouse: None,
            show_points: true,
            show_dimensions: true,
            show_constraints: true,
            snap_to_grid: false,
            polygon_sides: 6,
            fillet_radius: "2 mm".into(),
            offset_distance: "2 mm".into(),
            pattern_count: "4".into(),
        }
    }

    /// Drop state that refers to geometry (after undo/redo).
    pub fn reset_transient(&mut self) {
        self.analysis = None;
        self.selection.clear();
        self.clicks.clear();
        self.dim_edits.clear();
        self.dragging = None;
        self.inline = None;
        self.draw_input = None;
    }

    pub fn hint(&self) -> &'static str {
        let n = self.clicks.len();
        match self.tool {
            Tool::Select => "Click to select (Shift adds) · drag points · double-click a dimension to edit · Del deletes",
            Tool::Line if n == 0 => "Line: click the start point",
            Tool::Line => "Line: click the next point, or type length · Tab · angle · Enter · Esc ends the chain",
            Tool::Rectangle if n == 0 => "Rectangle: click the first corner",
            Tool::Rectangle => "Rectangle: click the opposite corner, or type width · Tab · height · Enter",
            Tool::Circle if n == 0 => "Circle: click the center",
            Tool::Circle => "Circle: click to set the diameter, or type it and press Enter",
            Tool::Arc if n == 0 => "Arc: click the center",
            Tool::Arc if n == 1 => "Arc: click the start point",
            Tool::Arc => "Arc: click the end point (counter-clockwise)",
            Tool::Dimension => match self.selection.as_slice() {
                [] => "Dimension: pick a line, circle, arc or point",
                [Pick::Entity(_)] => "Dimension: click to place, or pick a second line for an angle",
                [Pick::Point(_)] => "Dimension: pick a second point",
                _ => "Dimension: click to place (beside: vertical, above/below: horizontal)",
            },
            Tool::Constrain(k) => k.tooltip(),
            Tool::CenterRectangle if n == 0 => "Center rectangle: click the centre",
            Tool::CenterRectangle => "Center rectangle: click a corner, or type width · Tab · height · Enter",
            Tool::Arc3 if n == 0 => "3-point arc: click the start point",
            Tool::Arc3 if n == 1 => "3-point arc: click the end point",
            Tool::Arc3 => "3-point arc: click a point on the arc",
            Tool::Polygon if n == 0 => "Polygon: click the centre (sides in the palette)",
            Tool::Polygon => "Polygon: click a corner",
            Tool::Slot if n == 0 => "Slot: click the first centre",
            Tool::Slot if n == 1 => "Slot: click the second centre",
            Tool::Slot => "Slot: click to set the width",
            Tool::Trim => "Trim: click the piece of a curve to remove (T)",
            Tool::Extend => "Extend: click near the end of a line",
            Tool::Fillet => "Sketch fillet: click a corner point (radius in the palette)",
            Tool::Offset if self.selection.is_empty() => "Offset: click a curve – its connected chain is selected (O)",
            Tool::Offset => "Offset: click the side to offset to (distance in the palette)",
            Tool::Mirror => "Mirror: click the mirror line (select the curves first)",
            Tool::CircPattern => "Circular pattern: click the centre (select the curves first)",
            Tool::RectPattern => "Rectangular pattern: click where the first copy goes (select the curves first)",
            Tool::Project => "Project: click body edges to bring them into the sketch (P)",
            Tool::Spline if n < 2 => "Spline: click fit points",
            Tool::Spline => "Spline: click more fit points · Enter or double-click to finish · Esc cancels",
        }
    }
}

// ------------------------------------------------------------------ geometry

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Polyline of an entity in sketch coordinates.
pub fn entity_polyline(sk: &Sketch, e: &Entity) -> Vec<[f64; 2]> {
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
        Entity::Spline { ref points } => {
            let fit: Vec<[f64; 2]> = points.iter().map(|p| sk.point_pos(*p)).collect();
            ferrocad_sketch::spline_samples(&fit, 16)
        }
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

fn is_line(sk: &Sketch, e: EntityId) -> bool {
    matches!(sk.entity(e), Entity::Line { .. })
}

/// Screen projection helpers for one sketch plane.
pub struct Projector<'a> {
    pub cam: &'a Camera,
    pub plane: Plane,
    pub vp: &'a Viewport,
}

impl Projector<'_> {
    pub fn screen(&self, uv: [f64; 2]) -> Option<Pos2> {
        self.cam
            .project(self.plane.to_world(uv), self.vp)
            .map(|s| egui::pos2(s[0] as f32, s[1] as f32))
    }

    pub fn local(&self, pos: Pos2) -> Option<[f64; 2]> {
        let (o, d) = self.cam.ray([pos.x as f64, pos.y as f64], self.vp);
        self.plane.intersect_ray(o, d).map(|w| self.plane.to_local(w))
    }

    pub fn polyline(&self, pts: &[[f64; 2]]) -> Vec<Pos2> {
        pts.iter().filter_map(|p| self.screen(*p)).collect()
    }

    /// Screen directions of the sketch's +u and +v axes at `uv`.
    fn axes(&self, uv: [f64; 2]) -> Option<(Vec2, Vec2)> {
        let o = self.screen(uv)?;
        let x = self.screen([uv[0] + 1.0, uv[1]])?;
        let y = self.screen([uv[0], uv[1] + 1.0])?;
        Some(((x - o).normalized(), (y - o).normalized()))
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
        Constraint::PointLineDistance(p, l, _) => {
            let Entity::Line { a, .. } = *sk.entity(l) else { return None };
            let d = line_dir(sk, l)?;
            let q = sk.point_pos(p);
            let a = sk.point_pos(a);
            ((d[0] * (q[1] - a[1]) - d[1] * (q[0] - a[0])) / d[0].hypot(d[1]).max(1e-12)).abs()
        }
        _ => return None,
    })
}

fn e_center(sk: &Sketch, e: EntityId) -> Option<PointId> {
    match *sk.entity(e) {
        Entity::Circle { center, .. } | Entity::Arc { center, .. } => Some(center),
        Entity::Line { .. } | Entity::Spline { .. } => None,
    }
}

fn format_dimension(c: &Constraint, v: f64) -> String {
    match c {
        Constraint::Angle(..) => format!("{:.1}°", v.to_degrees()),
        Constraint::Diameter(..) => format!("Ø{v:.2} mm"),
        Constraint::Radius(..) => format!("R{v:.2} mm"),
        _ => format!("{:.2} mm", v.abs()),
    }
}

/// The expression to show when editing a dimension.
fn dimension_text(sf: &SketchFeature, id: ConstraintId, c: &Constraint) -> String {
    sf.driven.iter().find(|(d, _)| *d == id).map(|(_, e)| e.clone()).unwrap_or_else(|| dimension_value(c))
}

/// The current value of a dimension with its unit, e.g. "30 mm" or "45°".
fn dimension_value(c: &Constraint) -> String {
    match c {
        Constraint::Angle(_, _, v) => fmt_value(v.to_degrees(), "°"),
        _ => fmt_value(c.value().unwrap_or(0.0), "mm"),
    }
}

/// Set a dimension from an expression (value or parameters) and re-solve.
fn apply_dimension(sf: &mut SketchFeature, params: &ParamTable, id: ConstraintId, expr: &str) -> Result<(), String> {
    let c = sf.sketch.constraint(id).cloned().ok_or("the dimension no longer exists")?;
    let v = params.eval(expr).map_err(|e| e.to_string())?;
    let v = if matches!(c, Constraint::Angle(..)) { v.to_radians() } else { v };
    let old = c.value().unwrap_or(0.0);
    sf.sketch.set_dimension(id, v);
    match sf.sketch.solve() {
        Ok(_) => {
            let unit = if matches!(c, Constraint::Angle(..)) { "°" } else { "mm" };
            sf.set_expression(id, with_unit(expr, unit));
            Ok(())
        }
        Err(e) => {
            sf.sketch.set_dimension(id, old);
            let _ = sf.sketch.solve();
            Err(format!("value not possible: {e}"))
        }
    }
}

fn split_selection(sel: &[Pick], sk: &Sketch) -> (Vec<PointId>, Vec<EntityId>, Vec<EntityId>, usize) {
    let mut points = Vec::new();
    let mut lines = Vec::new();
    let mut curves = Vec::new();
    let mut constraints = 0;
    for p in sel {
        match *p {
            Pick::Point(p) => points.push(p),
            Pick::Entity(e) if is_line(sk, e) => lines.push(e),
            Pick::Entity(e) => curves.push(e),
            Pick::Constraint(_) => constraints += 1,
        }
    }
    (points, lines, curves, constraints)
}

enum Eval {
    Ready(Vec<Constraint>),
    Fix(Vec<PointId>),
    Incomplete,
    Invalid,
}

/// What a constraint tool makes of the current picks.
fn evaluate(kind: ConstraintKind, sel: &[Pick], sk: &Sketch) -> Eval {
    use ConstraintKind as K;
    let (points, lines, curves, nk) = split_selection(sel, sk);
    let (np, nl, nc) = (points.len(), lines.len(), curves.len());
    if nk > 0 {
        return Eval::Invalid;
    }
    if np + nl + nc == 0 {
        return Eval::Incomplete;
    }
    let only = |p, l, c| np == p && nl == l && nc == c;
    match kind {
        K::Horizontal | K::Vertical if np == 0 && nc == 0 => Eval::Ready(
            lines
                .iter()
                .map(|&l| if kind == K::Horizontal { Constraint::Horizontal(l) } else { Constraint::Vertical(l) })
                .collect(),
        ),
        K::Fix if nl == 0 && nc == 0 => Eval::Fix(points),
        K::Coincident if only(2, 0, 0) => Eval::Ready(vec![Constraint::Coincident(points[0], points[1])]),
        K::Coincident if only(1, 1, 0) => Eval::Ready(vec![Constraint::PointOnLine(points[0], lines[0])]),
        K::Coincident if only(1, 0, 1) => Eval::Ready(vec![Constraint::PointOnCircle(points[0], curves[0])]),
        K::Coincident if only(1, 0, 0) || only(0, 1, 0) || only(0, 0, 1) => Eval::Incomplete,
        K::Parallel if only(0, 2, 0) => Eval::Ready(vec![Constraint::Parallel(lines[0], lines[1])]),
        K::Perpendicular if only(0, 2, 0) => Eval::Ready(vec![Constraint::Perpendicular(lines[0], lines[1])]),
        K::Parallel | K::Perpendicular if only(0, 1, 0) => Eval::Incomplete,
        K::Equal if only(0, 2, 0) => Eval::Ready(vec![Constraint::Equal(lines[0], lines[1])]),
        K::Equal if only(0, 0, 2) => Eval::Ready(vec![Constraint::Equal(curves[0], curves[1])]),
        K::Equal if only(0, 1, 0) || only(0, 0, 1) => Eval::Incomplete,
        K::Tangent if only(0, 1, 1) => Eval::Ready(vec![Constraint::Tangent(lines[0], curves[0])]),
        K::Tangent if only(0, 1, 0) || only(0, 0, 1) => Eval::Incomplete,
        K::Concentric if only(0, 0, 2) => Eval::Ready(vec![Constraint::Concentric(curves[0], curves[1])]),
        K::Concentric if only(0, 0, 1) => Eval::Incomplete,
        K::Midpoint if only(1, 1, 0) => Eval::Ready(vec![Constraint::Midpoint(points[0], lines[0])]),
        K::Midpoint if only(1, 0, 0) || only(0, 1, 0) => Eval::Incomplete,
        _ => Eval::Invalid,
    }
}

/// Dimension implied by a selection made with the Select tool.
fn dimension_from_selection(sel: &[Pick], sk: &Sketch) -> Option<Constraint> {
    let (points, lines, curves, nk) = split_selection(sel, sk);
    let only = |p, l, c| points.len() == p && lines.len() == l && curves.len() == c && nk == 0;
    if only(0, 1, 0) {
        Some(Constraint::Length(lines[0], 0.0))
    } else if only(0, 0, 1) {
        Some(circle_dimension(sk, curves[0]))
    } else if only(2, 0, 0) {
        Some(Constraint::Distance(points[0], points[1], 0.0))
    } else if only(0, 2, 0) {
        Some(Constraint::Angle(lines[0], lines[1], 0.0))
    } else if only(1, 1, 0) {
        Some(Constraint::PointLineDistance(points[0], lines[0], 0.0))
    } else {
        None
    }
}

fn circle_dimension(sk: &Sketch, e: EntityId) -> Constraint {
    match sk.entity(e) {
        Entity::Circle { .. } => Constraint::Diameter(e, 0.0),
        _ => Constraint::Radius(e, 0.0),
    }
}

/// Add a dimension at the current size (so nothing jumps) and open its editor.
fn create_dimension(sess: &mut SketchSession, sk: &mut Sketch, mut c: Constraint, offset: Option<f32>) -> Result<(), String> {
    if let (Some(v), Some(slot)) = (measure(sk, &c), c.value_mut()) {
        *slot = v;
    }
    let id = try_constrain(sk, c.clone())?;
    let shown = dimension_value(&c);
    sess.dim_edits.insert(id, shown.clone());
    if let Some(off) = offset {
        sess.dim_offsets.insert(id, off);
    }
    sess.inline = Some(InlineEdit { id, text: shown, pos: None, focus: true });
    sess.analysis = None;
    Ok(())
}

// ------------------------------------------------------- dimension drawing

/// Screen-space geometry of a drawn dimension.
struct DimGeom {
    lines: Vec<[Pos2; 2]>,
    arrows: Vec<(Pos2, Vec2)>,
    arc: Vec<Pos2>,
    text_at: Pos2,
}

/// Normal used for linear dimensions: positive offsets go up/left on screen.
fn dim_normal(axis: Vec2) -> Vec2 {
    let n = egui::vec2(-axis.y, axis.x);
    if n.y > 0.0 || (n.y == 0.0 && n.x > 0.0) {
        -n
    } else {
        n
    }
}

/// Dimension measuring `a`–`b` along the screen direction `axis`.
fn linear_geom(a: Pos2, b: Pos2, axis: Vec2, off: f32) -> DimGeom {
    let n = dim_normal(axis);
    let (ha, hb) = (a.to_vec2().dot(n), b.to_vec2().dot(n));
    let h = if off >= 0.0 { ha.max(hb) + off } else { ha.min(hb) + off };
    let (a2, b2) = (a + n * (h - ha), b + n * (h - hb));
    let mut lines = vec![[a2, b2]];
    for (p, q) in [(a, a2), (b, b2)] {
        let d = q - p;
        if d.length() > 4.0 {
            let u = d.normalized();
            lines.push([p + u * 3.0, q + u * 4.0]);
        }
    }
    let dir = (b2 - a2).normalized();
    DimGeom { lines, arrows: vec![(a2, -dir), (b2, dir)], arc: vec![], text_at: a2 + (b2 - a2) * 0.5 }
}

/// Signed offset for a dimension placed at `cursor`.
fn linear_offset(a: Pos2, b: Pos2, axis: Vec2, cursor: Pos2) -> f32 {
    let n = dim_normal(axis);
    let (ha, hb, hc) = (a.to_vec2().dot(n), b.to_vec2().dot(n), cursor.to_vec2().dot(n));
    if hc >= 0.5 * (ha + hb) {
        (hc - ha.max(hb)).max(10.0)
    } else {
        (hc - ha.min(hb)).min(-10.0)
    }
}

/// Which kind of two-point distance a placement click asks for.
fn distance_kind(a: [f64; 2], b: [f64; 2], at: [f64; 2]) -> u8 {
    let inside = |v: f64, p: f64, q: f64| v >= p.min(q) && v <= p.max(q);
    let (in_x, in_y) = (inside(at[0], a[0], b[0]), inside(at[1], a[1], b[1]));
    match (in_x, in_y) {
        (true, false) => 1, // above/below: horizontal distance
        (false, true) => 2, // beside: vertical distance
        _ => 0,             // aligned
    }
}

fn dimension_geom(sk: &Sketch, pr: &Projector, c: &Constraint, off: f32) -> Option<DimGeom> {
    let sp = |p: PointId| pr.screen(sk.point_pos(p));
    match *c {
        Constraint::Length(l, _) => match *sk.entity(l) {
            Entity::Line { a, b } => {
                let (a, b) = (sp(a)?, sp(b)?);
                Some(linear_geom(a, b, (b - a).normalized(), off))
            }
            _ => None,
        },
        Constraint::Distance(a, b, _) => {
            let (a, b) = (sp(a)?, sp(b)?);
            Some(linear_geom(a, b, (b - a).normalized(), off))
        }
        Constraint::PointLineDistance(p, l, _) => {
            let Entity::Line { a, b } = *sk.entity(l) else { return None };
            let (pa, pb, q) = (sk.point_pos(a), sk.point_pos(b), sk.point_pos(p));
            let d = [pb[0] - pa[0], pb[1] - pa[1]];
            let t = ((q[0] - pa[0]) * d[0] + (q[1] - pa[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-12);
            let foot = [pa[0] + t * d[0], pa[1] + t * d[1]];
            let (sq, sf) = (pr.screen(q)?, pr.screen(foot)?);
            let axis = if (sf - sq).length() > 1e-3 { (sf - sq).normalized() } else { unit((pr.screen(pb)? - pr.screen(pa)?).rot90()) };
            Some(linear_geom(sq, sf, axis, off))
        }
        Constraint::HorizontalDistance(a, b, _) | Constraint::VerticalDistance(a, b, _) => {
            let (ex, ey) = pr.axes(sk.point_pos(a))?;
            let axis = if matches!(c, Constraint::HorizontalDistance(..)) { ex } else { ey };
            Some(linear_geom(sp(a)?, sp(b)?, axis, off))
        }
        Constraint::Radius(e, _) | Constraint::Diameter(e, _) => {
            let center = sk.point_pos(e_center(sk, e)?);
            let r = sk.radius(e)?;
            let angle = match *sk.entity(e) {
                Entity::Arc { start, end, .. } => {
                    let (s, t) = (sk.point_pos(start), sk.point_pos(end));
                    let a0 = (s[1] - center[1]).atan2(s[0] - center[0]);
                    let a1 = (t[1] - center[1]).atan2(t[0] - center[0]);
                    a0 + (a1 - a0).rem_euclid(std::f64::consts::TAU) * 0.5
                }
                _ => std::f64::consts::FRAC_PI_4,
            };
            let on = |a: f64| pr.screen([center[0] + r * a.cos(), center[1] + r * a.sin()]);
            let p = on(angle)?;
            let start = if matches!(c, Constraint::Diameter(..)) {
                on(angle + std::f64::consts::PI)?
            } else {
                pr.screen(center)?
            };
            let dir = (p - start).normalized();
            let tail = p + dir * 18.0;
            let mut arrows = vec![(p, dir)];
            if matches!(c, Constraint::Diameter(..)) {
                arrows.push((start, -dir));
            }
            Some(DimGeom { lines: vec![[start, tail]], arrows, arc: vec![], text_at: tail + dir * 22.0 })
        }
        Constraint::Angle(l1, l2, _) => {
            let (&Entity::Line { a: a1, b: b1 }, &Entity::Line { a: a2, b: b2 }) = (sk.entity(l1), sk.entity(l2)) else {
                return None;
            };
            let (p1, q1, p2, q2) = (sk.point_pos(a1), sk.point_pos(b1), sk.point_pos(a2), sk.point_pos(b2));
            let d1 = [q1[0] - p1[0], q1[1] - p1[1]];
            let d2 = [q2[0] - p2[0], q2[1] - p2[1]];
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            if den.abs() < 1e-12 {
                return None;
            }
            let t = ((p2[0] - p1[0]) * d2[1] - (p2[1] - p1[1]) * d2[0]) / den;
            let i = [p1[0] + t * d1[0], p1[1] + t * d1[1]];
            let far = |p: [f64; 2], q: [f64; 2]| if dist(i, p) > dist(i, q) { p } else { q };
            let is = pr.screen(i)?;
            let u1 = (pr.screen(far(p1, q1))? - is).normalized();
            let u2 = (pr.screen(far(p2, q2))? - is).normalized();
            let (t1, t2) = (u1.y.atan2(u1.x), u2.y.atan2(u2.x));
            let mut sweep = (t2 - t1).rem_euclid(std::f32::consts::TAU);
            let start = if sweep > std::f32::consts::PI {
                sweep = std::f32::consts::TAU - sweep;
                t2
            } else {
                t1
            };
            let radius = off.abs().max(24.0) + 16.0;
            let arc: Vec<Pos2> = (0..=24)
                .map(|k| {
                    let a = start + sweep * k as f32 / 24.0;
                    is + egui::vec2(a.cos(), a.sin()) * radius
                })
                .collect();
            let mid = start + sweep * 0.5;
            let text_at = is + egui::vec2(mid.cos(), mid.sin()) * (radius + 16.0);
            Some(DimGeom { lines: vec![], arrows: vec![], arc, text_at })
        }
        _ => None,
    }
}

fn paint_dimension(painter: &egui::Painter, g: &DimGeom, text: &str, highlight: Option<Color32>) -> Rect {
    let color = highlight.unwrap_or(COL_DIM);
    let stroke = Stroke::new(1.0, color);
    for l in &g.lines {
        painter.line_segment(*l, stroke);
    }
    if g.arc.len() > 1 {
        painter.add(egui::Shape::line(g.arc.clone(), stroke));
    }
    for (tip, dir) in &g.arrows {
        let side = egui::vec2(-dir.y, dir.x);
        let back = *tip - *dir * 8.0;
        painter.add(egui::Shape::convex_polygon(vec![*tip, back + side * 3.0, back - side * 3.0], color, Stroke::NONE));
    }
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(13.0), color);
    let rect = Rect::from_center_size(g.text_at, galley.size() + egui::vec2(8.0, 4.0));
    painter.rect_filled(rect, 3.0, COL_DIM_BG);
    if highlight.is_some() {
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    rect
}

/// Small label next to the cursor while drawing.
fn cursor_label(painter: &egui::Painter, at: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.0), COL_DIM);
    let rect = Rect::from_min_size(at + egui::vec2(16.0, 14.0), galley.size() + egui::vec2(8.0, 4.0));
    painter.rect_filled(rect, 3.0, COL_DIM_BG);
    painter.galley(rect.min + egui::vec2(4.0, 2.0), galley, COL_DIM);
}

// ------------------------------------------------------- constraint badges

#[derive(Clone, Copy, PartialEq)]
enum BadgeTarget {
    Constraint(ConstraintId),
    FixedPoint(PointId),
}

struct Badge {
    rect: Rect,
    icon: Icon,
    target: BadgeTarget,
}

/// Anchor on an entity for badges: (position, direction along, normal).
fn entity_anchor(sk: &Sketch, pr: &Projector, e: EntityId) -> Option<(Pos2, Vec2, Vec2)> {
    match *sk.entity(e) {
        Entity::Spline { .. } => None,
        Entity::Line { a, b } => {
            let (a, b) = (pr.screen(sk.point_pos(a))?, pr.screen(sk.point_pos(b))?);
            let along = (b - a).normalized();
            Some((a + (b - a) * 0.5, along, -dim_normal(along)))
        }
        Entity::Circle { center, .. } | Entity::Arc { center, .. } => {
            let c = sk.point_pos(center);
            let r = sk.radius(e)?;
            let angle = match *sk.entity(e) {
                Entity::Arc { start, end, .. } => {
                    let (s, t) = (sk.point_pos(start), sk.point_pos(end));
                    let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                    let a1 = (t[1] - c[1]).atan2(t[0] - c[0]);
                    a0 + (a1 - a0).rem_euclid(std::f64::consts::TAU) * 0.5
                }
                _ => 2.4,
            };
            let p = pr.screen([c[0] + r * angle.cos(), c[1] + r * angle.sin()])?;
            let n = (p - pr.screen(c)?).normalized();
            Some((p, egui::vec2(-n.y, n.x), n))
        }
    }
}

fn badge_icon(c: &Constraint) -> Option<Icon> {
    Some(match c {
        Constraint::Horizontal(_) => Icon::Horizontal,
        Constraint::Vertical(_) => Icon::Vertical,
        Constraint::Parallel(..) => Icon::Parallel,
        Constraint::Perpendicular(..) => Icon::Perpendicular,
        Constraint::Equal(..) => Icon::Equal,
        Constraint::Tangent(..) => Icon::Tangent,
        Constraint::Concentric(..) => Icon::Concentric,
        Constraint::Midpoint(..) => Icon::Midpoint,
        Constraint::PointOnLine(..) | Constraint::PointOnCircle(..) => Icon::Coincident,
        _ => return None,
    })
}

fn layout_badges(sk: &Sketch, pr: &Projector, centre: Pos2) -> Vec<Badge> {
    let mut badges = Vec::new();
    let mut per_entity: HashMap<EntityId, usize> = HashMap::new();
    let mut per_point: HashMap<PointId, usize> = HashMap::new();
    let size = egui::vec2(17.0, 17.0);
    let mut at_point = |p: PointId, icon: Icon, target: BadgeTarget, badges: &mut Vec<Badge>| {
        if let Some(s) = pr.screen(sk.point_pos(p)) {
            let k = per_point.entry(p).or_default();
            let center = s + egui::vec2(14.0 + *k as f32 * 19.0, -14.0);
            *k += 1;
            badges.push(Badge { rect: Rect::from_center_size(center, size), icon, target });
        }
    };
    for (id, c) in sk.constraints() {
        let Some(icon) = badge_icon(c) else { continue };
        let target = BadgeTarget::Constraint(id);
        match *c {
            Constraint::Midpoint(p, _) | Constraint::PointOnLine(p, _) | Constraint::PointOnCircle(p, _) => {
                at_point(p, icon, target, &mut badges);
            }
            _ => {
                for e in c.entities() {
                    if let Some((pos, along, normal)) = entity_anchor(sk, pr, e) {
                        // Badges go inside the shape; dimensions go outside.
                        let normal = if (centre - pos).dot(normal) < 0.0 { -normal } else { normal };
                        let k = per_entity.entry(e).or_default();
                        let shift = (*k as f32 - 0.0) * 19.0;
                        *k += 1;
                        let center = pos + normal * 15.0 + along * shift;
                        badges.push(Badge { rect: Rect::from_center_size(center, size), icon, target });
                    }
                }
            }
        }
    }
    for p in visible_points(sk) {
        if sk.is_point_fixed(p) {
            at_point(p, Icon::Fix, BadgeTarget::FixedPoint(p), &mut badges);
        }
    }
    badges
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
        let grid = crate::viewport::grid_step(cam.distance);
        let params = self.doc.params.clone();
        let param_names: Vec<String> = self.doc.params.iter().map(|p| p.name.clone()).collect();
        let projecting = matches!(&self.mode, Mode::Sketch(s) if s.tool == Tool::Project);
        let project_hover = if projecting { response.hover_pos().and_then(|p| self.pick_edge(p, vp)) } else { None };
        let Mode::Sketch(sess) = &mut self.mode else { return };
        // (a missing sketch makes the palette finish the session)
        let Some(sf) = self.doc.sketch_mut(sess.feature) else { return };
        let pr = Projector { cam: &cam, plane: sf.plane, vp };
        let mut changed = false;
        let mut message: Option<(String, bool)> = None;

        // ---- what is under the cursor
        let hover = response.hover_pos();
        let centre = {
            let pts: Vec<Pos2> =
                visible_points(&sf.sketch).into_iter().filter_map(|p| pr.screen(sf.sketch.point_pos(p))).collect();
            let n = pts.len().max(1) as f32;
            pts.iter().fold(Pos2::ZERO, |a, p| a + p.to_vec2() / n)
        };
        let badges = if sess.show_constraints { layout_badges(&sf.sketch, &pr, centre) } else { vec![] };
        let mut dims: Vec<(ConstraintId, DimGeom)> = Vec::new();
        if sess.show_dimensions {
            // Unplaced dimensions go on the side away from the sketch's centre.
            for (id, c) in sf.sketch.constraints().filter(|(_, c)| c.is_dimension()) {
                let off = match sess.dim_offsets.get(&id) {
                    Some(o) => *o,
                    None => match dimension_geom(&sf.sketch, &pr, c, DEFAULT_DIM_OFFSET) {
                        Some(g) if !g.lines.is_empty() && g.arc.is_empty() => {
                            let [a, b] = g.lines[0];
                            let mid = a + (b - a) * 0.5;
                            let base = dimension_geom(&sf.sketch, &pr, c, 0.0).map(|g0| g0.text_at).unwrap_or(mid);
                            if (mid - base).dot(base - centre) >= 0.0 { DEFAULT_DIM_OFFSET } else { -DEFAULT_DIM_OFFSET }
                        }
                        _ => DEFAULT_DIM_OFFSET,
                    },
                };
                if let Some(g) = dimension_geom(&sf.sketch, &pr, c, off) {
                    dims.push((id, g));
                }
            }
        }
        // Text rects are known after painting; use last frame's layout via a rough box.
        let dim_rect = |g: &DimGeom| Rect::from_center_size(g.text_at, egui::vec2(56.0, 20.0));
        let hover_badge = hover.and_then(|h| badges.iter().find(|b| b.rect.contains(h)).map(|b| b.target));
        let hover_dim = hover.and_then(|h| dims.iter().find(|(_, g)| dim_rect(g).contains(h)).map(|(id, _)| *id));
        let hover_point = hover.and_then(|p| pick_point(&sf.sketch, &pr, p));
        let hover_entity = hover.and_then(|p| pick_entity(&sf.sketch, &pr, p));
        let snap_to_grid = sess.snap_to_grid;
        let snap_uv = |uv: [f64; 2]| {
            if snap_to_grid {
                [(uv[0] / grid).round() * grid, (uv[1] / grid).round() * grid]
            } else {
                uv
            }
        };
        let hover_uv = hover.and_then(|p| match hover_point {
            Some(s) => Some(sf.sketch.point_pos(s)),
            None => pr.local(p).map(snap_uv),
        });

        // ---- value fields while drawing (Fusion-style dimension inputs)
        if let Some(uv) = hover.and_then(|p| pr.local(p)) {
            sess.last_mouse = Some(uv);
        }
        if matches!(sess.tool, Tool::Line | Tool::Rectangle | Tool::Circle | Tool::CenterRectangle)
            && sess.clicks.len() == 1
        {
            let start = sess.clicks[0].pos(&sf.sketch);
            if sess.draw_input.as_ref().is_none_or(|d| d.start != start) {
                sess.draw_input = Some(DrawInput::new(sess.tool, start));
            }
        } else {
            sess.draw_input = None;
        }
        let typed = sess.draw_input.as_ref().map(|d| d.values(sess.tool, &params)).unwrap_or([None, None]);
        let locked = typed.iter().any(Option::is_some);
        // With a typed value the end follows the raw cursor, otherwise snaps.
        let mouse_uv = if locked { hover.and_then(|p| pr.local(p)) } else { hover_uv }.or(sess.last_mouse);
        let target = sess.draw_input.as_ref().zip(mouse_uv).map(|(d, m)| draw_target(sess.tool, d.start, m, typed));
        if sess.draw_input.as_ref().is_some_and(|d| d.commit) {
            if let Some(t) = target {
                let anchor = match hover_point {
                    Some(p) if !locked => Anchor::Existing(p),
                    _ => Anchor::New(t),
                };
                changed |= draw_commit(sess, sf, anchor, typed, &mut message);
            }
            if let Some(d) = &mut sess.draw_input {
                d.commit = false;
            }
        }

        // ---- double-click a dimension: edit its value in place
        if response.double_clicked() {
            if let Some(id) = hover_dim {
                if let Some(c) = sf.sketch.constraint(id).cloned() {
                    sess.inline = Some(InlineEdit { id, text: dimension_text(sf, id, &c), pos: None, focus: true });
                }
            }
        }

        // ---- dragging points (select tool)
        if sess.tool == Tool::Select {
            if response.drag_started_by(egui::PointerButton::Primary) {
                let origin = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos());
                sess.dragging = origin.and_then(|p| pick_point(&sf.sketch, &pr, p));
                // On empty space the drag draws a selection rectangle.
                sess.box_start = origin.filter(|p| sess.dragging.is_none() && pick_entity(&sf.sketch, &pr, *p).is_none());
            }
            if let (Some(start), Some(now)) = (sess.box_start, response.interact_pointer_pos()) {
                let rect = egui::Rect::from_two_pos(start, now);
                let window = now.x >= start.x;
                if response.drag_stopped() {
                    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                    if !additive {
                        sess.selection.clear();
                    }
                    for pick in box_select(&sf.sketch, &pr, rect, window) {
                        if !sess.selection.contains(&pick) {
                            sess.selection.push(pick);
                        }
                    }
                    sess.box_start = None;
                } else {
                    // Window (left → right): fully inside; crossing (right → left): touching.
                    let (fill, stroke) = if window {
                        (theme::ACCENT.gamma_multiply(0.12), Stroke::new(1.0, theme::ACCENT))
                    } else {
                        (theme::GREEN.gamma_multiply(0.12), Stroke::new(1.0, theme::GREEN))
                    };
                    painter.rect_filled(rect, 0.0, fill);
                    if window {
                        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
                    } else {
                        let c = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                        painter.extend(egui::Shape::dashed_line(&c, stroke, 6.0, 4.0));
                    }
                }
            }
            if let (Some(p), true) = (sess.dragging, response.dragged_by(egui::PointerButton::Primary)) {
                if let Some(uv) = response.interact_pointer_pos().and_then(|pos| pr.local(pos)) {
                    let _ = sf.sketch.drag_point(p, snap_uv(uv)); // refuses silently if impossible
                    sess.analysis = None;
                }
            }
            if response.drag_stopped() {
                if let Some(p) = sess.dragging.take() {
                    changed = true;
                    // Dropped onto another point: make them coincident, like Fusion.
                    let here = pr.screen(sf.sketch.point_pos(p));
                    let target = here.and_then(|h| {
                        visible_points(&sf.sketch)
                            .into_iter()
                            .filter(|q| *q != p)
                            .filter_map(|q| pr.screen(sf.sketch.point_pos(q)).map(|s| (q, s.distance(h))))
                            .filter(|(_, d)| (*d as f64) < SNAP_PX)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(q, _)| q)
                    });
                    if let Some(q) = target {
                        if let Err(e) = try_constrain(&mut sf.sketch, Constraint::Coincident(p, q)) {
                            message = Some((e, true));
                        }
                    }
                }
            }
        }

        // ---- clicks
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                match sess.tool {
                    Tool::Select => {
                        let pick = match hover_badge {
                            Some(BadgeTarget::Constraint(c)) => Some(Pick::Constraint(c)),
                            Some(BadgeTarget::FixedPoint(p)) => Some(Pick::Point(p)),
                            None => hover_dim
                                .map(Pick::Constraint)
                                .or(hover_point.map(Pick::Point))
                                .or(hover_entity.map(Pick::Entity)),
                        };
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
                    }
                    Tool::Dimension => {
                        let pick = hover_point.map(Pick::Point).or(hover_entity.map(Pick::Entity));
                        match dimension_click(sess, &mut sf.sketch, &pr, pick, pos) {
                            Ok(c) => changed |= c,
                            Err(e) => message = Some((e, true)),
                        }
                    }
                    Tool::Constrain(kind) => {
                        let pick = if kind.wants_points() {
                            hover_point.map(Pick::Point).or(hover_entity.map(Pick::Entity))
                        } else {
                            hover_entity.map(Pick::Entity).or(hover_point.map(Pick::Point))
                        };
                        if let Some(p) = pick {
                            if let Some(i) = sess.selection.iter().position(|q| *q == p) {
                                sess.selection.remove(i);
                            } else {
                                sess.selection.push(p);
                            }
                            match apply_constraint_tool(sess, &mut sf.sketch, kind) {
                                Ok(c) => changed |= c,
                                Err(e) => message = Some((e, true)),
                            }
                        }
                    }
                    Tool::Trim | Tool::Extend => {
                        if let (Some(e), Some(uv)) = (hover_entity, pr.local(pos)) {
                            let r = if sess.tool == Tool::Trim { sf.sketch.trim(e, uv) } else { sf.sketch.extend(e, uv) };
                            match r {
                                Ok(()) => changed = true,
                                Err(e) => message = Some((e, true)),
                            }
                        }
                    }
                    Tool::Fillet => match hover_point {
                        Some(p) => match params.eval(&sess.fillet_radius) {
                            Ok(r) => match sf.sketch.fillet_corner(p, r) {
                                Ok((_, dim)) => {
                                    let text = sess.fillet_radius.trim().to_string();
                                    if text.parse::<f64>().is_err() {
                                        sf.set_expression(dim, text.clone());
                                    }
                                    sess.dim_edits.insert(dim, text);
                                    changed = true;
                                }
                                Err(e) => message = Some((e, true)),
                            },
                            Err(e) => message = Some((format!("fillet radius: {e}"), true)),
                        },
                        None => message = Some(("click the corner point where two lines meet".into(), false)),
                    },
                    Tool::Offset => {
                        let chain: Vec<EntityId> =
                            sess.selection.iter().filter_map(|p| if let Pick::Entity(e) = p { Some(*e) } else { None }).collect();
                        if chain.is_empty() {
                            if let Some(e) = hover_entity {
                                sess.selection = connected_chain(&sf.sketch, e).into_iter().map(Pick::Entity).collect();
                            }
                        } else if let Some(uv) = pr.local(pos) {
                            match params.eval(&sess.offset_distance) {
                                Ok(d) => match sf.sketch.offset(&chain, d, uv) {
                                    Ok(_) => {
                                        changed = true;
                                        sess.selection.clear();
                                    }
                                    Err(e) => message = Some((e, true)),
                                },
                                Err(e) => message = Some((format!("offset distance: {e}"), true)),
                            }
                        }
                    }
                    Tool::Mirror | Tool::CircPattern | Tool::RectPattern => {
                        let ids: Vec<EntityId> =
                            sess.selection.iter().filter_map(|p| if let Pick::Entity(e) = p { Some(*e) } else { None }).collect();
                        if ids.is_empty() {
                            message = Some(("select the curves first (Select tool, Shift adds)".into(), true));
                            sess.tool = Tool::Select;
                        } else {
                            let at = hover_point.map(|p| sf.sketch.point_pos(p)).or_else(|| pr.local(pos));
                            let count = params.eval(&sess.pattern_count).map(|v| v.round() as usize).unwrap_or(0);
                            let result = match (sess.tool, at) {
                                (Tool::Mirror, _) => match hover_entity {
                                    Some(axis) => sf.sketch.mirror(&ids, axis).map(|_| ()),
                                    None => Err("click a line to mirror at".into()),
                                },
                                (_, None) => Err("click on the sketch plane".into()),
                                (_, Some(_)) if !(2..=200).contains(&count) => Err("count must be 2 … 200".into()),
                                (Tool::CircPattern, Some(c)) => {
                                    let fs: Vec<Box<PointMap>> = (1..count)
                                        .map(|k| {
                                            let a = std::f64::consts::TAU * k as f64 / count as f64;
                                            Box::new(Sketch::rotate_about(c, a)) as Box<PointMap>
                                        })
                                        .collect();
                                    let refs: Vec<&dyn Fn([f64; 2]) -> [f64; 2]> = fs.iter().map(|f| f.as_ref()).collect();
                                    sf.sketch.copy_transformed(&ids, &refs);
                                    Ok(())
                                }
                                (_, Some(p)) => {
                                    // Step from the selection's centre to the click.
                                    let pts: Vec<[f64; 2]> =
                                        ids.iter().flat_map(|e| sf.sketch.entity(*e).points()).map(|q| sf.sketch.point_pos(q)).collect();
                                    let n = pts.len().max(1) as f64;
                                    let c = [pts.iter().map(|q| q[0]).sum::<f64>() / n, pts.iter().map(|q| q[1]).sum::<f64>() / n];
                                    let step = [p[0] - c[0], p[1] - c[1]];
                                    let fs: Vec<Box<PointMap>> = (1..count)
                                        .map(|k| {
                                            let v = [step[0] * k as f64, step[1] * k as f64];
                                            Box::new(move |q: [f64; 2]| [q[0] + v[0], q[1] + v[1]]) as Box<PointMap>
                                        })
                                        .collect();
                                    let refs: Vec<&dyn Fn([f64; 2]) -> [f64; 2]> = fs.iter().map(|f| f.as_ref()).collect();
                                    sf.sketch.copy_transformed(&ids, &refs);
                                    Ok(())
                                }
                            };
                            match result {
                                Ok(()) => {
                                    changed = true;
                                    sess.selection.clear();
                                    sess.tool = Tool::Select;
                                }
                                Err(e) => message = Some((e, true)),
                            }
                        }
                    }
                    Tool::Project => {
                        if let Some((_, poly)) = &project_hover {
                            match ferrocad_model::project::project_edge(sf, poly) {
                                Ok(()) => changed = true,
                                Err(e) => message = Some((e, true)),
                            }
                        }
                    }
                    // Clicks on labels and badges never create geometry.
                    _ if hover_dim.is_some() || hover_badge.is_some() => {}
                    _ => {
                        let anchor = match (target, hover_point) {
                            (Some(t), _) if locked => Some(Anchor::New(t)),
                            (_, Some(p)) => Some(Anchor::Existing(p)),
                            _ => pr.local(pos).map(|uv| match hover_entity {
                                Some(e) => Anchor::OnCurve(uv, e),
                                None => Anchor::New(snap_uv(uv)),
                            }),
                        };
                        if let Some(anchor) = anchor {
                            changed |= draw_commit(sess, sf, anchor, typed, &mut message);
                        }
                    }
                }
            }
        }

        // Finish a spline with Enter or a double-click.
        let enter = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if sess.tool == Tool::Spline && (enter || response.double_clicked()) {
            changed |= finish_spline(sess, &mut sf.sketch);
        }

        if changed {
            if let Err(e) = sf.sketch.solve() {
                message = Some((format!("sketch does not solve: {e}"), true));
            }
            let sk = &sf.sketch;
            sf.driven.retain(|(c, _)| sk.constraint(*c).is_some());
            sess.analysis = None;
        }
        if sess.analysis.as_ref().is_none_or(|a| !sf.sketch.analysis_is_current(a)) {
            sess.analysis = Some(sf.sketch.analyze());
        }

        // ---- drawing
        let sk = &sf.sketch;
        let analysis = sess.analysis.as_ref();
        let pre_highlight = matches!(sess.tool, Tool::Select | Tool::Dimension | Tool::Constrain(_));
        let mut highlighted: Vec<EntityId> = Vec::new();
        let mut highlighted_points: Vec<PointId> = Vec::new();
        for pick in &sess.selection {
            if let Pick::Constraint(c) = pick {
                if let Some(c) = sk.constraint(*c) {
                    highlighted.extend(c.entities());
                    highlighted_points.extend(c.points());
                }
            }
        }
        for (id, e) in sk.entities() {
            let pts = pr.polyline(&entity_polyline(sk, e));
            let selected = sess.selection.contains(&Pick::Entity(id)) || highlighted.contains(&id);
            let hovered = pre_highlight && hover_point.is_none() && hover_entity == Some(id);
            let construction = sk.is_construction(id);
            let color = if selected {
                COL_SELECTED
            } else if hovered {
                COL_HOVER
            } else if sf.projections.iter().any(|p| p.entity == id) {
                COL_PROJECTED
            } else if construction {
                COL_CONSTRUCTION
            } else if analysis.is_some_and(|a| sk.is_entity_determined(a, id)) {
                COL_FIXED
            } else {
                COL_FREE
            };
            let width = if selected || hovered { 2.6 } else { 1.7 };
            if construction {
                painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(width, color), 7.0, 5.0));
            } else {
                painter.add(egui::Shape::line(pts, Stroke::new(width, color)));
            }
        }
        if sess.show_points {
            for p in visible_points(sk) {
                if let Some(s) = pr.screen(sk.point_pos(p)) {
                    let selected = sess.selection.contains(&Pick::Point(p)) || highlighted_points.contains(&p);
                    let color = if selected {
                        COL_SELECTED
                    } else if analysis.is_some_and(|a| sk.is_point_determined(a, p)) {
                        COL_FIXED
                    } else {
                        COL_FREE
                    };
                    let hovered = Some(p) == hover_point;
                    let r = if hovered { 4.5 } else { 3.0 };
                    painter.circle(s, r, color, Stroke::new(1.0, Color32::from_black_alpha(160)));
                }
            }
        }

        // dimensions
        for (id, g) in &dims {
            let Some(c) = sk.constraint(*id) else { continue };
            let Some(v) = c.value() else { continue };
            let mut text = format_dimension(c, v);
            let driven = sf.driven.iter().find(|(d, _)| d == id).map(|(_, e)| e.as_str());
            if driven.is_some_and(|e| param_names.iter().any(|n| e.contains(n.as_str()))) {
                text = format!("ƒx {text}");
            }
            let highlight = if sess.selection.contains(&Pick::Constraint(*id)) {
                Some(COL_SELECTED)
            } else if hover_dim == Some(*id) {
                Some(COL_HOVER)
            } else {
                None
            };
            let editing = sess.inline.as_ref().is_some_and(|e| e.id == *id);
            let rect = paint_dimension(painter, g, if editing { "" } else { &text }, highlight);
            if let Some(edit) = sess.inline.as_mut().filter(|e| e.id == *id) {
                edit.pos = Some(rect.center());
            }
        }

        // constraint badges
        for b in &badges {
            let selected = match b.target {
                BadgeTarget::Constraint(c) => sess.selection.contains(&Pick::Constraint(c)),
                BadgeTarget::FixedPoint(p) => sess.selection.contains(&Pick::Point(p)),
            };
            let hovered = hover_badge == Some(b.target);
            let (fill, stroke) = if selected {
                (theme::ACCENT_DIM, COL_SELECTED)
            } else if hovered {
                (theme::HOVER, theme::ACCENT)
            } else {
                (Color32::from_rgba_premultiplied(48, 52, 58, 230), theme::BORDER)
            };
            painter.rect(b.rect, 3.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
            icons::paint(painter, b.rect.shrink(2.5), b.icon, theme::ICON, true);
        }

        // ---- value fields: live values and screen positions
        if let (Some(d), Some(t)) = (sess.draw_input.as_mut(), target) {
            let s0 = d.start;
            let mid = |a: [f64; 2], b: [f64; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            match sess.tool {
                Tool::Line => {
                    d.fields[0].live = format!("{:.2} mm", dist(s0, t));
                    d.fields[1].live = format!("{:.1}°", (t[1] - s0[1]).atan2(t[0] - s0[0]).to_degrees());
                    if let (Some(a), Some(b)) = (pr.screen(s0), pr.screen(t)) {
                        let dir = unit(b - a);
                        d.fields[0].pos = Some(a + (b - a) * 0.5 + dim_normal(dir) * 24.0);
                        d.fields[1].pos = Some(b + dir * 56.0);
                    }
                }
                Tool::Rectangle | Tool::CenterRectangle => {
                    let s0 = if sess.tool == Tool::CenterRectangle { [2.0 * s0[0] - t[0], 2.0 * s0[1] - t[1]] } else { s0 };
                    d.fields[0].live = format!("{:.2} mm", (t[0] - s0[0]).abs());
                    d.fields[1].live = format!("{:.2} mm", (t[1] - s0[1]).abs());
                    let c = pr.screen(mid(s0, t));
                    let w = pr.screen([(s0[0] + t[0]) * 0.5, t[1]]);
                    let h = pr.screen([t[0], (s0[1] + t[1]) * 0.5]);
                    if let (Some(c), Some(w), Some(h)) = (c, w, h) {
                        d.fields[0].pos = Some(w + unit(w - c) * 24.0);
                        d.fields[1].pos = Some(h + unit(h - c) * 48.0);
                    }
                }
                Tool::Circle => {
                    d.fields[0].live = format!("{:.2} mm", 2.0 * dist(s0, t));
                    if let (Some(a), Some(b)) = (pr.screen(s0), pr.screen(t)) {
                        d.fields[0].pos = Some(a + (b - a) * 0.5 + dim_normal(unit(b - a)) * 24.0);
                    }
                }
                _ => {}
            }
        }

        // ---- tool previews
        if let Some(h) = target.or(hover_uv) {
            let hpos = hover;
            let preview = Stroke::new(1.2, COL_PREVIEW);
            let pts: Vec<[f64; 2]> = sess.clicks.iter().map(|a| a.pos(sk)).collect();
            let (shape, label): (Vec<[f64; 2]>, Option<String>) = match (sess.tool, pts.as_slice()) {
                (Tool::Line, [a]) => {
                    let d = [h[0] - a[0], h[1] - a[1]];
                    let angle = d[1].atan2(d[0]).to_degrees();
                    (vec![*a, h], Some(format!("{:.2} mm   {angle:.1}°", d[0].hypot(d[1]))))
                }
                (Tool::Rectangle, [a]) => (
                    vec![*a, [h[0], a[1]], h, [a[0], h[1]], *a],
                    Some(format!("{:.2} × {:.2} mm", (h[0] - a[0]).abs(), (h[1] - a[1]).abs())),
                ),
                (Tool::Circle, [c]) => (
                    Segment::Circle { center: *c, radius: dist(*c, h), ccw: true }.sample(64),
                    Some(format!("Ø{:.2} mm", 2.0 * dist(*c, h))),
                ),
                (Tool::Arc, [c]) => (vec![*c, h], Some(format!("R{:.2} mm", dist(*c, h)))),
                (Tool::Arc, [c, s]) => (Segment::Arc { center: *c, from: *s, to: h, ccw: true }.sample(32), None),
                (Tool::CenterRectangle, [c]) => {
                    let o = [2.0 * c[0] - h[0], 2.0 * c[1] - h[1]];
                    (
                        vec![o, [h[0], o[1]], h, [o[0], h[1]], o],
                        Some(format!("{:.2} × {:.2} mm", (h[0] - o[0]).abs(), (h[1] - o[1]).abs())),
                    )
                }
                (Tool::Arc3, [s0]) => (vec![*s0, h], None),
                (Tool::Arc3, [s0, e0]) => (arc3_points(*s0, h, *e0), None),
                (Tool::Polygon, [c]) => {
                    let n = sess.polygon_sides.max(3);
                    let r = dist(*c, h);
                    let a0 = (h[1] - c[1]).atan2(h[0] - c[0]);
                    let pts = (0..=n)
                        .map(|k| {
                            let a = a0 + std::f64::consts::TAU * k as f64 / n as f64;
                            [c[0] + r * a.cos(), c[1] + r * a.sin()]
                        })
                        .collect();
                    (pts, Some(format!("{n} sides  R{r:.2} mm")))
                }
                (Tool::Spline, pts) if !pts.is_empty() => {
                    let mut fit = pts.to_vec();
                    fit.push(h);
                    (ferrocad_sketch::spline_samples(&fit, 16), None)
                }
                (Tool::Slot, [c1]) => (vec![*c1, h], Some(format!("{:.2} mm", dist(*c1, h)))),
                (Tool::Slot, [c1, c2]) => {
                    let w = slot_half_width(*c1, *c2, h);
                    let mut tmp = Sketch::new();
                    let shape = match tmp.add_slot(*c1, *c2, 2.0 * w) {
                        Ok(ids) => ids.iter().flat_map(|e| entity_polyline(&tmp, tmp.entity(*e))).collect(),
                        Err(_) => vec![],
                    };
                    (shape, Some(format!("width {:.2} mm", 2.0 * w)))
                }
                _ => (vec![], None),
            };
            if shape.len() > 1 {
                painter.add(egui::Shape::line(pr.polyline(&shape), preview));
            }
            if let (Some(label), Some(hpos), None) = (label, hpos, &sess.draw_input) {
                cursor_label(painter, hpos, &label);
            }
            // Snap indicator.
            if sess.tool.is_drawing() || sess.tool == Tool::Dimension {
                if let Some(s) = hover_point.and_then(|p| pr.screen(sk.point_pos(p))) {
                    painter.rect_stroke(
                        Rect::from_center_size(s, egui::vec2(11.0, 11.0)),
                        1.0,
                        Stroke::new(1.5, COL_SELECTED),
                        egui::StrokeKind::Middle,
                    );
                }
            }
            // Dimension tool: show the dimension before it is placed.
            if let (Tool::Dimension, Some(hpos)) = (sess.tool, hpos) {
                if let Some((c, off)) = pending_dimension(sess, sk, &pr, hpos) {
                    if let Some(g) = dimension_geom(sk, &pr, &c, off) {
                        let v = measure(sk, &c).unwrap_or(0.0);
                        paint_dimension(painter, &g, &format_dimension(&c, v), Some(COL_PREVIEW));
                    }
                }
            }
        }

        if let Some((_, poly)) = &project_hover {
            let pts: Vec<Pos2> = poly.iter().filter_map(|p| cam.project(*p, vp)).map(|s| egui::pos2(s[0] as f32, s[1] as f32)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(3.0, theme::ACCENT)));
        }

        if changed {
            self.commit_sketch_edit("Sketch edit");
        }
        if let Some(m) = message {
            self.message = Some(m);
        }
    }

    /// Run a sketch command (ribbon, keyboard, context menu).
    pub fn sketch_command(&mut self, cmd: SketchCommand) {
        let Mode::Sketch(sess) = &mut self.mode else { return };
        let Some(sf) = self.doc.sketch_mut(sess.feature) else { return };
        let mut changed = false;
        let mut message: Option<(String, bool)> = None;
        match cmd {
            SketchCommand::Tool(tool) => {
                sess.clicks.clear();
                if tool == Tool::Dimension {
                    if let Some(c) = dimension_from_selection(&sess.selection, &sf.sketch) {
                        match create_dimension(sess, &mut sf.sketch, c, None) {
                            Ok(()) => changed = true,
                            Err(e) => message = Some((e, true)),
                        }
                    }
                }
                if tool.uses_selection() && !sess.selection.iter().any(|p| matches!(p, Pick::Entity(_))) {
                    message = Some(("select the curves first (Select tool, Shift adds), then pick this tool".into(), false));
                } else if tool != Tool::Select && !tool.uses_selection() {
                    sess.selection.clear();
                }
                sess.tool = tool;
            }
            SketchCommand::Constrain(kind) => {
                sess.clicks.clear();
                let had_selection = !sess.selection.is_empty() && sess.tool == Tool::Select;
                if !had_selection {
                    sess.selection.clear();
                }
                match apply_constraint_tool(sess, &mut sf.sketch, kind) {
                    Ok(c) => changed = c,
                    Err(e) => message = Some((e, true)),
                }
                // Applied straight to the selection: stay in Select, otherwise pick.
                if !(changed && had_selection) {
                    sess.tool = Tool::Constrain(kind);
                }
            }
            SketchCommand::Construction => {
                for pick in &sess.selection {
                    if let Pick::Entity(e) = *pick {
                        let c = sf.sketch.is_construction(e);
                        sf.sketch.set_construction(e, !c);
                        changed = true;
                    }
                }
                if !changed {
                    message = Some(("select curves to toggle construction".into(), false));
                }
            }
            SketchCommand::Delete => {
                for pick in sess.selection.drain(..) {
                    match pick {
                        Pick::Entity(e) => sf.sketch.remove_entity(e),
                        Pick::Constraint(c) => {
                            sf.sketch.remove_constraint(c);
                            sf.driven.retain(|(d, _)| *d != c);
                            sess.dim_edits.remove(&c);
                        }
                        Pick::Point(p) => {
                            if sf.sketch.is_point_fixed(p) {
                                sf.sketch.set_point_fixed(p, false);
                            }
                        }
                    }
                    changed = true;
                }
            }
            SketchCommand::Escape => {
                if !sess.clicks.is_empty() {
                    sess.clicks.clear();
                } else if sess.tool != Tool::Select {
                    sess.tool = Tool::Select;
                    sess.selection.clear();
                } else {
                    sess.selection.clear();
                }
            }
        }
        if changed {
            if let Err(e) = sf.sketch.solve() {
                message = Some((format!("sketch does not solve: {e}"), true));
            }
            sess.analysis = None;
            self.commit_sketch_edit("Sketch edit");
        }
        if let Some(m) = message {
            self.message = Some(m);
        }
    }

    /// Value fields next to the shape being drawn.
    fn draw_input_overlay(&mut self, ctx: &egui::Context) {
        let params = self.doc.params.clone();
        let Mode::Sketch(sess) = &mut self.mode else { return };
        if sess.inline.is_some() {
            return;
        }
        let Some(input) = &mut sess.draw_input else { return };
        let n = input.fields.len();
        let (tab, back, enter, esc) = ctx.input(|i| {
            (i.key_pressed(egui::Key::Tab), i.modifiers.shift, i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))
        });
        if tab {
            input.active = if back { (input.active + n - 1) % n } else { (input.active + 1) % n };
        }
        let active = input.active;
        let mut clicked = None;
        for (i, f) in input.fields.iter_mut().enumerate() {
            let Some(pos) = f.pos else { continue };
            let invalid = !f.text.trim().is_empty() && params.eval(&f.text).is_err();
            let stroke = if invalid {
                Stroke::new(1.5, theme::ERROR)
            } else if i == active {
                Stroke::new(1.5, theme::ACCENT)
            } else {
                Stroke::new(1.0, theme::BORDER)
            };
            egui::Area::new(egui::Id::new(("draw_field", i)))
                .fixed_pos(pos)
                .pivot(egui::Align2::CENTER_CENTER)
                .order(egui::Order::Foreground)
                .fade_in(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(theme::WINDOW)
                        .stroke(stroke)
                        .corner_radius(3.0)
                        .inner_margin(egui::Margin::symmetric(5, 2))
                        .show(ui, |ui| {
                            let edit = egui::TextEdit::singleline(&mut f.text)
                                .id(egui::Id::new(("draw_field_edit", i)))
                                .hint_text(f.live.clone())
                                .desired_width(68.0)
                                .lock_focus(true)
                                .frame(egui::Frame::NONE);
                            let r = ui.add(edit);
                            if r.clicked() {
                                clicked = Some(i);
                            } else if i == active && !r.has_focus() {
                                r.request_focus();
                            }
                        });
                });
            f.text.retain(|c| c != '\t');
        }
        if let Some(i) = clicked {
            input.active = i;
        }
        if enter {
            input.commit = true;
        }
        if esc {
            sess.clicks.clear();
            sess.draw_input = None;
        }
    }

    /// Floating parts of sketch mode: drawing value fields and the inline
    /// dimension editor.
    pub fn sketch_overlays(&mut self, ctx: &egui::Context) {
        self.draw_input_overlay(ctx);
        let params = self.doc.params.clone();
        let Mode::Sketch(sess) = &mut self.mode else { return };
        let Some(edit) = &mut sess.inline else { return };
        let Some(pos) = edit.pos else { return };
        let sf_dim_is_angle = self
            .doc
            .sketch(sess.feature)
            .and_then(|sf| sf.sketch.constraint(edit.id))
            .is_some_and(|c| matches!(c, Constraint::Angle(..)));
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("inline_dimension"))
            .fixed_pos(pos)
            .pivot(egui::Align2::CENTER_CENTER)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut edit.text)
                            .desired_width(110.0)
                            .hint_text("value or parameter"),
                    );
                    if edit.focus {
                        r.request_focus();
                        edit.focus = false;
                    }
                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                    if r.lost_focus() {
                        if esc {
                            cancel = true;
                        } else {
                            commit = true;
                        }
                    } else if enter {
                        commit = true;
                    }
                    match params.eval(&edit.text) {
                        Ok(v) if edit.text.trim().parse::<f64>().is_err() => {
                            let unit = match sf_dim_is_angle { true => "°", false => "mm" };
                            ui.weak(format!("= {}", fmt_value(v, unit)));
                        }
                        Err(_) if !edit.text.trim().is_empty() => {
                            ui.colored_label(theme::ERROR, "invalid expression");
                        }
                        _ => {}
                    }
                });
            });
        if !(commit || cancel) {
            return;
        }
        let edit = sess.inline.take().expect("inline editor is open");
        if cancel {
            return;
        }
        let Some(sf) = self.doc.sketch_mut(sess.feature) else { return };
        match apply_dimension(sf, &params, edit.id, &edit.text) {
            Ok(()) => {
                let shown = dimension_text(sf, edit.id, sf.sketch.constraint(edit.id).expect("just applied"));
                sess.dim_edits.insert(edit.id, shown);
                sess.analysis = None;
                self.commit_sketch_edit("Edit dimension");
            }
            Err(e) => self.error(e),
        }
    }

    // --------------------------------------------------------- palette panel

    pub fn sketch_palette(&mut self, ui: &mut egui::Ui) {
        let mut finish = false;
        let mut look_at = false;
        let params = self.doc.params.clone();
        let Mode::Sketch(sess) = &mut self.mode else { return };
        let name = self.doc.feature(sess.feature).map(|f| f.name.clone()).unwrap_or_default();
        let Some(sf) = self.doc.sketch_mut(sess.feature) else {
            self.mode = Mode::Model;
            return;
        };
        let mut message: Option<(String, bool)> = None;
        // Cheap per-frame copy of just this sketch to detect edits for undo.
        let snapshot = (sf.sketch.to_data(), sf.driven.clone());

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("SKETCH PALETTE").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(&name));
        });
        ui.add_space(4.0);

        // ---- status card
        if let Some(a) = &sess.analysis {
            egui::Frame::new()
                .fill(theme::WINDOW)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if a.is_fully_constrained() {
                        icons::icon_label(ui, Icon::Fix, egui::RichText::new("Fully constrained").color(COL_FIXED));
                    } else {
                        let n = a.dof;
                        icons::icon_label(
                            ui,
                            Icon::Dimension,
                            egui::RichText::new(format!("{n} degree{} of freedom left", if n == 1 { "" } else { "s" }))
                                .color(COL_FREE),
                        );
                        ui.weak("Blue geometry can still move; white is fixed.");
                    }
                    if a.redundant > 0 {
                        ui.colored_label(theme::ORANGE, format!("{} redundant constraint(s)", a.redundant));
                    }
                });
        }

        // ---- options
        theme::section(ui, "Options");
        ui.horizontal(|ui| {
            if ui.button("Look At").on_hover_text("Look straight at the sketch plane").clicked() {
                look_at = true;
            }
        });
        ui.checkbox(&mut sess.snap_to_grid, "Snap to grid");
        ui.checkbox(&mut sess.show_points, "Show points");
        ui.checkbox(&mut sess.show_dimensions, "Show dimensions");
        ui.checkbox(&mut sess.show_constraints, "Show constraints");
        let option = |ui: &mut egui::Ui, label: &str, text: &mut String| {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(text).desired_width(90.0));
            });
        };
        match sess.tool {
            Tool::Polygon => {
                theme::section(ui, "Polygon");
                ui.add(egui::Slider::new(&mut sess.polygon_sides, 3..=24).text("sides"));
            }
            Tool::Fillet => {
                theme::section(ui, "Sketch fillet");
                option(ui, "Radius", &mut sess.fillet_radius);
            }
            Tool::Offset => {
                theme::section(ui, "Offset");
                option(ui, "Distance", &mut sess.offset_distance);
            }
            Tool::CircPattern | Tool::RectPattern => {
                theme::section(ui, "Pattern");
                option(ui, "Count", &mut sess.pattern_count);
            }
            _ => {}
        }

        // ---- dimension table
        theme::section(ui, "Dimensions");
        let dims: Vec<(ConstraintId, Constraint)> =
            sf.sketch.constraints().filter(|(_, c)| c.is_dimension()).map(|(id, c)| (id, c.clone())).collect();
        if dims.is_empty() {
            ui.weak("None yet – press D and click geometry.");
        }
        let mut remove: Option<ConstraintId> = None;
        egui::Grid::new("dims").num_columns(3).striped(true).show(ui, |ui| {
            for (id, c) in &dims {
                let selected = sess.selection.contains(&Pick::Constraint(*id));
                if ui.selectable_label(selected, c.label()).clicked() {
                    sess.selection = vec![Pick::Constraint(*id)];
                    sess.tool = Tool::Select;
                }
                let text = sess.dim_edits.entry(*id).or_insert_with(|| dimension_text(sf, *id, c));
                let resp = ui.add_sized([96.0, 20.0], egui::TextEdit::singleline(text));
                if ui.small_button("×").on_hover_text("Delete dimension").clicked() {
                    remove = Some(*id);
                }
                ui.end_row();
                if resp.lost_focus() {
                    let expr = text.clone();
                    let unchanged = expr == dimension_text(sf, *id, c);
                    if !unchanged {
                        match apply_dimension(sf, &params, *id, &expr) {
                            Ok(()) => {
                                *text = dimension_text(sf, *id, c);
                                sess.analysis = None;
                            }
                            Err(e) => message = Some((e, true)),
                        }
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
        let geo: Vec<(ConstraintId, &'static str, Option<Icon>)> = sf
            .sketch
            .constraints()
            .filter(|(_, c)| !c.is_dimension())
            .map(|(id, c)| (id, c.label(), badge_icon(c)))
            .collect();
        egui::CollapsingHeader::new(format!("CONSTRAINTS ({})", geo.len())).id_salt("geo_constraints").show(ui, |ui| {
            for (id, label, icon) in geo {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    icons::paint(ui.painter(), rect, icon.unwrap_or(Icon::Coincident), theme::ICON, true);
                    let selected = sess.selection.contains(&Pick::Constraint(id));
                    if ui.selectable_label(selected, label).clicked() {
                        sess.selection = vec![Pick::Constraint(id)];
                        sess.tool = Tool::Select;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Delete constraint").clicked() {
                            sf.sketch.remove_constraint(id);
                            sess.analysis = None;
                        }
                    });
                });
            }
        });

        ui.add_space(12.0);
        let finish_button = egui::Button::new(egui::RichText::new("✔  Finish Sketch").color(Color32::WHITE).strong())
            .fill(theme::GREEN)
            .min_size(egui::vec2(ui.available_width(), 30.0));
        finish |= ui.add(finish_button).clicked();
        ui.add_space(6.0);
        ui.weak("Shortcuts: L line · R rectangle · C circle · A arc · D dimension · X construction · Esc select");

        let edited = (sf.sketch.to_data(), sf.driven.clone()) != snapshot;
        if let Some(m) = message {
            self.message = Some(m);
        }
        if edited {
            self.commit_sketch_edit("Sketch edit");
        }
        if look_at {
            self.run(crate::app::Command::LookAt);
        }
        if finish {
            self.finish_sketch();
        }
    }
}

/// Dimension that the Dimension tool would create if clicked at `pos`, with
/// its offset (for the placement preview).
fn pending_dimension(sess: &SketchSession, sk: &Sketch, pr: &Projector, pos: Pos2) -> Option<(Constraint, f32)> {
    match sess.selection.as_slice() {
        [Pick::Entity(l)] if is_line(sk, *l) => {
            let Entity::Line { a, b } = *sk.entity(*l) else { return None };
            let (a, b) = (pr.screen(sk.point_pos(a))?, pr.screen(sk.point_pos(b))?);
            Some((Constraint::Length(*l, 0.0), linear_offset(a, b, (b - a).normalized(), pos)))
        }
        [Pick::Point(p), Pick::Point(q)] => {
            let (pa, pb) = (sk.point_pos(*p), sk.point_pos(*q));
            let at = pr.local(pos)?;
            let (sa, sb) = (pr.screen(pa)?, pr.screen(pb)?);
            let (ex, ey) = pr.axes(pa)?;
            // Order points so horizontal/vertical distances come out positive.
            match distance_kind(pa, pb, at) {
                1 => {
                    let (a, b) = if pa[0] <= pb[0] { (*p, *q) } else { (*q, *p) };
                    Some((Constraint::HorizontalDistance(a, b, 0.0), linear_offset(sa, sb, ex, pos)))
                }
                2 => {
                    let (a, b) = if pa[1] <= pb[1] { (*p, *q) } else { (*q, *p) };
                    Some((Constraint::VerticalDistance(a, b, 0.0), linear_offset(sa, sb, ey, pos)))
                }
                _ => Some((Constraint::Distance(*p, *q, 0.0), linear_offset(sa, sb, (sb - sa).normalized(), pos))),
            }
        }
        _ => None,
    }
}

/// A click with the Dimension tool. Returns true if a dimension was added.
fn dimension_click(
    sess: &mut SketchSession,
    sk: &mut Sketch,
    pr: &Projector,
    pick: Option<Pick>,
    pos: Pos2,
) -> Result<bool, String> {
    let sel = sess.selection.clone();
    let create = |sess: &mut SketchSession, sk: &mut Sketch, c: Constraint, off: Option<f32>| {
        sess.selection.clear();
        create_dimension(sess, sk, c, off).map(|_| true)
    };
    match (sel.as_slice(), pick) {
        ([], Some(Pick::Entity(e))) if !is_line(sk, e) => create(sess, sk, circle_dimension(sk, e), None),
        ([], Some(p)) => {
            sess.selection = vec![p];
            Ok(false)
        }
        ([], None) => Ok(false),
        ([Pick::Entity(l1)], Some(Pick::Entity(l2))) if *l1 != l2 && is_line(sk, l2) => {
            create(sess, sk, Constraint::Angle(*l1, l2, 0.0), None)
        }
        ([Pick::Point(p)], Some(Pick::Point(q))) if *p != q => {
            sess.selection.push(Pick::Point(q));
            Ok(false)
        }
        ([Pick::Point(p)], Some(Pick::Entity(l))) if is_line(sk, l) => {
            create(sess, sk, Constraint::PointLineDistance(*p, l, 0.0), None)
        }
        ([Pick::Entity(l)], Some(Pick::Point(p))) if is_line(sk, *l) && !sk.entity(*l).points().contains(&p) => {
            create(sess, sk, Constraint::PointLineDistance(p, *l, 0.0), None)
        }
        ([Pick::Point(_)], Some(Pick::Entity(_))) => {
            sess.selection.clear();
            Err("pick a point and a line, two points, or two lines".into())
        }
        ([Pick::Entity(_)] | [Pick::Point(_), Pick::Point(_)], _) => match pending_dimension(sess, sk, pr, pos) {
            Some((c, off)) => create(sess, sk, c, Some(off)),
            None => {
                sess.selection.clear();
                Ok(false)
            }
        },
        _ => {
            sess.selection = pick.into_iter().collect();
            Ok(false)
        }
    }
}

/// Apply the constraint tool to the current picks if they are complete.
fn apply_constraint_tool(sess: &mut SketchSession, sk: &mut Sketch, kind: ConstraintKind) -> Result<bool, String> {
    match evaluate(kind, &sess.selection, sk) {
        Eval::Incomplete => Ok(false),
        Eval::Invalid => {
            // Keep only the last pick if it can start a valid set.
            let last = sess.selection.pop();
            sess.selection.clear();
            if let Some(p) = last {
                sess.selection.push(p);
                if matches!(evaluate(kind, &sess.selection, sk), Eval::Invalid) {
                    sess.selection.clear();
                }
            }
            Err(kind.tooltip().to_string())
        }
        Eval::Fix(points) => {
            for p in points {
                let fixed = sk.is_point_fixed(p);
                sk.set_point_fixed(p, !fixed);
            }
            sess.selection.clear();
            sess.analysis = None;
            Ok(true)
        }
        Eval::Ready(cs) => {
            sess.selection.clear();
            let mut any = false;
            let mut error = None;
            for c in cs {
                match try_constrain(sk, c) {
                    Ok(_) => any = true,
                    Err(e) => error = Some(e),
                }
            }
            sess.analysis = None;
            match error {
                Some(e) if !any => Err(e),
                _ => Ok(any),
            }
        }
    }
}

fn unit(v: Vec2) -> Vec2 {
    if v.length() > 1e-3 {
        v.normalized()
    } else {
        Vec2::X
    }
}

/// Create the shape for a drawing click and turn typed values into dimensions.
fn draw_commit(
    sess: &mut SketchSession,
    sf: &mut SketchFeature,
    anchor: Anchor,
    typed: [Option<f64>; 2],
    message: &mut Option<(String, bool)>,
) -> bool {
    let texts: Vec<String> = sess
        .draw_input
        .as_ref()
        .map(|d| d.fields.iter().map(|f| f.text.trim().to_owned()).collect())
        .unwrap_or_default();
    let mut err = None;
    let created = handle_click(sess, &mut sf.sketch, anchor, &mut err);
    if let Some(e) = err {
        *message = Some((e, true));
    }
    let Some(created) = created else { return false };
    let dims: Vec<(Constraint, usize)> = match created {
        Created::Line(l) => typed[0].map(|v| (Constraint::Length(l, v), 0)).into_iter().collect(),
        Created::Rectangle([bottom, right, ..]) => [
            typed[0].map(|v| (Constraint::Length(bottom, v), 0)),
            typed[1].map(|v| (Constraint::Length(right, v), 1)),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Created::Circle(c) => typed[0].map(|v| (Constraint::Diameter(c, v), 0)).into_iter().collect(),
        Created::Arc | Created::Shape => vec![],
    };
    for (c, field) in dims {
        match try_constrain(&mut sf.sketch, c) {
            Ok(id) => {
                let text = texts.get(field).cloned().unwrap_or_default();
                if text.parse::<f64>().is_err() {
                    sf.set_expression(id, text.clone());
                }
                sess.dim_edits.insert(id, with_unit(&text, "mm"));
            }
            Err(e) => *message = Some((e, true)),
        }
    }
    true
}

/// Apply a viewport click for a drawing tool. Returns the created geometry.
fn handle_click(sess: &mut SketchSession, sk: &mut Sketch, anchor: Anchor, error: &mut Option<String>) -> Option<Created> {
    let resolve = |a: Anchor, sk: &mut Sketch| match a {
        Anchor::Existing(p) => p,
        Anchor::New(uv) => sk.add_point(uv),
        Anchor::OnCurve(uv, e) => {
            let p = sk.add_point(uv);
            stick_to_curve(sk, p, e);
            p
        }
    };
    match sess.tool {
        Tool::Line => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return None;
            }
            let (a0, b0) = (sess.clicks[0], sess.clicks[1]);
            if dist(a0.pos(sk), b0.pos(sk)) < 1e-6 {
                sess.clicks.pop();
                return None;
            }
            let a = resolve(a0, sk);
            let b = resolve(b0, sk);
            let line = sk.add_line_between(a, b);
            // Auto-constrain nearly horizontal / vertical lines.
            let d = line_dir(sk, line).unwrap_or([1.0, 0.0]);
            let angle = d[1].atan2(d[0]).to_degrees().rem_euclid(180.0);
            if !(3.0..=177.0).contains(&angle) {
                let _ = try_constrain(sk, Constraint::Horizontal(line));
            } else if (angle - 90.0).abs() < 3.0 {
                let _ = try_constrain(sk, Constraint::Vertical(line));
            }
            // Snapping onto an existing point closes the chain.
            sess.clicks = if b0.existing().is_some() { vec![] } else { vec![Anchor::Existing(b)] };
            Some(Created::Line(line))
        }
        Tool::Rectangle => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return None;
            }
            let (c1, c2) = (sess.clicks[0], sess.clicks[1]);
            sess.clicks.clear();
            let (p1, p2) = (c1.pos(sk), c2.pos(sk));
            if (p1[0] - p2[0]).abs() < 1e-6 || (p1[1] - p2[1]).abs() < 1e-6 {
                *error = Some("rectangle needs width and height".into());
                return None;
            }
            let lines = sk.add_rectangle(p1, p2);
            let [bottom, _, top, _] = lines;
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
            if let Some(e) = c1.curve() {
                let q = corner(sk, bottom);
                stick_to_curve(sk, q, e);
            }
            if let Some(e) = c2.curve() {
                let q = corner(sk, top);
                stick_to_curve(sk, q, e);
            }
            Some(Created::Rectangle(lines))
        }
        Tool::Circle => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return None;
            }
            let (c, r) = (sess.clicks[0], sess.clicks[1]);
            sess.clicks.clear();
            let radius = dist(c.pos(sk), r.pos(sk));
            if radius < 1e-6 {
                return None;
            }
            let circle = sk.add_circle(c.pos(sk), radius);
            if let (Some(p), Some(center)) = (c.existing(), e_center(sk, circle)) {
                let _ = try_constrain(sk, Constraint::Coincident(center, p));
            }
            if let (Some(e), Some(center)) = (c.curve(), e_center(sk, circle)) {
                stick_to_curve(sk, center, e);
            }
            Some(Created::Circle(circle))
        }
        Tool::Arc => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 3 {
                return None;
            }
            let (c, s, e) = (sess.clicks[0], sess.clicks[1], sess.clicks[2]);
            sess.clicks.clear();
            if dist(c.pos(sk), s.pos(sk)) < 1e-6 {
                return None;
            }
            let arc = sk.add_arc(c.pos(sk), s.pos(sk), e.pos(sk));
            let pts = sk.entity(arc).points(); // [center, start, end]
            for (anchor, p) in [(c, pts[0]), (s, pts[1]), (e, pts[2])] {
                if let Some(existing) = anchor.existing() {
                    let _ = try_constrain(sk, Constraint::Coincident(p, existing));
                }
                if let Some(curve) = anchor.curve() {
                    stick_to_curve(sk, p, curve);
                }
            }
            Some(Created::Arc)
        }
        Tool::Spline => {
            // Points collect until Enter / double-click (see `finish_spline`).
            sess.clicks.push(anchor);
            None
        }
        Tool::CenterRectangle => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return None;
            }
            let (c, k) = (sess.clicks[0].pos(sk), sess.clicks[1].pos(sk));
            sess.clicks.clear();
            if (c[0] - k[0]).abs() < 1e-6 || (c[1] - k[1]).abs() < 1e-6 {
                *error = Some("rectangle needs width and height".into());
                return None;
            }
            Some(Created::Rectangle(sk.add_center_rectangle(c, k)))
        }
        Tool::Arc3 => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 3 {
                return None;
            }
            let (s, e, m) = (sess.clicks[0], sess.clicks[1], sess.clicks[2]);
            sess.clicks.clear();
            match sk.add_arc_3pt(s.pos(sk), m.pos(sk), e.pos(sk)) {
                Ok(arc) => {
                    let pts = sk.entity(arc).points();
                    for anchor in [s, e] {
                        if let Some(existing) = anchor.existing() {
                            let at = sk.point_pos(existing);
                            if let Some(&p) = pts[1..].iter().find(|p| dist(sk.point_pos(**p), at) < 1e-6) {
                                let _ = try_constrain(sk, Constraint::Coincident(p, existing));
                            }
                        }
                    }
                    Some(Created::Arc)
                }
                Err(e) => {
                    *error = Some(e);
                    None
                }
            }
        }
        Tool::Polygon => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 2 {
                return None;
            }
            let (c, k) = (sess.clicks[0].pos(sk), sess.clicks[1].pos(sk));
            sess.clicks.clear();
            match sk.add_polygon(c, k, sess.polygon_sides) {
                Ok(_) => Some(Created::Shape),
                Err(e) => {
                    *error = Some(e);
                    None
                }
            }
        }
        Tool::Slot => {
            sess.clicks.push(anchor);
            if sess.clicks.len() < 3 {
                return None;
            }
            let (c1, c2, w) = (sess.clicks[0].pos(sk), sess.clicks[1].pos(sk), sess.clicks[2].pos(sk));
            sess.clicks.clear();
            let width = 2.0 * slot_half_width(c1, c2, w);
            match sk.add_slot(c1, c2, width) {
                Ok(_) => Some(Created::Shape),
                Err(e) => {
                    *error = Some(e);
                    None
                }
            }
        }
        _ => None,
    }
}

/// Entities and points picked by a selection rectangle: fully inside
/// (`window`) or touching it.
fn box_select(sk: &Sketch, pr: &Projector, rect: Rect, window: bool) -> Vec<Pick> {
    let mut out = Vec::new();
    for (id, e) in sk.entities() {
        let pts = pr.polyline(&entity_polyline(sk, e));
        if pts.is_empty() {
            continue;
        }
        // Densify straight pieces so crossings are found.
        let samples: Vec<Pos2> = pts
            .windows(2)
            .flat_map(|w| (0..=8).map(move |k| w[0] + (w[1] - w[0]) * (k as f32 / 8.0)))
            .chain(pts.iter().copied())
            .collect();
        let hit = if window { samples.iter().all(|p| rect.contains(*p)) } else { samples.iter().any(|p| rect.contains(*p)) };
        if hit {
            out.push(Pick::Entity(id));
        }
    }
    for p in visible_points(sk) {
        if pr.screen(sk.point_pos(p)).is_some_and(|s| rect.contains(s)) {
            out.push(Pick::Point(p));
        }
    }
    out
}

/// Turn the collected clicks into a spline (consecutive duplicates dropped).
fn finish_spline(sess: &mut SketchSession, sk: &mut Sketch) -> bool {
    let mut anchors: Vec<Anchor> = Vec::new();
    for a in sess.clicks.drain(..) {
        if anchors.last().is_none_or(|l| dist(l.pos(sk), a.pos(sk)) > 1e-6) {
            anchors.push(a);
        }
    }
    if anchors.len() < 2 {
        return false;
    }
    let points = anchors
        .into_iter()
        .map(|a| match a {
            Anchor::Existing(p) => p,
            Anchor::New(uv) => sk.add_point(uv),
            Anchor::OnCurve(uv, e) => {
                let p = sk.add_point(uv);
                stick_to_curve(sk, p, e);
                p
            }
        })
        .collect();
    sk.add_spline_through(points);
    true
}

fn slot_half_width(c1: [f64; 2], c2: [f64; 2], p: [f64; 2]) -> f64 {
    let d = [c2[0] - c1[0], c2[1] - c1[1]];
    let len = d[0].hypot(d[1]).max(1e-12);
    ((d[0] * (p[1] - c1[1]) - d[1] * (p[0] - c1[0])) / len).abs()
}

/// Points of the arc from `s` through `m` to `e`.
fn arc3_points(s: [f64; 2], m: [f64; 2], e: [f64; 2]) -> Vec<[f64; 2]> {
    let (a, b) = ([m[0] - s[0], m[1] - s[1]], [e[0] - s[0], e[1] - s[1]]);
    let d = 2.0 * (a[0] * b[1] - a[1] * b[0]);
    if d.abs() < 1e-12 {
        return vec![s, e];
    }
    let (aa, bb) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1]);
    let c = [s[0] + (b[1] * aa - a[1] * bb) / d, s[1] + (a[0] * bb - b[0] * aa) / d];
    let ccw = (m[0] - s[0]) * (e[1] - m[1]) - (m[1] - s[1]) * (e[0] - m[0]) > 0.0;
    let (from, to) = if ccw { (s, e) } else { (e, s) };
    Segment::Arc { center: c, from, to, ccw: true }.sample(32)
}

/// Lines connected to `e` through shared end points (a circle on its own).
fn connected_chain(sk: &Sketch, e: EntityId) -> Vec<EntityId> {
    if !matches!(sk.entity(e), Entity::Line { .. }) {
        return vec![e];
    }
    let mut chain = vec![e];
    let mut frontier = vec![e];
    while let Some(cur) = frontier.pop() {
        let pts = sk.entity(cur).points();
        for (id, other) in sk.entities() {
            if chain.contains(&id) || !matches!(other, Entity::Line { .. }) || sk.is_construction(id) {
                continue;
            }
            if other.points().iter().any(|p| pts.contains(p)) {
                chain.push(id);
                frontier.push(id);
            }
        }
    }
    chain
}

