//! Profiles: the closed regions of a sketch that can be extruded or
//! revolved. The region finding itself lives in `region.rs`.

use crate::sketch::Sketch;
use std::f64::consts::TAU;

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line { a: [f64; 2], b: [f64; 2] },
    /// Circular arc traversed from `from` to `to`, counter-clockwise when
    /// `ccw` is true.
    Arc { center: [f64; 2], from: [f64; 2], to: [f64; 2], ccw: bool },
    /// Full circle (a loop by itself), traversed counter-clockwise when `ccw`.
    Circle { center: [f64; 2], radius: f64, ccw: bool },
    /// Smooth curve through dense points, in traversal order (a spline or a
    /// piece of one). Kernels interpolate a B-spline through them.
    Spline { points: Vec<[f64; 2]> },
}

impl Segment {
    pub fn start(&self) -> [f64; 2] {
        match self {
            Segment::Line { a, .. } => *a,
            Segment::Arc { from, .. } => *from,
            Segment::Circle { center, radius, .. } => [center[0] + radius, center[1]],
            Segment::Spline { points } => points[0],
        }
    }

    pub fn end(&self) -> [f64; 2] {
        match self {
            Segment::Line { b, .. } => *b,
            Segment::Arc { to, .. } => *to,
            Segment::Circle { .. } => self.start(),
            Segment::Spline { points } => points[points.len() - 1],
        }
    }

    pub fn reversed(&self) -> Segment {
        match self {
            Segment::Line { a, b } => Segment::Line { a: *b, b: *a },
            Segment::Arc { center, from, to, ccw } => {
                Segment::Arc { center: *center, from: *to, to: *from, ccw: !ccw }
            }
            Segment::Circle { center, radius, ccw } => Segment::Circle { center: *center, radius: *radius, ccw: !ccw },
            Segment::Spline { points } => Segment::Spline { points: points.iter().rev().copied().collect() },
        }
    }

    /// Signed sweep angle of an arc in the traversal direction.
    fn arc_sweep(center: [f64; 2], from: [f64; 2], to: [f64; 2], ccw: bool) -> f64 {
        let a0 = (from[1] - center[1]).atan2(from[0] - center[0]);
        let a1 = (to[1] - center[1]).atan2(to[0] - center[0]);
        let mut sweep = a1 - a0;
        if ccw {
            while sweep <= 0.0 {
                sweep += TAU;
            }
        } else {
            while sweep >= 0.0 {
                sweep -= TAU;
            }
        }
        sweep
    }

    /// Point halfway along the curve – handy for 3-point arc construction.
    pub fn midpoint(&self) -> [f64; 2] {
        self.point_at(0.5)
    }

    /// Point at curve parameter `t` ∈ [0, 1] (arc length for splines).
    pub fn point_at(&self, t: f64) -> [f64; 2] {
        match self {
            Segment::Line { a, b } => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
            Segment::Arc { center, from, to, ccw } => {
                let (center, from) = (*center, *from);
                let r = (from[0] - center[0]).hypot(from[1] - center[1]);
                let a0 = (from[1] - center[1]).atan2(from[0] - center[0]);
                let a = a0 + t * Self::arc_sweep(center, from, *to, *ccw);
                [center[0] + r * a.cos(), center[1] + r * a.sin()]
            }
            Segment::Circle { center, radius, ccw } => {
                let a = if *ccw { t * TAU } else { -t * TAU };
                [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
            }
            Segment::Spline { points } => {
                let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
                let total: f64 = points.windows(2).map(|w| d(w[0], w[1])).sum();
                let mut left = t.clamp(0.0, 1.0) * total;
                for w in points.windows(2) {
                    let l = d(w[0], w[1]);
                    if left <= l && l > 0.0 {
                        let f = left / l;
                        return [w[0][0] + f * (w[1][0] - w[0][0]), w[0][1] + f * (w[1][1] - w[0][1])];
                    }
                    left -= l;
                }
                points[points.len() - 1]
            }
        }
    }

    /// Polyline approximation including both endpoints.
    pub fn sample(&self, steps: usize) -> Vec<[f64; 2]> {
        match self {
            Segment::Line { a, b } => vec![*a, *b],
            Segment::Spline { points } => points.clone(),
            _ => (0..=steps).map(|i| self.point_at(i as f64 / steps as f64)).collect(),
        }
    }
}

/// A closed chain of segments, each starting where the previous one ends.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub segments: Vec<Segment>,
}

impl Loop {
    /// Closed polygon approximation (last point not repeated).
    pub fn polygon(&self) -> Vec<[f64; 2]> {
        let mut pts = Vec::new();
        for s in &self.segments {
            let mut sampled = s.sample(24);
            sampled.pop();
            pts.extend(sampled);
        }
        pts
    }

    /// Exact enclosed area (Green's theorem per segment), positive for
    /// counter-clockwise loops.
    pub fn signed_area(&self) -> f64 {
        self.segments
            .iter()
            .map(|s| match *s {
                Segment::Spline { ref points } => {
                    points.windows(2).map(|w| 0.5 * (w[0][0] * w[1][1] - w[1][0] * w[0][1])).sum()
                }
                Segment::Line { a, b } => 0.5 * (a[0] * b[1] - b[0] * a[1]),
                Segment::Arc { center: c, from, to, ccw } => {
                    let r2 = (from[0] - c[0]).powi(2) + (from[1] - c[1]).powi(2);
                    let sweep = Segment::arc_sweep(c, from, to, ccw);
                    0.5 * (r2 * sweep + c[0] * (to[1] - from[1]) - c[1] * (to[0] - from[0]))
                }
                Segment::Circle { radius, ccw, .. } => {
                    let a = std::f64::consts::PI * radius * radius;
                    if ccw {
                        a
                    } else {
                        -a
                    }
                }
            })
            .sum()
    }

    pub fn reversed(&self) -> Loop {
        Loop { segments: self.segments.iter().rev().map(Segment::reversed).collect() }
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        point_in_polygon(p, &self.polygon())
    }
}

/// A region bounded by a counter-clockwise outer loop with clockwise holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Loop,
    pub holes: Vec<Loop>,
}

impl Profile {
    pub fn area(&self) -> f64 {
        self.outer.signed_area() + self.holes.iter().map(Loop::signed_area).sum::<f64>()
    }

    pub fn contains(&self, p: [f64; 2]) -> bool {
        self.outer.contains(p) && !self.holes.iter().any(|h| h.contains(p))
    }
}

/// Find every closed region in the sketch – including regions formed by
/// crossing curves. Construction geometry is ignored. Each profile is a
/// counter-clockwise outer loop with clockwise holes.
pub fn find_profiles(sk: &Sketch) -> Vec<Profile> {
    crate::region::regions(sk)
}

/// Smallest profile containing `p` – what a click in the viewport selects.
pub fn profile_at(profiles: &[Profile], p: [f64; 2]) -> Option<usize> {
    profiles
        .iter()
        .enumerate()
        .filter(|(_, pr)| pr.contains(p))
        .min_by(|a, b| a.1.area().abs().total_cmp(&b.1.area().abs()))
        .map(|(i, _)| i)
}

pub(crate) fn point_in_polygon(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}
