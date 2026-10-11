//! Timeline behaviour against a mock kernel that records what it was asked
//! to do instead of building real geometry.

mod common;

use common::{plate_document, Mock};
use ferrocad_model::*;

#[test]
fn parameters_drive_the_model() {
    let (mut doc, _, extrude) = plate_document();
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(extrude), FeatureStatus::Ok);
    assert_eq!(model.bodies.len(), 1);
    assert_eq!(model.bodies[0].solid.desc, "extrude(area=800.0, d=5, holes=0, sym=false)");

    doc.params.set("width", "6 cm").unwrap();
    doc.params.set("thickness", "width / 10").unwrap();
    let model = doc.rebuild(&Mock);
    assert_eq!(model.bodies[0].solid.desc, "extrude(area=1800.0, d=6, holes=0, sym=false)");
}

#[test]
fn rollback_marker_and_suppression() {
    let (mut doc, _, extrude) = plate_document();
    doc.set_marker(1);
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(extrude), FeatureStatus::RolledBack);
    assert!(model.bodies.is_empty());

    doc.set_marker(2);
    doc.feature_mut(extrude).unwrap().suppressed = true;
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(extrude), FeatureStatus::Suppressed);
    assert!(model.bodies.is_empty());
}

#[test]
fn errors_do_not_stop_later_features() {
    let (mut doc, sketch, extrude) = plate_document();
    if let FeatureKind::Extrude(e) = &mut doc.feature_mut(extrude).unwrap().kind {
        e.distance = "nope * 2".into();
    }
    let second = doc.add(
        "Extrude2",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profile_point: [1.0, 1.0],
            distance: "3".into(),
            symmetric: true,
            operation: Operation::NewBody,
        }),
    );
    let model = doc.rebuild(&Mock);
    assert!(matches!(model.status(extrude), FeatureStatus::Error(e) if e.contains("nope")));
    assert_eq!(*model.status(second), FeatureStatus::Ok);
    assert_eq!(model.bodies.len(), 1);
    assert_eq!(model.bodies[0].name, "Body1");
}

#[test]
fn cut_and_join() {
    let (mut doc, sketch, _) = plate_document();
    let mut hole = SketchFeature::new(Plane::XY);
    hole.sketch.add_circle([20.0, 10.0], 3.0);
    let hole_sketch = doc.add("Sketch2", FeatureKind::Sketch(hole));
    doc.add(
        "Cut",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch: hole_sketch,
            profile_point: [20.0, 10.0],
            distance: "thickness".into(),
            symmetric: false,
            operation: Operation::Cut,
        }),
    );
    doc.add(
        "Boss",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profile_point: [1.0, 1.0],
            distance: "-2".into(),
            symmetric: false,
            operation: Operation::Join,
        }),
    );
    let model = doc.rebuild(&Mock);
    assert!(model.errors().next().is_none(), "{:?}", model.errors().collect::<Vec<_>>());
    assert_eq!(model.bodies.len(), 1);
    let desc = &model.bodies[0].solid.desc;
    assert!(desc.starts_with("Union(Subtract(extrude(area=800.0"), "{desc}");
    assert!(desc.ends_with("d=-2, holes=0, sym=false))"), "{desc}");
}

#[test]
fn fillet_finds_its_body_and_survives_small_changes() {
    let (mut doc, _, _) = plate_document();
    // Edge along the bottom of the plate: midpoint (20, 0, 0).
    let fillet = doc.add(
        "Fillet1",
        FeatureKind::Fillet(FilletFeature {
            edges: vec![EdgeRef { point: [20.0, 0.0, 0.0] }],
            radius: "1".into(),
            chamfer: false,
        }),
    );
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(fillet), FeatureStatus::Ok);
    assert!(model.bodies[0].solid.desc.starts_with("fillet("));

    // Small change: edge midpoint moves 0.5 mm – still found.
    doc.params.set("width", "41").unwrap();
    assert_eq!(*doc.rebuild(&Mock).status(fillet), FeatureStatus::Ok);

    // Large change: the edge is gone from where it was.
    doc.params.set("width", "80").unwrap();
    assert!(matches!(doc.rebuild(&Mock).status(fillet), FeatureStatus::Error(_)));
}

#[test]
fn dependency_rules() {
    let (mut doc, sketch, extrude) = plate_document();
    assert!(doc.remove(sketch).is_err());
    assert!(doc.move_feature(extrude, 0).is_err());
    assert_eq!(doc.index_of(extrude), Some(1));
    doc.remove(extrude).unwrap();
    doc.remove(sketch).unwrap();
    assert!(doc.features().is_empty());
    assert_eq!(doc.marker(), 0);
}

#[test]
fn profile_point_selects_hole_region() {
    let mut doc = Document::new();
    let mut sf = SketchFeature::new(Plane::XY);
    sf.sketch.add_rectangle([0.0, 0.0], [40.0, 20.0]);
    sf.sketch.add_circle([20.0, 10.0], 5.0);
    let sketch = doc.add("Sketch1", FeatureKind::Sketch(sf));
    let plate = doc.add(
        "Plate",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profile_point: [2.0, 2.0],
            distance: "4".into(),
            symmetric: false,
            operation: Operation::NewBody,
        }),
    );
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(plate), FeatureStatus::Ok);
    assert!(model.bodies[0].solid.desc.contains("holes=1"));
}

#[test]
fn revolve_about_sketch_axis_and_line() {
    let mut doc = Document::new();
    doc.params.set("angle", "180").unwrap();
    // Profile on the XZ plane, axis = sketch Y (world Z) or a construction line.
    let mut sf = SketchFeature::new(Plane::XZ);
    sf.sketch.add_rectangle([5.0, 0.0], [10.0, 20.0]);
    let axis = sf.sketch.add_line([0.0, 0.0], [0.0, 10.0]);
    sf.sketch.set_construction(axis, true);
    let sketch = doc.add("Sketch1", FeatureKind::Sketch(sf));

    let rev = |axis| {
        FeatureKind::Revolve(RevolveFeature {
            sketch,
            profile_point: [7.0, 5.0],
            axis,
            angle: "angle * 2".into(),
            operation: Operation::NewBody,
        })
    };
    let a = doc.add("Revolve1", rev(RevolveAxis::SketchY));
    let b = doc.add("Revolve2", rev(RevolveAxis::Line(axis)));
    let model = doc.rebuild(&Mock);
    assert_eq!(*model.status(a), FeatureStatus::Ok);
    assert_eq!(*model.status(b), FeatureStatus::Ok);
    assert_eq!(model.bodies[0].solid.desc, "revolve(segs=4, o=[0.0, 0.0, 0.0], d=[0.0, 0.0, 1.0], deg=360.0)");
    assert_eq!(model.bodies[1].solid.desc, model.bodies[0].solid.desc);

    doc.params.set("angle", "200").unwrap();
    assert!(matches!(doc.rebuild(&Mock).status(a), FeatureStatus::Error(e) if e.contains("360")));

    // The sketch cannot be deleted while a revolve uses it.
    assert!(doc.remove(sketch).is_err());
}
