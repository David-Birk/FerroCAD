//! Projecting body edges into sketches. Lines, circles and arcs stay linked
//! to their edge (`Projection`) and follow it on every rebuild; other curves
//! become fixed polylines.

use crate::feature::{Projection, SketchFeature};
use crate::geom::Vec3;
use crate::kernel::{polyline_midpoint, EdgeRef};
use ferrocad_sketch::{Entity, EntityId, Sketch};

type V = [f64; 2];

fn dist(a: V, b: V) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// What an edge looks like in the sketch plane.
#[derive(Debug, PartialEq)]
enum Shape {
    Line(V, V),
    Circle(V, f64),
    /// Counter-clockwise from `start` to `end`.
    Arc { center: V, start: V, end: V },
    Polyline(Vec<V>),
}

fn circumcircle(p0: V, p1: V, p2: V) -> Option<(V, f64)> {
    let (a, b) = ([p1[0] - p0[0], p1[1] - p0[1]], [p2[0] - p0[0], p2[1] - p0[1]]);
    let d = 2.0 * (a[0] * b[1] - a[1] * b[0]);
    if d.abs() < 1e-12 {
        return None;
    }
    let (aa, bb) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1]);
    let c = [p0[0] + (b[1] * aa - a[1] * bb) / d, p0[1] + (a[0] * bb - b[0] * aa) / d];
    Some((c, dist(c, p0)))
}

fn classify(sf: &SketchFeature, poly: &[Vec3]) -> Result<Shape, String> {
    let uv: Vec<V> = poly.iter().map(|p| sf.plane.to_local(*p)).collect();
    let (Some(&first), Some(&last)) = (uv.first(), uv.last()) else { return Err("empty edge".into()) };
    let len = dist(first, last);
    let off_line = |p: &V| ((p[0] - first[0]) * (last[1] - first[1]) - (p[1] - first[1]) * (last[0] - first[0])).abs() / len.max(1e-12);
    if len > 1e-6 && uv.iter().all(|p| off_line(p) < 1e-6 * (1.0 + len)) {
        return Ok(Shape::Line(first, last));
    }
    let n = uv.len();
    if n < 3 {
        return Err("the edge projects to a point".into());
    }
    let closed = len < 1e-6;
    let (p0, p1, p2) = if closed { (uv[0], uv[n / 3], uv[2 * n / 3]) } else { (first, uv[n / 2], last) };
    if let Some((c, r)) = circumcircle(p0, p1, p2) {
        if uv.iter().all(|p| (dist(*p, c) - r).abs() < 1e-4 * (1.0 + r)) {
            if closed {
                return Ok(Shape::Circle(c, r));
            }
            let mid = uv[n / 2];
            let ccw = (mid[0] - first[0]) * (last[1] - mid[1]) - (mid[1] - first[1]) * (last[0] - mid[0]) > 0.0;
            let (start, end) = if ccw { (first, last) } else { (last, first) };
            return Ok(Shape::Arc { center: c, start, end });
        }
    }
    if uv.windows(2).all(|w| dist(w[0], w[1]) < 1e-9) {
        return Err("the edge projects to a point".into());
    }
    Ok(Shape::Polyline(uv))
}

fn fix_all(sk: &mut Sketch, e: EntityId) {
    for p in sk.entity(e).points() {
        sk.set_point_fixed(p, true);
    }
}

/// Project an edge into the sketch as fixed geometry; lines, circles and
/// arcs are remembered so they follow the edge.
pub fn project_edge(sf: &mut SketchFeature, poly: &[Vec3]) -> Result<(), String> {
    let shape = classify(sf, poly)?;
    let sk = &mut sf.sketch;
    let linked = match shape {
        Shape::Line(a, b) => {
            let l = sk.add_line(a, b);
            fix_all(sk, l);
            Some(l)
        }
        Shape::Circle(c, r) => {
            let circle = sk.add_circle(c, r);
            if let Entity::Circle { center, radius } = *sk.entity(circle) {
                sk.set_point_fixed(center, true);
                sk.set_param_fixed(radius, true);
            }
            Some(circle)
        }
        Shape::Arc { center, start, end } => {
            let arc = sk.add_arc(center, start, end);
            fix_all(sk, arc);
            Some(arc)
        }
        Shape::Polyline(pts) => {
            for w in pts.windows(2) {
                if dist(w[0], w[1]) > 1e-9 {
                    let l = sk.add_line(w[0], w[1]);
                    fix_all(sk, l);
                }
            }
            None
        }
    };
    if let (Some(entity), Some(mid)) = (linked, polyline_midpoint(poly)) {
        sf.projections.push(Projection { edge: EdgeRef { point: mid }, entity });
    }
    Ok(())
}

/// Move linked projected geometry to where its edges are now. `edges` are
/// the polylines of every body edge before this sketch in the timeline.
pub fn update_projections(sf: &mut SketchFeature, edges: &[Vec<Vec3>]) -> Result<(), String> {
    for k in 0..sf.projections.len() {
        let Projection { edge, entity } = sf.projections[k];
        if !sf.sketch.is_alive(entity) {
            continue;
        }
        // Nearest edge (by midpoint) that still has the same kind of shape –
        // and for lines the same direction – so the projection survives
        // parameter changes that move the edge.
        let mut candidates: Vec<(f64, &Vec<Vec3>)> = edges
            .iter()
            .filter_map(|e| polyline_midpoint(e).map(|m| (crate::geom::distance(m, edge.point), e)))
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        let current = sf.sketch.entity(entity).clone();
        let fits = |shape: &Shape| match (shape, &current) {
            (Shape::Line(a, b), Entity::Line { a: pa, b: pb }) => {
                let (oa, ob) = (sf.sketch.point_pos(*pa), sf.sketch.point_pos(*pb));
                let (d0, d1) = ([b[0] - a[0], b[1] - a[1]], [ob[0] - oa[0], ob[1] - oa[1]]);
                let cross = (d0[0] * d1[1] - d0[1] * d1[0]).abs();
                cross <= 1e-6 * (d0[0].hypot(d0[1]) * d1[0].hypot(d1[1])).max(1e-12)
            }
            (Shape::Circle(..), Entity::Circle { .. }) | (Shape::Arc { .. }, Entity::Arc { .. }) => true,
            _ => false,
        };
        let found = candidates
            .iter()
            .filter_map(|(_, e)| classify(sf, e).ok().filter(|s| fits(s)).map(|s| (s, (*e).clone())))
            .next();
        let (shape, poly) = found.ok_or("a projected edge disappeared – project it again")?;
        let sk = &mut sf.sketch;
        match (shape, current) {
            (Shape::Line(a, b), Entity::Line { a: pa, b: pb }) => {
                // Keep the end order closest to the old one.
                let (oa, ob) = (sk.point_pos(pa), sk.point_pos(pb));
                let (a, b) = if dist(oa, a) + dist(ob, b) <= dist(oa, b) + dist(ob, a) { (a, b) } else { (b, a) };
                sk.set_point_pos(pa, a);
                sk.set_point_pos(pb, b);
            }
            (Shape::Circle(c, r), Entity::Circle { center, radius }) => {
                sk.set_point_pos(center, c);
                sk.set_param(radius, r);
            }
            (Shape::Arc { center: c, start: s, end: e }, Entity::Arc { center, start, end }) => {
                sk.set_point_pos(center, c);
                sk.set_point_pos(start, s);
                sk.set_point_pos(end, e);
            }
            _ => unreachable!("filtered by `fits`"),
        }
        if let Some(m) = polyline_midpoint(&poly) {
            sf.projections[k].edge = EdgeRef { point: m };
        }
    }
    Ok(())
}
