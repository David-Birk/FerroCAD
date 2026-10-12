//! ferrocad additions: modelling operations that return `Err` when OCCT
//! fails instead of aborting the process (see opencascade-sys `ferro.hxx`).

use crate::primitives::Shape;
use cxx::UniquePtr;
use glam::DVec3;
use opencascade_sys::{ferro, topo_ds::TopoDS_Shape};

pub type SafeResult = Result<Shape, String>;

fn wrap(r: Result<UniquePtr<TopoDS_Shape>, cxx::Exception>) -> SafeResult {
    match r {
        Ok(inner) if !inner.is_null() => Ok(Shape { inner }),
        Ok(_) => Err("operation produced no shape".into()),
        Err(e) => Err(e.what().to_owned()),
    }
}

/// Fillet (or chamfer) edges given by their index in `shape.edges()` order.
pub fn fillet(shape: &Shape, edges: &[u32], radius: f64, chamfer: bool) -> SafeResult {
    wrap(ferro::ferro_fillet(&shape.inner, edges, radius, chamfer))
}

/// Chamfer edges with `d1` on the first adjacent face and `d2` on the other.
pub fn chamfer2(shape: &Shape, edges: &[u32], d1: f64, d2: f64) -> SafeResult {
    wrap(ferro::ferro_chamfer2(&shape.inner, edges, d1, d2))
}

/// Draft faces (indices in `shape.faces()` order) by `angle` radians around
/// the neutral plane through `origin` with the pull direction `dir` as normal.
pub fn draft(shape: &Shape, faces: &[u32], origin: DVec3, dir: DVec3, angle: f64) -> SafeResult {
    let p = [origin.x, origin.y, origin.z, dir.x, dir.y, dir.z, angle];
    wrap(ferro::ferro_draft(&shape.inner, faces, &p))
}

#[derive(Clone, Copy, Debug)]
pub enum BoolOp {
    Union = 0,
    Subtract = 1,
    Intersect = 2,
}

pub fn boolean(a: &Shape, b: &Shape, op: BoolOp) -> SafeResult {
    wrap(ferro::ferro_boolean(&a.inner, &b.inner, op as u32))
}

/// Hollow `shape` with inward walls, removing the faces given by index in
/// `shape.faces()` order.
pub fn shell(shape: &Shape, faces: &[u32], thickness: f64) -> SafeResult {
    wrap(ferro::ferro_shell(&shape.inner, faces, thickness))
}

pub fn translate(shape: &Shape, by: DVec3) -> SafeResult {
    wrap(ferro::ferro_transform(&shape.inner, 0, &[0.0, 0.0, 0.0, by.x, by.y, by.z, 0.0]))
}

pub fn rotate(shape: &Shape, origin: DVec3, axis: DVec3, angle: f64) -> SafeResult {
    let p = [origin.x, origin.y, origin.z, axis.x, axis.y, axis.z, angle];
    wrap(ferro::ferro_transform(&shape.inner, 1, &p))
}

pub fn mirror(shape: &Shape, origin: DVec3, normal: DVec3) -> SafeResult {
    let p = [origin.x, origin.y, origin.z, normal.x, normal.y, normal.z, 0.0];
    wrap(ferro::ferro_transform(&shape.inner, 2, &p))
}

/// Planar face from encoded loops: `[nloops, (nseg, segments…)…]`, segments
/// `0 a b` (line), `1 start mid end` (arc), `2 center normal radius` (circle).
pub fn profile_face(data: &[f64]) -> SafeResult {
    wrap(ferro::ferro_profile_face(data))
}

pub fn prism(face: &Shape, dir: DVec3) -> SafeResult {
    wrap(ferro::ferro_prism(&face.inner, dir.x, dir.y, dir.z))
}

/// Tapered prism: `h1` along the face normal, `h2` against it, drafted by `angle`.
pub fn draft_prism(face: &Shape, h1: f64, h2: f64, angle: f64, reverse: bool) -> SafeResult {
    wrap(ferro::ferro_draft_prism(&face.inner, h1, h2, angle, reverse))
}

pub fn revolve(face: &Shape, origin: DVec3, axis: DVec3, angle: f64) -> SafeResult {
    let p = [origin.x, origin.y, origin.z, axis.x, axis.y, axis.z, angle];
    wrap(ferro::ferro_revol(&face.inner, &p))
}

/// Loft through encoded wires: `[nwires, (nseg, segments…)…]`.
pub fn loft(data: &[f64]) -> SafeResult {
    wrap(ferro::ferro_loft(data))
}

/// Sweep a face along an encoded open wire `[nseg, segments…]`.
pub fn sweep(face: &Shape, path: &[f64]) -> SafeResult {
    wrap(ferro::ferro_sweep(&face.inner, path))
}

/// The face with index `i` in `shape.faces()` order.
pub fn face(shape: &Shape, i: u32) -> SafeResult {
    wrap(ferro::ferro_sub_shape(&shape.inner, 0, i))
}

/// A face found by [`find_face`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceHit {
    pub index: u32,
    pub point: DVec3,
    pub normal: DVec3,
    pub planar: bool,
}

/// The face crossed by the line through `point` along `normal` whose outward
/// normal points the same way, nearest to `point`.
pub fn find_face(shape: &Shape, point: DVec3, normal: DVec3) -> Option<FaceHit> {
    let q = [point.x, point.y, point.z, normal.x, normal.y, normal.z];
    let v = ferro::ferro_find_face(&shape.inner, &q);
    (v.len() == 8).then(|| FaceHit {
        index: v[0] as u32,
        point: DVec3::new(v[1], v[2], v[3]),
        normal: DVec3::new(v[4], v[5], v[6]),
        planar: v[7] > 0.5,
    })
}

/// (volume, surface area, centre of mass)
pub fn properties(shape: &Shape) -> Option<(f64, f64, DVec3)> {
    let v = ferro::ferro_properties(&shape.inner);
    (v.len() == 5).then(|| (v[0], v[1], DVec3::new(v[2], v[3], v[4])))
}

/// Read a STEP file into one shape.
pub fn read_step(path: &str) -> SafeResult {
    wrap(ferro::ferro_read_step(path))
}

/// Every solid of a shape (e.g. an imported assembly), in order.
pub fn solids(shape: &Shape) -> Result<Vec<Shape>, String> {
    (0..solid_count(shape)).map(|i| wrap(ferro::ferro_sub_shape(&shape.inner, 2, i))).collect()
}

pub fn solid_count(shape: &Shape) -> u32 {
    ferro::ferro_count(&shape.inner, 2)
}
