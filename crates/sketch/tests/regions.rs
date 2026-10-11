//! Regions formed by crossing and branching curves.

use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::{find_profiles, Profile, Sketch};
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
