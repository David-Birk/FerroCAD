//! Headless end-to-end tests: drive the real app with simulated clicks and
//! keys, like a user would.

use crate::app::{Command, FerroApp, Mode};
use crate::features_ui::{FeatureType, Pick};
use crate::sketch_ui::{ConstraintKind, SketchCommand, Tool};
use egui::{Event, Key, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use ferrocad_model::{EdgeRef, FaceRef, FeatureKind, OriginPlane, PlaneRef};
use ferrocad_sketch::Constraint;
use ferrocad_view::Viewport;

fn harness() -> Harness<'static, FerroApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 950.0))
        .with_step_dt(0.05)
        .build_eframe(|cc| {
            crate::theme::setup(&cc.egui_ctx);
            FerroApp::new()
        });
    h.run_steps(3);
    h
}

fn viewport(app: &FerroApp) -> Viewport {
    let r = app.viewport_rect;
    Viewport { x: r.min.x as f64, y: r.min.y as f64, width: r.width() as f64, height: r.height() as f64 }
}

fn screen(app: &FerroApp, world: [f64; 3]) -> Pos2 {
    let s = app.camera.project(world, &viewport(app)).expect("in front of the camera");
    egui::pos2(s[0] as f32, s[1] as f32)
}

/// Click at a screen position: move, press, release over three frames.
fn click(h: &mut Harness<'static, FerroApp>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

/// Click a world position on the sketch/model.
fn click_at(h: &mut Harness<'static, FerroApp>, world: [f64; 3]) {
    let p = screen(h.state(), world);
    click(h, p);
}

fn type_text(h: &mut Harness<'static, FerroApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(1);
}

fn press(h: &mut Harness<'static, FerroApp>, key: Key) {
    h.key_press(key);
    h.run_steps(2);
}

fn run(h: &mut Harness<'static, FerroApp>, cmd: Command) {
    h.state_mut().run(cmd);
    h.run_steps(3);
}

fn no_errors(app: &FerroApp) {
    let errors: Vec<_> = app.model.errors().collect();
    assert!(errors.is_empty(), "model errors: {errors:?}");
}

fn volume(app: &FerroApp) -> f64 {
    use ferrocad_model::Kernel;
    app.model.bodies.iter().filter_map(|b| app.kernel.properties(&b.solid)).map(|p| p.volume).sum()
}

#[test]
fn sketch_extrude_press_pull_fillet_workflow() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15); // camera animation

    // Rectangle with typed width and height.
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Rectangle)));
    let corner = screen(h.state(), [5.0, 5.0, 0.0]);
    click(&mut h, corner);
    assert!(h.state().viewport_rect.contains(corner));
    let away = screen(h.state(), [12.0, 9.0, 0.0]);
    h.event(Event::PointerMoved(away));
    h.run_steps(2);
    type_text(&mut h, "30");
    press(&mut h, Key::Tab);
    type_text(&mut h, "15");
    press(&mut h, Key::Enter);

    let Mode::Sketch(sess) = &h.state().mode else { panic!("not sketching") };
    let sf = h.state().doc.sketch(sess.feature).unwrap();
    let lengths: Vec<f64> = sf
        .sketch
        .constraints()
        .filter_map(|(_, c)| if let Constraint::Length(_, v) = c { Some(*v) } else { None })
        .collect();
    assert_eq!(lengths, vec![30.0, 15.0], "typed values become dimensions");
    let mut shown: Vec<&String> = sess.dim_edits.values().collect();
    shown.sort();
    assert_eq!(shown, ["15 mm", "30 mm"], "typed values show their unit");
    assert_eq!(sf.sketch.entities().count(), 4);

    run(&mut h, Command::FinishSketch);
    assert!(matches!(h.state().mode, Mode::Model));

    // Extrude: click inside the profile, Enter.
    run(&mut h, Command::Extrude);
    let inside = screen(h.state(), [20.0, 12.0, 0.0]);
    click(&mut h, inside);
    press(&mut h, Key::Enter);
    h.run_steps(5);
    assert_eq!(h.state().bodies.len(), 1, "{:?}", h.state().message);
    no_errors(h.state());
    let v0 = volume(h.state());
    assert!((v0 - 30.0 * 15.0 * 10.0).abs() < 1e-6, "{v0}");

    // Press Pull the top face by the default 5 mm.
    h.state_mut().selection = vec![Pick::Face(FaceRef { point: [20.0, 12.0, 10.0], normal: [0.0, 0.0, 1.0] })];
    run(&mut h, Command::Feature(FeatureType::PressPull));
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let v1 = volume(h.state());
    assert!((v1 - 30.0 * 15.0 * 15.0).abs() < 1e-6, "{v1}");

    // Fillet a vertical edge picked from the displayed edges.
    let edge = h.state().bodies[0]
        .edges
        .iter()
        .find(|e| e.len() == 2 && (e[0][2] - e[1][2]).abs() > 10.0)
        .and_then(|e| ferrocad_model::kernel::polyline_midpoint(e))
        .expect("a vertical edge");
    h.state_mut().selection = vec![Pick::Edge(EdgeRef { point: edge })];
    run(&mut h, Command::Fillet);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert!(volume(h.state()) < v1);

    // A fillet that cannot work is an error on the feature, not a crash.
    let id = h.state().doc.features().last().unwrap().id;
    if let FeatureKind::Fillet(f) = &mut h.state_mut().doc.feature_mut(id).unwrap().kind {
        f.radius = "50 mm".into();
    }
    h.state_mut().rebuild();
    assert!(h.state().model.errors().next().is_some());

    // Undo everything, redo once.
    for _ in 0..12 {
        run(&mut h, Command::Undo);
    }
    run(&mut h, Command::Redo);
}

#[test]
fn every_command_opens_and_cancels() {
    let mut h = harness();
    // Something to work on: a box from the demo-style sketch.
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let Mode::Sketch(sess) = &h.state().mode else { panic!() };
    let id = sess.feature;
    h.state_mut().doc.sketch_mut(id).unwrap().sketch.add_rectangle([0.0, 0.0], [20.0, 10.0]);
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Extrude);
    click_at(&mut h, [5.0, 5.0, 0.0]);
    press(&mut h, Key::Enter);
    assert_eq!(h.state().bodies.len(), 1);

    let all = [
        FeatureType::PressPull,
        FeatureType::Shell,
        FeatureType::Draft,
        FeatureType::Hole,
        FeatureType::Mirror,
        FeatureType::RectPattern,
        FeatureType::CircPattern,
        FeatureType::Combine,
        FeatureType::Split,
        FeatureType::Move,
        FeatureType::Loft,
        FeatureType::Sweep,
        FeatureType::Plane,
        FeatureType::Midplane,
        FeatureType::AxisEdge,
        FeatureType::AxisFace,
    ];
    for ty in all {
        run(&mut h, Command::Feature(ty));
        assert!(matches!(h.state().mode, Mode::Feature(_)), "{ty:?}");
        // Hover over the body so every picker runs once.
        let p = screen(h.state(), [10.0, 5.0, 10.0]);
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
        press(&mut h, Key::Escape);
        assert!(matches!(h.state().mode, Mode::Model), "{ty:?} did not cancel");
    }
    for cmd in [Command::Measure, Command::CreateSketch, Command::Fillet, Command::Chamfer, Command::Revolve] {
        run(&mut h, cmd);
        let p = screen(h.state(), [10.0, 5.0, 10.0]);
        click(&mut h, p);
        press(&mut h, Key::Escape);
        if matches!(h.state().mode, Mode::Sketch(_)) {
            run(&mut h, Command::FinishSketch);
        }
    }
    run(&mut h, Command::Section);
    assert!(h.state().section.is_some());
    h.run_steps(3);
    run(&mut h, Command::Section);
    run(&mut h, Command::Parameters);
    run(&mut h, Command::Properties);

    // Mirror the body with the dialog, picking the YZ plane by reference.
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::Feature(FeatureType::Mirror));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.pick(Pick::Plane(PlaneRef::Origin(OriginPlane::YZ)));
    }
    h.run_steps(3);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 2);
}

#[test]
fn sketch_tools_by_clicking() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let tool = |h: &mut Harness<'static, FerroApp>, t: Tool| run(h, Command::Sketch(SketchCommand::Tool(t)));

    // Two crossing lines, then trim one end.
    tool(&mut h, Tool::Line);
    for p in [(0.0, 0.0), (40.0, 0.0)] {
        click_at(&mut h, [p.0, p.1, 0.0]);
    }
    press(&mut h, Key::Escape);
    for p in [(20.0, -10.0), (20.0, 10.0)] {
        click_at(&mut h, [p.0, p.1, 0.0]);
    }
    press(&mut h, Key::Escape);
    tool(&mut h, Tool::Trim);
    click_at(&mut h, [30.0, 0.0, 0.0]);

    // Circle, polygon, slot, centre rectangle, 3-point arc.
    tool(&mut h, Tool::Circle);
    click_at(&mut h, [60.0, 0.0, 0.0]);
    click_at(&mut h, [65.0, 0.0, 0.0]);
    tool(&mut h, Tool::Polygon);
    click_at(&mut h, [0.0, 40.0, 0.0]);
    click_at(&mut h, [8.0, 40.0, 0.0]);
    tool(&mut h, Tool::Slot);
    for p in [(30.0, 40.0), (50.0, 40.0), (40.0, 43.0)] {
        click_at(&mut h, [p.0, p.1, 0.0]);
    }
    tool(&mut h, Tool::CenterRectangle);
    click_at(&mut h, [0.0, -40.0, 0.0]);
    click_at(&mut h, [6.0, -36.0, 0.0]);
    tool(&mut h, Tool::Arc3);
    for p in [(30.0, -40.0), (50.0, -40.0), (40.0, -32.0)] {
        click_at(&mut h, [p.0, p.1, 0.0]);
    }
    // Offset the circle outwards, dimension it, constrain the trimmed line.
    tool(&mut h, Tool::Offset);
    click_at(&mut h, [65.0, 0.0, 0.0]);
    click_at(&mut h, [75.0, 0.0, 0.0]);
    tool(&mut h, Tool::Dimension);
    click_at(&mut h, [60.0, 5.0, 0.0]);
    press(&mut h, Key::Enter);
    run(&mut h, Command::Sketch(SketchCommand::Constrain(ConstraintKind::Horizontal)));
    click_at(&mut h, [10.0, 0.0, 0.0]);
    press(&mut h, Key::Escape);

    let Mode::Sketch(sess) = &h.state().mode else { panic!("left sketch mode") };
    let sf = h.state().doc.sketch(sess.feature).unwrap();
    let n = sf.sketch.entities().count();
    // 2 lines – 1 trimmed piece kept + circle + offset circle + hexagon (6 +
    // construction circle) + slot (4) + rectangle (4 + diagonal) + arc.
    assert!(n >= 20, "only {n} entities");
    assert!(sf.sketch.clone().solve().is_ok());
    let profiles = ferrocad_sketch::find_profiles(&sf.sketch);
    assert!(profiles.len() >= 4, "{} profiles", profiles.len());
    run(&mut h, Command::FinishSketch);
    no_errors(h.state());
}

/// Renders screenshots of states that are hard to reach by hand, for visual
/// review: `SHOTS=/tmp/shots cargo test --release -p ferrocad screenshots -- --ignored`.
#[test]
#[ignore]
fn screenshots() {
    let dir = std::env::var("SHOTS").unwrap_or_else(|_| "/tmp".into());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 950.0))
        .with_step_dt(0.05)
        .wgpu()
        .build_eframe(|cc| {
            crate::theme::setup(&cc.egui_ctx);
            FerroApp::new()
        });
    h.run_steps(3);
    let shot = |h: &mut Harness<'static, FerroApp>, name: &str| {
        h.run_steps(2);
        let img = h.render().expect("render");
        img.save(format!("{dir}/{name}.png")).expect("save");
    };
    // Sketch with a typed rectangle in progress.
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let id = match &h.state().mode {
        Mode::Sketch(s) => s.feature,
        _ => unreachable!(),
    };
    h.state_mut().doc.sketch_mut(id).unwrap().sketch.add_circle([-20.0, 10.0], 6.0);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Rectangle)));
    click_at(&mut h, [5.0, 5.0, 0.0]);
    let away = screen(h.state(), [30.0, 20.0, 0.0]);
    h.event(Event::PointerMoved(away));
    h.run_steps(2);
    type_text(&mut h, "30");
    shot(&mut h, "1_sketch_typing");
    press(&mut h, Key::Tab);
    type_text(&mut h, "15");
    press(&mut h, Key::Enter);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Dimension)));
    click_at(&mut h, [-20.0, 16.0, 0.0]);
    press(&mut h, Key::Enter);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Select)));
    shot(&mut h, "2_sketch_done");
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Extrude);
    click_at(&mut h, [20.0, 12.0, 0.0]);
    h.run_steps(10);
    shot(&mut h, "3_extrude_preview");
    press(&mut h, Key::Enter);
    run(&mut h, Command::Home);
    h.run_steps(15);
    h.state_mut().selection = vec![Pick::Face(FaceRef { point: [20.0, 12.0, 10.0], normal: [0.0, 0.0, 1.0] })];
    shot(&mut h, "4_face_selected");
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::Feature(FeatureType::RectPattern));
    h.run_steps(10);
    shot(&mut h, "5_pattern_dialog");
    press(&mut h, Key::Enter);
    run(&mut h, Command::Section);
    h.run_steps(5);
    shot(&mut h, "6_section");
    run(&mut h, Command::Section);

    // Hole dialog on the top face, then measure.
    h.state_mut().selection = vec![Pick::Face(FaceRef { point: [20.0, 12.0, 10.0], normal: [0.0, 0.0, 1.0] })];
    run(&mut h, Command::Feature(FeatureType::Hole));
    h.run_steps(5);
    shot(&mut h, "7_hole_dialog");
    press(&mut h, Key::Enter);
    run(&mut h, Command::Measure);
    click_at(&mut h, [5.0, 5.0, 10.0]);
    click_at(&mut h, [35.0, 20.0, 10.0]);
    shot(&mut h, "8_measure");
    press(&mut h, Key::Escape);

    // A sketch showing the newer tools.
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let id = match &h.state().mode {
        Mode::Sketch(s) => s.feature,
        _ => unreachable!(),
    };
    {
        let sk = &mut h.state_mut().doc.sketch_mut(id).unwrap().sketch;
        sk.add_polygon([-40.0, 40.0], [-30.0, 40.0], 6).unwrap();
        sk.add_slot([0.0, 40.0], [25.0, 40.0], 8.0).unwrap();
        sk.add_spline(&[[40.0, 30.0], [50.0, 45.0], [60.0, 35.0], [70.0, 50.0]]);
        let [bottom, ..] = sk.add_rectangle([-40.0, -30.0], [-10.0, -10.0]);
        let corner = match *sk.entity(bottom) {
            ferrocad_sketch::Entity::Line { b, .. } => b,
            _ => unreachable!(),
        };
        sk.fillet_corner(corner, 4.0).unwrap();
        let c = sk.add_circle([30.0, -20.0], 6.0);
        sk.offset(&[c], 3.0, [45.0, -20.0]).unwrap();
        sk.solve().unwrap();
    }
    run(&mut h, Command::Fit);
    h.run_steps(15);
    shot(&mut h, "9_sketch_tools");

    // Two-sided, tapered extrude of the hexagon.
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Extrude);
    click_at(&mut h, [-35.0, 40.0, 0.0]);
    if let Mode::Extrude(e) = &mut h.state_mut().mode {
        e.two_sides = true;
        e.distance2 = "5 mm".into();
        e.taper = "10".into();
    }
    h.run_steps(10);
    shot(&mut h, "10_extrude_two_sides_taper");
    press(&mut h, Key::Enter);

    // Draft two side faces of the first body, pulling up from its top face.
    h.state_mut().selection = vec![
        Pick::Face(FaceRef { point: [5.0, 12.0, 5.0], normal: [-1.0, 0.0, 0.0] }),
        Pick::Face(FaceRef { point: [20.0, 5.0, 5.0], normal: [0.0, -1.0, 0.0] }),
    ];
    run(&mut h, Command::Feature(FeatureType::Draft));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.sels[1].picks = vec![Pick::Plane(PlaneRef::Origin(OriginPlane::XY))];
        f.exprs[0].text = "10".into();
    }
    run(&mut h, Command::Home);
    h.run_steps(15);
    shot(&mut h, "11_draft_dialog");
    press(&mut h, Key::Enter);
    no_errors(h.state());

    // Text and points on a new sketch, then holes on the points.
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Text)));
    click_at(&mut h, [5.0, -30.0, 0.0]);
    type_text(&mut h, "Voron 2.4");
    if let Mode::Sketch(s) = &mut h.state_mut().mode {
        s.text_height = "8 mm".into();
    }
    h.run_steps(5);
    shot(&mut h, "12_sketch_text");
    press(&mut h, Key::Enter);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Point)));
    for x in [10.0, 20.0, 30.0] {
        click_at(&mut h, [x, 12.0, 0.0]);
    }
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Feature(FeatureType::Hole));
    for x in [10.0, 20.0, 30.0] {
        click_at(&mut h, [x, 12.0, 0.0]);
    }
    if let Mode::Feature(s) = &mut h.state_mut().mode {
        s.flags[0].1 = true;
        s.flags[1].1 = true;
    }
    run(&mut h, Command::Home);
    h.run_steps(15);
    shot(&mut h, "13_holes_on_points");
    press(&mut h, Key::Enter);
}

#[test]
fn closed_spline_by_clicking_extrudes() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Spline)));
    for p in [(0.0, 0.0), (20.0, -5.0), (30.0, 10.0), (10.0, 20.0), (0.0, 0.0)] {
        click_at(&mut h, [p.0, p.1, 0.0]);
    }
    press(&mut h, Key::Enter);
    {
        let Mode::Sketch(sess) = &h.state().mode else { panic!() };
        let sf = h.state().doc.sketch(sess.feature).unwrap();
        assert_eq!(sf.sketch.entities().count(), 1);
        let profiles = ferrocad_sketch::find_profiles(&sf.sketch);
        assert_eq!(profiles.len(), 1, "the last click closes the spline");
    }
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Extrude);
    click_at(&mut h, [12.0, 6.0, 0.0]);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 1);
    assert!(volume(h.state()) > 1000.0, "{}", volume(h.state()));
    // Survives save/load.
    let text = ferrocad_model::io::save(&h.state().doc);
    let back = ferrocad_model::io::load(&text).unwrap();
    assert_eq!(ferrocad_model::io::save(&back), text);
}

/// A 30 × 20 × 10 box at the origin, viewed from the home view.
fn box_model(h: &mut Harness<'static, FerroApp>) {
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([0.0, 0.0], [30.0, 20.0]);
    let app = h.state_mut();
    let s = app.doc.add("Sketch1", FeatureKind::Sketch(sf));
    app.doc.add(
        "Extrude1",
        FeatureKind::Extrude(ferrocad_model::ExtrudeFeature {
            sketch: s,
            profile_point: [1.0, 1.0],
            distance: "10".into(),
            symmetric: false,
            operation: ferrocad_model::Operation::NewBody,
            distance2: None,
            taper: "0".into(),
            extent: ferrocad_model::ExtrudeExtent::Distance,
            thin: None,
        }),
    );
    app.hidden_sketches.insert(s);
    app.rebuild();
    app.run(Command::Home);
    h.run_steps(15);
}

#[test]
fn sketch_on_clicked_face_then_join_and_measure() {
    let mut h = harness();
    box_model(&mut h);

    // Click the top face in Create Sketch.
    run(&mut h, Command::CreateSketch);
    click_at(&mut h, [15.0, 10.0, 10.0]);
    h.run_steps(15);
    let Mode::Sketch(sess) = &h.state().mode else { panic!("no sketch started: {:?}", h.state().message) };
    let id = sess.feature;
    let sf = h.state().doc.sketch(id).unwrap();
    assert!(matches!(sf.attach, Some(PlaneRef::Face(_))), "attached to the face");
    assert!((sf.plane.origin[2] - 10.0).abs() < 1e-6, "{:?}", sf.plane.origin);
    let plane = sf.plane;

    // Circle on the face, extruded: Join is the default.
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Circle)));
    let c = plane.to_local([15.0, 10.0, 10.0]);
    click_at(&mut h, plane.to_world(c));
    click_at(&mut h, plane.to_world([c[0] + 4.0, c[1]]));
    run(&mut h, Command::FinishSketch);
    run(&mut h, Command::Extrude);
    assert!(matches!(&h.state().mode, Mode::Extrude(e) if e.operation == ferrocad_model::Operation::Join));
    click_at(&mut h, plane.to_world(c));
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 1, "joined into the box");
    assert!(volume(h.state()) > 6000.0);

    // The face sketch follows when the box gets taller.
    let extrude = h.state().doc.features()[1].id;
    if let FeatureKind::Extrude(e) = &mut h.state_mut().doc.feature_mut(extrude).unwrap().kind {
        e.distance = "15".into();
    }
    h.state_mut().rebuild();
    no_errors(h.state());
    assert!((h.state().doc.sketch(id).unwrap().plane.origin[2] - 15.0).abs() < 1e-6);

    // Click a face in the model: it becomes the selection.
    run(&mut h, Command::Home);
    h.run_steps(15);
    click_at(&mut h, [5.0, 3.0, 15.0]);
    assert!(matches!(h.state().selection.as_slice(), [Pick::Face(_)]), "{:?}", h.state().selection);

    // Measure between two box corners.
    run(&mut h, Command::Measure);
    click_at(&mut h, [0.0, 0.0, 15.0]);
    click_at(&mut h, [30.0, 0.0, 15.0]);
    let Mode::Measure(m) = &h.state().mode else { panic!() };
    assert_eq!(m.picks.len(), 2);
    let d = ferrocad_model::geom::distance(m.picks[0].point, m.picks[1].point);
    assert!((d - 30.0).abs() < 1e-6, "{d}");
    press(&mut h, Key::Escape);
}

#[test]
fn opening_a_file_frames_the_part() {
    let path = std::env::temp_dir().join("ferrocad_fit_test.fcad");
    let mut doc = ferrocad_model::Document::new();
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([100.0, 100.0], [160.0, 140.0]);
    let s = doc.add("Sketch1", FeatureKind::Sketch(sf));
    doc.add(
        "Extrude1",
        FeatureKind::Extrude(ferrocad_model::ExtrudeFeature {
            sketch: s,
            profile_point: [101.0, 101.0],
            distance: "10".into(),
            symmetric: false,
            operation: ferrocad_model::Operation::NewBody,
            distance2: None,
            taper: "0".into(),
            extent: ferrocad_model::ExtrudeExtent::Distance,
            thin: None,
        }),
    );
    std::fs::write(&path, ferrocad_model::io::save(&doc)).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 950.0))
        .with_step_dt(0.05)
        .build_eframe(|cc| {
            crate::theme::setup(&cc.egui_ctx);
            let mut app = FerroApp::new();
            app.doc_path = path.display().to_string();
            app.open_document_from_path();
            app
        });
    h.run_steps(30);
    let t = h.state().camera.target;
    assert!((t[0] - 130.0).abs() < 1e-6 && (t[1] - 120.0).abs() < 1e-6, "camera target {t:?}");
}

#[test]
fn dragging_a_point_onto_another_makes_them_coincident() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let id = match &h.state().mode {
        Mode::Sketch(s) => s.feature,
        _ => unreachable!(),
    };
    {
        let sk = &mut h.state_mut().doc.sketch_mut(id).unwrap().sketch;
        sk.add_line([0.0, 0.0], [20.0, 0.0]);
        sk.add_line([30.0, 5.0], [30.0, 20.0]);
    }
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Select)));
    let from = screen(h.state(), [30.0, 5.0, 0.0]);
    let to = screen(h.state(), [20.0, 0.0, 0.0]);
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for k in 1..=10 {
        let t = k as f32 / 10.0;
        h.event(Event::PointerMoved(from + (to - from) * t));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    let sk = &h.state().doc.sketch(id).unwrap().sketch;
    assert!(
        sk.constraints().any(|(_, c)| matches!(c, Constraint::Coincident(..))),
        "no coincident constraint: {:?}",
        sk.constraints().collect::<Vec<_>>()
    );
}

#[test]
fn clicking_onto_a_curve_keeps_the_point_on_it() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let id = match &h.state().mode {
        Mode::Sketch(s) => s.feature,
        _ => unreachable!(),
    };
    h.state_mut().doc.sketch_mut(id).unwrap().sketch.add_line([0.0, 0.0], [40.0, 0.0]);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Line)));
    click_at(&mut h, [15.0, 0.0, 0.0]); // on the existing line, not at a point
    click_at(&mut h, [15.0, 20.0, 0.0]);
    press(&mut h, Key::Escape);
    let sk = &h.state().doc.sketch(id).unwrap().sketch;
    assert!(sk.constraints().any(|(_, c)| matches!(c, Constraint::PointOnLine(..))));
    assert!(sk.constraints().any(|(_, c)| matches!(c, Constraint::Vertical(..))), "auto vertical");
}

#[test]
fn model_parameters_edit_feature_values() {
    let mut h = harness();
    box_model(&mut h);
    run(&mut h, Command::Parameters);
    h.run_steps(3);
    // The extrude distance is listed; editing it rebuilds the model.
    let key = h
        .state()
        .model_param_edits
        .iter()
        .find(|(_, v)| v.trim() == "10")
        .map(|(k, _)| *k)
        .expect("extrude distance listed");
    let ext = h.state().doc.features()[1].id;
    assert_eq!(key.0, ext);
    let v0 = volume(h.state());
    if let Some(FeatureKind::Extrude(e)) = h.state_mut().doc.feature_mut(ext).map(|f| &mut f.kind) {
        assert_eq!(e.distance, "10");
    }
    // Simulate the edit the dialog would apply.
    let mut kind = h.state().doc.feature(ext).unwrap().kind.clone();
    *kind.expressions_mut()[0].1 = "20".into();
    h.state_mut().doc.feature_mut(ext).unwrap().kind = kind;
    h.state_mut().rebuild();
    assert!((volume(h.state()) - 2.0 * v0).abs() < 1e-6);
    h.run_steps(3);
}

#[test]
fn import_step_and_appearance() {
    use ferrocad_model::Kernel;
    let mut h = harness();
    box_model(&mut h);
    // Export the box, then import it as a second body.
    let path = std::env::temp_dir().join("ferrocad_ui_import.step");
    let solids: Vec<_> = h.state().model.bodies.iter().map(|b| b.solid.clone()).collect();
    h.state().kernel.export_step(&solids, &path).unwrap();
    h.state_mut().import_step(&path);
    h.run_steps(3);
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 2);
    assert!(matches!(h.state().doc.features().last().unwrap().kind, FeatureKind::Import(_)));

    h.state_mut().set_appearance("Body2", Some("Brass"));
    h.run_steps(3);
    assert_eq!(h.state().body_color("Body2"), crate::theme::material_color("Brass"));
    h.state_mut().selection = vec![Pick::Body("Body2".into())];
    run(&mut h, Command::Properties);
    let msg = h.state().message.clone().unwrap().0;
    // 30 × 20 × 10 mm brass: 6 cm³ × 8.5 g/cm³ = 51 g.
    assert!(msg.contains("Brass") && msg.contains("mass 51.0 g"), "{msg}");
    run(&mut h, Command::Undo);
    assert_ne!(h.state().body_color("Body2"), crate::theme::material_color("Brass"));
}

#[test]
fn dragging_the_extrude_arrow_changes_the_distance() {
    let mut h = harness();
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([0.0, 0.0], [30.0, 20.0]);
    h.state_mut().doc.add("Sketch1", FeatureKind::Sketch(sf));
    h.state_mut().rebuild();
    run(&mut h, Command::Home);
    h.run_steps(15);
    run(&mut h, Command::Extrude);
    // Pick the profile by clicking inside it.
    click_at(&mut h, [15.0, 10.0, 0.0]);
    assert!(matches!(&h.state().mode, Mode::Extrude(e) if e.profile_point.is_some()));
    // Grab the arrow tip (centre of the profile, 10 mm up, plus the arrow).
    let len = h.state().camera.distance * 0.12;
    let tip = screen(h.state(), [15.0, 10.0, 10.0 + len]);
    let higher = screen(h.state(), [15.0, 10.0, 25.0 + len]);
    h.event(Event::PointerMoved(tip));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: tip, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for k in 1..=10 {
        h.event(Event::PointerMoved(tip + (higher - tip) * (k as f32 / 10.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: higher, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    let Mode::Extrude(e) = &h.state().mode else { panic!("left the dialog") };
    let d: f64 = e.distance.trim_end_matches(" mm").parse().unwrap();
    assert!((d - 25.0).abs() < 0.5, "distance {d}");
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert!((volume(h.state()) - 600.0 * d).abs() < 1.0);
}

#[test]
fn joint_dialog_and_interference() {
    let mut h = harness();
    box_model(&mut h);
    // A second, overlapping box.
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::Feature(FeatureType::Move));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.exprs[0].text = "10 mm".into();
        f.flags[0].1 = true; // copy
    }
    press(&mut h, Key::Enter);
    assert_eq!(h.state().bodies.len(), 2);
    run(&mut h, Command::Interference);
    assert_eq!(h.state().interference.len(), 1, "{:?}", h.state().message);

    // Joint: put the copy's bottom onto the original's top.
    h.state_mut().selection = vec![Pick::Body("Body2".into())];
    run(&mut h, Command::Feature(FeatureType::Joint));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        assert_eq!(f.active, 1, "moves on to the first origin");
        f.pick(Pick::Face(FaceRef { point: [25.0, 10.0, 0.0], normal: [0.0, 0.0, -1.0] }));
        f.pick(Pick::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
    }
    h.run_steps(3);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    run(&mut h, Command::Interference);
    assert!(h.state().interference.is_empty(), "stacked boxes only touch");
    // Revolute: turn it by an angle through the model parameter.
    let joint = h.state().doc.features().last().unwrap().id;
    if let Some(f) = h.state_mut().doc.feature_mut(joint) {
        *f.kind.expressions_mut()[0].1 = "45".into();
    }
    h.state_mut().rebuild();
    no_errors(h.state());
}

#[test]
fn animating_a_revolute_joint() {
    let mut h = harness();
    box_model(&mut h);
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::Feature(FeatureType::Move));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.exprs[0].text = "50 mm".into();
        f.flags[0].1 = true;
    }
    press(&mut h, Key::Enter);
    h.state_mut().selection = vec![Pick::Body("Body2".into())];
    run(&mut h, Command::Feature(FeatureType::Joint));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.pick(Pick::Face(FaceRef { point: [65.0, 10.0, 0.0], normal: [0.0, 0.0, -1.0] }));
        f.pick(Pick::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
        f.op = 1; // revolute
    }
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let joint = h.state().doc.features().last().unwrap().id;
    h.state_mut().start_animation(joint);
    h.run_steps(20);
    let angle = |h: &Harness<'static, FerroApp>| match &h.state().doc.feature(joint).unwrap().kind {
        FeatureKind::Joint(j) => j.angle.clone(),
        _ => unreachable!(),
    };
    let moving: f64 = angle(&h).parse().unwrap();
    assert!(moving > 1.0, "angle advanced: {moving}");
    no_errors(h.state());
    press(&mut h, Key::Escape);
    assert!(h.state().animation.is_none());
    assert_eq!(angle(&h), "0°");
}

#[test]
fn components_move_together_in_a_joint() {
    let mut h = harness();
    box_model(&mut h);
    // Two copies far away, grouped into a component.
    for dx in ["60 mm", "100 mm"] {
        h.state_mut().selection = vec![Pick::Body("Body1".into())];
        run(&mut h, Command::Feature(FeatureType::Move));
        if let Mode::Feature(f) = &mut h.state_mut().mode {
            f.exprs[0].text = dx.into();
            f.flags[0].1 = true;
        }
        press(&mut h, Key::Enter);
    }
    h.state_mut().selection = vec![Pick::Body("Body2".into()), Pick::Body("Body3".into())];
    run(&mut h, Command::NewComponent);
    no_errors(h.state());
    assert_eq!(h.state().model.components, vec![("Component1".to_string(), vec!["Body2".to_string(), "Body3".to_string()])]);

    // In the joint dialog, clicking one body of the component picks the component.
    run(&mut h, Command::Home);
    h.run_steps(15);
    h.state_mut().selection.clear();
    run(&mut h, Command::Feature(FeatureType::Joint));
    click_at(&mut h, [75.0, 10.0, 10.0]);
    let Mode::Feature(f) = &h.state().mode else { panic!() };
    assert_eq!(f.sels[0].picks, vec![Pick::Body("Component1".into())]);
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.active = 1;
        f.pick(Pick::Face(FaceRef { point: [75.0, 10.0, 0.0], normal: [0.0, 0.0, -1.0] }));
        f.pick(Pick::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
    }
    h.run_steps(3);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    // Both component bodies moved up by 10 mm (onto the box top).
    use ferrocad_model::Kernel;
    for name in ["Body2", "Body3"] {
        let b = h.state().model.body(name).unwrap();
        let c = h.state().kernel.properties(&b.solid).unwrap().center;
        assert!((c[2] - 15.0).abs() < 1e-6, "{name} centre {c:?}");
    }
}

#[test]
fn active_component_collects_new_bodies_and_joint_limits_clamp() {
    let mut h = harness();
    box_model(&mut h);
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::NewComponent);
    // Activate it: a copy made now joins the component.
    h.state_mut().doc.view.active_component = Some("Component1".into());
    h.state_mut().selection = vec![Pick::Body("Body1".into())];
    run(&mut h, Command::Feature(FeatureType::Move));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.exprs[0].text = "60 mm".into();
        f.flags[0].1 = true;
    }
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert_eq!(h.state().model.component_of("Body2"), Some("Component1"));
    h.state_mut().doc.view.active_component = None;

    // A second, separate body for a limited revolute joint.
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([100.0, 0.0], [110.0, 10.0]);
    let s = h.state_mut().doc.add("Sketch9", FeatureKind::Sketch(sf));
    h.state_mut().doc.add(
        "Extrude9",
        FeatureKind::Extrude(ferrocad_model::ExtrudeFeature {
            sketch: s,
            profile_point: [101.0, 1.0],
            distance: "5".into(),
            symmetric: false,
            operation: ferrocad_model::Operation::NewBody,
            distance2: None,
            taper: "0".into(),
            extent: ferrocad_model::ExtrudeExtent::Distance,
            thin: None,
        }),
    );
    h.state_mut().rebuild();
    assert_eq!(h.state().model.component_of("Body3"), None);
    h.state_mut().selection = vec![Pick::Body("Body3".into())];
    run(&mut h, Command::Feature(FeatureType::Joint));
    if let Mode::Feature(f) = &mut h.state_mut().mode {
        f.pick(Pick::Face(FaceRef { point: [105.0, 5.0, 0.0], normal: [0.0, 0.0, -1.0] }));
        f.pick(Pick::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
        f.op = 1;
        f.flags[1].1 = true; // limits 0 … 90
        f.exprs[0].text = "200".into();
    }
    h.run_steps(3);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let FeatureKind::Joint(j) = &h.state().doc.features().last().unwrap().kind else { panic!() };
    assert_eq!(j.limits, Some(("0".to_string(), "90".to_string())));
}

fn drag(h: &mut Harness<'static, FerroApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    for k in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn window_and_crossing_selection_in_sketches() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    let id = match &h.state().mode {
        Mode::Sketch(s) => s.feature,
        _ => unreachable!(),
    };
    let (inner, long) = {
        let sk = &mut h.state_mut().doc.sketch_mut(id).unwrap().sketch;
        (sk.add_line([0.0, 0.0], [10.0, 0.0]), sk.add_line([0.0, 10.0], [60.0, 10.0]))
    };
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Select)));
    let selected = |h: &Harness<'static, FerroApp>| match &h.state().mode {
        Mode::Sketch(s) => s.selection.clone(),
        _ => vec![],
    };
    // Window (left → right) around x −5 … 20: only the short line is fully inside.
    let (a, b) = (screen(h.state(), [-5.0, -5.0, 0.0]), screen(h.state(), [20.0, 15.0, 0.0]));
    drag(&mut h, a, b);
    let sel = selected(&h);
    assert!(sel.contains(&crate::sketch_ui::Pick::Entity(inner)));
    assert!(!sel.contains(&crate::sketch_ui::Pick::Entity(long)), "{sel:?}");
    // Crossing (right → left) over the same area: both lines touch it.
    drag(&mut h, b, a);
    let sel = selected(&h);
    assert!(sel.contains(&crate::sketch_ui::Pick::Entity(inner)) && sel.contains(&crate::sketch_ui::Pick::Entity(long)), "{sel:?}");
}

#[test]
fn two_sided_and_tapered_extrude_dialog() {
    let mut h = harness();
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([0.0, 0.0], [30.0, 20.0]);
    h.state_mut().doc.add("Sketch1", FeatureKind::Sketch(sf));
    h.state_mut().rebuild();
    run(&mut h, Command::Home);
    h.run_steps(15);
    run(&mut h, Command::Extrude);
    click_at(&mut h, [15.0, 10.0, 0.0]);
    h.get_by_label("Two sides").click();
    h.run_steps(2);
    assert!(h.query_by_label("Distance 2").is_some(), "second distance field shown");
    if let Mode::Extrude(e) = &mut h.state_mut().mode {
        assert!(e.two_sides && !e.symmetric);
        e.distance = "10".into();
        e.distance2 = "4".into();
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert!((volume(h.state()) - 600.0 * 14.0).abs() < 1e-3, "{}", volume(h.state()));
    let bb = |app: &FerroApp| {
        let z = app.bodies[0].mesh.positions.iter().map(|p| p[2]);
        (z.clone().fold(f64::MAX, f64::min), z.fold(f64::MIN, f64::max))
    };
    let (lo, hi) = bb(h.state());
    assert!((lo + 4.0).abs() < 1e-3 && (hi - 10.0).abs() < 1e-3, "{lo} {hi}");

    // Editing keeps both sides; a taper makes the solid smaller.
    let ext = h.state().doc.features()[1].id;
    h.state_mut().edit_feature(ext);
    h.run_steps(2);
    if let Mode::Extrude(e) = &mut h.state_mut().mode {
        assert!(e.two_sides && e.distance2 == "4");
        e.taper = "10".into();
    } else {
        panic!("not editing the extrude");
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let v = volume(h.state());
    assert!(v < 600.0 * 14.0 - 100.0 && v > 3000.0, "{v}");
    let (lo, hi) = bb(h.state());
    assert!((lo + 4.0).abs() < 1e-3 && (hi - 10.0).abs() < 1e-3, "taper keeps the heights: {lo} {hi}");
}

#[test]
fn draft_midplane_and_two_distance_chamfer() {
    let mut h = harness();
    box_model(&mut h);
    let v0 = volume(h.state());

    // Draft the two x faces around the top face, pulling downwards.
    h.state_mut().selection = vec![
        Pick::Face(FaceRef { point: [0.0, 10.0, 5.0], normal: [-1.0, 0.0, 0.0] }),
        Pick::Face(FaceRef { point: [30.0, 10.0, 5.0], normal: [1.0, 0.0, 0.0] }),
    ];
    run(&mut h, Command::Feature(FeatureType::Draft));
    let Mode::Feature(s) = &h.state().mode else { panic!("no draft dialog") };
    assert_eq!(s.sels[0].picks.len(), 2, "selection pre-fills the faces");
    assert_eq!(s.active, 1, "pull direction is asked for next");
    click_at(&mut h, [15.0, 10.0, 10.0]);
    h.get_by_label("Flip").click();
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let t = 3f64.to_radians().tan();
    let expected = (30.0 + (30.0 - 2.0 * 10.0 * t)) / 2.0 * 10.0 * 20.0;
    assert!((volume(h.state()) - expected).abs() < 1e-3, "{} vs {expected}", volume(h.state()));
    assert_eq!(h.state().doc.features().last().unwrap().kind.type_name(), "Draft");

    // Midplane between the XY plane and the top face.
    run(&mut h, Command::Feature(FeatureType::Midplane));
    if let Mode::Feature(s) = &mut h.state_mut().mode {
        s.sels[0].picks = vec![Pick::Plane(PlaneRef::Origin(OriginPlane::XY))];
        s.active = 1;
    }
    click_at(&mut h, [15.0, 10.0, 10.0]);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let mid = h.state().doc.features().last().unwrap().id;
    assert!((h.state().model.planes[&mid].origin[2] - 5.0).abs() < 1e-6);

    // Undo back to the plain box, then a 1 × 3 chamfer on a top edge.
    run(&mut h, Command::Undo);
    run(&mut h, Command::Undo);
    assert!((volume(h.state()) - v0).abs() < 1e-6);
    h.state_mut().selection = vec![Pick::Edge(EdgeRef { point: [15.0, 0.0, 10.0] })];
    run(&mut h, Command::Chamfer);
    h.get_by_label("Two distances").click();
    h.run_steps(2);
    if let Mode::Fillet(s) = &mut h.state_mut().mode {
        s.radius = "1".into();
        s.distance2 = "3".into();
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert!((volume(h.state()) - (v0 - 45.0)).abs() < 1e-6, "{}", volume(h.state()));
    let ch = h.state().doc.features().last().unwrap().id;
    h.state_mut().edit_feature(ch);
    h.run_steps(2);
    assert!(matches!(&h.state().mode, Mode::Fillet(s) if s.two_distances && s.distance2 == "3"));
}

#[test]
fn measure_face_area_and_angles() {
    let mut h = harness();
    box_model(&mut h);
    run(&mut h, Command::Measure);
    // Top face of the 30 × 20 box.
    click_at(&mut h, [15.0, 10.0, 10.0]);
    assert!(h.query_by_label("600.000 mm²").is_some(), "area shown");
    // Front face: perpendicular to the top.
    click_at(&mut h, [15.0, 0.0, 5.0]);
    let Mode::Measure(m) = &h.state().mode else { panic!() };
    assert_eq!(m.picks.iter().map(|p| p.what).collect::<Vec<_>>(), ["face", "face"]);
    assert!(h.query_by_label("90.000°").is_some(), "angle between the faces");
    // A top edge and the top face: the edge lies in the face.
    run(&mut h, Command::Measure);
    if let Mode::Measure(m) = &mut h.state_mut().mode {
        m.picks.clear();
    }
    click_at(&mut h, [15.0, 0.0, 10.0]);
    click_at(&mut h, [15.0, 10.0, 10.0]);
    let Mode::Measure(m) = &h.state().mode else { panic!() };
    assert_eq!(m.picks.iter().map(|p| p.what).collect::<Vec<_>>(), ["edge", "face"]);
    assert!(h.query_by_label("0.000°").is_some(), "edge in the face plane");
    press(&mut h, Key::Escape);
}

#[test]
fn values_always_show_their_unit() {
    use crate::dialogs::{fmt_value, with_unit};
    assert_eq!(fmt_value(10.0, "mm"), "10 mm");
    assert_eq!(fmt_value(2.5, "mm"), "2.5 mm");
    assert_eq!(fmt_value(1.0 / 3.0, "mm"), "0.333 mm");
    assert_eq!(fmt_value(45.0, "°"), "45°");
    assert_eq!(fmt_value(-0.0001, "mm"), "0 mm");
    assert_eq!(fmt_value(6.0, ""), "6");
    assert_eq!(with_unit(" 12 ", "mm"), "12 mm");
    assert_eq!(with_unit("5", "°"), "5°");
    assert_eq!(with_unit("width / 2", "mm"), "width / 2");
    assert_eq!(with_unit("3", ""), "3");

    // A plain number typed into a dialog field gets its unit when the field
    // loses focus.
    let mut h = harness();
    box_model(&mut h);
    run(&mut h, Command::Feature(FeatureType::Shell));
    h.query_all_by_value("1 mm").next().expect("thickness field").click();
    h.run_steps(1);
    press(&mut h, Key::Backspace);
    press(&mut h, Key::Backspace);
    press(&mut h, Key::Backspace);
    press(&mut h, Key::Backspace);
    type_text(&mut h, "2");
    press(&mut h, Key::Tab);
    h.run_steps(2);
    let Mode::Feature(s) = &h.state().mode else { panic!("left the dialog") };
    assert_eq!(s.exprs[0].text, "2 mm");
    assert!(h.query_by_label("= 2 mm").is_some());
}

#[test]
fn extrude_all_to_object_and_thin() {
    let mut h = harness();
    box_model(&mut h);
    let v0 = volume(h.state());

    // A 4 × 4 square on the top face, cut through all.
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.attach = Some(PlaneRef::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
    sf.sketch.add_rectangle([5.0, 5.0], [9.0, 9.0]);
    h.state_mut().doc.add("Sketch2", FeatureKind::Sketch(sf));
    h.state_mut().rebuild();
    run(&mut h, Command::Extrude);
    click_at(&mut h, [7.0, 7.0, 10.0]);
    h.get_by_label("All").click();
    h.run_steps(2);
    if let Mode::Extrude(e) = &mut h.state_mut().mode {
        e.operation = ferrocad_model::Operation::Cut;
        e.flip = true;
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert!((volume(h.state()) - (v0 - 160.0)).abs() < 1e-6, "{}", volume(h.state()));

    // A square beside the box, extruded up to the box's top face.
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.sketch.add_rectangle([40.0, 0.0], [50.0, 10.0]);
    h.state_mut().doc.add("Sketch3", FeatureKind::Sketch(sf));
    h.state_mut().rebuild();
    run(&mut h, Command::Extrude);
    click_at(&mut h, [45.0, 5.0, 0.0]);
    h.get_by_label("To object").click();
    h.run_steps(2);
    assert!(matches!(&h.state().mode, Mode::Extrude(e) if e.picking_to));
    click_at(&mut h, [20.0, 15.0, 10.0]);
    assert!(matches!(&h.state().mode, Mode::Extrude(e) if e.to.is_some() && !e.picking_to));
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let tower = h.state().model.bodies.last().unwrap().name.clone();
    let v = |h: &Harness<'static, FerroApp>, name: &str| {
        let b = h.state().model.body(name).unwrap();
        ferrocad_model::Kernel::properties(&h.state().kernel, &b.solid).unwrap().volume
    };
    assert!((v(&h, &tower) - 1000.0).abs() < 1e-6, "{}", v(&h, &tower));

    // Up to a plane tilted by 10° around X: the top follows the slope.
    let tilted = h.state_mut().doc.add(
        "Plane1",
        FeatureKind::Plane(ferrocad_model::PlaneFeature {
            base: PlaneRef::Origin(OriginPlane::XY),
            offset: "0".into(),
            angle: "0".into(),
            base2: None,
        }),
    );
    let ext = h.state().doc.features().iter().find(|f| f.name == "Extrude3").map(|f| f.id).unwrap();
    {
        let doc = &mut h.state_mut().doc;
        if let FeatureKind::Plane(p) = &mut doc.feature_mut(tilted).unwrap().kind {
            p.base = PlaneRef::Face(FaceRef { point: [20.0, 15.0, 10.0], normal: [0.0, 0.0, 1.0] });
            p.angle = "10".into();
        }
        let at = doc.index_of(ext).unwrap();
        doc.move_feature(tilted, at).unwrap();
        if let FeatureKind::Extrude(e) = &mut doc.feature_mut(ext).unwrap().kind {
            e.extent = ferrocad_model::ExtrudeExtent::To(PlaneRef::Construction(tilted));
        }
    }
    h.state_mut().rebuild();
    no_errors(h.state());
    let slope = 10f64.to_radians().tan() * 5.0 * 100.0; // mean y (5) × tan × area
    let got = v(&h, &tower);
    assert!(((got - 1000.0).abs() - slope).abs() < 1e-3, "{got}");

    // Thin: a 1 mm wall inside the square instead of a solid tower.
    if let FeatureKind::Extrude(e) = &mut h.state_mut().doc.feature_mut(ext).unwrap().kind {
        e.extent = ferrocad_model::ExtrudeExtent::Distance;
        e.distance = "10".into();
        e.thin = Some(ferrocad_model::ThinWall { thickness: "1".into(), side: ferrocad_model::ThinSide::Inside });
    }
    h.state_mut().rebuild();
    no_errors(h.state());
    assert!((v(&h, &tower) - 360.0).abs() < 1e-6, "{}", v(&h, &tower));
    h.state_mut().edit_feature(ext);
    h.run_steps(2);
    assert!(matches!(&h.state().mode, Mode::Extrude(e) if e.thin && e.thin_wall.thickness == "1"));
    assert!(h.query_by_label("Wall thickness").is_some());
}

#[test]
fn holes_on_sketch_points_and_pattern_features() {
    let mut h = harness();
    box_model(&mut h);
    let v0 = volume(h.state());

    // Two circles on the top face; holes go on their centres.
    let mut sf = ferrocad_model::SketchFeature::new(ferrocad_model::Plane::XY);
    sf.attach = Some(PlaneRef::Face(FaceRef { point: [15.0, 10.0, 10.0], normal: [0.0, 0.0, 1.0] }));
    sf.sketch.add_circle([5.0, 5.0], 1.0);
    sf.sketch.add_circle([5.0, 15.0], 1.0);
    h.state_mut().doc.add("Sketch2", FeatureKind::Sketch(sf));
    h.state_mut().rebuild();
    run(&mut h, Command::Feature(FeatureType::Hole));
    click_at(&mut h, [5.0, 5.0, 10.0]);
    click_at(&mut h, [5.0, 15.0, 10.0]);
    let Mode::Feature(s) = &h.state().mode else { panic!("no hole dialog") };
    assert!(s.sels[0].picks.iter().all(|p| matches!(p, Pick::SketchPoint(..))), "{:?}", s.sels[0].picks);
    assert_eq!(s.sels[0].picks.len(), 2);
    if let Mode::Feature(s) = &mut h.state_mut().mode {
        s.exprs[0].text = "4".into();
        s.flags[0].1 = true; // through all
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let hole = std::f64::consts::PI * 4.0 * 10.0;
    assert!((volume(h.state()) - (v0 - 2.0 * hole)).abs() < 1e-3, "{}", volume(h.state()));
    let hole_id = h.state().doc.features().last().unwrap().id;

    // Pattern the hole feature: 3 along X, 8 mm apart.
    run(&mut h, Command::Feature(FeatureType::RectPattern));
    h.get_by_label("Features").click();
    h.run_steps(2);
    h.get_by_label("Hole1").click();
    h.run_steps(2);
    if let Mode::Feature(s) = &mut h.state_mut().mode {
        assert!(s.pattern_features && s.features == vec![hole_id]);
        s.exprs[1].text = "8 mm".into();
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 1, "feature patterns don't add bodies");
    assert!((volume(h.state()) - (v0 - 6.0 * hole)).abs() < 1e-3, "{}", volume(h.state()));

    // Editing reopens it as a feature pattern; save/load keeps everything.
    let pattern = h.state().doc.features().last().unwrap().id;
    h.state_mut().edit_feature(pattern);
    h.run_steps(2);
    assert!(matches!(&h.state().mode, Mode::Feature(s) if s.pattern_features && s.features == vec![hole_id]));
    press(&mut h, Key::Escape);
    let text = ferrocad_model::io::save(&h.state().doc);
    assert_eq!(ferrocad_model::io::save(&ferrocad_model::io::load(&text).unwrap()), text);
}

#[test]
fn point_tool_places_hole_positions() {
    let mut h = harness();
    box_model(&mut h);
    let v0 = volume(h.state());
    run(&mut h, Command::CreateSketch);
    click_at(&mut h, [15.0, 10.0, 10.0]);
    h.run_steps(15);
    let Mode::Sketch(sess) = &h.state().mode else { panic!("no sketch") };
    let plane = h.state().doc.sketch(sess.feature).unwrap().plane;
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Point)));
    click_at(&mut h, [10.0, 10.0, 10.0]);
    click_at(&mut h, [20.0, 10.0, 10.0]);
    let Mode::Sketch(sess) = &h.state().mode else { panic!("left the sketch") };
    let sk = &h.state().doc.sketch(sess.feature).unwrap().sketch;
    let points: Vec<[f64; 2]> = sk
        .entities()
        .filter_map(|(_, e)| if let ferrocad_sketch::Entity::Point { p } = e { Some(sk.point_pos(*p)) } else { None })
        .collect();
    assert_eq!(points.len(), 2, "two standalone points");
    let w = plane.to_world(points[0]);
    assert!((w[0] - 10.0).abs() < 1e-6 && (w[1] - 10.0).abs() < 1e-6, "{w:?}");
    run(&mut h, Command::FinishSketch);

    run(&mut h, Command::Feature(FeatureType::Hole));
    click_at(&mut h, [10.0, 10.0, 10.0]);
    click_at(&mut h, [20.0, 10.0, 10.0]);
    if let Mode::Feature(s) = &mut h.state_mut().mode {
        assert_eq!(s.sels[0].picks.len(), 2, "{:?}", s.sels[0].picks);
        s.exprs[0].text = "4".into();
        s.exprs[1].text = "5".into();
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    no_errors(h.state());
    let hole = std::f64::consts::PI * 4.0 * 5.0;
    assert!((volume(h.state()) - (v0 - 2.0 * hole)).abs() < 1e-3, "{}", volume(h.state()));
}

#[test]
fn sketch_text_extrudes() {
    let mut h = harness();
    h.state_mut().start_sketch_on(PlaneRef::Origin(OriginPlane::XY));
    h.run_steps(15);
    run(&mut h, Command::Sketch(SketchCommand::Tool(Tool::Text)));
    click_at(&mut h, [0.0, 0.0, 0.0]);
    assert!(matches!(&h.state().mode, Mode::Sketch(s) if s.text_input.is_some()));
    type_text(&mut h, "Ho");
    if let Mode::Sketch(s) = &mut h.state_mut().mode {
        s.text_height = "10 mm".into();
    }
    h.run_steps(2);
    press(&mut h, Key::Enter);
    let Mode::Sketch(sess) = &h.state().mode else { panic!("left the sketch") };
    assert!(sess.text_input.is_none(), "{:?}", h.state().message);
    let sk = h.state().doc.sketch(sess.feature).unwrap().sketch.clone();
    let profiles = ferrocad_sketch::find_profiles(&sk);
    // H, and o with its counter (plus the counter's own disk).
    assert_eq!(profiles.len(), 3, "{}", profiles.len());
    // Undo removes the whole text in one step.
    run(&mut h, Command::Undo);
    let Mode::Sketch(sess) = &h.state().mode else { panic!("left the sketch") };
    assert_eq!(h.state().doc.sketch(sess.feature).unwrap().sketch.entities().count(), 0);
    run(&mut h, Command::Redo);
    run(&mut h, Command::FinishSketch);

    // Extrude every letter body (the profiles with the larger areas).
    let sketch = h.state().doc.features()[0].id;
    let mut picks: Vec<&ferrocad_sketch::Profile> = profiles.iter().collect();
    picks.sort_by(|a, b| b.area().total_cmp(&a.area()));
    for p in picks.iter().take(2) {
        let poly = p.outer.polygon();
        let inside = (0..400)
            .map(|k| {
                let (a, b) = (poly[k % poly.len()], poly[(k * 7 + 3) % poly.len()]);
                [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
            })
            .find(|q| ferrocad_sketch::profile::profile_at(&profiles, *q).is_some_and(|i| std::ptr::eq(&profiles[i], *p)))
            .expect("a point inside the letter");
        h.state_mut().doc.add(
            "Extrude",
            FeatureKind::Extrude(ferrocad_model::ExtrudeFeature {
                sketch,
                profile_point: inside,
                distance: "2".into(),
                symmetric: false,
                operation: ferrocad_model::Operation::NewBody,
                distance2: None,
                taper: "0".into(),
                extent: ferrocad_model::ExtrudeExtent::Distance,
                thin: None,
            }),
        );
    }
    h.state_mut().rebuild();
    no_errors(h.state());
    assert_eq!(h.state().bodies.len(), 2);
    let total: f64 = picks.iter().take(2).map(|p| p.area() * 2.0).sum();
    assert!((volume(h.state()) - total).abs() < 0.01 * total, "{} vs {total}", volume(h.state()));
    let text = ferrocad_model::io::save(&h.state().doc);
    assert_eq!(ferrocad_model::io::save(&ferrocad_model::io::load(&text).unwrap()), text);
}
