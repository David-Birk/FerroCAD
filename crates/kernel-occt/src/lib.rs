//! [`Kernel`] backed by OpenCascade through `opencascade-rs`.
//!
//! Every modelling operation goes through `opencascade::safe` (our vendored
//! fork), which checks `IsDone()` and catches OCCT exceptions in C++, so a
//! fillet radius that is too big becomes a [`KernelError`] instead of
//! aborting the process.

use ferrocad_model::geom::{distance, scale, Vec3};
use ferrocad_model::kernel::polyline_midpoint;
use ferrocad_model::{
    BooleanOp, EdgeRef, FaceInfo, FaceRef, Kernel, KernelError, Loop3, MassProperties, PlacedProfile, Seg3,
    ThinSide, Transform, TriMesh,
};
use glam::{dvec3, DVec3};
use opencascade::primitives::Shape;
use opencascade::safe::{self, BoolOp};
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

fn err(e: String) -> KernelError {
    KernelError(e)
}

/// Encode a loop for `opencascade::safe` (see `ferro.hxx`).
fn encode_loop(lp: &Loop3, out: &mut Vec<f64>) {
    out.push(lp.segments.len() as f64);
    for seg in &lp.segments {
        match *seg {
            Seg3::Line { a, b } => {
                out.push(0.0);
                out.extend(a);
                out.extend(b);
            }
            Seg3::Arc { start, mid, end } => {
                out.push(1.0);
                out.extend(start);
                out.extend(mid);
                out.extend(end);
            }
            Seg3::Circle { center, normal, radius } => {
                out.push(2.0);
                out.extend(center);
                out.extend(normal);
                out.push(radius);
            }
            Seg3::Spline { ref points } => {
                out.push(3.0);
                out.push(points.len() as f64);
                for p in points {
                    out.extend(*p);
                }
            }
        }
    }
}

fn profile_face(profile: &PlacedProfile) -> Result<Shape, KernelError> {
    if profile.outer.segments.is_empty() {
        return Err(KernelError("empty profile loop".into()));
    }
    let mut data = vec![(1 + profile.holes.len()) as f64];
    encode_loop(&profile.outer, &mut data);
    for h in &profile.holes {
        encode_loop(h, &mut data);
    }
    safe::profile_face(&data).map_err(err)
}

/// Polylines of every edge of `shape`, in `shape.edges()` order.
fn edge_polylines(shape: &Shape) -> Vec<Vec<Vec3>> {
    shape.edges().map(|e| e.approximation_segments().map(a).collect()).collect()
}

/// Index (in `shape.faces()` order) of the face matched by `face`.
fn face_index(shape: &Shape, face: &FaceRef) -> Result<(u32, safe::FaceHit), KernelError> {
    safe::find_face(shape, v(face.point), v(face.normal))
        .map(|h| (h.index, h))
        .ok_or_else(|| KernelError("the face no longer exists – re-pick it".into()))
}

/// Indices (in `edges()` order) of the edges nearest to each reference by
/// midpoint. The explorer visits shared edges once per face, so de-duplicate.
fn edge_indices(solid: &Shape, refs: &[EdgeRef]) -> Result<Vec<u32>, KernelError> {
    let mids: Vec<Option<Vec3>> = edge_polylines(solid).iter().map(|pl| polyline_midpoint(pl)).collect();
    let mut picked: Vec<usize> = Vec::new();
    for r in refs {
        let best = mids
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.map(|m| (i, distance(m, r.point))))
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .ok_or_else(|| KernelError("solid has no edges".into()))?;
        let m = mids[best.0].expect("filtered above");
        if !picked.iter().any(|&p| mids[p].is_some_and(|q| distance(q, m) < 1e-9)) {
            picked.push(best.0);
        }
    }
    Ok(picked.iter().map(|&i| i as u32).collect())
}

impl Kernel for OcctKernel {
    type Solid = Rc<Shape>;

    fn extrude(&self, profile: &PlacedProfile, dist: f64, symmetric: bool) -> Result<Self::Solid, KernelError> {
        let mut face = profile_face(profile)?;
        if symmetric {
            face = safe::translate(&face, v(scale(profile.normal, -dist * 0.5))).map_err(err)?;
        }
        let solid = safe::prism(&face, v(scale(profile.normal, dist))).map_err(err)?;
        Ok(Rc::new(solid))
    }

    fn extrude_tapered(
        &self,
        profile: &PlacedProfile,
        forward: f64,
        backward: f64,
        taper: f64,
    ) -> Result<Self::Solid, KernelError> {
        let face = profile_face(profile)?;
        let n = v(profile.normal);
        if taper.abs() < 1e-12 {
            let base = safe::translate(&face, -n * backward).map_err(err)?;
            return Ok(Rc::new(safe::prism(&base, n * (forward + backward)).map_err(err)?));
        }
        // LocOpe_DPrism takes a height along the normal and one against it;
        // a flipped extrude (negative forward) goes against the normal.
        // The draft continues through the face, so on the far side the
        // angle flips to keep "positive narrows".
        let (h1, h2, angle) = if forward < 0.0 && backward <= 0.0 {
            (-backward, -forward, -taper)
        } else {
            (forward, backward, taper)
        };
        // LocOpe_DPrism measures along the drafted side, not perpendicular.
        let c = taper.cos();
        Ok(Rc::new(safe::draft_prism(&face, h1 / c, h2 / c, angle, false).map_err(err)?))
    }

    fn extrude_thin(
        &self,
        profile: &PlacedProfile,
        forward: f64,
        backward: f64,
        thickness: f64,
        side: ThinSide,
    ) -> Result<Self::Solid, KernelError> {
        // Distance of the wall into the material and out of it.
        let into = match side {
            ThinSide::Inside => thickness,
            ThinSide::Outside => 0.0,
            ThinSide::Center => thickness / 2.0,
        };
        let out = thickness - into;
        let n = v(profile.normal).normalize();
        // One band per loop; material lies inside the outer loop and outside the holes.
        let mut result: Option<Shape> = None;
        let loops = std::iter::once((&profile.outer, true)).chain(profile.holes.iter().map(|h| (h, false)));
        for (l, outer) in loops {
            let single = PlacedProfile { outer: l.clone(), holes: vec![], normal: profile.normal };
            let face = profile_face(&single)?;
            let (grow, shrink) = if outer { (out, into) } else { (into, out) };
            let band = safe::thin_band(&face, n, forward, backward, grow, shrink).map_err(err)?;
            result = Some(match result {
                None => band,
                Some(r) => safe::boolean(&r, &band, BoolOp::Union).map_err(err)?,
            });
        }
        Ok(Rc::new(result.expect("the outer loop always exists")))
    }

    fn revolve(
        &self,
        profile: &PlacedProfile,
        axis_origin: Vec3,
        axis_dir: Vec3,
        angle: f64,
    ) -> Result<Self::Solid, KernelError> {
        let face = profile_face(profile)?;
        let solid = safe::revolve(&face, v(axis_origin), v(axis_dir), angle).map_err(err)?;
        Ok(Rc::new(solid))
    }

    fn boolean(&self, op: BooleanOp, a: &Self::Solid, b: &Self::Solid) -> Result<Self::Solid, KernelError> {
        let op = match op {
            BooleanOp::Union => BoolOp::Union,
            BooleanOp::Subtract => BoolOp::Subtract,
            BooleanOp::Intersect => BoolOp::Intersect,
        };
        Ok(Rc::new(safe::boolean(a, b, op).map_err(err)?))
    }

    fn fillet(
        &self,
        solid: &Self::Solid,
        refs: &[EdgeRef],
        radius: f64,
        chamfer: bool,
    ) -> Result<Self::Solid, KernelError> {
        let indices = edge_indices(solid, refs)?;
        Ok(Rc::new(safe::fillet(solid, &indices, radius, chamfer).map_err(err)?))
    }

    fn chamfer_two(&self, solid: &Self::Solid, refs: &[EdgeRef], d1: f64, d2: f64) -> Result<Self::Solid, KernelError> {
        let indices = edge_indices(solid, refs)?;
        Ok(Rc::new(safe::chamfer2(solid, &indices, d1, d2).map_err(err)?))
    }

    fn draft(
        &self,
        solid: &Self::Solid,
        faces: &[FaceRef],
        origin: Vec3,
        pull: Vec3,
        angle: f64,
    ) -> Result<Self::Solid, KernelError> {
        let mut indices = Vec::new();
        for f in faces {
            let (i, _) = face_index(solid, f)?;
            if !indices.contains(&i) {
                indices.push(i);
            }
        }
        Ok(Rc::new(safe::draft(solid, &indices, v(origin), v(pull), angle).map_err(err)?))
    }

    fn extrude_face(&self, solid: &Self::Solid, face: &FaceRef, distance: f64) -> Result<Self::Solid, KernelError> {
        let (index, hit) = face_index(solid, face)?;
        if !hit.planar {
            return Err(KernelError("only flat faces can be pushed or pulled".into()));
        }
        let f = safe::face(solid, index).map_err(err)?;
        let tool = safe::prism(&f, hit.normal.normalize() * distance).map_err(err)?;
        Ok(Rc::new(tool))
    }

    fn shell(&self, solid: &Self::Solid, faces: &[FaceRef], thickness: f64) -> Result<Self::Solid, KernelError> {
        let mut indices = Vec::new();
        for f in faces {
            let (i, _) = face_index(solid, f)?;
            if !indices.contains(&i) {
                indices.push(i);
            }
        }
        Ok(Rc::new(safe::shell(solid, &indices, thickness).map_err(err)?))
    }

    fn transform(&self, solid: &Self::Solid, t: &Transform) -> Result<Self::Solid, KernelError> {
        let shape = match *t {
            Transform::Translate(by) => safe::translate(solid, v(by)),
            Transform::Rotate { origin, axis, angle } => safe::rotate(solid, v(origin), v(axis), angle),
            Transform::Mirror { origin, normal } => safe::mirror(solid, v(origin), v(normal)),
        };
        Ok(Rc::new(shape.map_err(err)?))
    }

    fn loft(&self, sections: &[Loop3]) -> Result<Self::Solid, KernelError> {
        let mut data = vec![sections.len() as f64];
        for s in sections {
            encode_loop(s, &mut data);
        }
        Ok(Rc::new(safe::loft(&data).map_err(err)?))
    }

    fn sweep(&self, profile: &PlacedProfile, path: &[Seg3]) -> Result<Self::Solid, KernelError> {
        let face = profile_face(profile)?;
        let mut data = Vec::new();
        encode_loop(&Loop3 { segments: path.to_vec() }, &mut data);
        Ok(Rc::new(safe::sweep(&face, &data).map_err(err)?))
    }

    fn find_face(&self, solid: &Self::Solid, face: &FaceRef) -> Option<FaceInfo> {
        safe::find_face(solid, v(face.point), v(face.normal)).map(|h| FaceInfo {
            point: a(h.point),
            normal: a(h.normal),
            planar: h.planar,
        })
    }

    fn is_empty(&self, solid: &Self::Solid) -> bool {
        safe::solid_count(solid) == 0
    }

    fn properties(&self, solid: &Self::Solid) -> Option<MassProperties> {
        safe::properties(solid).map(|(volume, area, c)| MassProperties { volume, area, center: a(c) })
    }

    fn face_area(&self, solid: &Self::Solid, face: &FaceRef) -> Option<f64> {
        let (i, _) = face_index(solid, face).ok()?;
        safe::properties(&safe::face(solid, i).ok()?).map(|(_, area, _)| area)
    }

    fn import_step(&self, path: &Path) -> Result<Vec<Self::Solid>, KernelError> {
        if !path.exists() {
            return Err(KernelError(format!("file not found: {}", path.display())));
        }
        let shape = safe::read_step(&path.to_string_lossy()).map_err(err)?;
        Ok(safe::solids(&shape).map_err(err)?.into_iter().map(Rc::new).collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use ferrocad_model::Plane;

    fn rect(w: f64, h: f64) -> PlacedProfile {
        let plane = Plane::XY;
        let c = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].map(|uv| plane.to_world(uv));
        let segments = (0..4).map(|i| Seg3::Line { a: c[i], b: c[(i + 1) % 4] }).collect();
        PlacedProfile { outer: Loop3 { segments }, holes: vec![], normal: plane.normal() }
    }

    fn volume(k: &OcctKernel, s: &Rc<Shape>) -> f64 {
        k.properties(s).unwrap().volume
    }

    impl OcctKernel {
        /// (min z, max z) of the tessellated solid.
        fn bounds_z(&self, s: &Rc<Shape>) -> (f64, f64) {
            let m = self.tessellate(s, 0.01).unwrap();
            let zs = m.positions.iter().map(|p| p[2]);
            let lo = zs.clone().fold(f64::INFINITY, f64::min);
            let hi = zs.fold(f64::NEG_INFINITY, f64::max);
            ((lo * 1e6).round() / 1e6, (hi * 1e6).round() / 1e6)
        }
    }

    #[test]
    fn box_operations() {
        let k = OcctKernel::new();
        let b = k.extrude(&rect(10.0, 20.0), 5.0, false).unwrap();
        assert!((volume(&k, &b) - 1000.0).abs() < 1e-6);

        // A huge fillet is an error, not an abort.
        let edge = EdgeRef { point: [5.0, 0.0, 5.0] };
        assert!(k.fillet(&b, &[edge], 50.0, false).is_err());
        let f = k.fillet(&b, &[edge], 1.0, false).unwrap();
        assert!(volume(&k, &f) < 1000.0);

        let top = FaceRef { point: [5.0, 5.0, 5.0], normal: [0.0, 0.0, 1.0] };
        let info = k.find_face(&b, &top).unwrap();
        assert!(info.planar && (info.point[2] - 5.0).abs() < 1e-9);
        // Found again after the face moved along its normal.
        let taller = k.extrude(&rect(10.0, 20.0), 8.0, false).unwrap();
        assert!((k.find_face(&taller, &top).unwrap().point[2] - 8.0).abs() < 1e-9);

        let tool = k.extrude_face(&b, &top, 3.0).unwrap();
        let pulled = k.boolean(BooleanOp::Union, &b, &tool).unwrap();
        assert!((volume(&k, &pulled) - 1600.0).abs() < 1e-6);

        let shelled = k.shell(&b, &[top], 1.0).unwrap();
        assert!((volume(&k, &shelled) - (1000.0 - 8.0 * 18.0 * 4.0)).abs() < 1e-3);

        let moved = k.transform(&b, &Transform::Translate([100.0, 0.0, 0.0])).unwrap();
        assert!(k.is_empty(&k.boolean(BooleanOp::Intersect, &b, &moved).unwrap()));
        let mirrored = k.transform(&b, &Transform::Mirror { origin: [0.0; 3], normal: [1.0, 0.0, 0.0] }).unwrap();
        assert!((volume(&k, &mirrored) - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn tapered_and_two_sided_extrudes() {
        let k = OcctKernel::new();
        // 10 × 10 square, 10 high, 10° taper: a frustum narrowing upwards.
        let t = 10f64.to_radians();
        let s = k.extrude_tapered(&rect(10.0, 10.0), 10.0, 0.0, t).unwrap();
        let top = 10.0 - 2.0 * 10.0 * t.tan();
        let expected = 10.0 / 3.0 * (100.0 + top * top + 10.0 * top);
        let got = volume(&k, &s);
        assert!((got - expected).abs() < 1e-3, "{got} vs {expected}");
        let b = k.bounds_z(&s);
        assert!((b.0 - 0.0).abs() < 1e-6 && (b.1 - 10.0).abs() < 1e-6, "{b:?}");
        // Two sides, no taper: 4 up and 6 down.
        let two = k.extrude_tapered(&rect(10.0, 10.0), 4.0, 6.0, 0.0).unwrap();
        assert!((volume(&k, &two) - 1000.0).abs() < 1e-6);
        assert_eq!(k.bounds_z(&two), (-6.0, 4.0));
        // Downwards with a taper.
        let down = k.extrude_tapered(&rect(10.0, 10.0), -10.0, 0.0, t).unwrap();
        assert!((volume(&k, &down) - expected).abs() < 1e-3);
        assert!((k.bounds_z(&down).0 + 10.0).abs() < 1e-6);
        // Far too steep must not crash (OCCT may still build something).
        let _ = k.extrude_tapered(&rect(10.0, 10.0), 50.0, 0.0, 60f64.to_radians());
    }

    #[test]
    fn two_distance_chamfer_and_draft() {
        let k = OcctKernel::new();
        let cube = k.extrude(&rect(10.0, 10.0), 10.0, false).unwrap();
        // Top edge along y at x = 10: a 1 × 3 bevel removes 15 mm³.
        let c = k.chamfer_two(&cube, &[EdgeRef { point: [10.0, 5.0, 10.0] }], 1.0, 3.0).unwrap();
        assert!((volume(&k, &c) - 985.0).abs() < 1e-6, "{}", volume(&k, &c));
        assert!(k.chamfer_two(&cube, &[EdgeRef { point: [10.0, 5.0, 10.0] }], 20.0, 3.0).is_err());

        // Draft the x faces by 5° around the bottom: narrower towards +z.
        let sides = [
            FaceRef { point: [0.0, 5.0, 5.0], normal: [-1.0, 0.0, 0.0] },
            FaceRef { point: [10.0, 5.0, 5.0], normal: [1.0, 0.0, 0.0] },
        ];
        let a = 5f64.to_radians();
        let d = k.draft(&cube, &sides, [0.0; 3], [0.0, 0.0, 1.0], a).unwrap();
        let expected = (10.0 + (10.0 - 20.0 * a.tan())) / 2.0 * 10.0 * 10.0;
        assert!((volume(&k, &d) - expected).abs() < 1e-3, "{} vs {expected}", volume(&k, &d));
        // Pulling the other way widens it.
        let w = k.draft(&cube, &sides, [0.0; 3], [0.0, 0.0, -1.0], a).unwrap();
        assert!(volume(&k, &w) > 1000.0);
        // Face areas are exact, also on curved faces.
        assert!((k.face_area(&cube, &sides[0]).unwrap() - 100.0).abs() < 1e-9);
        // The top face is perpendicular to the pull direction: no draft.
        let top = FaceRef { point: [5.0, 5.0, 10.0], normal: [0.0, 0.0, 1.0] };
        assert!(k.draft(&cube, &[top], [0.0; 3], [0.0, 0.0, 1.0], a).is_err());
    }

    #[test]
    fn thin_extrudes() {
        let k = OcctKernel::new();

        let pi = std::f64::consts::PI;
        let vol = |side, b| volume(&k, &k.extrude_thin(&rect(10.0, 10.0), 10.0, b, 1.0, side).unwrap());
        // Inside: 10 × 10 minus 8 × 8.
        assert!((vol(ThinSide::Inside, 0.0) - 360.0).abs() < 1e-6, "{}", vol(ThinSide::Inside, 0.0));
        // Outside: a rounded 12 × 12 square around the profile.
        let outside = (4.0 * 10.0 + pi) * 10.0;
        assert!((vol(ThinSide::Outside, 0.0) - outside).abs() < 1e-3, "{}", vol(ThinSide::Outside, 0.0));
        let center = (100.0 + 20.0 + pi / 4.0 - 81.0) * 10.0;
        assert!((vol(ThinSide::Center, 0.0) - center).abs() < 1e-3, "{}", vol(ThinSide::Center, 0.0));
        // Two sides: 10 up and 5 down.
        let two = k.extrude_thin(&rect(10.0, 10.0), 10.0, 5.0, 1.0, ThinSide::Inside).unwrap();
        assert!((volume(&k, &two) - 540.0).abs() < 1e-6);
        assert_eq!(k.bounds_z(&two), (-5.0, 10.0));
        // A wall thicker than half the profile fills it.
        let full = k.extrude_thin(&rect(4.0, 4.0), 1.0, 0.0, 3.0, ThinSide::Inside).unwrap();
        assert!((volume(&k, &full) - 16.0).abs() < 1e-6);
        // A profile with a hole gets walls around the hole too.
        let mut ring = rect(10.0, 10.0);
        ring.holes.push(Loop3 { segments: vec![Seg3::Circle { center: [5.0, 5.0, 0.0], normal: [0.0, 0.0, 1.0], radius: 2.0 }] });
        let walls = k.extrude_thin(&ring, 1.0, 0.0, 0.5, ThinSide::Inside).unwrap();
        let expected = (100.0 - 81.0) + (pi * 2.5 * 2.5 - pi * 4.0);
        assert!((volume(&k, &walls) - expected).abs() < 1e-3, "{} vs {expected}", volume(&k, &walls));
    }

    #[test]
    fn step_roundtrip_keeps_every_solid() {
        let k = OcctKernel::new();
        let a = k.extrude(&rect(10.0, 20.0), 5.0, false).unwrap();
        let b = k.transform(&a, &Transform::Translate([50.0, 0.0, 0.0])).unwrap();
        let path = std::env::temp_dir().join("ferrocad_roundtrip.step");
        k.export_step(&[a, b], &path).unwrap();
        let back = k.import_step(&path).unwrap();
        assert_eq!(back.len(), 2);
        for s in &back {
            assert!((volume(&k, s) - 1000.0).abs() < 1e-3);
        }
        assert!(k.import_step(std::path::Path::new("/nonexistent.step")).is_err());
        let garbage = std::env::temp_dir().join("ferrocad_garbage.step");
        std::fs::write(&garbage, "not a step file").unwrap();
        assert!(k.import_step(&garbage).is_err());
    }

    #[test]
    fn loft_and_sweep() {
        let k = OcctKernel::new();
        let low = rect(10.0, 10.0).outer;
        let high = Loop3 {
            segments: vec![Seg3::Circle { center: [5.0, 5.0, 20.0], normal: [0.0, 0.0, 1.0], radius: 3.0 }],
        };
        let loft = k.loft(&[low, high]).unwrap();
        assert!(volume(&k, &loft) > 0.0);

        let path = [
            Seg3::Line { a: [5.0, 5.0, 0.0], b: [5.0, 5.0, 30.0] },
            Seg3::Line { a: [5.0, 5.0, 30.0], b: [25.0, 5.0, 30.0] },
        ];
        let circle = PlacedProfile {
            outer: Loop3 {
                segments: vec![Seg3::Circle { center: [5.0, 5.0, 0.0], normal: [0.0, 0.0, 1.0], radius: 2.0 }],
            },
            holes: vec![],
            normal: [0.0, 0.0, 1.0],
        };
        let swept = k.sweep(&circle, &path).unwrap();
        assert!(volume(&k, &swept) > 0.0);
    }

    #[test]
    fn spline_profile_extrudes_smoothly() {
        let k = OcctKernel::new();
        let mut sk = ferrocad_sketch::Sketch::new();
        sk.add_spline(&[[0.0, 0.0], [20.0, -5.0], [30.0, 10.0], [10.0, 20.0], [0.0, 0.0]]);
        sk.add_line([15.0, -30.0], [15.0, 40.0]);
        let profiles = ferrocad_sketch::find_profiles(&sk);
        assert_eq!(profiles.len(), 2);
        let mut total = 0.0;
        for p in &profiles {
            let placed = PlacedProfile::new(p, &Plane::XY);
            let solid = k.extrude(&placed, 5.0, false).unwrap();
            let v = volume(&k, &solid);
            // Smooth B-spline faces: volume matches the sampled area closely.
            assert!((v - 5.0 * p.area()).abs() < 0.01 * v, "{v} vs {}", 5.0 * p.area());
            // One spline edge per region, not dozens of facets.
            assert!(k.edges(&solid).len() < 20, "{} edges", k.edges(&solid).len());
            total += v;
        }
        assert!(total > 0.0);
    }
}

#[cfg(test)]
mod hole_tests {
    use super::*;
    use ferrocad_model::*;

    #[test]
    fn counterbore_and_countersink_cut_real_solids() {
        let k = OcctKernel::new();
        let mut doc = Document::new();
        let mut sf = SketchFeature::new(Plane::XY);
        sf.sketch.add_rectangle([0.0, 0.0], [40.0, 20.0]);
        let s = doc.add("Sketch1", FeatureKind::Sketch(sf));
        doc.add(
            "Extrude1",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch: s,
                profile_point: [1.0, 1.0],
                distance: "10".into(),
                symmetric: false,
                operation: Operation::NewBody,
                distance2: None,
                taper: "0".into(),
                extent: ExtrudeExtent::Distance,
                thin: None,
            }),
        );
        let face = |x: f64| FaceRef { point: [x, 10.0, 10.0], normal: [0.0, 0.0, 1.0] };
        doc.add(
            "Hole1",
            FeatureKind::Hole(HoleFeature {
                placement: HolePlacement::Face(face(10.0)),
                diameter: "4".into(),
                depth: "0".into(),
                through: true,
                head: HoleHead::Counterbore { diameter: "8".into(), depth: "3".into() },
            }),
        );
        doc.add(
            "Hole2",
            FeatureKind::Hole(HoleFeature {
                placement: HolePlacement::Face(face(30.0)),
                diameter: "4".into(),
                depth: "0".into(),
                through: true,
                head: HoleHead::Countersink { diameter: "8".into(), angle: "90".into() },
            }),
        );
        let m = doc.rebuild(&k);
        assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
        let v = k.properties(&m.bodies[0].solid).unwrap().volume;
        let pi = std::f64::consts::PI;
        let through = pi * 4.0 * 10.0;
        let counterbore = pi * (16.0 - 4.0) * 3.0;
        // Countersink cone (r 4 → 2 over 2 mm) minus the part inside the hole.
        let cone = pi * 2.0 / 3.0 * (16.0 + 8.0 + 4.0) - pi * 4.0 * 2.0;
        let expected = 40.0 * 20.0 * 10.0 - 2.0 * through - counterbore - cone;
        assert!((v - expected).abs() < 0.5, "{v} vs {expected}");
    }
}

#[cfg(test)]
mod joint_tests {
    use super::*;
    use ferrocad_model::*;

    #[test]
    fn pin_joins_hole_on_circular_edges() {
        let k = OcctKernel::new();
        let mut doc = Document::new();
        let mut plate = SketchFeature::new(Plane::XY);
        plate.sketch.add_rectangle([0.0, 0.0], [40.0, 20.0]);
        plate.sketch.add_circle([10.0, 10.0], 3.0);
        let s1 = doc.add("Plate", FeatureKind::Sketch(plate));
        doc.add(
            "Extrude1",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch: s1,
                profile_point: [1.0, 1.0],
                distance: "5".into(),
                symmetric: false,
                operation: Operation::NewBody,
                distance2: None,
                taper: "0".into(),
                extent: ExtrudeExtent::Distance,
                thin: None,
            }),
        );
        let mut pin = SketchFeature::new(Plane::XY);
        pin.sketch.add_circle([80.0, 50.0], 3.0);
        let s2 = doc.add("Pin", FeatureKind::Sketch(pin));
        doc.add(
            "Extrude2",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch: s2,
                profile_point: [80.0, 50.0],
                distance: "15".into(),
                symmetric: false,
                operation: Operation::NewBody,
                distance2: None,
                taper: "0".into(),
                extent: ExtrudeExtent::Distance,
                thin: None,
            }),
        );
        // Bottom edge of the pin onto the top edge of the hole.
        let pin_bottom = EdgeRef { point: [83.0, 50.0, 0.0] };
        let hole_top = EdgeRef { point: [13.0, 10.0, 5.0] };
        let m0 = doc.rebuild(&k);
        let mid = |m: &Model<Rc<Shape>>, p: [f64; 3]| {
            m.bodies.iter().flat_map(|b| k.edges(&b.solid)).filter_map(|e| polyline_midpoint(&e)).min_by(|a, b| {
                ferrocad_model::geom::distance(*a, p).total_cmp(&ferrocad_model::geom::distance(*b, p))
            })
        };
        let from = EdgeRef { point: mid(&m0, pin_bottom.point).unwrap() };
        let to = EdgeRef { point: mid(&m0, hole_top.point).unwrap() };
        doc.add(
            "Joint1",
            FeatureKind::Joint(JointFeature {
                moving: vec!["Body2".into()],
                from: JointOrigin::Edge(from),
                to: JointOrigin::Edge(to),
                kind: JointKind::Rigid,
                angle: "0".into(),
                offset: "0".into(),
                flip: false,
                limits: None,
            }),
        );
        let m = doc.rebuild(&k);
        assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
        let c = k.properties(&m.bodies[1].solid).unwrap().center;
        assert!((c[0] - 10.0).abs() < 1e-6 && (c[1] - 10.0).abs() < 1e-6, "pin centre {c:?}");
        // The pin's bottom sits on the hole's top edge plane (z = 5), pointing one way or the other.
        assert!(((c[2] - 12.5).abs() < 1e-6) || ((c[2] + 2.5).abs() < 1e-6), "{c:?}");
    }
}
