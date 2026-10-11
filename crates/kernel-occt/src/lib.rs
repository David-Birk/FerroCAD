//! [`Kernel`] backed by OpenCascade through `opencascade-rs`.
//!
//! Known gap: `opencascade-rs` does not yet report failed fillets/chamfers
//! as errors – OCCT throws a C++ exception, which aborts the process. The
//! fix is a small upstream change (check `IsDone()` before `Shape()`); until
//! then avoid fillet radii larger than the adjacent faces allow.

use ferrocad_model::geom::{distance, scale, Vec3};
use ferrocad_model::kernel::polyline_midpoint;
use ferrocad_model::{BooleanOp, EdgeRef, Kernel, KernelError, Loop3, PlacedProfile, Seg3, TriMesh};
use glam::{dvec3, DVec3};
use opencascade::primitives::{Edge, Face, Shape, Wire};
use std::path::Path;
use std::rc::Rc;

#[derive(Default)]
pub struct OcctKernel;

impl OcctKernel {
    pub fn new() -> Self {
        Self
    }
}

fn v(p: Vec3) -> DVec3 {
    dvec3(p[0], p[1], p[2])
}

fn a(p: DVec3) -> Vec3 {
    [p.x, p.y, p.z]
}

fn edge(seg: &Seg3) -> Edge {
    match *seg {
        Seg3::Line { a, b } => Edge::segment(v(a), v(b)),
        Seg3::Arc { start, mid, end } => Edge::arc(v(start), v(mid), v(end)),
        Seg3::Circle { center, normal, radius } => Edge::circle(v(center), v(normal), radius),
    }
}

fn wire(lp: &Loop3) -> Result<Wire, KernelError> {
    if lp.segments.is_empty() {
        return Err(KernelError("empty profile loop".into()));
    }
    let edges: Vec<Edge> = lp.segments.iter().map(edge).collect();
    Ok(Wire::from_edges(&edges))
}

/// Polylines of every edge of `shape`.
fn edge_polylines(shape: &Shape) -> Vec<Vec<Vec3>> {
    shape.edges().map(|e| e.approximation_segments().map(a).collect()).collect()
}

impl Kernel for OcctKernel {
    type Solid = Rc<Shape>;

    fn extrude(
        &self,
        profile: &PlacedProfile,
        dist: f64,
        symmetric: bool,
    ) -> Result<Self::Solid, KernelError> {
        let mut outer = wire(&profile.outer)?;
        let mut holes = profile.holes.iter().map(wire).collect::<Result<Vec<_>, _>>()?;
        if symmetric {
            let back = v(scale(profile.normal, -dist * 0.5));
            outer = outer.translate(back);
            holes = holes.iter().map(|h| h.translate(back)).collect();
        }
        let face = Face::from_wire_with_holes(&outer, &holes);
        let solid = face.extrude(v(scale(profile.normal, dist)));
        Ok(Rc::new(Shape::from(solid)))
    }

    fn boolean(
        &self,
        op: BooleanOp,
        a: &Self::Solid,
        b: &Self::Solid,
    ) -> Result<Self::Solid, KernelError> {
        let result = match op {
            BooleanOp::Union => a.union(b),
            BooleanOp::Subtract => a.subtract(b),
            BooleanOp::Intersect => a.intersect(b),
        };
        Ok(Rc::new(result.shape))
    }

    fn fillet(
        &self,
        solid: &Self::Solid,
        refs: &[EdgeRef],
        radius: f64,
        chamfer: bool,
    ) -> Result<Self::Solid, KernelError> {
        // Match each reference to the nearest edge by midpoint. The explorer
        // visits shared edges once per face, so de-duplicate by index.
        let edges: Vec<Edge> = solid.edges().collect();
        let mids: Vec<Option<Vec3>> = edges
            .iter()
            .map(|e| polyline_midpoint(&e.approximation_segments().map(a).collect::<Vec<_>>()))
            .collect();
        let mut picked: Vec<usize> = Vec::new();
        for r in refs {
            let best = mids
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.map(|m| (i, distance(m, r.point))))
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .ok_or_else(|| KernelError("solid has no edges".into()))?;
            // The same edge is seen twice (once per adjacent face); identical
            // midpoints mean it is already picked.
            let m = mids[best.0].expect("filtered above");
            if !picked.iter().any(|&p| mids[p].is_some_and(|q| distance(q, m) < 1e-9)) {
                picked.push(best.0);
            }
        }
        let chosen: Vec<&Edge> = picked.iter().map(|&i| &edges[i]).collect();
        let shape = if chamfer {
            solid.chamfer_edges(radius, chosen)
        } else {
            solid.fillet_edges(radius, chosen)
        };
        Ok(Rc::new(shape))
    }

    fn tessellate(&self, solid: &Self::Solid, tolerance: f64) -> Result<TriMesh, KernelError> {
        let mesh = solid.mesh_with_tolerance(tolerance).map_err(|e| KernelError(e.to_string()))?;
        Ok(TriMesh {
            positions: mesh.vertices.into_iter().map(a).collect(),
            normals: mesh.normals.into_iter().map(a).collect(),
            indices: mesh.indices.into_iter().map(|i| i as u32).collect(),
        })
    }

    fn edges(&self, solid: &Self::Solid) -> Vec<Vec<Vec3>> {
        edge_polylines(solid)
    }

    fn export_step(&self, solids: &[Self::Solid], path: &Path) -> Result<(), KernelError> {
        let shapes: Vec<&Shape> = solids.iter().map(|s| &**s).collect();
        Shape::write_all_step(shapes, path).map_err(|e| KernelError(e.to_string()))
    }

    fn export_stl(&self, solid: &Self::Solid, path: &Path) -> Result<(), KernelError> {
        solid.write_stl(path).map_err(|e| KernelError(e.to_string()))
    }
}
