use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::{find_profiles, Constraint, Entity, Segment, Sketch};
use std::f64::consts::PI;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn line_ends(sk: &Sketch, id: ferrocad_sketch::EntityId) -> ([f64; 2], [f64; 2]) {
    match *sk.entity(id) {
        Entity::Line { a, b } => (sk.point_pos(a), sk.point_pos(b)),
        _ => panic!("not a line"),
    }
}

/// A sloppy rectangle gets dimensioned and pinned to the origin – the most
/// common first sketch in any CAD program.
#[test]
fn dimensioned_rectangle_is_fully_constrained() {
    let mut sk = Sketch::new();
    let [bottom, right, ..] = sk.add_rectangle([0.3, -0.2], [37.0, 18.0]);
    let origin = match *sk.entity(bottom) {
        Entity::Line { a, .. } => a,
        _ => unreachable!(),
    };
    sk.set_point_pos(origin, [0.0, 0.0]);
    sk.set_point_fixed(origin, true);
    sk.constrain(Constraint::Length(bottom, 40.0));
    let h = sk.constrain(Constraint::Length(right, 20.0));

    sk.solve().unwrap();
    let a = sk.analyze();
    assert_eq!(a.dof, 0, "rectangle should be fully constrained");
    let (p0, p1) = line_ends(&sk, bottom);
    assert!(close(p0[0], 0.0) && close(p0[1], 0.0));
    assert!(close(p1[0], 40.0) && close(p1[1], 0.0));

    // Change a driving dimension, the way a parameter edit would.
    sk.set_dimension(h, 25.0);
    sk.solve().unwrap();
    let (_, top_right) = line_ends(&sk, right);
    assert!(close(top_right[1], 25.0));
}

#[test]
fn dof_counts() {
    let mut sk = Sketch::new();
    let l = sk.add_line([0.0, 0.0], [10.0, 1.0]);
    assert_eq!(sk.analyze().dof, 4);
    sk.constrain(Constraint::Horizontal(l));
    sk.solve().unwrap();
    assert_eq!(sk.analyze().dof, 3);
    sk.constrain(Constraint::Length(l, 12.0));
    sk.solve().unwrap();
    let a = sk.analyze();
    assert_eq!(a.dof, 2);
    assert!(!sk.is_entity_determined(&a, l));
}

#[test]
fn conflicting_constraints_leave_sketch_untouched() {
    let mut sk = Sketch::new();
    let l = sk.add_line([0.0, 0.0], [10.0, 0.0]);
    sk.constrain(Constraint::Horizontal(l));
    sk.solve().unwrap();
    let before = sk.params().to_vec();
    sk.constrain(Constraint::Vertical(l));
    sk.constrain(Constraint::Length(l, 10.0));
    assert!(sk.solve().is_err());
    assert_eq!(sk.params(), &before[..]);
}

#[test]
fn dragging_keeps_constraints() {
    let mut sk = Sketch::new();
    let [bottom, right, top, left] = sk.add_rectangle([0.0, 0.0], [10.0, 5.0]);
    let corner = match *sk.entity(top) {
        Entity::Line { a, .. } => a,
        _ => unreachable!(),
    };
    sk.drag_point(corner, [20.0, 8.0]).unwrap();
    assert!(close(sk.point_pos(corner)[0], 20.0));
    for l in [bottom, top] {
        let (a, b) = line_ends(&sk, l);
        assert!(close(a[1], b[1]), "horizontal kept");
    }
    for l in [left, right] {
        let (a, b) = line_ends(&sk, l);
        assert!(close(a[0], b[0]), "vertical kept");
    }
}

#[test]
fn tangent_and_angle() {
    let mut sk = Sketch::new();
    let l = sk.add_line([0.0, 0.0], [10.0, 0.5]);
    let c = sk.add_circle([5.0, 4.0], 2.0);
    sk.constrain(Constraint::Horizontal(l));
    sk.constrain(Constraint::Tangent(l, c));
    sk.constrain(Constraint::Radius(c, 3.0));
    sk.solve().unwrap();
    let (a, _) = line_ends(&sk, l);
    let center = match *sk.entity(c) {
        Entity::Circle { center, .. } => sk.point_pos(center),
        _ => unreachable!(),
    };
    assert!(close((center[1] - a[1]).abs(), 3.0));

    let mut sk = Sketch::new();
    let l1 = sk.add_line([0.0, 0.0], [10.0, 0.0]);
    let l2 = sk.add_line([0.0, 0.0], [5.0, 5.0]);
    sk.constrain(Constraint::Horizontal(l1));
    sk.constrain(Constraint::Angle(l1, l2, 30f64.to_radians()));
    sk.solve().unwrap();
    let (a, b) = line_ends(&sk, l2);
    let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
    assert!(close(ang, PI / 6.0));
}

#[test]
fn rectangle_with_hole_gives_plate_and_disk() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [40.0, 20.0]);
    sk.add_circle([20.0, 10.0], 5.0);
    let profiles = find_profiles(&sk);
    assert_eq!(profiles.len(), 2);

    let plate = profile_at(&profiles, [2.0, 2.0]).unwrap();
    let disk = profile_at(&profiles, [20.0, 10.0]).unwrap();
    assert_ne!(plate, disk);
    assert_eq!(profiles[plate].holes.len(), 1);
    let expected = 40.0 * 20.0 - PI * 25.0;
    assert!((profiles[plate].area() - expected).abs() < 1e-6, "{}", profiles[plate].area());
    assert!(profiles[disk].holes.is_empty());
}

#[test]
fn slot_made_of_lines_and_arcs_is_a_loop() {
    // Stadium shape: two lines joined by two half circles.
    let mut sk = Sketch::new();
    let top = sk.add_line([10.0, 5.0], [0.0, 5.0]);
    let bottom = sk.add_line([0.0, -5.0], [10.0, -5.0]);
    let left = sk.add_arc([0.0, 0.0], [0.0, 5.0], [0.0, -5.0]);
    let right = sk.add_arc([10.0, 0.0], [10.0, -5.0], [10.0, 5.0]);
    let pts = |e| sk.entity(e).points();
    let (t, b, l, r) = (pts(top), pts(bottom), pts(left), pts(right));
    // arc points are [center, start, end]
    let joints = [(t[1], l[1]), (l[2], b[0]), (b[1], r[1]), (r[2], t[0])];
    for (p, q) in joints {
        sk.constrain(Constraint::Coincident(p, q));
    }
    sk.solve().unwrap();

    let profiles = find_profiles(&sk);
    assert_eq!(profiles.len(), 1);
    let area = profiles[0].area();
    let expected = 10.0 * 10.0 + PI * 25.0;
    assert!((area - expected).abs() < 1e-6, "area {area}");
    assert!(profiles[0].outer.segments.iter().any(|s| matches!(s, Segment::Arc { .. })));
}

#[test]
fn open_chain_is_not_a_profile() {
    let mut sk = Sketch::new();
    sk.add_line([0.0, 0.0], [10.0, 0.0]);
    sk.add_line([10.0, 0.0], [10.0, 10.0]);
    assert!(find_profiles(&sk).is_empty());
}

#[test]
fn construction_lines_are_ignored() {
    let mut sk = Sketch::new();
    let lines = sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    assert_eq!(find_profiles(&sk).len(), 1);
    sk.set_construction(lines[0], true);
    assert!(find_profiles(&sk).is_empty());
}
