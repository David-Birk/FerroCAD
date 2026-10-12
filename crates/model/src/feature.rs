//! Timeline features.

use crate::geom::Plane;
use crate::kernel::{EdgeRef, FaceRef};
use ferrocad_sketch::{ConstraintId, EntityId, PointId, Sketch};

/// Stable id: survives reordering and deleting other features.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FeatureId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    NewBody,
    /// Union with the most recent body.
    Join,
    /// Subtract from every body.
    Cut,
    /// Keep only the overlap with the most recent body.
    Intersect,
}

impl Operation {
    pub const ALL: [Operation; 4] =
        [Operation::NewBody, Operation::Join, Operation::Cut, Operation::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            Operation::NewBody => "New body",
            Operation::Join => "Join",
            Operation::Cut => "Cut",
            Operation::Intersect => "Intersect",
        }
    }
}

/// The three planes through the origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginPlane {
    XY,
    XZ,
    YZ,
}

impl OriginPlane {
    pub const ALL: [OriginPlane; 3] = [OriginPlane::XY, OriginPlane::XZ, OriginPlane::YZ];

    pub fn plane(self) -> Plane {
        match self {
            OriginPlane::XY => Plane::XY,
            OriginPlane::XZ => Plane::XZ,
            OriginPlane::YZ => Plane::YZ,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OriginPlane::XY => "XY",
            OriginPlane::XZ => "XZ",
            OriginPlane::YZ => "YZ",
        }
    }
}

/// A plane something is placed on, resolved on every rebuild.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaneRef {
    Origin(OriginPlane),
    /// A flat face of a body (found again by point and normal).
    Face(FaceRef),
    /// A construction plane feature.
    Construction(FeatureId),
}

/// An axis: a world axis through the origin or a construction axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisRef {
    /// 0 = X, 1 = Y, 2 = Z.
    World(usize),
    Construction(FeatureId),
}

impl AxisRef {
    pub fn label(self) -> String {
        match self {
            AxisRef::World(i) => format!("{} axis", ["X", "Y", "Z"][i.min(2)]),
            AxisRef::Construction(id) => format!("axis #{}", id.0),
        }
    }
}

/// Sketch geometry linked to a body edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    pub edge: EdgeRef,
    pub entity: EntityId,
}

/// A sketch profile: the region of `sketch` containing `point`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileRef {
    pub sketch: FeatureId,
    pub point: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct SketchFeature {
    /// Where the sketch lies. Updated from `attach` on every rebuild.
    pub plane: Plane,
    /// What the sketch is attached to; `None` keeps `plane` fixed.
    pub attach: Option<PlaneRef>,
    pub sketch: Sketch,
    /// Projected body edges that follow the model (see `project.rs`).
    pub projections: Vec<Projection>,
    /// Dimensions driven by parameter expressions, e.g. `(id, "width / 2")`.
    /// Angle expressions are in degrees.
    pub driven: Vec<(ConstraintId, String)>,
}

impl SketchFeature {
    pub fn new(plane: Plane) -> Self {
        Self { plane, attach: None, projections: Vec::new(), sketch: Sketch::new(), driven: Vec::new() }
    }

    pub fn expression_for(&self, id: ConstraintId) -> Option<&str> {
        self.driven.iter().find(|(c, _)| *c == id).map(|(_, e)| e.as_str())
    }

    pub fn set_expression(&mut self, id: ConstraintId, expression: impl Into<String>) {
        let expression = expression.into();
        match self.driven.iter_mut().find(|(c, _)| *c == id) {
            Some(slot) => slot.1 = expression,
            None => self.driven.push((id, expression)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtrudeFeature {
    pub sketch: FeatureId,
    /// A point inside the profile to extrude, in sketch coordinates. Unlike
    /// a profile index this stays valid when the sketch is edited.
    pub profile_point: [f64; 2],
    /// Expression, e.g. `"height"` or `"10 mm"`.
    pub distance: String,
    pub symmetric: bool,
    pub operation: Operation,
    /// Second side: extrude this far against the normal as well (two sides).
    pub distance2: Option<String>,
    /// Taper angle in degrees (positive narrows), "0" for straight walls.
    pub taper: String,
    /// How far to go: the distance(s) above, through everything, or up to a plane/face.
    pub extent: ExtrudeExtent,
    /// Extrude only a wall along the profile boundary.
    pub thin: Option<ThinWall>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExtrudeExtent {
    Distance,
    /// Through everything (cut/intersect only); the sign of `distance`
    /// picks the direction, two sides/symmetric go both ways.
    All,
    /// Up to a plane or flat face (tilted targets trim the end).
    To(PlaneRef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThinSide {
    Inside,
    Outside,
    Center,
}

impl ThinSide {
    pub const ALL: [ThinSide; 3] = [ThinSide::Inside, ThinSide::Outside, ThinSide::Center];

    pub fn label(self) -> &'static str {
        match self {
            ThinSide::Inside => "Inside",
            ThinSide::Outside => "Outside",
            ThinSide::Center => "Center",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThinWall {
    pub thickness: String,
    pub side: ThinSide,
}

/// Axis of a revolve, in terms of the profile's sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevolveAxis {
    /// The sketch's horizontal axis through its origin.
    SketchX,
    /// The sketch's vertical axis through its origin.
    SketchY,
    /// A line of the sketch (usually a construction line).
    Line(EntityId),
    /// A world or construction axis.
    Axis(AxisRef),
}

#[derive(Clone, Debug)]
pub struct RevolveFeature {
    pub sketch: FeatureId,
    pub profile_point: [f64; 2],
    pub axis: RevolveAxis,
    /// Expression in degrees, e.g. `"360"` or `"angle / 2"`.
    pub angle: String,
    pub operation: Operation,
}

#[derive(Clone, Debug)]
pub struct FilletFeature {
    pub edges: Vec<EdgeRef>,
    pub radius: String,
    /// Bevel instead of round.
    pub chamfer: bool,
    /// Chamfer only: distance on the second face (`radius` is the first).
    pub distance2: Option<String>,
}

/// Construction plane: `base` rotated by `angle` (degrees) around its own
/// x axis, then moved by `offset` along its normal. With `base2` the plane
/// starts halfway between the two (parallel) planes instead (midplane).
#[derive(Clone, Debug)]
pub struct PlaneFeature {
    pub base: PlaneRef,
    pub offset: String,
    pub angle: String,
    pub base2: Option<PlaneRef>,
}

/// Tilt `faces` by `angle` (degrees) around the neutral plane `pull`, whose
/// normal (reversed with `flip`) is the pull direction. Positive angles make
/// the body narrower along the pull direction, so it releases from a mould.
#[derive(Clone, Debug)]
pub struct DraftFeature {
    pub faces: Vec<FaceRef>,
    pub pull: PlaneRef,
    pub angle: String,
    pub flip: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisSource {
    /// Along a straight edge, or the axis of a circular edge.
    Edge(EdgeRef),
    /// Normal to a face at a point.
    FaceNormal(FaceRef),
}

#[derive(Clone, Debug)]
pub struct AxisFeature {
    pub source: AxisSource,
}

/// Push or pull a face: positive distances add material, negative cut.
#[derive(Clone, Debug)]
pub struct PressPullFeature {
    pub face: FaceRef,
    pub distance: String,
}

/// Hollow a body, removing `faces` (open faces), walls `thickness` inwards.
#[derive(Clone, Debug)]
pub struct ShellFeature {
    pub faces: Vec<FaceRef>,
    pub thickness: String,
}

/// Round holes drilled at a face point or at sketch points.
#[derive(Clone, Debug)]
pub struct HoleFeature {
    pub placement: HolePlacement,
    pub diameter: String,
    pub depth: String,
    /// Through all instead of `depth`.
    pub through: bool,
    pub head: HoleHead,
    /// Tapped: ISO metric size ("M5"). The hole is drilled at the tap drill
    /// diameter (ignoring `diameter`) and the thread is modelled.
    pub thread: Option<String>,
}

/// A modelled ISO metric thread on a cylindrical face: outer surfaces get an
/// external thread, bores an internal one. It starts at the end of the face
/// nearer to `face.point` and runs `length` along it (`None`: full length).
#[derive(Clone, Debug)]
pub struct ThreadFeature {
    pub face: FaceRef,
    /// "M5", or "auto" for the size that fits the cylinder.
    pub size: String,
    pub length: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HolePlacement {
    /// One hole into the face at `point`, along the inward normal.
    Face(FaceRef),
    /// One hole per sketch point, drilled against the sketch normal (along
    /// it with `flip`).
    Points { sketch: FeatureId, points: Vec<PointId>, flip: bool },
}

/// Counterbore or countersink at the top of a hole.
#[derive(Clone, Debug, PartialEq)]
pub enum HoleHead {
    None,
    Counterbore { diameter: String, depth: String },
    /// `angle` is the included cone angle in degrees (90° for metric screws).
    Countersink { diameter: String, angle: String },
}

#[derive(Clone, Debug)]
pub struct MirrorFeature {
    pub bodies: Vec<String>,
    pub plane: PlaneRef,
    /// Join the mirror image to the original instead of making new bodies.
    pub join: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Rectangular {
        direction: AxisRef,
        count: String,
        spacing: String,
        /// Optional second direction (count "1" disables it).
        direction2: AxisRef,
        count2: String,
        spacing2: String,
    },
    Circular {
        axis: AxisRef,
        count: String,
        /// Total angle in degrees the instances are spread over.
        angle: String,
    },
}

#[derive(Clone, Debug)]
pub struct PatternFeature {
    pub bodies: Vec<String>,
    /// Pattern these features (their tool solids) instead of bodies.
    pub features: Vec<FeatureId>,
    pub kind: PatternKind,
    /// Join the copies to their original instead of making new bodies.
    pub join: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombineOp {
    Join,
    Cut,
    Intersect,
}

impl CombineOp {
    pub const ALL: [CombineOp; 3] = [CombineOp::Join, CombineOp::Cut, CombineOp::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            CombineOp::Join => "Join",
            CombineOp::Cut => "Cut",
            CombineOp::Intersect => "Intersect",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CombineFeature {
    pub target: String,
    pub tools: Vec<String>,
    pub op: CombineOp,
    pub keep_tools: bool,
}

/// Split a body in two at a plane.
#[derive(Clone, Debug)]
pub struct SplitFeature {
    pub body: String,
    pub plane: PlaneRef,
}

/// Translate (x, y, z expressions in mm), then rotate around `axis`.
#[derive(Clone, Debug)]
pub struct MoveFeature {
    pub bodies: Vec<String>,
    pub translate: [String; 3],
    pub axis: Option<AxisRef>,
    /// Degrees.
    pub angle: String,
    /// Keep the original and move a copy.
    pub copy: bool,
}

#[derive(Clone, Debug)]
pub struct LoftFeature {
    pub profiles: Vec<ProfileRef>,
    pub operation: Operation,
}

#[derive(Clone, Debug)]
pub struct SweepFeature {
    pub profile: ProfileRef,
    /// Connected lines/arcs of `path_sketch` forming the path.
    pub path_sketch: FeatureId,
    pub path: Vec<EntityId>,
    pub operation: Operation,
}

/// Where a joint attaches on a part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointOrigin {
    /// The clicked point on a face, z along its normal.
    Face(FaceRef),
    /// The centre of a circular edge (axis = z), or the middle of a straight edge.
    Edge(EdgeRef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    Rigid,
    /// Turns around the joint axis by `angle`.
    Revolute,
    /// Slides along the joint axis by `offset`.
    Slider,
}

impl JointKind {
    pub const ALL: [JointKind; 3] = [JointKind::Rigid, JointKind::Revolute, JointKind::Slider];

    pub fn label(self) -> &'static str {
        match self {
            JointKind::Rigid => "Rigid",
            JointKind::Revolute => "Revolute",
            JointKind::Slider => "Slider",
        }
    }
}

/// Moves `moving` so its joint origin `from` sits on `to` (z axes opposed,
/// i.e. mated), then turns by `angle` around and shifts by `offset` along
/// the joint axis.
#[derive(Clone, Debug)]
pub struct JointFeature {
    pub moving: Vec<String>,
    pub from: JointOrigin,
    pub to: JointOrigin,
    pub kind: JointKind,
    /// Degrees.
    pub angle: String,
    pub offset: String,
    /// Align the z axes instead of opposing them.
    pub flip: bool,
    /// Optional (min, max) for the motion: angle (°) of a revolute joint,
    /// offset of a slider.
    pub limits: Option<(String, String)>,
}

impl JointFeature {
    /// Clamp a motion value to the limits (if any).
    pub fn clamp(&self, value: f64, eval: &dyn Fn(&str) -> Result<f64, String>) -> Result<f64, String> {
        match &self.limits {
            None => Ok(value),
            Some((lo, hi)) => {
                let (lo, hi) = (eval(lo)?, eval(hi)?);
                if lo > hi {
                    return Err("the joint's minimum is larger than its maximum".into());
                }
                Ok(value.clamp(lo, hi))
            }
        }
    }
}

/// Group bodies into a named component. Features that take bodies (joint,
/// move, pattern, mirror) accept the component name for all its bodies.
#[derive(Clone, Debug)]
pub struct ComponentFeature {
    pub name: String,
    pub bodies: Vec<String>,
}

/// Bodies read from a STEP file (one body per solid).
#[derive(Clone, Debug)]
pub struct ImportFeature {
    pub path: String,
}

/// Delete bodies from the model at this point of the timeline.
#[derive(Clone, Debug)]
pub struct RemoveFeature {
    pub bodies: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum FeatureKind {
    Sketch(SketchFeature),
    Extrude(ExtrudeFeature),
    Revolve(RevolveFeature),
    Fillet(FilletFeature),
    Plane(PlaneFeature),
    Axis(AxisFeature),
    PressPull(PressPullFeature),
    Shell(ShellFeature),
    Draft(DraftFeature),
    Thread(ThreadFeature),
    Hole(HoleFeature),
    Mirror(MirrorFeature),
    Pattern(PatternFeature),
    Combine(CombineFeature),
    Split(SplitFeature),
    Move(MoveFeature),
    Loft(LoftFeature),
    Sweep(SweepFeature),
    Remove(RemoveFeature),
    Import(ImportFeature),
    Joint(JointFeature),
    Component(ComponentFeature),
}

impl FeatureKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            FeatureKind::Sketch(_) => "Sketch",
            FeatureKind::Extrude(_) => "Extrude",
            FeatureKind::Revolve(_) => "Revolve",
            FeatureKind::Fillet(f) if f.chamfer => "Chamfer",
            FeatureKind::Fillet(_) => "Fillet",
            FeatureKind::Plane(p) if p.base2.is_some() => "Midplane",
            FeatureKind::Plane(_) => "Construction plane",
            FeatureKind::Draft(_) => "Draft",
            FeatureKind::Thread(_) => "Thread",
            FeatureKind::Axis(_) => "Construction axis",
            FeatureKind::PressPull(_) => "Press Pull",
            FeatureKind::Shell(_) => "Shell",
            FeatureKind::Hole(_) => "Hole",
            FeatureKind::Mirror(_) => "Mirror",
            FeatureKind::Pattern(p) if matches!(p.kind, PatternKind::Circular { .. }) => "Circular Pattern",
            FeatureKind::Pattern(_) => "Rectangular Pattern",
            FeatureKind::Combine(_) => "Combine",
            FeatureKind::Split(_) => "Split Body",
            FeatureKind::Move(m) if m.copy => "Copy Body",
            FeatureKind::Move(_) => "Move Body",
            FeatureKind::Loft(_) => "Loft",
            FeatureKind::Sweep(_) => "Sweep",
            FeatureKind::Remove(_) => "Remove Body",
            FeatureKind::Import(_) => "Import",
            FeatureKind::Joint(_) => "Joint",
            FeatureKind::Component(_) => "Component",
        }
    }

    /// The feature's value expressions with labels (sketch dimensions are
    /// listed separately via `SketchFeature::driven`).
    pub fn expressions_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        match self {
            FeatureKind::Sketch(_) | FeatureKind::Axis(_) | FeatureKind::Mirror(_) | FeatureKind::Combine(_) => vec![],
            FeatureKind::Split(_) | FeatureKind::Loft(_) | FeatureKind::Sweep(_) | FeatureKind::Remove(_) => vec![],
            FeatureKind::Import(_) => vec![],
            FeatureKind::Joint(j) => {
                let mut v: Vec<(&'static str, &mut String)> = vec![("Angle (°)", &mut j.angle), ("Offset", &mut j.offset)];
                if let Some((lo, hi)) = &mut j.limits {
                    v.push(("Minimum", lo));
                    v.push(("Maximum", hi));
                }
                v
            }
            FeatureKind::Component(_) => vec![],
            FeatureKind::Extrude(e) => {
                let mut v: Vec<(&'static str, &mut String)> = vec![("Distance", &mut e.distance)];
                if let Some(d2) = &mut e.distance2 {
                    v.push(("Distance 2", d2));
                }
                v.push(("Taper (°)", &mut e.taper));
                if let Some(t) = &mut e.thin {
                    v.push(("Thickness", &mut t.thickness));
                }
                v
            }
            FeatureKind::Revolve(r) => vec![("Angle (°)", &mut r.angle)],
            FeatureKind::Fillet(f) => {
                let mut v: Vec<(&'static str, &mut String)> =
                    vec![(if f.chamfer { "Distance" } else { "Radius" }, &mut f.radius)];
                if let (true, Some(d2)) = (f.chamfer, &mut f.distance2) {
                    v.push(("Distance 2", d2));
                }
                v
            }
            FeatureKind::Draft(d) => vec![("Angle (°)", &mut d.angle)],
            FeatureKind::Thread(t) => t.length.iter_mut().map(|l| ("Length", l)).collect(),
            FeatureKind::Plane(p) => vec![("Offset", &mut p.offset), ("Angle (°)", &mut p.angle)],
            FeatureKind::PressPull(p) => vec![("Distance", &mut p.distance)],
            FeatureKind::Shell(sh) => vec![("Thickness", &mut sh.thickness)],
            FeatureKind::Hole(h) => {
                let mut v: Vec<(&'static str, &mut String)> = vec![("Diameter", &mut h.diameter)];
                if !h.through {
                    v.push(("Depth", &mut h.depth));
                }
                match &mut h.head {
                    HoleHead::None => {}
                    HoleHead::Counterbore { diameter, depth } => {
                        v.push(("Counterbore Ø", diameter));
                        v.push(("Counterbore depth", depth));
                    }
                    HoleHead::Countersink { diameter, angle } => {
                        v.push(("Countersink Ø", diameter));
                        v.push(("Countersink angle (°)", angle));
                    }
                }
                v
            }
            FeatureKind::Pattern(p) => match &mut p.kind {
                PatternKind::Rectangular { count, spacing, count2, spacing2, .. } => vec![
                    ("Count", count),
                    ("Spacing", spacing),
                    ("Count 2", count2),
                    ("Spacing 2", spacing2),
                ],
                PatternKind::Circular { count, angle, .. } => vec![("Count", count), ("Angle (°)", angle)],
            },
            FeatureKind::Move(m) => {
                let [x, y, z] = &mut m.translate;
                vec![("X", x), ("Y", y), ("Z", z), ("Angle (°)", &mut m.angle)]
            }
        }
    }

    /// Every feature this one depends on (must come earlier in the timeline).
    pub fn references(&self) -> Vec<FeatureId> {
        let plane = |p: &PlaneRef| match p {
            PlaneRef::Construction(id) => vec![*id],
            _ => vec![],
        };
        let axis = |a: &AxisRef| match a {
            AxisRef::Construction(id) => vec![*id],
            _ => vec![],
        };
        match self {
            FeatureKind::Sketch(s) => s.attach.as_ref().map(plane).unwrap_or_default(),
            FeatureKind::Extrude(e) => match &e.extent {
                ExtrudeExtent::To(p) => [vec![e.sketch], plane(p)].concat(),
                _ => vec![e.sketch],
            },
            FeatureKind::Revolve(r) => match r.axis {
                RevolveAxis::Axis(AxisRef::Construction(a)) => vec![r.sketch, a],
                _ => vec![r.sketch],
            },
            FeatureKind::Hole(h) => match &h.placement {
                HolePlacement::Points { sketch, .. } => vec![*sketch],
                HolePlacement::Face(_) => vec![],
            },
            FeatureKind::Plane(p) => [plane(&p.base), p.base2.as_ref().map(plane).unwrap_or_default()].concat(),
            FeatureKind::Draft(d) => plane(&d.pull),
            FeatureKind::Mirror(m) => plane(&m.plane),
            FeatureKind::Split(s) => plane(&s.plane),
            FeatureKind::Pattern(p) => {
                let axes = match &p.kind {
                    PatternKind::Rectangular { direction, direction2, .. } => {
                        [axis(direction), axis(direction2)].concat()
                    }
                    PatternKind::Circular { axis: a, .. } => axis(a),
                };
                [axes, p.features.clone()].concat()
            }
            FeatureKind::Move(m) => m.axis.as_ref().map(axis).unwrap_or_default(),
            FeatureKind::Loft(l) => l.profiles.iter().map(|p| p.sketch).collect(),
            FeatureKind::Sweep(s) => vec![s.profile.sketch, s.path_sketch],
            _ => vec![],
        }
    }

    /// The sketches whose profiles this feature consumes.
    pub fn consumed_sketches(&self) -> Vec<FeatureId> {
        match self {
            FeatureKind::Extrude(e) => vec![e.sketch],
            FeatureKind::Revolve(r) => match r.axis {
                RevolveAxis::Axis(AxisRef::Construction(a)) => vec![r.sketch, a],
                _ => vec![r.sketch],
            },
            FeatureKind::Loft(l) => l.profiles.iter().map(|p| p.sketch).collect(),
            FeatureKind::Sweep(s) => vec![s.profile.sketch, s.path_sketch],
            _ => vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    pub suppressed: bool,
    pub kind: FeatureKind,
    /// Component that was active when the feature was made: bodies it
    /// creates belong to that component.
    pub component: Option<String>,
}
