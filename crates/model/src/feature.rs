//! Timeline features.

use crate::geom::Plane;
use crate::kernel::EdgeRef;
use ferrocad_sketch::{ConstraintId, EntityId, Sketch};

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

#[derive(Clone, Debug)]
pub struct SketchFeature {
    pub plane: Plane,
    pub sketch: Sketch,
    /// Dimensions driven by parameter expressions, e.g. `(id, "width / 2")`.
    /// Angle expressions are in degrees.
    pub driven: Vec<(ConstraintId, String)>,
}

impl SketchFeature {
    pub fn new(plane: Plane) -> Self {
        Self { plane, sketch: Sketch::new(), driven: Vec::new() }
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
}

#[derive(Clone, Debug)]
pub enum FeatureKind {
    Sketch(SketchFeature),
    Extrude(ExtrudeFeature),
    Revolve(RevolveFeature),
    Fillet(FilletFeature),
}

impl FeatureKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            FeatureKind::Sketch(_) => "Sketch",
            FeatureKind::Extrude(_) => "Extrude",
            FeatureKind::Revolve(_) => "Revolve",
            FeatureKind::Fillet(f) if f.chamfer => "Chamfer",
            FeatureKind::Fillet(_) => "Fillet",
        }
    }

    /// The sketch this feature consumes, if any.
    pub fn sketch_ref(&self) -> Option<FeatureId> {
        match self {
            FeatureKind::Extrude(e) => Some(e.sketch),
            FeatureKind::Revolve(r) => Some(r.sketch),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Feature {
    pub id: FeatureId,
    pub name: String,
    pub suppressed: bool,
    pub kind: FeatureKind,
}
