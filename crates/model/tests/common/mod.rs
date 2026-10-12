//! Shared test helpers: a mock kernel and a parametric plate document.
#![allow(dead_code)]

use ferrocad_model::geom::Vec3;
use ferrocad_model::*;
use ferrocad_sketch::{Constraint, Entity};
use std::path::Path;
use std::rc::Rc;

#[derive(Debug)]
pub struct MockSolid {
    pub desc: String,
    pub edges: Vec<Vec<Vec3>>,
}

pub struct Mock;

impl Kernel for Mock {
    type Solid = Rc<MockSolid>;

    fn extrude(&self, p: &PlacedProfile, d: f64, sym: bool) -> Result<Self::Solid, KernelError> {
        let mut area = 0.0;
        let mut edges = Vec::new();
        for s in &p.outer.segments {
            if let Seg3::Line { a, b } = s {
                area += 0.5 * (a[0] * b[1] - b[0] * a[1]);
                edges.push(vec![*a, *b]);
            }
        }
        let desc = format!("extrude(area={area:.1}, d={d}, holes={}, sym={sym})", p.holes.len());
        Ok(Rc::new(MockSolid { desc, edges }))
    }

    fn revolve(&self, p: &PlacedProfile, o: Vec3, d: Vec3, a: f64) -> Result<Self::Solid, KernelError> {
        let desc = format!(
            "revolve(segs={}, o={o:?}, d={d:?}, deg={:.1})",
            p.outer.segments.len(),
            a.to_degrees()
        );
        Ok(Rc::new(MockSolid { desc, edges: vec![] }))
    }

    fn boolean(&self, op: BooleanOp, a: &Self::Solid, b: &Self::Solid) -> Result<Self::Solid, KernelError> {
        let desc = format!("{op:?}({}, {})", a.desc, b.desc);
        Ok(Rc::new(MockSolid { desc, edges: a.edges.clone() }))
    }

    fn fillet(&self, s: &Self::Solid, e: &[EdgeRef], r: f64, chamfer: bool) -> Result<Self::Solid, KernelError> {
        let desc = format!("fillet({}, n={}, r={r}, chamfer={chamfer})", s.desc, e.len());
        Ok(Rc::new(MockSolid { desc, edges: s.edges.clone() }))
    }

    fn extrude_face(&self, s: &Self::Solid, f: &FaceRef, d: f64) -> Result<Self::Solid, KernelError> {
        let desc = format!("face({}, n={:?}, d={d})", s.desc, f.normal);
        Ok(Rc::new(MockSolid { desc, edges: vec![] }))
    }

    fn shell(&self, s: &Self::Solid, faces: &[FaceRef], t: f64) -> Result<Self::Solid, KernelError> {
        let desc = format!("shell({}, n={}, t={t})", s.desc, faces.len());
        Ok(Rc::new(MockSolid { desc, edges: s.edges.clone() }))
    }

    fn transform(&self, s: &Self::Solid, t: &Transform) -> Result<Self::Solid, KernelError> {
        let t = match t {
            Transform::Translate(v) => format!("move{v:?}"),
            Transform::Rotate { angle, .. } => format!("rot{:.0}", angle.to_degrees()),
            Transform::Mirror { normal, .. } => format!("mirror{normal:?}"),
        };
        Ok(Rc::new(MockSolid { desc: format!("{t}({})", s.desc), edges: s.edges.clone() }))
    }

    fn loft(&self, sections: &[Loop3]) -> Result<Self::Solid, KernelError> {
        Ok(Rc::new(MockSolid { desc: format!("loft(n={})", sections.len()), edges: vec![] }))
    }

    fn sweep(&self, p: &PlacedProfile, path: &[Seg3]) -> Result<Self::Solid, KernelError> {
        let desc = format!("sweep(segs={}, path={})", p.outer.segments.len(), path.len());
        Ok(Rc::new(MockSolid { desc, edges: vec![] }))
    }

    /// Every face query hits, as a flat face exactly at the reference.
    fn find_face(&self, _: &Self::Solid, f: &FaceRef) -> Option<FaceInfo> {
        Some(FaceInfo { point: f.point, normal: f.normal, planar: true })
    }

    /// Empty when the description says so (tests build such solids).
    fn is_empty(&self, s: &Self::Solid) -> bool {
        s.desc.contains("empty")
    }

    fn import_step(&self, path: &Path) -> Result<Vec<Self::Solid>, KernelError> {
        let n = if path.to_string_lossy().contains("two") { 2 } else { 1 };
        Ok((0..n).map(|i| Rc::new(MockSolid { desc: format!("import({i})"), edges: vec![] })).collect())
    }

    fn properties(&self, _: &Self::Solid) -> Option<MassProperties> {
        None
    }

    fn tessellate(&self, _: &Self::Solid, _: f64) -> Result<TriMesh, KernelError> {
        Ok(TriMesh::default())
    }

    fn edges(&self, s: &Self::Solid) -> Vec<Vec<Vec3>> {
        s.edges.clone()
    }

    fn export_step(&self, _: &[Self::Solid], _: &Path) -> Result<(), KernelError> {
        Ok(())
    }

    fn export_stl(&self, _: &Self::Solid, _: &Path) -> Result<(), KernelError> {
        Ok(())
    }
}

/// Parametric plate: `width` × `width/2`, extruded by `thickness`.
pub fn plate_document() -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new();
    doc.params.set("width", "40 mm").unwrap();
    doc.params.set("thickness", "5").unwrap();

    let mut sf = SketchFeature::new(Plane::XY);
    let [bottom, right, ..] = sf.sketch.add_rectangle([0.0, 0.0], [30.0, 10.0]);
    let origin = match *sf.sketch.entity(bottom) {
        Entity::Line { a, .. } => a,
        _ => unreachable!(),
    };
    sf.sketch.set_point_fixed(origin, true);
    let w = sf.sketch.constrain(Constraint::Length(bottom, 30.0));
    let h = sf.sketch.constrain(Constraint::Length(right, 10.0));
    sf.set_expression(w, "width");
    sf.set_expression(h, "width / 2");
    let sketch = doc.add("Sketch1", FeatureKind::Sketch(sf));

    let extrude = doc.add(
        "Extrude1",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profile_point: [1.0, 1.0],
            distance: "thickness".into(),
            symmetric: false,
            operation: Operation::NewBody,
        }),
    );
    (doc, sketch, extrude)
}

