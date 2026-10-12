//! Command dialogs for the solid and construction features, built from a
//! declarative list of inputs: selections (faces, edges, bodies, profiles,
//! planes, axes, sketch curves) and values (expressions, checkboxes, an
//! operation). Picking, highlighting, live preview and editing are shared.

use crate::app::{FerroApp, Mode};
use crate::dialogs::{command_dialog, expression_field, form_label, DialogAction};
use crate::icons::{self, Icon};
use crate::sketch_ui::entity_polyline;
use crate::theme;
use egui::{Color32, Pos2, RichText, Stroke};
use ferrocad_model::geom::{add, dot, normalize, scale, sub, Vec3};
use ferrocad_model::*;
use ferrocad_sketch::find_profiles;
use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::EntityId;
use ferrocad_view::Viewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureType {
    PressPull,
    Shell,
    Hole,
    Mirror,
    RectPattern,
    CircPattern,
    Combine,
    Split,
    Move,
    Loft,
    Sweep,
    Plane,
    AxisEdge,
    AxisFace,
    Joint,
}

impl FeatureType {
    pub fn title(self) -> &'static str {
        match self {
            FeatureType::PressPull => "Press Pull",
            FeatureType::Shell => "Shell",
            FeatureType::Hole => "Hole",
            FeatureType::Mirror => "Mirror",
            FeatureType::RectPattern => "Rectangular Pattern",
            FeatureType::CircPattern => "Circular Pattern",
            FeatureType::Combine => "Combine",
            FeatureType::Split => "Split Body",
            FeatureType::Move => "Move/Copy",
            FeatureType::Loft => "Loft",
            FeatureType::Sweep => "Sweep",
            FeatureType::Plane => "Construction Plane",
            FeatureType::AxisEdge => "Axis Through Edge",
            FeatureType::AxisFace => "Axis Normal to Face",
            FeatureType::Joint => "Joint",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            FeatureType::PressPull => Icon::PressPull,
            FeatureType::Shell => Icon::Shell,
            FeatureType::Hole => Icon::Hole,
            FeatureType::Mirror => Icon::Mirror,
            FeatureType::RectPattern => Icon::RectPattern,
            FeatureType::CircPattern => Icon::CircPattern,
            FeatureType::Combine => Icon::Combine,
            FeatureType::Split => Icon::Split,
            FeatureType::Move => Icon::Move,
            FeatureType::Loft => Icon::Loft,
            FeatureType::Sweep => Icon::Sweep,
            FeatureType::Plane => Icon::Plane,
            FeatureType::AxisEdge | FeatureType::AxisFace => Icon::Axis,
            FeatureType::Joint => Icon::Joint,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            FeatureType::PressPull => "PressPull",
            FeatureType::RectPattern | FeatureType::CircPattern => "Pattern",
            FeatureType::Split => "Split",
            FeatureType::Move => "Move",
            FeatureType::AxisEdge | FeatureType::AxisFace => "Axis",
            other => other.title(),
        }
    }
}

/// Something picked in the viewport (also the model-mode selection).
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Face(FaceRef),
    Edge(EdgeRef),
    Body(String),
    Profile(ProfileRef),
    Plane(PlaneRef),
    Axis(AxisRef),
    Curve(FeatureId, EntityId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelKind {
    Face,
    FlatFace,
    Edge,
    /// A body, or a whole component when the body belongs to one.
    Body,
    /// A single body even inside a component (combine, split).
    SingleBody,
    Profile,
    Plane,
    Axis,
    Curve,
    /// A joint origin: a circular/straight edge or a point on a face.
    JointOrigin,
}

impl SelKind {
    fn accepts(self, p: &Pick) -> bool {
        matches!(
            (self, p),
            (SelKind::Face | SelKind::FlatFace, Pick::Face(_))
                | (SelKind::Edge, Pick::Edge(_))
                | (SelKind::Body | SelKind::SingleBody, Pick::Body(_))
                | (SelKind::Profile, Pick::Profile(_))
                | (SelKind::Plane, Pick::Plane(_))
                | (SelKind::Axis, Pick::Axis(_))
                | (SelKind::Curve, Pick::Curve(..))
                | (SelKind::JointOrigin, Pick::Edge(_) | Pick::Face(_))
        )
    }

    fn noun(self) -> &'static str {
        match self {
            SelKind::Face => "a face",
            SelKind::FlatFace => "a flat face",
            SelKind::Edge => "an edge",
            SelKind::Body => "a body or component",
            SelKind::SingleBody => "a body",
            SelKind::Profile => "a profile",
            SelKind::Plane => "a plane or flat face",
            SelKind::Axis => "an axis",
            SelKind::Curve => "sketch curves",
            SelKind::JointOrigin => "a round edge or a face point",
        }
    }
}

pub struct SelInput {
    pub label: &'static str,
    pub kind: SelKind,
    pub multi: bool,
    pub optional: bool,
    pub picks: Vec<Pick>,
}

pub struct ExprInput {
    pub label: &'static str,
    pub text: String,
    pub unit: &'static str,
}

pub struct FeatureSession {
    pub ty: FeatureType,
    pub editing: Option<FeatureId>,
    pub sels: Vec<SelInput>,
    pub active: usize,
    pub exprs: Vec<ExprInput>,
    pub flags: Vec<(&'static str, bool)>,
    /// Index into `Operation::ALL` (loft, sweep) or `CombineOp::ALL` (combine).
    pub op: usize,
}

fn sel(label: &'static str, kind: SelKind, multi: bool) -> SelInput {
    SelInput { label, kind, multi, optional: false, picks: Vec::new() }
}

fn expr(label: &'static str, text: &str, unit: &'static str) -> ExprInput {
    ExprInput { label, text: text.into(), unit }
}

impl FeatureSession {
    fn blank(ty: FeatureType) -> Self {
        use FeatureType as T;
        use SelKind as K;
        let (sels, exprs, flags): (Vec<SelInput>, Vec<ExprInput>, Vec<(&str, bool)>) = match ty {
            T::PressPull => (vec![sel("Face", K::FlatFace, false)], vec![expr("Distance", "5 mm", "mm")], vec![]),
            T::Shell => (vec![sel("Faces to remove", K::Face, true)], vec![expr("Thickness", "1 mm", "mm")], vec![]),
            T::Hole => (
                vec![sel("Position", K::FlatFace, false)],
                vec![
                    expr("Diameter", "5 mm", "mm"),
                    expr("Depth", "10 mm", "mm"),
                    expr("Head diameter", "9 mm", "mm"),
                    expr("Head depth", "3 mm", "mm"),
                    expr("Head angle", "90", "°"),
                ],
                vec![("Through all", false)],
            ),
            T::Mirror => (
                vec![sel("Bodies", K::Body, true), sel("Mirror plane", K::Plane, false)],
                vec![],
                vec![("Join to original", false)],
            ),
            T::RectPattern => {
                let mut d2 = sel("Direction 2", K::Axis, false);
                d2.optional = true;
                d2.picks.push(Pick::Axis(AxisRef::World(1)));
                let mut d1 = sel("Direction 1", K::Axis, false);
                d1.picks.push(Pick::Axis(AxisRef::World(0)));
                (
                    vec![sel("Bodies", K::Body, true), d1, d2],
                    vec![
                        expr("Count", "3", ""),
                        expr("Spacing", "20 mm", "mm"),
                        expr("Count 2", "1", ""),
                        expr("Spacing 2", "20 mm", "mm"),
                    ],
                    vec![("Join to original", false)],
                )
            }
            T::CircPattern => {
                let mut axis = sel("Axis", K::Axis, false);
                axis.picks.push(Pick::Axis(AxisRef::World(2)));
                (
                    vec![sel("Bodies", K::Body, true), axis],
                    vec![expr("Count", "6", ""), expr("Total angle", "360", "°")],
                    vec![("Join to original", false)],
                )
            }
            T::Combine => (
                vec![sel("Target body", K::SingleBody, false), sel("Tool bodies", K::SingleBody, true)],
                vec![],
                vec![("Keep tools", false)],
            ),
            T::Split => (vec![sel("Body", K::SingleBody, false), sel("Splitting plane", K::Plane, false)], vec![], vec![]),
            T::Move => {
                let mut axis = sel("Rotation axis", K::Axis, false);
                axis.optional = true;
                (
                    vec![sel("Bodies", K::Body, true), axis],
                    vec![
                        expr("X distance", "0 mm", "mm"),
                        expr("Y distance", "0 mm", "mm"),
                        expr("Z distance", "0 mm", "mm"),
                        expr("Angle", "0", "°"),
                    ],
                    vec![("Create copy", false)],
                )
            }
            T::Loft => (vec![sel("Profiles (in order)", K::Profile, true)], vec![], vec![]),
            T::Sweep => (vec![sel("Profile", K::Profile, false), sel("Path", K::Curve, true)], vec![], vec![]),
            T::Plane => (
                vec![sel("Reference", K::Plane, false)],
                vec![expr("Offset", "10 mm", "mm"), expr("Angle", "0", "°")],
                vec![],
            ),
            T::AxisEdge => (vec![sel("Edge", K::Edge, false)], vec![], vec![]),
            T::AxisFace => (vec![sel("Face", K::Face, false)], vec![], vec![]),
            T::Joint => (
                vec![
                    sel("Moving bodies", K::Body, true),
                    sel("Origin on moving part", K::JointOrigin, false),
                    sel("Origin on fixed part", K::JointOrigin, false),
                ],
                vec![expr("Angle", "0", "°"), expr("Offset", "0 mm", "mm"), expr("Minimum", "0", ""), expr("Maximum", "90", "")],
                vec![("Flip", false), ("Limits", false)],
            ),
        };
        let mut s = FeatureSession { ty, editing: None, sels, active: 0, exprs, flags, op: 0 };
        s.active = s.first_empty();
        s
    }

    /// A new command, pre-filled from the model selection.
    pub fn new(ty: FeatureType, selection: &[Pick]) -> Self {
        let mut s = Self::blank(ty);
        for p in selection {
            let p = match (p, s.sels.iter().any(|i| i.kind == SelKind::Plane)) {
                (Pick::Face(f), true) if !s.sels.iter().any(|i| matches!(i.kind, SelKind::Face | SelKind::FlatFace)) => {
                    Pick::Plane(PlaneRef::Face(*f))
                }
                _ => p.clone(),
            };
            if let Some(input) = s.sels.iter_mut().find(|i| i.kind.accepts(&p) && (i.multi || i.picks.is_empty())) {
                input.picks.push(p);
            }
        }
        s.active = s.first_empty();
        s
    }

    fn first_empty(&self) -> usize {
        self.sels.iter().position(|i| i.picks.is_empty() && !i.optional).unwrap_or(0)
    }

    pub fn active_kind(&self) -> Option<SelKind> {
        self.sels.get(self.active).map(|i| i.kind)
    }

    /// Add (or toggle off) a pick for the active input.
    pub fn pick(&mut self, p: Pick) {
        let Some(input) = self.sels.get_mut(self.active) else { return };
        if input.multi {
            if let Some(i) = input.picks.iter().position(|q| *q == p) {
                input.picks.remove(i);
            } else {
                input.picks.push(p);
            }
        } else {
            input.picks = vec![p];
            // Like Fusion: move on to the next selection still needed.
            let next = self.first_empty();
            if self.sels[next].picks.is_empty() {
                self.active = next;
            }
        }
    }

    fn one(&self, i: usize) -> Result<&Pick, String> {
        self.sels[i].picks.first().ok_or_else(|| format!("select {}", self.sels[i].kind.noun()))
    }

    fn bodies(&self, i: usize) -> Result<Vec<String>, String> {
        let names: Vec<String> = self.sels[i]
            .picks
            .iter()
            .filter_map(|p| if let Pick::Body(n) = p { Some(n.clone()) } else { None })
            .collect();
        if names.is_empty() {
            return Err("select at least one body".into());
        }
        Ok(names)
    }

    fn face(&self, i: usize) -> Result<FaceRef, String> {
        match self.one(i)? {
            Pick::Face(f) => Ok(*f),
            _ => Err("select a face".into()),
        }
    }

    fn plane(&self, i: usize) -> Result<PlaneRef, String> {
        match self.one(i)? {
            Pick::Plane(p) => Ok(*p),
            Pick::Face(f) => Ok(PlaneRef::Face(*f)),
            _ => Err("select a plane".into()),
        }
    }

    fn axis(&self, i: usize) -> Result<AxisRef, String> {
        match self.one(i)? {
            Pick::Axis(a) => Ok(*a),
            _ => Err("select an axis".into()),
        }
    }

    fn profile(&self, i: usize) -> Result<ProfileRef, String> {
        match self.one(i)? {
            Pick::Profile(p) => Ok(*p),
            _ => Err("select a profile".into()),
        }
    }

    fn e(&self, i: usize) -> String {
        self.exprs[i].text.clone()
    }

    fn operation(&self) -> Operation {
        Operation::ALL[self.op.min(Operation::ALL.len() - 1)]
    }

    /// The feature described by the dialog, or what is still missing.
    pub fn build(&self) -> Result<FeatureKind, String> {
        use FeatureType as T;
        Ok(match self.ty {
            T::PressPull => FeatureKind::PressPull(PressPullFeature { face: self.face(0)?, distance: self.e(0) }),
            T::Shell => {
                let faces: Vec<FaceRef> =
                    self.sels[0].picks.iter().filter_map(|p| if let Pick::Face(f) = p { Some(*f) } else { None }).collect();
                if faces.is_empty() {
                    return Err("select the faces to remove".into());
                }
                FeatureKind::Shell(ShellFeature { faces, thickness: self.e(0) })
            }
            T::Hole => FeatureKind::Hole(HoleFeature {
                face: self.face(0)?,
                diameter: self.e(0),
                depth: self.e(1),
                through: self.flags[0].1,
                head: match self.op {
                    1 => HoleHead::Counterbore { diameter: self.e(2), depth: self.e(3) },
                    2 => HoleHead::Countersink { diameter: self.e(2), angle: self.e(4) },
                    _ => HoleHead::None,
                },
            }),
            T::Mirror => FeatureKind::Mirror(MirrorFeature {
                bodies: self.bodies(0)?,
                plane: self.plane(1)?,
                join: self.flags[0].1,
            }),
            T::RectPattern => FeatureKind::Pattern(PatternFeature {
                bodies: self.bodies(0)?,
                kind: PatternKind::Rectangular {
                    direction: self.axis(1)?,
                    count: self.e(0),
                    spacing: self.e(1),
                    direction2: self.axis(2).unwrap_or(AxisRef::World(1)),
                    count2: if self.sels[2].picks.is_empty() { "1".into() } else { self.e(2) },
                    spacing2: self.e(3),
                },
                join: self.flags[0].1,
            }),
            T::CircPattern => FeatureKind::Pattern(PatternFeature {
                bodies: self.bodies(0)?,
                kind: PatternKind::Circular { axis: self.axis(1)?, count: self.e(0), angle: self.e(1) },
                join: self.flags[0].1,
            }),
            T::Combine => {
                let target = match self.one(0)? {
                    Pick::Body(n) => n.clone(),
                    _ => return Err("select the target body".into()),
                };
                let tools = self.bodies(1)?;
                FeatureKind::Combine(CombineFeature {
                    target,
                    tools,
                    op: CombineOp::ALL[self.op.min(2)],
                    keep_tools: self.flags[0].1,
                })
            }
            T::Split => FeatureKind::Split(SplitFeature {
                body: match self.one(0)? {
                    Pick::Body(n) => n.clone(),
                    _ => return Err("select a body".into()),
                },
                plane: self.plane(1)?,
            }),
            T::Move => FeatureKind::Move(MoveFeature {
                bodies: self.bodies(0)?,
                translate: [self.e(0), self.e(1), self.e(2)],
                axis: self.axis(1).ok(),
                angle: self.e(3),
                copy: self.flags[0].1,
            }),
            T::Loft => {
                let profiles: Vec<ProfileRef> =
                    self.sels[0].picks.iter().filter_map(|p| if let Pick::Profile(r) = p { Some(*r) } else { None }).collect();
                if profiles.len() < 2 {
                    return Err("select at least two profiles".into());
                }
                FeatureKind::Loft(LoftFeature { profiles, operation: self.operation() })
            }
            T::Sweep => {
                let profile = self.profile(0)?;
                let curves: Vec<(FeatureId, EntityId)> =
                    self.sels[1].picks.iter().filter_map(|p| if let Pick::Curve(s, e) = p { Some((*s, *e)) } else { None }).collect();
                let Some(&(path_sketch, _)) = curves.first() else { return Err("select the path curves".into()) };
                if curves.iter().any(|(s, _)| *s != path_sketch) {
                    return Err("all path curves must be in one sketch".into());
                }
                FeatureKind::Sweep(SweepFeature {
                    profile,
                    path_sketch,
                    path: curves.into_iter().map(|(_, e)| e).collect(),
                    operation: self.operation(),
                })
            }
            T::Plane => FeatureKind::Plane(PlaneFeature { base: self.plane(0)?, offset: self.e(0), angle: self.e(1) }),
            T::AxisEdge => FeatureKind::Axis(AxisFeature {
                source: match self.one(0)? {
                    Pick::Edge(e) => AxisSource::Edge(*e),
                    _ => return Err("select an edge".into()),
                },
            }),
            T::AxisFace => FeatureKind::Axis(AxisFeature { source: AxisSource::FaceNormal(self.face(0)?) }),
            T::Joint => {
                let origin = |i: usize| -> Result<JointOrigin, String> {
                    match self.one(i)? {
                        Pick::Edge(e) => Ok(JointOrigin::Edge(*e)),
                        Pick::Face(f) => Ok(JointOrigin::Face(*f)),
                        _ => Err("pick a round edge or a face".into()),
                    }
                };
                FeatureKind::Joint(JointFeature {
                    moving: self.bodies(0)?,
                    from: origin(1)?,
                    to: origin(2)?,
                    kind: JointKind::ALL[self.op.min(2)],
                    angle: self.e(0),
                    offset: self.e(1),
                    flip: self.flags[0].1,
                    limits: self.flags[1].1.then(|| (self.e(2), self.e(3))),
                })
            }
        })
    }

    /// A dialog for editing an existing feature (None for kinds with their
    /// own dialog).
    pub fn edit(id: FeatureId, kind: &FeatureKind) -> Option<Self> {
        use FeatureType as T;
        let (ty, picks, exprs, flags, op): (T, Vec<Vec<Pick>>, Vec<String>, Vec<bool>, usize) = match kind {
            FeatureKind::PressPull(p) => (T::PressPull, vec![vec![Pick::Face(p.face)]], vec![p.distance.clone()], vec![], 0),
            FeatureKind::Shell(s) => {
                (T::Shell, vec![s.faces.iter().map(|f| Pick::Face(*f)).collect()], vec![s.thickness.clone()], vec![], 0)
            }
            FeatureKind::Hole(h) => {
                let (op, hd, hdepth, angle) = match &h.head {
                    HoleHead::None => (0, "9 mm".to_string(), "3 mm".to_string(), "90".to_string()),
                    HoleHead::Counterbore { diameter, depth } => (1, diameter.clone(), depth.clone(), "90".into()),
                    HoleHead::Countersink { diameter, angle } => (2, diameter.clone(), "3 mm".into(), angle.clone()),
                };
                (T::Hole, vec![vec![Pick::Face(h.face)]], vec![h.diameter.clone(), h.depth.clone(), hd, hdepth, angle], vec![h.through], op)
            }
            FeatureKind::Mirror(m) => (
                T::Mirror,
                vec![m.bodies.iter().cloned().map(Pick::Body).collect(), vec![Pick::Plane(m.plane)]],
                vec![],
                vec![m.join],
                0,
            ),
            FeatureKind::Pattern(p) => match &p.kind {
                PatternKind::Rectangular { direction, count, spacing, direction2, count2, spacing2 } => (
                    T::RectPattern,
                    vec![
                        p.bodies.iter().cloned().map(Pick::Body).collect(),
                        vec![Pick::Axis(*direction)],
                        vec![Pick::Axis(*direction2)],
                    ],
                    vec![count.clone(), spacing.clone(), count2.clone(), spacing2.clone()],
                    vec![p.join],
                    0,
                ),
                PatternKind::Circular { axis, count, angle } => (
                    T::CircPattern,
                    vec![p.bodies.iter().cloned().map(Pick::Body).collect(), vec![Pick::Axis(*axis)]],
                    vec![count.clone(), angle.clone()],
                    vec![p.join],
                    0,
                ),
            },
            FeatureKind::Combine(c) => (
                T::Combine,
                vec![vec![Pick::Body(c.target.clone())], c.tools.iter().cloned().map(Pick::Body).collect()],
                vec![],
                vec![c.keep_tools],
                CombineOp::ALL.iter().position(|o| *o == c.op).unwrap_or(0),
            ),
            FeatureKind::Split(s) => {
                (T::Split, vec![vec![Pick::Body(s.body.clone())], vec![Pick::Plane(s.plane)]], vec![], vec![], 0)
            }
            FeatureKind::Move(m) => (
                T::Move,
                vec![m.bodies.iter().cloned().map(Pick::Body).collect(), m.axis.map(Pick::Axis).into_iter().collect()],
                vec![m.translate[0].clone(), m.translate[1].clone(), m.translate[2].clone(), m.angle.clone()],
                vec![m.copy],
                0,
            ),
            FeatureKind::Loft(l) => (
                T::Loft,
                vec![l.profiles.iter().map(|p| Pick::Profile(*p)).collect()],
                vec![],
                vec![],
                Operation::ALL.iter().position(|o| *o == l.operation).unwrap_or(0),
            ),
            FeatureKind::Sweep(s) => (
                T::Sweep,
                vec![vec![Pick::Profile(s.profile)], s.path.iter().map(|e| Pick::Curve(s.path_sketch, *e)).collect()],
                vec![],
                vec![],
                Operation::ALL.iter().position(|o| *o == s.operation).unwrap_or(0),
            ),
            FeatureKind::Plane(p) => (T::Plane, vec![vec![Pick::Plane(p.base)]], vec![p.offset.clone(), p.angle.clone()], vec![], 0),
            FeatureKind::Axis(a) => match a.source {
                AxisSource::Edge(e) => (T::AxisEdge, vec![vec![Pick::Edge(e)]], vec![], vec![], 0),
                AxisSource::FaceNormal(f) => (T::AxisFace, vec![vec![Pick::Face(f)]], vec![], vec![], 0),
            },
            FeatureKind::Joint(j) => {
                let origin = |o: &JointOrigin| match o {
                    JointOrigin::Edge(e) => Pick::Edge(*e),
                    JointOrigin::Face(f) => Pick::Face(*f),
                };
                (
                    T::Joint,
                    vec![j.moving.iter().cloned().map(Pick::Body).collect(), vec![origin(&j.from)], vec![origin(&j.to)]],
                    {
                        let (lo, hi) = j.limits.clone().unwrap_or(("0".into(), "90".into()));
                        vec![j.angle.clone(), j.offset.clone(), lo, hi]
                    },
                    vec![j.flip, j.limits.is_some()],
                    JointKind::ALL.iter().position(|k| *k == j.kind).unwrap_or(0),
                )
            }
            _ => return None,
        };
        let mut s = Self::blank(ty);
        s.editing = Some(id);
        for (input, p) in s.sels.iter_mut().zip(picks) {
            input.picks = p;
        }
        for (input, t) in s.exprs.iter_mut().zip(exprs) {
            input.text = t;
        }
        for (flag, v) in s.flags.iter_mut().zip(flags) {
            flag.1 = v;
        }
        s.op = op;
        s.active = s.first_empty();
        Some(s)
    }
}

/// Size of drawn construction planes.
fn plane_size(cam_distance: f64) -> f64 {
    cam_distance * 0.35
}

impl FerroApp {
    /// Screen polygon of a plane: origin planes as a square in their positive
    /// quadrant, others centred on their origin.
    pub fn plane_quad(&self, plane: &Plane, centred: bool, vp: &Viewport) -> Option<Vec<Pos2>> {
        let s = plane_size(self.camera.distance);
        let (a, b) = if centred { (-s / 2.0, s / 2.0) } else { (0.0, s) };
        let pts: Vec<Pos2> =
            [[a, a], [b, a], [b, b], [a, b]].iter().filter_map(|uv| self.to_screen(plane.to_world(*uv), vp)).collect();
        (pts.len() == 4).then_some(pts)
    }

    /// World corners of a drawn plane (see `plane_quad`).
    pub fn plane_corners(&self, plane: &Plane, centred: bool) -> [Vec3; 4] {
        let s = plane_size(self.camera.distance);
        let (a, b) = if centred { (-s / 2.0, s / 2.0) } else { (0.0, s) };
        [[a, a], [b, a], [b, b], [a, b]].map(|uv| plane.to_world(uv))
    }

    /// A translucent plane: depth-tested fill on the GPU (painter fill
    /// otherwise) plus an outline.
    pub fn draw_plane(&self, painter: &egui::Painter, vp: &Viewport, plane: &Plane, centred: bool, fill: Color32, stroke: Stroke) {
        let Some(q) = self.plane_quad(plane, centred, vp) else { return };
        if self.gpu {
            let c = self.plane_corners(plane, centred);
            crate::render3d::push_triangle(&self.overlay, [c[0], c[1], c[2]], fill);
            crate::render3d::push_triangle(&self.overlay, [c[0], c[2], c[3]], fill);
            painter.add(egui::Shape::closed_line(q, stroke));
        } else {
            painter.add(egui::Shape::convex_polygon(q, fill, stroke));
        }
    }

    /// Construction planes and axes of the current model.
    pub fn construction_planes(&self) -> Vec<(FeatureId, Plane)> {
        let mut v: Vec<(FeatureId, Plane)> = self.model.planes.iter().map(|(k, p)| (*k, *p)).collect();
        v.sort_by_key(|(k, _)| *k);
        v.retain(|(k, _)| !self.hidden_construction.contains(k));
        v
    }

    pub fn construction_axes(&self) -> Vec<(FeatureId, (Vec3, Vec3))> {
        let mut v: Vec<(FeatureId, (Vec3, Vec3))> = self.model.axes.iter().map(|(k, a)| (*k, *a)).collect();
        v.sort_by_key(|(k, _)| *k);
        v.retain(|(k, _)| !self.hidden_construction.contains(k));
        v
    }

    /// Resolve a plane reference for display.
    pub fn plane_of(&self, p: &PlaneRef) -> Option<(Plane, bool)> {
        match p {
            PlaneRef::Origin(o) => Some((o.plane(), false)),
            PlaneRef::Construction(id) => self.model.planes.get(id).map(|p| (*p, true)),
            PlaneRef::Face(f) => Some((Plane::at_face(f.point, f.normal), true)),
        }
    }

    pub fn axis_of(&self, a: &AxisRef) -> Option<(Vec3, Vec3)> {
        match a {
            AxisRef::World(i) => {
                let mut d = [0.0; 3];
                d[(*i).min(2)] = 1.0;
                Some(([0.0; 3], d))
            }
            AxisRef::Construction(id) => self.model.axes.get(id).copied(),
        }
    }

    /// The pick of `kind` under the cursor.
    pub fn hover_pick(&self, kind: SelKind, pos: Pos2, vp: &Viewport) -> Option<Pick> {
        match kind {
            SelKind::Face => self.pick_face(pos, vp).map(|f| Pick::Face(f.face)),
            SelKind::FlatFace => self.pick_face(pos, vp).filter(|f| f.planar).map(|f| Pick::Face(f.face)),
            SelKind::Edge => self.pick_edge(pos, vp).map(|(e, _)| Pick::Edge(e)),
            SelKind::JointOrigin => self
                .pick_edge(pos, vp)
                .map(|(e, _)| Pick::Edge(e))
                .or_else(|| self.pick_face(pos, vp).map(|f| Pick::Face(f.face))),
            SelKind::Body => self.pick_body(pos, vp).map(|b| {
                let name = &self.bodies[b].name;
                Pick::Body(self.model.component_of(name).unwrap_or(name).to_string())
            }),
            SelKind::SingleBody => self.pick_body(pos, vp).map(|b| Pick::Body(self.bodies[b].name.clone())),
            SelKind::Profile => {
                let sketches = self.visible_sketches();
                self.pick_profile(pos, vp, &sketches).map(Pick::Profile)
            }
            SelKind::Curve => {
                let sketches = self.visible_sketches();
                self.pick_curve(pos, vp, &sketches).map(|(s, e)| Pick::Curve(s, e))
            }
            SelKind::Axis => {
                let mut candidates: Vec<(AxisRef, Vec3, Vec3)> =
                    (0..3).filter_map(|i| self.axis_of(&AxisRef::World(i)).map(|(o, d)| (AxisRef::World(i), o, d))).collect();
                for (id, (o, d)) in self.construction_axes() {
                    candidates.push((AxisRef::Construction(id), o, d));
                }
                candidates
                    .into_iter()
                    .map(|(a, o, d)| (self.line_screen_distance(pos, vp, o, d), a))
                    .filter(|(d, _)| *d < crate::picking::PICK_PX)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, a)| Pick::Axis(a))
            }
            SelKind::Plane => {
                let (o, d) = self.camera.ray([pos.x as f64, pos.y as f64], vp);
                let d = normalize(d);
                let s = plane_size(self.camera.distance);
                let mut best: Option<(f64, Pick)> = None;
                let mut consider = |plane: Plane, centred: bool, pick: Pick| {
                    let Some(hit) = plane.intersect_ray(o, d) else { return };
                    let t = dot(sub(hit, o), d);
                    let uv = plane.to_local(hit);
                    let (a, b) = if centred { (-s / 2.0, s / 2.0) } else { (0.0, s) };
                    let inside = (a..=b).contains(&uv[0]) && (a..=b).contains(&uv[1]);
                    if t > 0.0 && inside && best.as_ref().is_none_or(|(bt, _)| t < *bt) {
                        best = Some((t, pick));
                    }
                };
                for o in OriginPlane::ALL {
                    consider(o.plane(), false, Pick::Plane(PlaneRef::Origin(o)));
                }
                for (id, p) in self.construction_planes() {
                    consider(p, true, Pick::Plane(PlaneRef::Construction(id)));
                }
                // Planes are see-through: a flat body face under the cursor wins.
                if let Some(f) = self.pick_face(pos, vp).filter(|f| f.planar) {
                    return Some(Pick::Plane(PlaneRef::Face(f.face)));
                }
                best.map(|(_, p)| p)
            }
        }
    }

    /// Highlight a pick in the viewport.
    pub fn draw_pick(&self, painter: &egui::Painter, vp: &Viewport, pick: &Pick, color: Color32) {
        let line = |pts: Vec<Pos2>, w: f32| {
            painter.add(egui::Shape::line(pts, Stroke::new(w, color)));
        };
        match pick {
            Pick::Face(f) | Pick::Plane(PlaneRef::Face(f)) => {
                if let Some((bi, tris)) = self.face_triangles(f) {
                    let mesh = &self.bodies[bi].mesh;
                    if self.gpu {
                        for tri in tris {
                            let p = [0, 1, 2].map(|k| mesh.positions[mesh.indices[tri * 3 + k] as usize]);
                            crate::render3d::push_triangle(&self.overlay, p, color.gamma_multiply(0.55));
                        }
                        return;
                    }
                    let mut overlay = egui::Mesh::default();
                    for tri in tris {
                        let pts: Vec<Pos2> = mesh.indices[tri * 3..tri * 3 + 3]
                            .iter()
                            .filter_map(|&i| self.to_screen(mesh.positions[i as usize], vp))
                            .collect();
                        if pts.len() == 3 {
                            let base = overlay.vertices.len() as u32;
                            for p in pts {
                                overlay.colored_vertex(p, color.gamma_multiply(0.5));
                            }
                            overlay.add_triangle(base, base + 1, base + 2);
                        }
                    }
                    painter.add(overlay);
                }
            }
            Pick::Edge(e) => {
                if let Some(poly) = self.edge_polyline(e) {
                    line(poly.iter().filter_map(|p| self.to_screen(*p, vp)).collect(), 3.5);
                }
            }
            Pick::Body(name) => {
                // A component name highlights all its bodies.
                let members: Vec<String> = match self.model.components.iter().find(|(c, _)| c == name) {
                    Some((_, list)) => list.clone(),
                    None => vec![name.clone()],
                };
                for b in self.bodies.iter().filter(|b| members.contains(&b.name)) {
                    for e in &b.edges {
                        line(e.iter().filter_map(|p| self.to_screen(*p, vp)).collect(), 2.5);
                    }
                }
            }
            Pick::Profile(p) => {
                let Some(sf) = self.doc.sketch(p.sketch) else { return };
                let profiles = find_profiles(&sf.sketch);
                if let Some(i) = profile_at(&profiles, p.point) {
                    for lp in std::iter::once(&profiles[i].outer).chain(&profiles[i].holes) {
                        let mut pts: Vec<Pos2> =
                            lp.polygon().iter().filter_map(|uv| self.to_screen(sf.plane.to_world(*uv), vp)).collect();
                        if let Some(f) = pts.first().copied() {
                            pts.push(f);
                        }
                        line(pts, 3.0);
                    }
                }
            }
            Pick::Plane(pr) => {
                if let Some((plane, centred)) = self.plane_of(pr) {
                    self.draw_plane(painter, vp, &plane, centred, color.gamma_multiply(0.35), Stroke::new(2.0, color));
                }
            }
            Pick::Axis(a) => {
                if let Some((o, d)) = self.axis_of(a) {
                    let reach = self.camera.distance * 2.0;
                    let p = self.to_screen(add(o, scale(normalize(d), -reach)), vp);
                    let q = self.to_screen(add(o, scale(normalize(d), reach)), vp);
                    if let (Some(p), Some(q)) = (p, q) {
                        line(vec![p, q], 3.0);
                    }
                }
            }
            Pick::Curve(s, e) => {
                if let Some(sf) = self.doc.sketch(*s) {
                    if sf.sketch.is_alive(*e) {
                        let pts = entity_polyline(&sf.sketch, sf.sketch.entity(*e));
                        line(pts.iter().filter_map(|uv| self.to_screen(sf.plane.to_world(*uv), vp)).collect(), 3.5);
                    }
                }
            }
        }
    }

    /// Construction geometry that a selection kind can pick from.
    fn draw_candidates(&self, painter: &egui::Painter, vp: &Viewport, kind: SelKind) {
        match kind {
            SelKind::Plane => {
                for o in OriginPlane::ALL {
                    let fill = theme::ORANGE.gamma_multiply(0.12);
                    self.draw_plane(painter, vp, &o.plane(), false, fill, Stroke::new(1.0, theme::ORANGE));
                }
            }
            SelKind::Axis => {
                for i in 0..3 {
                    self.draw_pick(painter, vp, &Pick::Axis(AxisRef::World(i)), theme::ORANGE.gamma_multiply(0.5));
                }
            }
            SelKind::Profile => {
                for id in self.visible_sketches() {
                    let Some(sf) = self.doc.sketch(id) else { continue };
                    for p in find_profiles(&sf.sketch) {
                        let mut pts: Vec<Pos2> =
                            p.outer.polygon().iter().filter_map(|uv| self.to_screen(sf.plane.to_world(*uv), vp)).collect();
                        if let Some(f) = pts.first().copied() {
                            pts.push(f);
                        }
                        painter.add(egui::Shape::line(pts, Stroke::new(1.0, theme::ACCENT)));
                    }
                }
            }
            SelKind::Curve => {
                for id in self.visible_sketches() {
                    let Some(sf) = self.doc.sketch(id) else { continue };
                    for (_, e) in sf.sketch.entities() {
                        let pts: Vec<Pos2> = entity_polyline(&sf.sketch, e)
                            .iter()
                            .filter_map(|uv| self.to_screen(sf.plane.to_world(*uv), vp))
                            .collect();
                        painter.add(egui::Shape::line(pts, Stroke::new(1.2, theme::ACCENT.gamma_multiply(0.7))));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn feature_viewport(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        let Mode::Feature(sess) = &self.mode else { return };
        let Some(kind) = sess.active_kind() else { return };
        self.draw_candidates(painter, vp, kind);
        // Selected picks of every input.
        for (i, input) in sess.sels.iter().enumerate() {
            let color = if i == sess.active { crate::sketch_ui::COL_SELECTED } else { theme::ACCENT };
            for p in &input.picks {
                self.draw_pick(painter, vp, p, color);
            }
        }
        let hovered = response.hover_pos().and_then(|pos| self.hover_pick(kind, pos, vp));
        if let Some(p) = &hovered {
            self.draw_pick(painter, vp, p, theme::ACCENT);
        } else if let (Some(pos), SelKind::FlatFace) = (response.hover_pos(), kind) {
            if self.pick_face(pos, vp).is_some_and(|f| !f.planar) {
                painter.text(
                    pos + egui::vec2(14.0, 14.0),
                    egui::Align2::LEFT_TOP,
                    "needs a flat face",
                    egui::FontId::proportional(12.0),
                    theme::ERROR,
                );
            }
        }
        if response.clicked() {
            if let (Some(p), Mode::Feature(sess)) = (hovered, &mut self.mode) {
                sess.pick(p);
            }
        }
    }

    pub fn feature_window(&mut self, ctx: &egui::Context) {
        let params = self.doc.params.clone();
        let preview_error = self.preview.as_ref().and_then(|p| p.error.clone());
        let viewport = self.viewport_rect;
        let Mode::Feature(sess) = &mut self.mode else { return };
        let built = sess.build();
        let exprs_ok = sess.exprs.iter().all(|e| params.eval(&e.text).is_ok());
        let ready = built.is_ok() && exprs_ok;
        let title = sess.ty.title();
        let icon = sess.ty.icon();

        let action = command_dialog(ctx, viewport, title, icon, ready, |ui| {
            let active = sess.active;
            let mut activate = None;
            for (i, input) in sess.sels.iter_mut().enumerate() {
                form_label(ui, input.label);
                ui.horizontal(|ui| {
                    let n = input.picks.len();
                    let text = match (n, input.multi) {
                        (0, _) if input.optional => "optional".to_string(),
                        (0, _) => "Select".to_string(),
                        (1, false) => pick_label(&input.picks[0]),
                        (n, _) => format!("{n} selected"),
                    };
                    let (fill, color) = if i == active {
                        (theme::ACCENT_DIM, Color32::WHITE)
                    } else {
                        (theme::WINDOW, theme::TEXT)
                    };
                    let chip = egui::Button::new(RichText::new(text).color(color))
                        .fill(fill)
                        .stroke(Stroke::new(1.0, if i == active { theme::ACCENT } else { theme::BORDER }))
                        .min_size(egui::vec2(120.0, 22.0));
                    if ui.add(chip).on_hover_text(format!("Click, then pick {} in the viewport", input.kind.noun())).clicked() {
                        activate = Some(i);
                    }
                    if n > 0 && icons::icon_button(ui, Icon::Cancel, 18.0, "Clear", true, false).clicked() {
                        input.picks.clear();
                        activate = Some(i);
                    }
                });
                ui.end_row();
            }
            if let Some(i) = activate {
                sess.active = i;
            }
            if sess.ty == FeatureType::Hole {
                form_label(ui, "Type");
                ui.horizontal(|ui| {
                    for (i, name) in ["Simple", "Counterbore", "Countersink"].iter().enumerate() {
                        ui.selectable_value(&mut sess.op, i, *name);
                    }
                });
                ui.end_row();
            }
            let (ty, op) = (sess.ty, sess.op);
            let limits_on = sess.ty == FeatureType::Joint && sess.flags.get(1).is_some_and(|f| f.1);
            for (i, e) in sess.exprs.iter_mut().enumerate() {
                // Hole head values only for the chosen head type.
                let shown = match ty {
                    FeatureType::Hole => i < 2 || (op == 1 && i < 4) || (op == 2 && (i == 2 || i == 4)),
                    // Joint limits only when switched on.
                    FeatureType::Joint => i < 2 || limits_on,
                    _ => true,
                };
                if !shown {
                    continue;
                }
                form_label(ui, e.label);
                expression_field(ui, &params, &mut e.text, e.unit);
                ui.end_row();
            }
            for (label, value) in sess.flags.iter_mut() {
                form_label(ui, "");
                ui.checkbox(value, *label);
                ui.end_row();
            }
            match sess.ty {
                FeatureType::Joint => {
                    form_label(ui, "Type");
                    ui.horizontal(|ui| {
                        for (i, k) in JointKind::ALL.iter().enumerate() {
                            ui.selectable_value(&mut sess.op, i, k.label());
                        }
                    });
                    ui.end_row();
                }
                FeatureType::Loft | FeatureType::Sweep => {
                    form_label(ui, "Operation");
                    egui::ComboBox::from_id_salt("feature_op")
                        .selected_text(Operation::ALL[sess.op].label())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (i, o) in Operation::ALL.iter().enumerate() {
                                ui.selectable_value(&mut sess.op, i, o.label());
                            }
                        });
                    ui.end_row();
                }
                FeatureType::Combine => {
                    form_label(ui, "Operation");
                    egui::ComboBox::from_id_salt("combine_op")
                        .selected_text(CombineOp::ALL[sess.op.min(2)].label())
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (i, o) in CombineOp::ALL.iter().enumerate() {
                                ui.selectable_value(&mut sess.op, i, o.label());
                            }
                        });
                    ui.end_row();
                }
                _ => {}
            }
            let problem = match &built {
                Err(e) => Some(e.clone()),
                Ok(_) => preview_error.clone(),
            };
            if let Some(p) = problem {
                form_label(ui, "");
                let color = if built.is_err() { theme::TEXT_WEAK } else { theme::ERROR };
                ui.colored_label(color, p);
                ui.end_row();
            }
        });

        let ty = sess.ty;
        let editing = sess.editing;
        match action {
            DialogAction::Ok => {
                let Ok(kind) = built else { return };
                self.mode = Mode::Model;
                self.checkpoint(ty.title());
                self.hidden_sketches.extend(kind.consumed_sketches());
                crate::dialogs::apply_feature(&mut self.doc, editing, ty.prefix(), kind);
                self.message = None;
                self.rebuild();
            }
            DialogAction::Cancel => self.cancel(),
            DialogAction::None => match built.ok().filter(|_| exprs_ok) {
                Some(kind) => {
                    let key = format!("{editing:?} {kind:?}");
                    self.update_preview(key, |doc| crate::dialogs::apply_feature(doc, editing, ty.prefix(), kind));
                }
                None => self.preview = None,
            },
        }
    }
}

/// Short text for a single pick.
pub fn pick_label(p: &Pick) -> String {
    match p {
        Pick::Face(_) => "1 face".into(),
        Pick::Edge(_) => "1 edge".into(),
        Pick::Body(n) => n.clone(),
        Pick::Profile(_) => "1 profile".into(),
        Pick::Plane(PlaneRef::Origin(o)) => format!("{} plane", o.label()),
        Pick::Plane(PlaneRef::Face(_)) => "face plane".into(),
        Pick::Plane(PlaneRef::Construction(id)) => format!("plane #{}", id.0),
        Pick::Axis(a) => a.label(),
        Pick::Curve(..) => "1 curve".into(),
    }
}
