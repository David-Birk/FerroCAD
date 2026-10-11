//! Save → load must reproduce the document exactly.

mod common;

use common::{plate_document, Mock};
use ferrocad_model::*;

fn full_document() -> Document {
    let (mut doc, sketch, _) = plate_document();
    doc.params.set_comment("width", "outer width – \"quoted\" ünïcode");

    let mut hole = SketchFeature::new(Plane::from_normal([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]));
    hole.sketch.add_circle([20.0, 10.0], 3.0);
    let hole_sketch = doc.add("Hole sketch", FeatureKind::Sketch(hole));
    doc.add(
        "Hole",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch: hole_sketch,
            profile_point: [20.0, 10.0],
            distance: "-thickness".into(),
            symmetric: false,
            operation: Operation::Cut,
        }),
    );
    doc.add(
        "Round",
        FeatureKind::Fillet(FilletFeature {
            edges: vec![EdgeRef { point: [20.0, 0.0, 0.0] }],
            radius: "0.5 mm".into(),
            chamfer: false,
        }),
    );
    let ring = doc.add(
        "Ring",
        FeatureKind::Revolve(RevolveFeature {
            sketch,
            profile_point: [1.0, 1.0],
            axis: RevolveAxis::SketchY,
            angle: "90".into(),
            operation: Operation::NewBody,
        }),
    );
    doc.feature_mut(ring).unwrap().suppressed = true;
    doc.set_marker(4);
    doc
}

#[test]
fn save_load_save_is_identical() {
    let doc = full_document();
    let text = io::save(&doc);
    let loaded = io::load(&text).unwrap();
    assert_eq!(io::save(&loaded), text);
    assert_eq!(loaded.marker(), 4);
    assert_eq!(loaded.features().len(), doc.features().len());
}

#[test]
fn loaded_document_rebuilds_the_same() {
    let mut doc = full_document();
    let mut loaded = io::load(&io::save(&doc)).unwrap();
    let a = doc.rebuild(&Mock);
    let b = loaded.rebuild(&Mock);
    let descs = |m: &Model<_>| m.bodies.iter().map(|b: &Body<std::rc::Rc<common::MockSolid>>| b.solid.desc.clone()).collect::<Vec<_>>();
    assert_eq!(descs(&a), descs(&b));
    for f in doc.features() {
        assert_eq!(a.status(f.id), b.status(f.id), "{}", f.name);
    }
    assert!(a.errors().next().is_none());
}

#[test]
fn new_features_after_load_get_fresh_ids() {
    let doc = full_document();
    let mut loaded = io::load(&io::save(&doc)).unwrap();
    let max = loaded.features().iter().map(|f| f.id).max().unwrap();
    loaded.set_marker(loaded.features().len());
    let id = loaded.add("New", FeatureKind::Sketch(SketchFeature::new(Plane::XY)));
    assert!(id > max);
}

#[test]
fn damaged_references_are_errors_not_panics() {
    let text = io::save(&full_document());
    // Point a line at a point that does not exist.
    let broken = text.replacen("\"a\": 0", "\"a\": 999", 1);
    assert_ne!(broken, text);
    let err = io::load(&broken).unwrap_err();
    assert!(err.0.contains("Sketch1"), "{err}");
}
