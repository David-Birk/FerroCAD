//! Closed-loop detection: turns sketch curves into extrudable profiles.
//!
//! Current scope: every connected group of curves whose junctions all have
//! exactly two curves forms one loop; circles are loops on their own. Loops
//! nested inside other loops become holes. Branched networks (a line across
//! a rectangle splitting it into two regions) are not split into regions yet.

use crate::constraint::Constraint;
use crate::sketch::{Entity, PointId, Sketch};
use std::collections::HashMap;
use std::f64::consts::TAU;

/// Points closer than this are treated as connected even without an
/// explicit coincident constraint.
const MERGE_TOL: f64 = 1e-6;

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line { a: [f64; 2], b: [f64; 2] },
    /// Circular arc traversed from `from` to `to`, counter-clockwise when
    /// `ccw` is true.
    Arc { center: [f64; 2], from: [f64; 2], to: [f64; 2], ccw: bool },
    /// Full circle (a loop by itself), traversed counter-clockwise when `ccw`.
    Circle { center: [f64; 2], radius: f64, ccw: bool },
}

impl Segment {
    pub fn start(&self) -> [f64; 2] {
        match *self {
            Segment::Line { a, .. } => a,
            Segment::Arc { from, .. } => from,
            Segment::Circle { center, radius, .. } => [center[0] + radius, center[1]],
        }
    }

    pub fn end(&self) -> [f64; 2] {
        match *self {
            Segment::Line { b, .. } => b,
            Segment::Arc { to, .. } => to,
            Segment::Circle { .. } => self.start(),
        }
    }

    pub fn reversed(&self) -> Segment {
        match *self {
            Segment::Line { a, b } => Segment::Line { a: b, b: a },
            Segment::Arc { center, from, to, ccw } => {
                Segment::Arc { center, from: to, to: from, ccw: !ccw }
            }
            Segment::Circle { center, radius, ccw } => Segment::Circle { center, radius, ccw: !ccw },
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

    /// Point at curve parameter `t` ∈ [0, 1].
    pub fn point_at(&self, t: f64) -> [f64; 2] {
        match *self {
            Segment::Line { a, b } => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
            Segment::Arc { center, from, to, ccw } => {
                let r = (from[0] - center[0]).hypot(from[1] - center[1]);
                let a0 = (from[1] - center[1]).atan2(from[0] - center[0]);
                let a = a0 + t * Self::arc_sweep(center, from, to, ccw);
                [center[0] + r * a.cos(), center[1] + r * a.sin()]
            }
            Segment::Circle { center, radius, ccw } => {
                let a = if ccw { t * TAU } else { -t * TAU };
                [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
            }
        }
    }

    /// Polyline approximation including both endpoints.
    pub fn sample(&self, steps: usize) -> Vec<[f64; 2]> {
        match self {
            Segment::Line { a, b } => vec![*a, *b],
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

/// Find every closed region in the sketch. Construction geometry is ignored.
pub fn find_profiles(sk: &Sketch) -> Vec<Profile> {
    let loops = find_loops(sk);

    // Every loop becomes an outer boundary; loops directly inside it are its
    // holes. Concentric circles thus give a disk *and* a ring, like Fusion.
    let polys: Vec<Vec<[f64; 2]>> = loops.iter().map(Loop::polygon).collect();
    let inside = |inner: usize, outer: usize| -> bool {
        inner != outer
            && polys[inner].iter().all(|p| point_in_polygon(*p, &polys[outer]))
            && polygon_area(&polys[inner]).abs() < polygon_area(&polys[outer]).abs()
    };

    let mut profiles = Vec::new();
    for o in 0..loops.len() {
        let children: Vec<usize> = (0..loops.len()).filter(|&i| inside(i, o)).collect();
        // direct children only: not inside another child
        let holes = children
            .iter()
            .filter(|&&c| !children.iter().any(|&d| d != c && inside(c, d)))
            .map(|&c| loops[c].reversed())
            .collect();
        profiles.push(Profile { outer: loops[o].clone(), holes });
    }
    profiles
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

/// All closed loops, oriented counter-clockwise.
pub fn find_loops(sk: &Sketch) -> Vec<Loop> {
    let p = sk.params();
    let mut uf = UnionFind::new(sk.points.len());
    for (_, c) in sk.constraints() {
        if let Constraint::Coincident(a, b) = *c {
            uf.union(a.0, b.0);
        }
    }

    let mut loops = Vec::new();
    // (curve endpoints as point ids, segment going a→b)
    let mut edges: Vec<(PointId, PointId, Segment)> = Vec::new();
    for (id, e) in sk.entities() {
        if sk.is_construction(id) {
            continue;
        }
        match *e {
            Entity::Line { a, b } => {
                edges.push((a, b, Segment::Line { a: sk.pt(p, a), b: sk.pt(p, b) }))
            }
            Entity::Arc { center, start, end } => edges.push((
                start,
                end,
                Segment::Arc {
                    center: sk.pt(p, center),
                    from: sk.pt(p, start),
                    to: sk.pt(p, end),
                    ccw: true,
                },
            )),
            Entity::Circle { center, radius } => loops.push(Loop {
                segments: vec![Segment::Circle {
                    center: sk.pt(p, center),
                    radius: p[radius.0],
                    ccw: true,
                }],
            }),
        }
    }

    // Merge endpoints that coincide numerically.
    let endpoints: Vec<PointId> = edges.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    for i in 0..endpoints.len() {
        for j in i + 1..endpoints.len() {
            let (a, b) = (sk.pt(p, endpoints[i]), sk.pt(p, endpoints[j]));
            if (a[0] - b[0]).hypot(a[1] - b[1]) < MERGE_TOL {
                uf.union(endpoints[i].0, endpoints[j].0);
            }
        }
    }

    // node -> incident edge indices
    let mut incident: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, (a, b, _)) in edges.iter().enumerate() {
        incident.entry(uf.find(a.0)).or_default().push(i);
        incident.entry(uf.find(b.0)).or_default().push(i);
    }

    let mut used = vec![false; edges.len()];
    for start_edge in 0..edges.len() {
        if used[start_edge] {
            continue;
        }
        let start_node = uf.find(edges[start_edge].0 .0);
        let mut segments = Vec::new();
        let mut node = start_node;
        let mut edge = start_edge;
        let mut ok = true;
        let mut visited = Vec::new();
        loop {
            if used[edge] {
                ok = false;
                break;
            }
            used[edge] = true;
            visited.push(edge);
            let (a, b, seg) = &edges[edge];
            let (na, nb) = (uf.find(a.0), uf.find(b.0));
            let (seg, next) = if na == node { (seg.clone(), nb) } else { (seg.reversed(), na) };
            segments.push(seg);
            node = next;
            if node == start_node {
                break;
            }
            let inc = &incident[&node];
            if inc.len() != 2 {
                ok = false; // open end or branch
                break;
            }
            edge = if inc[0] == edge { inc[1] } else { inc[0] };
        }
        // A valid loop also needs every node it passed to have degree 2.
        let degree_ok = visited.iter().all(|&e| {
            let (a, b, _) = &edges[e];
            incident[&uf.find(a.0)].len() == 2 && incident[&uf.find(b.0)].len() == 2
        });
        if ok && degree_ok && !segments.is_empty() {
            let lp = Loop { segments };
            loops.push(if lp.signed_area() < 0.0 { lp.reversed() } else { lp });
        }
    }
    loops
}

fn polygon_area(pts: &[[f64; 2]]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

fn point_in_polygon(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
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

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}
