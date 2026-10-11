//! The interface every geometry kernel implements.

use crate::geom::{distance, scale, Plane, Vec3};
use ferrocad_sketch::{Loop, Profile, Segment};
use std::fmt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct KernelError(pub String);

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KernelError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Subtract,
    Intersect,
}

/// A curve segment in world space.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg3 {
    Line { a: Vec3, b: Vec3 },
    /// Three-point arc: start, a point in the middle, end.
    Arc { start: Vec3, mid: Vec3, end: Vec3 },
    /// Orientation follows the right-hand rule around `normal`.
    Circle { center: Vec3, normal: Vec3, radius: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loop3 {
    pub segments: Vec<Seg3>,
}

/// A sketch profile placed on its plane, ready for extrusion.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedProfile {
    pub outer: Loop3,
    pub holes: Vec<Loop3>,
    /// Plane normal; the outer loop runs counter-clockwise around it.
    pub normal: Vec3,
}

impl PlacedProfile {
    pub fn new(profile: &Profile, plane: &Plane) -> Self {
        let normal = plane.normal();
        let place = |lp: &Loop| Loop3 {
            segments: lp
                .segments
                .iter()
                .map(|s| match *s {
                    Segment::Line { a, b } => Seg3::Line { a: plane.to_world(a), b: plane.to_world(b) },
                    Segment::Arc { from, to, .. } => Seg3::Arc {
                        start: plane.to_world(from),
                        mid: plane.to_world(s.midpoint()),
                        end: plane.to_world(to),
                    },
                    Segment::Circle { center, radius, ccw } => Seg3::Circle {
                        center: plane.to_world(center),
                        normal: if ccw { normal } else { scale(normal, -1.0) },
                        radius,
                    },
                })
                .collect(),
        };
        PlacedProfile {
            outer: place(&profile.outer),
            holes: profile.holes.iter().map(place).collect(),
            normal,
        }
    }
}

/// Picks an edge of a solid by a point on it. This is the simplest possible
/// answer to the *topological naming problem*: it survives most parameter
/// changes that don't move the edge far, but not big topology changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeRef {
    pub point: Vec3,
}

/// Midpoint (by arc length) of a polyline – how edges are matched to
/// [`EdgeRef`]s. Kernels should use this so matching is consistent.
pub fn polyline_midpoint(pts: &[Vec3]) -> Option<Vec3> {
    if pts.is_empty() {
        return None;
    }
    let total: f64 = pts.windows(2).map(|w| distance(w[0], w[1])).sum();
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let d = distance(w[0], w[1]);
        if acc + d >= total * 0.5 && d > 0.0 {
            let t = (total * 0.5 - acc) / d;
            return Some([
                w[0][0] + t * (w[1][0] - w[0][0]),
                w[0][1] + t * (w[1][1] - w[0][1]),
                w[0][2] + t * (w[1][2] - w[0][2]),
            ]);
        }
        acc += d;
    }
    Some(pts[0])
}

/// Triangle mesh for display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

/// A geometry kernel. `Solid` should be cheap to clone (e.g. an `Rc`).
pub trait Kernel {
    type Solid: Clone;

    /// Extrude along the profile normal. Negative distances go the other way;
    /// `symmetric` extrudes `distance / 2` to each side.
    fn extrude(
        &self,
        profile: &PlacedProfile,
        distance: f64,
        symmetric: bool,
    ) -> Result<Self::Solid, KernelError>;

    fn boolean(
        &self,
        op: BooleanOp,
        a: &Self::Solid,
        b: &Self::Solid,
    ) -> Result<Self::Solid, KernelError>;

    /// Round (or with `chamfer`, bevel) the edges matched by `edges`.
    fn fillet(
        &self,
        solid: &Self::Solid,
        edges: &[EdgeRef],
        radius: f64,
        chamfer: bool,
    ) -> Result<Self::Solid, KernelError>;

    fn tessellate(&self, solid: &Self::Solid, tolerance: f64) -> Result<TriMesh, KernelError>;

    /// Every edge as a polyline – for drawing outlines and picking edges.
    fn edges(&self, solid: &Self::Solid) -> Vec<Vec<Vec3>>;

    fn export_step(&self, solids: &[Self::Solid], path: &Path) -> Result<(), KernelError>;

    fn export_stl(&self, solid: &Self::Solid, path: &Path) -> Result<(), KernelError>;
}
