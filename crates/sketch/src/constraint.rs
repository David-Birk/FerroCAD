use crate::sketch::{dist, Entity, EntityId, PointId, Sketch};
use std::f64::consts::PI;

/// Geometric relations and driving dimensions. Lengths are in model units
/// (millimetres), angles in radians.
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    // --- geometric -------------------------------------------------------
    Coincident(PointId, PointId),
    Horizontal(EntityId),
    Vertical(EntityId),
    Parallel(EntityId, EntityId),
    Perpendicular(EntityId, EntityId),
    /// Equal length (lines) or equal radius (circles/arcs).
    Equal(EntityId, EntityId),
    PointOnLine(PointId, EntityId),
    PointOnCircle(PointId, EntityId),
    Midpoint(PointId, EntityId),
    /// Line tangent to a circle or arc.
    Tangent(EntityId, EntityId),
    Concentric(EntityId, EntityId),

    // --- dimensions ------------------------------------------------------
    Distance(PointId, PointId, f64),
    Length(EntityId, f64),
    /// Signed: `b.x - a.x = d`.
    HorizontalDistance(PointId, PointId, f64),
    /// Signed: `b.y - a.y = d`.
    VerticalDistance(PointId, PointId, f64),
    Radius(EntityId, f64),
    Diameter(EntityId, f64),
    /// Angle from line 1 to line 2, counter-clockwise.
    Angle(EntityId, EntityId, f64),
}

impl Constraint {
    pub fn is_dimension(&self) -> bool {
        self.value().is_some()
    }

    pub fn value(&self) -> Option<f64> {
        use Constraint::*;
        match *self {
            Distance(_, _, v)
            | Length(_, v)
            | HorizontalDistance(_, _, v)
            | VerticalDistance(_, _, v)
            | Radius(_, v)
            | Diameter(_, v)
            | Angle(_, _, v) => Some(v),
            _ => None,
        }
    }

    pub fn value_mut(&mut self) -> Option<&mut f64> {
        use Constraint::*;
        match self {
            Distance(_, _, v)
            | Length(_, v)
            | HorizontalDistance(_, _, v)
            | VerticalDistance(_, _, v)
            | Radius(_, v)
            | Diameter(_, v)
            | Angle(_, _, v) => Some(v),
            _ => None,
        }
    }

    pub fn entities(&self) -> Vec<EntityId> {
        use Constraint::*;
        match *self {
            Horizontal(e) | Vertical(e) | Length(e, _) | Radius(e, _) | Diameter(e, _) => vec![e],
            PointOnLine(_, e) | PointOnCircle(_, e) | Midpoint(_, e) => vec![e],
            Parallel(a, b)
            | Perpendicular(a, b)
            | Equal(a, b)
            | Tangent(a, b)
            | Concentric(a, b)
            | Angle(a, b, _) => vec![a, b],
            Coincident(..) | Distance(..) | HorizontalDistance(..) | VerticalDistance(..) => {
                vec![]
            }
        }
    }

    pub fn points(&self) -> Vec<PointId> {
        use Constraint::*;
        match *self {
            Coincident(a, b)
            | Distance(a, b, _)
            | HorizontalDistance(a, b, _)
            | VerticalDistance(a, b, _) => vec![a, b],
            PointOnLine(p, _) | PointOnCircle(p, _) | Midpoint(p, _) => vec![p],
            _ => vec![],
        }
    }

    /// Short label for the UI.
    pub fn label(&self) -> &'static str {
        use Constraint::*;
        match self {
            Coincident(..) => "Coincident",
            Horizontal(..) => "Horizontal",
            Vertical(..) => "Vertical",
            Parallel(..) => "Parallel",
            Perpendicular(..) => "Perpendicular",
            Equal(..) => "Equal",
            PointOnLine(..) | PointOnCircle(..) => "On curve",
            Midpoint(..) => "Midpoint",
            Tangent(..) => "Tangent",
            Concentric(..) => "Concentric",
            Distance(..) => "Distance",
            Length(..) => "Length",
            HorizontalDistance(..) => "Horizontal distance",
            VerticalDistance(..) => "Vertical distance",
            Radius(..) => "Radius",
            Diameter(..) => "Diameter",
            Angle(..) => "Angle",
        }
    }

    /// Append this constraint's residuals (zero when satisfied). Constraints
    /// applied to the wrong entity kind contribute nothing.
    pub(crate) fn residuals(&self, sk: &Sketch, p: &[f64], out: &mut Vec<f64>) {
        use Constraint::*;
        match *self {
            Coincident(a, b) => {
                let (a, b) = (sk.pt(p, a), sk.pt(p, b));
                out.push(b[0] - a[0]);
                out.push(b[1] - a[1]);
            }
            Horizontal(l) => {
                if let Some((a, b)) = sk.line(p, l) {
                    out.push(b[1] - a[1]);
                }
            }
            Vertical(l) => {
                if let Some((a, b)) = sk.line(p, l) {
                    out.push(b[0] - a[0]);
                }
            }
            Parallel(l1, l2) => {
                if let (Some(d1), Some(d2)) = (dir(sk, p, l1), dir(sk, p, l2)) {
                    out.push(cross(d1, d2) / (norm(d1) * norm(d2)).max(1e-12));
                }
            }
            Perpendicular(l1, l2) => {
                if let (Some(d1), Some(d2)) = (dir(sk, p, l1), dir(sk, p, l2)) {
                    out.push(dot(d1, d2) / (norm(d1) * norm(d2)).max(1e-12));
                }
            }
            Equal(e1, e2) => {
                if let (Some((a1, b1)), Some((a2, b2))) = (sk.line(p, e1), sk.line(p, e2)) {
                    out.push(dist(a1, b1) - dist(a2, b2));
                } else if let (Some((_, r1)), Some((_, r2))) = (sk.circle(p, e1), sk.circle(p, e2))
                {
                    out.push(r1 - r2);
                }
            }
            PointOnLine(pt, l) => {
                if let Some((a, b)) = sk.line(p, l) {
                    let q = sk.pt(p, pt);
                    let d = sub(b, a);
                    out.push(cross(d, sub(q, a)) / norm(d).max(1e-12));
                }
            }
            PointOnCircle(pt, c) => {
                if let Some((center, r)) = sk.circle(p, c) {
                    out.push(dist(center, sk.pt(p, pt)) - r);
                }
            }
            Midpoint(pt, l) => {
                if let Some((a, b)) = sk.line(p, l) {
                    let q = sk.pt(p, pt);
                    out.push(q[0] - 0.5 * (a[0] + b[0]));
                    out.push(q[1] - 0.5 * (a[1] + b[1]));
                }
            }
            Tangent(l, c) => {
                // Accept the arguments in either order.
                let (l, c) = if sk.line(p, l).is_some() { (l, c) } else { (c, l) };
                if let (Some((a, b)), Some((center, r))) = (sk.line(p, l), sk.circle(p, c)) {
                    let d = sub(b, a);
                    let signed = cross(d, sub(center, a)) / norm(d).max(1e-12);
                    out.push(signed.abs() - r);
                }
            }
            Concentric(c1, c2) => {
                if let (Some((a, _)), Some((b, _))) = (sk.circle(p, c1), sk.circle(p, c2)) {
                    out.push(b[0] - a[0]);
                    out.push(b[1] - a[1]);
                }
            }
            Distance(a, b, d) => out.push(dist(sk.pt(p, a), sk.pt(p, b)) - d),
            Length(l, d) => {
                if let Some((a, b)) = sk.line(p, l) {
                    out.push(dist(a, b) - d);
                }
            }
            HorizontalDistance(a, b, d) => out.push(sk.pt(p, b)[0] - sk.pt(p, a)[0] - d),
            VerticalDistance(a, b, d) => out.push(sk.pt(p, b)[1] - sk.pt(p, a)[1] - d),
            Radius(c, r) => {
                if let Some((_, cr)) = sk.circle(p, c) {
                    out.push(cr - r);
                }
            }
            Diameter(c, d) => {
                if let Some((_, cr)) = sk.circle(p, c) {
                    out.push(2.0 * cr - d);
                }
            }
            Angle(l1, l2, a) => {
                if let (Some(d1), Some(d2)) = (dir(sk, p, l1), dir(sk, p, l2)) {
                    let current = cross(d1, d2).atan2(dot(d1, d2));
                    out.push(wrap_angle(current - a));
                }
            }
        }
    }
}

/// Residuals that come from the entities themselves rather than from user
/// constraints (currently: an arc's end stays on its circle).
pub(crate) fn implicit_residuals(sk: &Sketch, p: &[f64], out: &mut Vec<f64>) {
    for slot in sk.entities.iter().filter(|s| s.alive) {
        if let Entity::Arc { center, start, end } = slot.entity {
            let c = sk.pt(p, center);
            out.push(dist(c, sk.pt(p, end)) - dist(c, sk.pt(p, start)));
        }
    }
}

fn dir(sk: &Sketch, p: &[f64], l: EntityId) -> Option<[f64; 2]> {
    sk.line(p, l).map(|(a, b)| sub(b, a))
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn norm(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

fn wrap_angle(a: f64) -> f64 {
    let mut a = a % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    } else if a < -PI {
        a += 2.0 * PI;
    }
    a
}
