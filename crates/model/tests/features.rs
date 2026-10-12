//! The solid and construction features added on top of extrude/revolve.

mod common;

use common::{plate_document, Mock};
use ferrocad_model::*;

fn body_descs(m: &Model<std::rc::Rc<common::MockSolid>>) -> Vec<String> {
    m.bodies.iter().map(|b| format!("{}: {}", b.name, b.solid.desc)).collect()
}

fn top_face() -> FaceRef {
    FaceRef { point: [5.0, 5.0, 5.0], normal: [0.0, 0.0, 1.0] }
}

#[test]
fn sketch_follows_offset_plane() {
    let (mut doc, _, _) = plate_document();
    let plane = doc.add(
        "Plane1",
        FeatureKind::Plane(PlaneFeature {
            base: PlaneRef::Origin(OriginPlane::XY),
            offset: "thickness * 2".into(),
            angle: "0".into(),
            base2: None,
        }),
    );
    let mut sf = SketchFeature::new(Plane::XY);
    sf.attach = Some(PlaneRef::Construction(plane));
    let sketch = doc.add("Sketch2", FeatureKind::Sketch(sf));
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    assert_eq!(doc.sketch(sketch).unwrap().plane.origin, [0.0, 0.0, 10.0]);

    doc.params.set("thickness", "7").unwrap();
    doc.rebuild(&Mock);
    assert_eq!(doc.sketch(sketch).unwrap().plane.origin, [0.0, 0.0, 14.0]);

    // The plane cannot be deleted or moved behind the sketch.
    assert!(doc.remove(plane).unwrap_err().contains("Sketch2"));
    assert!(doc.move_feature(plane, 3).is_err());
}

#[test]
fn angled_plane_turns_around_its_x_axis() {
    let mut doc = Document::new();
    let p = doc.add(
        "Plane1",
        FeatureKind::Plane(PlaneFeature { base: PlaneRef::Origin(OriginPlane::XY), offset: "0".into(), angle: "90".into(), base2: None }),
    );
    let m = doc.rebuild(&Mock);
    let n = m.planes[&p].normal();
    assert!((n[1] + 1.0).abs() < 1e-9 && n[2].abs() < 1e-9, "{n:?}");
}

#[test]
fn press_pull_joins_or_cuts() {
    let (mut doc, _, _) = plate_document();
    let pp = doc.add("PP", FeatureKind::PressPull(PressPullFeature { face: top_face(), distance: "3".into() }));
    let m = doc.rebuild(&Mock);
    assert!(body_descs(&m)[0].contains("Union("), "{:?}", body_descs(&m));
    if let FeatureKind::PressPull(p) = &mut doc.feature_mut(pp).unwrap().kind {
        p.distance = "-2".into();
    }
    let m = doc.rebuild(&Mock);
    assert!(body_descs(&m)[0].contains("Subtract("));
}

#[test]
fn shell_hole_and_remove() {
    let (mut doc, _, _) = plate_document();
    doc.add("Shell1", FeatureKind::Shell(ShellFeature { faces: vec![top_face()], thickness: "1".into() }));
    doc.add(
        "Hole1",
        FeatureKind::Hole(HoleFeature { face: top_face(), diameter: "4".into(), depth: "3".into(), through: false, head: HoleHead::None }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none());
    let d = &body_descs(&m)[0];
    assert!(d.contains("shell(") && d.starts_with("Body1: Subtract("), "{d}");

    doc.add("Remove1", FeatureKind::Remove(RemoveFeature { bodies: vec!["Body1".into()] }));
    assert!(doc.rebuild(&Mock).bodies.is_empty());
    doc.add("Remove2", FeatureKind::Remove(RemoveFeature { bodies: vec!["Body1".into()] }));
    let m = doc.rebuild(&Mock);
    assert_eq!(m.errors().next().unwrap().1, "Body1 no longer exists");
}

#[test]
fn mirror_pattern_move_make_bodies() {
    let (mut doc, _, _) = plate_document();
    doc.add(
        "Mirror1",
        FeatureKind::Mirror(MirrorFeature {
            bodies: vec!["Body1".into()],
            plane: PlaneRef::Origin(OriginPlane::YZ),
            join: false,
        }),
    );
    doc.add(
        "Pattern1",
        FeatureKind::Pattern(PatternFeature {
            bodies: vec!["Body2".into()],
            kind: PatternKind::Circular { axis: AxisRef::World(2), count: "4".into(), angle: "360".into() },
            join: false,
        }),
    );
    doc.add(
        "Pattern2",
        FeatureKind::Pattern(PatternFeature {
            bodies: vec!["Body1".into()],
            kind: PatternKind::Rectangular {
                direction: AxisRef::World(0),
                count: "2".into(),
                spacing: "50".into(),
                direction2: AxisRef::World(1),
                count2: "2".into(),
                spacing2: "30".into(),
            },
            join: true,
        }),
    );
    doc.add(
        "Copy1",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Body1".into()],
            translate: ["0".into(), "0".into(), "thickness".into()],
            axis: Some(AxisRef::World(2)),
            angle: "45".into(),
            copy: true,
        }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = body_descs(&m);
    assert_eq!(d.len(), 1 + 1 + 3 + 1, "{d:#?}");
    assert!(d[1].starts_with("Body2: mirror[1.0, 0.0, 0.0]("));
    assert!(d[2].starts_with("Body3: rot90(") && d[4].starts_with("Body5: rot270("));
    // rectangular 2×2 joined: three unions
    assert_eq!(d[0].matches("Union(").count(), 3, "{}", d[0]);
    assert!(d[5].starts_with("Body6: rot45(move[0.0, 0.0, 5.0]("));
}

#[test]
fn combine_and_split() {
    let (mut doc, _, _) = plate_document();
    doc.add(
        "Copy",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Body1".into()],
            translate: ["5".into(), "0".into(), "0".into()],
            axis: None,
            angle: "0".into(),
            copy: true,
        }),
    );
    doc.add(
        "Combine1",
        FeatureKind::Combine(CombineFeature {
            target: "Body1".into(),
            tools: vec!["Body2".into()],
            op: CombineOp::Cut,
            keep_tools: false,
        }),
    );
    doc.add("Split1", FeatureKind::Split(SplitFeature { body: "Body1".into(), plane: PlaneRef::Origin(OriginPlane::YZ) }));
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = body_descs(&m);
    assert_eq!(d.len(), 2, "{d:#?}");
    assert!(d[0].starts_with("Body1: Subtract(Subtract("));
    assert!(d[1].starts_with("Body3: Intersect("));
}

#[test]
fn loft_and_sweep_use_their_sketches() {
    let (mut doc, sketch, _) = plate_document();
    let mut top = SketchFeature::new(Plane { origin: [0.0, 0.0, 20.0], ..Plane::XY });
    top.sketch.add_circle([10.0, 5.0], 3.0);
    let top = doc.add("Sketch2", FeatureKind::Sketch(top));
    doc.add(
        "Loft1",
        FeatureKind::Loft(LoftFeature {
            profiles: vec![ProfileRef { sketch, point: [1.0, 1.0] }, ProfileRef { sketch: top, point: [10.0, 5.0] }],
            operation: Operation::NewBody,
        }),
    );
    let mut path = SketchFeature::new(Plane::XZ);
    // An L-shaped path given out of order and with one line reversed.
    let l1 = path.sketch.add_line([0.0, 0.0], [0.0, 30.0]);
    let l2 = path.sketch.add_line([20.0, 30.0], [0.0, 30.0]);
    let path = doc.add("Path", FeatureKind::Sketch(path));
    doc.add(
        "Sweep1",
        FeatureKind::Sweep(SweepFeature {
            profile: ProfileRef { sketch: top, point: [10.0, 5.0] },
            path_sketch: path,
            path: vec![l2, l1],
            operation: Operation::NewBody,
        }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = body_descs(&m);
    assert_eq!(&d[1], "Body2: loft(n=2)");
    assert_eq!(&d[2], "Body3: sweep(segs=1, path=2)");
    // The loft depends on both sketches.
    assert!(doc.remove(top).unwrap_err().contains("Loft1"));
}

#[test]
fn polyline_axis_finds_lines_and_circles() {
    use ferrocad_model::document::polyline_axis;
    let (o, d) = polyline_axis(&[[1.0, 2.0, 0.0], [1.0, 2.0, 5.0]]).unwrap();
    assert_eq!(o, [1.0, 2.0, 0.0]);
    assert!((d[2] - 1.0).abs() < 1e-12);
    let circle: Vec<[f64; 3]> = (0..=32)
        .map(|i| {
            let t = i as f64 / 32.0 * std::f64::consts::TAU;
            [3.0 + 2.0 * t.cos(), -1.0 + 2.0 * t.sin(), 4.0]
        })
        .collect();
    let (c, n) = polyline_axis(&circle).unwrap();
    assert!((c[0] - 3.0).abs() < 1e-9 && (c[1] + 1.0).abs() < 1e-9 && (c[2] - 4.0).abs() < 1e-9, "{c:?}");
    assert!((n[2].abs() - 1.0).abs() < 1e-9);
}

#[test]
fn every_new_feature_roundtrips_through_fcad() {
    let (mut doc, sketch, _) = plate_document();
    let plane = doc.add(
        "Plane1",
        FeatureKind::Plane(PlaneFeature { base: PlaneRef::Face(top_face()), offset: "2".into(), angle: "10".into(), base2: None }),
    );
    let mut sf = SketchFeature::new(Plane::XY);
    sf.attach = Some(PlaneRef::Construction(plane));
    doc.add("Sketch2", FeatureKind::Sketch(sf));
    let axis = doc.add("Axis1", FeatureKind::Axis(AxisFeature { source: AxisSource::Edge(EdgeRef { point: [1.0, 2.0, 3.0] }) }));
    doc.add("Axis2", FeatureKind::Axis(AxisFeature { source: AxisSource::FaceNormal(top_face()) }));
    doc.add("PP", FeatureKind::PressPull(PressPullFeature { face: top_face(), distance: "1".into() }));
    doc.add("Shell", FeatureKind::Shell(ShellFeature { faces: vec![top_face()], thickness: "1".into() }));
    doc.add("Hole", FeatureKind::Hole(HoleFeature { face: top_face(), diameter: "3".into(), depth: "2".into(), through: true, head: HoleHead::None }));
    doc.add("Mirror", FeatureKind::Mirror(MirrorFeature { bodies: vec!["Body1".into()], plane: PlaneRef::Origin(OriginPlane::XZ), join: true }));
    doc.add(
        "PatR",
        FeatureKind::Pattern(PatternFeature {
            bodies: vec!["Body1".into()],
            kind: PatternKind::Rectangular {
                direction: AxisRef::Construction(axis),
                count: "3".into(),
                spacing: "10".into(),
                direction2: AxisRef::World(1),
                count2: "1".into(),
                spacing2: "0".into(),
            },
            join: false,
        }),
    );
    doc.add(
        "PatC",
        FeatureKind::Pattern(PatternFeature {
            bodies: vec!["Body1".into()],
            kind: PatternKind::Circular { axis: AxisRef::World(2), count: "6".into(), angle: "360".into() },
            join: true,
        }),
    );
    doc.add("Comb", FeatureKind::Combine(CombineFeature { target: "Body1".into(), tools: vec!["Body2".into()], op: CombineOp::Intersect, keep_tools: true }));
    doc.add("Split", FeatureKind::Split(SplitFeature { body: "Body1".into(), plane: PlaneRef::Construction(plane) }));
    doc.add(
        "Move",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Body1".into()],
            translate: ["1".into(), "2".into(), "3".into()],
            axis: Some(AxisRef::Construction(axis)),
            angle: "30".into(),
            copy: false,
        }),
    );
    doc.add("Loft", FeatureKind::Loft(LoftFeature { profiles: vec![ProfileRef { sketch, point: [1.0, 1.0] }; 2], operation: Operation::Join }));
    doc.add(
        "Sweep",
        FeatureKind::Sweep(SweepFeature {
            profile: ProfileRef { sketch, point: [1.0, 1.0] },
            path_sketch: sketch,
            path: vec![ferrocad_sketch::EntityId(0)],
            operation: Operation::Cut,
        }),
    );
    doc.add("Remove", FeatureKind::Remove(RemoveFeature { bodies: vec!["Body3".into()] }));

    let text = io::save(&doc);
    let back = io::load(&text).unwrap();
    assert_eq!(io::save(&back), text);
    assert_eq!(back.features().len(), doc.features().len());
}

#[test]
fn revolve_around_world_and_construction_axes() {
    let (mut doc, sketch, _) = plate_document();
    let axis = doc.add("Axis1", FeatureKind::Axis(AxisFeature { source: AxisSource::FaceNormal(top_face()) }));
    let rev = |axis| {
        FeatureKind::Revolve(RevolveFeature {
            sketch,
            profile_point: [1.0, 1.0],
            axis,
            angle: "90".into(),
            operation: Operation::NewBody,
        })
    };
    doc.add("R1", rev(RevolveAxis::Axis(AxisRef::World(1))));
    doc.add("R2", rev(RevolveAxis::Axis(AxisRef::Construction(axis))));
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = body_descs(&m);
    assert!(d[1].contains("d=[0.0, 1.0, 0.0]"), "{}", d[1]);
    assert!(d[2].contains("o=[5.0, 5.0, 5.0]"), "{}", d[2]);
    assert!(doc.remove(axis).is_err());
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
}

#[test]
fn counterbore_and_countersink_add_tools() {
    let (mut doc, _, _) = plate_document();
    let hole = |head| {
        FeatureKind::Hole(HoleFeature { face: top_face(), diameter: "4".into(), depth: "3".into(), through: false, head })
    };
    doc.add("Hole1", hole(HoleHead::Counterbore { diameter: "8".into(), depth: "2".into() }));
    doc.add("Hole2", hole(HoleHead::Countersink { diameter: "8".into(), angle: "90".into() }));
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = &body_descs(&m)[0];
    assert_eq!(d.matches("Subtract(").count(), 4, "{d}");
    assert!(d.contains("revolve("));
    doc.add("Bad", hole(HoleHead::Counterbore { diameter: "2".into(), depth: "2".into() }));
    assert!(doc.rebuild(&Mock).errors().next().is_some());
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
}

#[test]
fn projected_edges_follow_the_model() {
    let (mut doc, _, _) = plate_document();
    let m = doc.rebuild(&Mock);
    // The mock's edges are the plate outline at z = 0; project the bottom edge.
    let bottom = m.bodies[0].solid.edges.iter().find(|e| e[0][1].abs() < 1e-9 && e[1][1].abs() < 1e-9).unwrap().clone();
    let mut sf = SketchFeature::new(Plane { origin: [0.0, 0.0, 5.0], ..Plane::XY });
    ferrocad_model::project::project_edge(&mut sf, &bottom).unwrap();
    assert_eq!(sf.projections.len(), 1);
    let line = sf.projections[0].entity;
    let sketch = doc.add("Sketch2", FeatureKind::Sketch(sf));

    doc.params.set("width", "60 mm").unwrap();
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let sk = &doc.sketch(sketch).unwrap().sketch;
    let ferrocad_sketch::Entity::Line { a, b } = *sk.entity(line) else { panic!() };
    let xs = [sk.point_pos(a)[0], sk.point_pos(b)[0]];
    assert!(xs.iter().any(|x| (x - 60.0).abs() < 1e-6), "{xs:?}");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(back.sketch(sketch).unwrap().projections, doc.sketch(sketch).unwrap().projections);
}

#[test]
fn import_makes_one_body_per_solid() {
    let mut doc = Document::new();
    doc.add("Import1", FeatureKind::Import(ImportFeature { path: "/tmp/two_parts.step".into() }));
    let m = doc.rebuild(&Mock);
    assert_eq!(body_descs(&m), vec!["Body1: import(0)", "Body2: import(1)"]);
    doc.view.appearances = vec![("Body1".into(), "Brass".into())];
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(back.view, doc.view);
    assert_eq!(io::save(&back), io::save(&doc));
}

#[test]
fn joint_mates_faces_and_turns() {
    let (mut doc, _, _) = plate_document();
    // A second body, then joint its bottom onto the plate's top.
    doc.add(
        "Copy",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Body1".into()],
            translate: ["100".into(), "0".into(), "0".into()],
            axis: None,
            angle: "0".into(),
            copy: true,
        }),
    );
    let joint = doc.add(
        "Joint1",
        FeatureKind::Joint(JointFeature {
            moving: vec!["Body2".into()],
            from: JointOrigin::Face(FaceRef { point: [110.0, 5.0, 0.0], normal: [0.0, 0.0, -1.0] }),
            to: JointOrigin::Face(top_face()),
            kind: JointKind::Revolute,
            angle: "30".into(),
            offset: "0".into(),
            flip: false,
            limits: Some(("0".into(), "20".into())),
        }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    let d = &body_descs(&m)[1];
    // Normals already opposed: no alignment turn, a move, then the joint angle.
    assert!(d.starts_with("Body2: rot20(move[-105.0, 0.0, 5.0]("), "clamped to the limit: {d}");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    // The angle is a model parameter.
    let mut kind = doc.feature(joint).unwrap().kind.clone();
    assert_eq!(kind.expressions_mut()[0].1.as_str(), "30");
}

#[test]
fn components_group_bodies_for_joints_and_moves() {
    let (mut doc, _, _) = plate_document();
    let copy = |dx: &str| {
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Body1".into()],
            translate: [dx.into(), "0".into(), "0".into()],
            axis: None,
            angle: "0".into(),
            copy: true,
        })
    };
    doc.add("C1", copy("50"));
    doc.add("C2", copy("100"));
    doc.add("Comp", FeatureKind::Component(ComponentFeature { name: "Lid".into(), bodies: vec!["Body2".into(), "Body3".into()] }));
    doc.add(
        "MoveLid",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Lid".into()],
            translate: ["0".into(), "0".into(), "7".into()],
            axis: None,
            angle: "0".into(),
            copy: false,
        }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    assert_eq!(m.component_of("Body3"), Some("Lid"));
    assert_eq!(m.component_of("Body1"), None);
    let d = body_descs(&m);
    assert!(d[1].starts_with("Body2: move[0.0, 0.0, 7.0]") && d[2].starts_with("Body3: move[0.0, 0.0, 7.0]"), "{d:#?}");
    assert!(d[0].starts_with("Body1: extrude"));

    // Removing a body takes it out of the component.
    doc.add("Rm", FeatureKind::Remove(RemoveFeature { bodies: vec!["Body2".into()] }));
    let m = doc.rebuild(&Mock);
    assert_eq!(m.components, vec![("Lid".to_string(), vec!["Body3".to_string()])]);
    // Names must be unique.
    doc.add("Dup", FeatureKind::Component(ComponentFeature { name: "Lid".into(), bodies: vec!["Body1".into()] }));
    assert!(doc.rebuild(&Mock).errors().next().is_some());
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
}

#[test]
fn bodies_made_while_a_component_is_active_belong_to_it() {
    let (mut doc, sketch, _) = plate_document();
    doc.view.active_component = Some("Lid".into());
    doc.add(
        "Extrude2",
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profile_point: [1.0, 1.0],
            distance: "3".into(),
            symmetric: false,
            operation: Operation::NewBody,
            distance2: None,
            taper: "0".into(),
        }),
    );
    doc.view.active_component = None;
    doc.add(
        "Copy",
        FeatureKind::Move(MoveFeature {
            bodies: vec!["Lid".into()],
            translate: ["0".into(), "0".into(), "20".into()],
            axis: None,
            angle: "0".into(),
            copy: true,
        }),
    );
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    assert_eq!(m.components, vec![("Lid".to_string(), vec!["Body2".to_string()])]);
    assert_eq!(m.component_of("Body3"), None, "made after deactivating");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    assert_eq!(back.features()[2].component.as_deref(), Some("Lid"));
}

#[test]
fn two_sided_and_tapered_extrude() {
    let (mut doc, _, extrude) = plate_document();
    if let FeatureKind::Extrude(e) = &mut doc.feature_mut(extrude).unwrap().kind {
        e.distance2 = Some("2".into());
        e.taper = "5".into();
    }
    let m = doc.rebuild(&Mock);
    assert!(m.errors().next().is_none(), "{:?}", m.errors().collect::<Vec<_>>());
    assert_eq!(body_descs(&m)[0], "Body1: taper(segs=4, f=5, b=2, t=5.0)");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    if let FeatureKind::Extrude(e) = &mut doc.feature_mut(extrude).unwrap().kind {
        e.taper = "95".into();
    }
    assert!(doc.rebuild(&Mock).errors().next().is_some());
}

#[test]
fn chamfer_with_two_distances() {
    let (mut doc, _, _) = plate_document();
    let chamfer = doc.add(
        "Chamfer1",
        FeatureKind::Fillet(FilletFeature {
            edges: vec![EdgeRef { point: [20.0, 0.0, 0.0] }],
            radius: "1".into(),
            chamfer: true,
            distance2: Some("thickness / 2".into()),
        }),
    );
    let m = doc.rebuild(&Mock);
    assert_eq!(*m.status(chamfer), FeatureStatus::Ok);
    assert!(body_descs(&m)[0].contains("chamfer2(") && body_descs(&m)[0].ends_with("d1=1, d2=2.5)"));
    assert_eq!(doc.feature(chamfer).unwrap().kind.type_name(), "Chamfer");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    if let FeatureKind::Fillet(f) = &mut doc.feature_mut(chamfer).unwrap().kind {
        f.distance2 = Some("0".into());
    }
    assert!(matches!(doc.rebuild(&Mock).status(chamfer), FeatureStatus::Error(_)));
}

#[test]
fn draft_tilts_faces_around_the_pull_plane() {
    let (mut doc, _, _) = plate_document();
    let draft = doc.add(
        "Draft1",
        FeatureKind::Draft(DraftFeature {
            faces: vec![
                FaceRef { point: [20.0, 0.0, 2.5], normal: [0.0, -1.0, 0.0] },
                FaceRef { point: [20.0, 20.0, 2.5], normal: [0.0, 1.0, 0.0] },
            ],
            pull: PlaneRef::Origin(OriginPlane::XY),
            angle: "3".into(),
            flip: true,
        }),
    );
    let m = doc.rebuild(&Mock);
    assert_eq!(*m.status(draft), FeatureStatus::Ok);
    assert!(body_descs(&m)[0].ends_with("n=2, o=[0.0, 0.0, 0.0], pull=[-0.0, -0.0, -1.0], a=3.0)"), "{:?}", body_descs(&m));
    assert_eq!(doc.feature_mut(draft).unwrap().kind.expressions_mut().len(), 1);
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    if let FeatureKind::Draft(d) = &mut doc.feature_mut(draft).unwrap().kind {
        d.faces.clear();
    }
    assert!(matches!(doc.rebuild(&Mock).status(draft), FeatureStatus::Error(_)));
}

#[test]
fn midplane_between_two_faces() {
    let (mut doc, _, _) = plate_document();
    let mid = doc.add(
        "Plane1",
        FeatureKind::Plane(PlaneFeature {
            base: PlaneRef::Origin(OriginPlane::XY),
            offset: "0".into(),
            angle: "0".into(),
            base2: Some(PlaneRef::Face(top_face())),
        }),
    );
    let m = doc.rebuild(&Mock);
    assert_eq!(*m.status(mid), FeatureStatus::Ok);
    assert!((m.planes[&mid].origin[2] - 2.5).abs() < 1e-9);
    assert_eq!(doc.feature(mid).unwrap().kind.type_name(), "Midplane");
    let back = io::load(&io::save(&doc)).unwrap();
    assert_eq!(io::save(&back), io::save(&doc));
    // Planes that are not parallel have no midplane.
    if let FeatureKind::Plane(p) = &mut doc.feature_mut(mid).unwrap().kind {
        p.base2 = Some(PlaneRef::Origin(OriginPlane::XZ));
    }
    assert!(matches!(doc.rebuild(&Mock).status(mid), FeatureStatus::Error(_)));
}
