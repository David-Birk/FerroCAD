//! Regions formed by crossing and branching curves.

use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::{find_profiles, Constraint, Profile, Sketch};
use std::f64::consts::PI;

fn areas(profiles: &[Profile]) -> Vec<f64> {
    let mut a: Vec<f64> = profiles.iter().map(Profile::area).collect();
    a.sort_by(f64::total_cmp);
    a
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn line_across_rectangle_makes_two_regions() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [40.0, 20.0]);
    // overshoots both sides – the stubs outside must be ignored
    sk.add_line([10.0, -5.0], [10.0, 25.0]);
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 2);
    let a = areas(&p);
    assert!(close(a[0], 200.0) && close(a[1], 600.0), "{a:?}");
    let left = profile_at(&p, [5.0, 10.0]).unwrap();
    let right = profile_at(&p, [30.0, 10.0]).unwrap();
    assert_ne!(left, right);
}

#[test]
fn diagonal_makes_two_triangles() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    sk.add_line([0.0, 0.0], [10.0, 10.0]);
    let a = areas(&find_profiles(&sk));
    assert_eq!(a.len(), 2);
    assert!(close(a[0], 50.0) && close(a[1], 50.0));
}

#[test]
fn overlapping_circles_make_three_regions() {
    let mut sk = Sketch::new();
    sk.add_circle([0.0, 0.0], 10.0);
    sk.add_circle([10.0, 0.0], 10.0);
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 3);
    // lens area of two r=10 circles at distance 10
    let r: f64 = 10.0;
    let lens = 2.0 * r * r * (0.5f64).acos() - 0.5 * 10.0 * (4.0 * r * r - 100.0).sqrt();
    let full = PI * r * r;
    let a = areas(&p);
    assert!(close(a[0], lens), "{a:?} lens {lens}");
    assert!(close(a[1], full - lens) && close(a[2], full - lens));
    let middle = profile_at(&p, [5.0, 0.0]).unwrap();
    assert!(close(p[middle].area(), lens));
}

#[test]
fn circle_crossing_rectangle_edge() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [20.0, 20.0]);
    sk.add_circle([20.0, 10.0], 5.0); // centred on the right edge
    let a = areas(&find_profiles(&sk));
    assert_eq!(a.len(), 3);
    let half = PI * 25.0 / 2.0;
    assert!(close(a[0], half) && close(a[1], half), "{a:?}");
    assert!(close(a[2], 400.0 - half));
}

#[test]
fn nested_islands_become_holes() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [100.0, 100.0]);
    sk.add_rectangle([10.0, 10.0], [50.0, 50.0]);
    sk.add_circle([30.0, 30.0], 5.0);
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 3);
    let outer = profile_at(&p, [90.0, 90.0]).unwrap();
    let ring = profile_at(&p, [15.0, 15.0]).unwrap();
    let disk = profile_at(&p, [30.0, 30.0]).unwrap();
    assert_eq!(p[outer].holes.len(), 1, "only the direct child is a hole");
    assert_eq!(p[ring].holes.len(), 1);
    assert!(p[disk].holes.is_empty());
    assert!(close(p[outer].area(), 10000.0 - 1600.0));
    assert!(close(p[ring].area(), 1600.0 - PI * 25.0));
}

#[test]
fn dangling_line_inside_region_is_ignored() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    sk.add_line([0.0, 5.0], [5.0, 5.0]); // T-junction into the middle
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 1);
    assert!(close(p[0].area(), 100.0));
}

#[test]
fn arc_closing_a_u_shape() {
    // U made of three lines, closed on top by a half-circle bulging upwards.
    let mut sk = Sketch::new();
    sk.add_line([0.0, 10.0], [0.0, 0.0]);
    sk.add_line([0.0, 0.0], [10.0, 0.0]);
    sk.add_line([10.0, 0.0], [10.0, 10.0]);
    sk.add_arc([5.0, 10.0], [10.0, 10.0], [0.0, 10.0]);
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 1);
    assert!(close(p[0].area(), 100.0 + PI * 25.0 / 2.0));
}

#[test]
fn duplicate_lines_do_not_break_regions() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    sk.add_line([0.0, 0.0], [10.0, 0.0]); // drawn twice
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 1);
    assert!(close(p[0].area(), 100.0));
}

#[test]
fn tangent_circle_inside_square() {
    let mut sk = Sketch::new();
    sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
    sk.add_circle([5.0, 5.0], 5.0); // touches all four sides
    let a = areas(&find_profiles(&sk));
    let total: f64 = a.iter().sum();
    assert!(close(total, 100.0), "regions must tile the square: {a:?}");
    assert!(a.iter().any(|x| close(*x, PI * 25.0)));
}

#[test]
fn closed_spline_is_one_smooth_profile() {
    let mut sk = Sketch::new();
    let fit = [[0.0, 0.0], [10.0, -2.0], [14.0, 8.0], [3.0, 12.0], [0.0, 0.0]];
    sk.add_spline(&fit);
    let profiles = find_profiles(&sk);
    assert_eq!(profiles.len(), 1);
    let segs = &profiles[0].outer.segments;
    assert_eq!(segs.len(), 1, "pieces merged into one spline: {}", segs.len());
    assert!(matches!(segs[0], ferrocad_sketch::Segment::Spline { .. }));
    assert!(profiles[0].area() > 50.0);
    // The curve passes through its fit points.
    let pts = sk.spline_points(ferrocad_sketch::EntityId(0), 12);
    for f in &fit {
        assert!(pts.iter().any(|p| (p[0] - f[0]).hypot(p[1] - f[1]) < 1e-9), "{f:?}");
    }
}

#[test]
fn line_across_spline_makes_two_regions() {
    let mut sk = Sketch::new();
    sk.add_spline(&[[0.0, 0.0], [10.0, -3.0], [20.0, 0.0], [10.0, 10.0], [0.0, 0.0]]);
    sk.add_line([10.0, -20.0], [10.0, 30.0]);
    let profiles = find_profiles(&sk);
    assert_eq!(profiles.len(), 2);
    for p in &profiles {
        let kinds: Vec<bool> = p.outer.segments.iter().map(|s| matches!(s, ferrocad_sketch::Segment::Spline { .. })).collect();
        assert_eq!(kinds.iter().filter(|k| **k).count(), 1, "one spline piece per region: {kinds:?}");
        assert_eq!(kinds.len(), 2, "spline piece + line piece");
    }
    let total: f64 = profiles.iter().map(|p| p.area()).sum();
    let whole = {
        let mut only = Sketch::new();
        only.add_spline(&[[0.0, 0.0], [10.0, -3.0], [20.0, 0.0], [10.0, 10.0], [0.0, 0.0]]);
        find_profiles(&only)[0].area()
    };
    assert!((total - whole).abs() < 1e-6 * whole, "{total} vs {whole}");
}

#[test]
fn spline_points_drag_and_survive_data_roundtrip() {
    let mut sk = Sketch::new();
    let s = sk.add_spline(&[[0.0, 0.0], [5.0, 5.0], [10.0, 0.0]]);
    let mid = sk.entity(s).points()[1];
    sk.drag_point(mid, [5.0, 8.0]).unwrap();
    let pts = sk.spline_points(s, 8);
    assert!(pts.iter().any(|p| (p[1] - 8.0).abs() < 1e-9));
    let back = Sketch::from_data(sk.to_data()).unwrap();
    assert_eq!(back.to_data(), sk.to_data());
}

/// Exploded text: closed polygons of fixed points. A contour inside another
/// becomes a hole (and a profile of its own, like in Fusion), and the fixed
/// glyph points add no unknowns to the solver.
#[test]
fn fixed_polygons_nest_and_cost_the_solver_nothing() {
    let mut sk = Sketch::new();
    let square = |x: f64, s: f64| vec![[x, x], [x + s, x], [x + s, x + s], [x, x + s], [x, x]];
    let outer = sk.add_fixed_polygon(&square(0.0, 10.0));
    assert_eq!(outer.len(), 4, "closing duplicate dropped");
    // Inner contour clockwise (TrueType style) – orientation must not matter.
    let mut inner = square(3.0, 4.0);
    inner.reverse();
    sk.add_fixed_polygon(&inner);
    assert!(sk.add_fixed_polygon(&[[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]]).is_empty());
    let p = find_profiles(&sk);
    assert_eq!(p.len(), 2);
    let ring = profile_at(&p, [1.0, 1.0]).unwrap();
    assert_eq!(p[ring].holes.len(), 1);
    assert!(close(p[ring].area(), 100.0 - 16.0));

    // Hundreds of fixed glyph-like polygons: zero free parameters.
    for k in 0..150 {
        let x = 20.0 + 3.0 * k as f64;
        sk.add_fixed_polygon(&[[x, 0.0], [x + 1.0, 0.0], [x + 1.0, 1.0], [x + 0.5, 1.5], [x, 1.0]]);
    }
    let free = sk.add_line([0.0, -5.0], [10.0, -5.0]);
    sk.constrain(Constraint::Horizontal(free));
    let t = std::time::Instant::now();
    let a = sk.analyze();
    sk.solve().unwrap();
    assert_eq!(a.dof, 3);
    assert!(t.elapsed().as_millis() < 200, "solve + analyze took {:?}", t.elapsed());
    let t = std::time::Instant::now();
    assert_eq!(find_profiles(&sk).len(), 152);
    assert!(t.elapsed().as_millis() < 500, "regions took {:?}", t.elapsed());
}
