//! Writes a demo part exercising most features: `cargo run -p ferrocad-model --example demo -- out.fcad`.

use ferrocad_model::*;
use ferrocad_sketch::Constraint;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "demo.fcad".into());
    let mut doc = Document::new();
    doc.params.set("width", "60 mm").unwrap();
    doc.params.set("depth", "40 mm").unwrap();
    doc.params.set("thickness", "10 mm").unwrap();

    // Base plate.
    let mut sf = SketchFeature::new(Plane::XY);
    let [bottom, right, ..] = sf.sketch.add_rectangle([0.0, 0.0], [60.0, 40.0]);
    if let ferrocad_sketch::Entity::Line { a, .. } = *sf.sketch.entity(bottom) {
        sf.sketch.set_point_fixed(a, true);
    }
    let w = sf.sketch.constrain(Constraint::Length(bottom, 60.0));
    let d = sf.sketch.constrain(Constraint::Length(right, 40.0));
    sf.set_expression(w, "width");
    sf.set_expression(d, "depth");
    let plate = doc.add("Sketch1", FeatureKind::Sketch(sf));
    doc.add(
        "Extrude1",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch: plate,
            profile_point: [1.0, 1.0],
            distance: "thickness".into(),
            symmetric: false,
            operation: Operation::NewBody,
        }),
    );

    // Round the four vertical edges.
    let edges = [[0.0, 0.0, 5.0], [60.0, 0.0, 5.0], [60.0, 40.0, 5.0], [0.0, 40.0, 5.0]]
        .map(|point| EdgeRef { point })
        .to_vec();
    doc.add("Fillet1", FeatureKind::Fillet(FilletFeature { edges, radius: "6 mm".into(), chamfer: false }));

    // A boss sketched on the top face, then extruded and joined.
    let top = FaceRef { point: [30.0, 20.0, 10.0], normal: [0.0, 0.0, 1.0] };
    let mut boss = SketchFeature::new(Plane::at_face(top.point, top.normal));
    boss.attach = Some(PlaneRef::Face(top));
    boss.sketch.add_circle([15.0, 20.0], 7.0);
    let boss_sketch = doc.add("Sketch2", FeatureKind::Sketch(boss));
    doc.add(
        "Extrude2",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch: boss_sketch,
            profile_point: [15.0, 20.0],
            distance: "12 mm".into(),
            symmetric: false,
            operation: Operation::Join,
        }),
    );

    // Hole through the boss and a pattern of small holes.
    let boss_top = FaceRef { point: [15.0, 20.0, 22.0], normal: [0.0, 0.0, 1.0] };
    doc.add(
        "Hole1",
        FeatureKind::Hole(HoleFeature { face: boss_top, diameter: "6 mm".into(), depth: "0".into(), through: true, head: HoleHead::None }),
    );
    doc.add(
        "Hole2",
        FeatureKind::Hole(HoleFeature {
            face: FaceRef { point: [45.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] },
            diameter: "4 mm".into(),
            depth: "6 mm".into(),
            through: false,
            head: HoleHead::Countersink { diameter: "8 mm".into(), angle: "90".into() },
        }),
    );
    doc.add("Chamfer1", FeatureKind::Fillet(FilletFeature {
        edges: vec![EdgeRef { point: [30.0, 0.0, 10.0] }],
        radius: "2 mm".into(),
        chamfer: true,
    }));
    doc.add(
        "Plane1",
        FeatureKind::Plane(PlaneFeature { base: PlaneRef::Origin(OriginPlane::XY), offset: "30 mm".into(), angle: "0".into() }),
    );
    std::fs::write(&path, io::save(&doc)).unwrap();
    println!("wrote {path}");
}
