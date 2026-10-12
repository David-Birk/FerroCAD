//! Planar arrangement: splits sketch curves at every intersection and
//! finds the faces of the resulting planar graph. This is what lets a line
//! drawn across a rectangle create two selectable regions, or two
//! overlapping circles create three – exactly like Fusion's profiles.
//!
//! Steps:
//! 1. collect curves (lines, arcs, circles) that are not construction
//! 2. intersect every pair, split curves at interior intersection points
//! 3. merge nearby vertices, drop duplicate and dangling edges
//! 4. walk half-edges ("turn right at every vertex") to get face cycles;
//!    counter-clockwise cycles are bounded faces, clockwise cycles are the
//!    outer boundaries of connected components
//! 5. nest components: a component's outer boundary becomes a hole of the
//!    smallest face (of another component) that contains it

use crate::profile::{point_in_polygon, Loop, Profile, Segment};
use crate::sketch::{Entity, Sketch};
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug)]
enum Curve {
    /// `spline` tags pieces of a sketch spline so loops can merge them back.
    Line { a: [f64; 2], b: [f64; 2], spline: Option<u32> },
    /// Counter-clockwise from angle `a0` by `sweep` (0 < sweep ≤ 2π).
    Arc { c: [f64; 2], r: f64, a0: f64, sweep: f64 },
}

impl Curve {
    fn closed(&self) -> bool {
        matches!(*self, Curve::Arc { sweep, .. } if sweep >= TAU - 1e-12)
    }

    fn point(&self, s: f64) -> [f64; 2] {
        match *self {
            Curve::Line { a, b, .. } => [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1])],
            Curve::Arc { c, r, a0, sweep } => {
                let t = a0 + s * sweep;
                [c[0] + r * t.cos(), c[1] + r * t.sin()]
            }
        }
    }

    fn length(&self) -> f64 {
        match *self {
            Curve::Line { a, b, .. } => dist(a, b),
            Curve::Arc { r, sweep, .. } => r * sweep,
        }
    }

    /// Parameter of angle `phi` on an arc, if it lies on the arc.
    fn arc_param(a0: f64, sweep: f64, phi: f64, tol: f64) -> Option<f64> {
        let mut rel = (phi - a0).rem_euclid(TAU);
        if rel > TAU - tol {
            rel -= TAU; // just before the start counts as the start
        }
        let s = rel / sweep;
        (s >= -tol / sweep && s <= 1.0 + tol / sweep).then_some(s.clamp(0.0, 1.0))
    }

    /// Sub-curve between parameters `s0 < s1`.
    fn piece(&self, s0: f64, s1: f64) -> Curve {
        match *self {
            Curve::Line { spline, .. } => Curve::Line { a: self.point(s0), b: self.point(s1), spline },
            Curve::Arc { c, r, a0, sweep } => {
                Curve::Arc { c, r, a0: a0 + s0 * sweep, sweep: (s1 - s0) * sweep }
            }
        }
    }

    fn segment(&self) -> Segment {
        match *self {
            Curve::Line { a, b, .. } => Segment::Line { a, b },
            Curve::Arc { c, r, .. } if self.closed() => Segment::Circle { center: c, radius: r, ccw: true },
            Curve::Arc { c, .. } => Segment::Arc { center: c, from: self.point(0.0), to: self.point(1.0), ccw: true },
        }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

/// Intersection parameters (on `p`, on `q`). `tol` is a length.
fn intersect(p: &Curve, q: &Curve, tol: f64) -> Vec<(f64, f64)> {
    match (*p, *q) {
        (Curve::Line { a, b, .. }, Curve::Line { a: c, b: d, .. }) => {
            let (d1, d2) = (sub(b, a), sub(d, c));
            let den = cross(d1, d2);
            let (l1, l2) = (dist(a, b), dist(c, d));
            if den.abs() <= 1e-12 * l1 * l2 {
                return vec![]; // parallel; collinear overlap is not split
            }
            let t = cross(sub(c, a), d2) / den;
            let u = cross(sub(c, a), d1) / den;
            let (tt, tu) = (tol / l1, tol / l2);
            if t >= -tt && t <= 1.0 + tt && u >= -tu && u <= 1.0 + tu {
                vec![(t.clamp(0.0, 1.0), u.clamp(0.0, 1.0))]
            } else {
                vec![]
            }
        }
        (Curve::Line { .. }, Curve::Arc { .. }) => line_arc(p, q, tol),
        (Curve::Arc { .. }, Curve::Line { .. }) => {
            line_arc(q, p, tol).into_iter().map(|(t, s)| (s, t)).collect()
        }
        (Curve::Arc { c: c1, r: r1, a0: s1, sweep: w1 }, Curve::Arc { c: c2, r: r2, a0: s2, sweep: w2 }) => {
            let d = dist(c1, c2);
            if d < tol || d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
                return vec![]; // concentric (incl. identical) or apart
            }
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a * a).max(0.0).sqrt();
            let e = [(c2[0] - c1[0]) / d, (c2[1] - c1[1]) / d];
            let m = [c1[0] + a * e[0], c1[1] + a * e[1]];
            let mut pts = vec![[m[0] - h * e[1], m[1] + h * e[0]]];
            if h > tol {
                pts.push([m[0] + h * e[1], m[1] - h * e[0]]);
            }
            pts.into_iter()
                .filter_map(|x| {
                    let p1 = Curve::arc_param(s1, w1, (x[1] - c1[1]).atan2(x[0] - c1[0]), tol / r1)?;
                    let p2 = Curve::arc_param(s2, w2, (x[1] - c2[1]).atan2(x[0] - c2[0]), tol / r2)?;
                    Some((p1, p2))
                })
                .collect()
        }
    }
}

fn line_arc(l: &Curve, arc: &Curve, tol: f64) -> Vec<(f64, f64)> {
    let (Curve::Line { a, b, .. }, Curve::Arc { c, r, a0, sweep }) = (*l, *arc) else { return vec![] };
    let d = sub(b, a);
    let f = sub(a, c);
    let qa = d[0] * d[0] + d[1] * d[1];
    let qb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let qc = f[0] * f[0] + f[1] * f[1] - r * r;
    let len = qa.sqrt();
    // Distance from centre to the infinite line, to accept tangency within tol.
    let dist_line = cross(d, sub(c, a)).abs() / len;
    if dist_line > r + tol {
        return vec![];
    }
    let disc = (qb * qb - 4.0 * qa * qc).max(0.0);
    let sq = disc.sqrt();
    let mut ts = vec![(-qb - sq) / (2.0 * qa)];
    if dist_line < r - tol {
        ts.push((-qb + sq) / (2.0 * qa));
    }
    let tt = tol / len;
    ts.into_iter()
        .filter(|t| *t >= -tt && *t <= 1.0 + tt)
        .filter_map(|t| {
            let t = t.clamp(0.0, 1.0);
            let x = [a[0] + t * d[0], a[1] + t * d[1]];
            let s = Curve::arc_param(a0, sweep, (x[1] - c[1]).atan2(x[0] - c[0]), tol / r)?;
            Some((t, s))
        })
        .collect()
}

fn collect_curves(sk: &Sketch) -> Vec<Curve> {
    let mut out = Vec::new();
    for (id, e) in sk.entities() {
        if sk.is_construction(id) {
            continue;
        }
        match *e {
            Entity::Point { .. } => {}
            Entity::Line { a, b } => {
                let (a, b) = (sk.point_pos(a), sk.point_pos(b));
                if dist(a, b) > 0.0 {
                    out.push(Curve::Line { a, b, spline: None });
                }
            }
            Entity::Spline { .. } => {
                let pts = sk.spline_points(id, 12);
                for w in pts.windows(2) {
                    if dist(w[0], w[1]) > 0.0 {
                        out.push(Curve::Line { a: w[0], b: w[1], spline: Some(id.0 as u32) });
                    }
                }
            }
            Entity::Circle { center, radius } => {
                let r = sk.param(radius);
                if r > 0.0 {
                    out.push(Curve::Arc { c: sk.point_pos(center), r, a0: 0.0, sweep: TAU });
                }
            }
            Entity::Arc { center, start, end } => {
                let (c, s, e) = (sk.point_pos(center), sk.point_pos(start), sk.point_pos(end));
                let r = dist(c, s);
                if r > 0.0 {
                    let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
                    let a1 = (e[1] - c[1]).atan2(e[0] - c[0]);
                    let mut sweep = (a1 - a0).rem_euclid(TAU);
                    if sweep < 1e-12 {
                        sweep = TAU; // start == end: full circle
                    }
                    out.push(Curve::Arc { c, r, a0, sweep });
                }
            }
        }
    }
    out
}

struct Edge {
    u: usize,
    v: usize,
    curve: Curve,
}

/// All regions of the sketch as profiles.
pub fn regions(sk: &Sketch) -> Vec<Profile> {
    let curves = collect_curves(sk);
    if curves.is_empty() {
        return vec![];
    }
    let scale = curves
        .iter()
        .flat_map(|c| [c.point(0.0), c.point(1.0)])
        .fold(1.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
    let tol = 1e-6 * scale;

    // ---- 2. split parameters per curve
    let mut splits: Vec<Vec<f64>> = vec![Vec::new(); curves.len()];
    for i in 0..curves.len() {
        for j in i + 1..curves.len() {
            for (si, sj) in intersect(&curves[i], &curves[j], tol) {
                splits[i].push(si);
                splits[j].push(sj);
            }
        }
    }

    // ---- 3. pieces and vertices
    let mut vertices: Vec<[f64; 2]> = Vec::new();
    let mut vertex = |p: [f64; 2]| -> usize {
        match vertices.iter().position(|q| dist(*q, p) <= tol) {
            Some(i) => i,
            None => {
                vertices.push(p);
                vertices.len() - 1
            }
        }
    };
    let mut edges: Vec<Edge> = Vec::new();
    let mut free_circles: Vec<Curve> = Vec::new();

    for (curve, mut s) in curves.iter().zip(splits) {
        let ptol = (tol / curve.length()).min(1e-3);
        s.sort_by(f64::total_cmp);
        s.dedup_by(|a, b| (*a - *b).abs() <= ptol);
        let mut cuts: Vec<(f64, f64)> = Vec::new();
        if curve.closed() {
            // Wrap-around: s=0 and s=1 are the same point on a circle.
            if s.len() > 1 && (s[0] + 1.0 - s[s.len() - 1]) <= ptol {
                s.pop();
            }
            match s.len() {
                0 => {
                    free_circles.push(*curve);
                    continue;
                }
                1 => cuts.push((s[0], s[0] + 1.0)),
                n => {
                    for k in 0..n {
                        let next = if k + 1 < n { s[k + 1] } else { s[0] + 1.0 };
                        cuts.push((s[k], next));
                    }
                }
            }
        } else {
            let mut bounds = vec![0.0];
            bounds.extend(s.into_iter().filter(|x| *x > ptol && *x < 1.0 - ptol));
            bounds.push(1.0);
            cuts.extend(bounds.windows(2).map(|w| (w[0], w[1])));
        }
        for (s0, s1) in cuts {
            let piece = curve.piece(s0, s1);
            let (u, v) = (vertex(piece.point(0.0)), vertex(piece.point(1.0)));
            if u == v {
                // One cut on a circle gives a closed piece: it still bounds
                // a disk, so keep it as a free circle.
                if piece.closed() {
                    if let Curve::Arc { c, r, .. } = piece {
                        free_circles.push(Curve::Arc { c, r, a0: 0.0, sweep: TAU });
                    }
                }
                continue;
            }
            // drop duplicates (the same curve drawn twice)
            let mid = piece.point(0.5);
            let dup = edges.iter().any(|e| {
                ((e.u == u && e.v == v) || (e.u == v && e.v == u)) && dist(e.curve.point(0.5), mid) <= tol
            });
            if !dup {
                edges.push(Edge { u, v, curve: piece });
            }
        }
    }
    let nv = vertices.len();

    // ---- prune dangling edges
    let mut alive = vec![true; edges.len()];
    loop {
        let mut degree = vec![0usize; nv];
        for (e, ok) in edges.iter().zip(&alive) {
            if *ok {
                degree[e.u] += 1;
                degree[e.v] += 1;
            }
        }
        let mut removed = false;
        for (i, e) in edges.iter().enumerate() {
            if alive[i] && (degree[e.u] < 2 || degree[e.v] < 2) {
                alive[i] = false;
                removed = true;
            }
        }
        if !removed {
            break;
        }
    }

    // ---- 4. half-edges: 2i = u→v along the curve, 2i+1 = reverse
    let half_segment = |h: usize| -> (Segment, Option<u32>) {
        let curve = &edges[h / 2].curve;
        let seg = curve.segment();
        let tag = match *curve {
            Curve::Line { spline, .. } => spline,
            _ => None,
        };
        (if h.is_multiple_of(2) { seg } else { seg.reversed() }, tag)
    };
    let half_from = |h: usize| if h.is_multiple_of(2) { edges[h / 2].u } else { edges[h / 2].v };
    // Leaving direction, measured slightly along the curve so tangent ties
    // (a line and an arc leaving in the same direction) resolve correctly.
    let half_angle = |h: usize| -> f64 {
        let c = &edges[h / 2].curve;
        let (p0, p1) = if h.is_multiple_of(2) { (c.point(0.0), c.point(1e-4)) } else { (c.point(1.0), c.point(1.0 - 1e-4)) };
        (p1[1] - p0[1]).atan2(p1[0] - p0[0])
    };
    let mut outgoing: Vec<Vec<(f64, usize)>> = vec![Vec::new(); nv];
    for i in (0..edges.len()).filter(|&i| alive[i]) {
        for h in [2 * i, 2 * i + 1] {
            outgoing[half_from(h)].push((half_angle(h), h));
        }
    }
    for list in &mut outgoing {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let next = |h: usize| -> usize {
        let twin = h ^ 1;
        let list = &outgoing[half_from(twin)];
        let k = list.iter().position(|(_, x)| *x == twin).expect("twin is outgoing");
        list[(k + list.len() - 1) % list.len()].1
    };

    let mut used = vec![false; edges.len() * 2];
    let mut faces: Vec<(Loop, usize)> = Vec::new(); // (ccw loop, component vertex)
    let mut outers: Vec<(Loop, usize)> = Vec::new(); // (cw loop, component vertex)
    for start in 0..edges.len() * 2 {
        if used[start] || !alive[start / 2] {
            continue;
        }
        let mut segs = Vec::new();
        let mut h = start;
        let mut guard = 0;
        while !used[h] && guard <= edges.len() * 2 {
            used[h] = true;
            segs.push(half_segment(h));
            h = next(h);
            guard += 1;
        }
        let lp = Loop { segments: merge_spline_pieces(segs) };
        let anchor = half_from(start);
        let area = lp.signed_area();
        if area > tol * tol {
            faces.push((lp, anchor));
        } else if area < -tol * tol {
            outers.push((lp, anchor));
        }
    }

    // Free circles are components of their own: a disk face plus a CW outer.
    let mut uf = UnionFind::new(nv + free_circles.len());
    for (i, e) in edges.iter().enumerate() {
        if alive[i] {
            uf.union(e.u, e.v);
        }
    }
    for (k, c) in free_circles.iter().enumerate() {
        let lp = Loop { segments: vec![c.segment()] };
        faces.push((lp.clone(), nv + k));
        outers.push((lp.reversed(), nv + k));
    }

    // ---- 5. nesting
    let polys: Vec<Vec<[f64; 2]>> = faces.iter().map(|(l, _)| l.polygon()).collect();
    let areas: Vec<f64> = faces.iter().map(|(l, _)| l.signed_area()).collect();
    let mut holes: Vec<Vec<Loop>> = vec![Vec::new(); faces.len()];
    for (outer, comp) in &outers {
        let comp = uf.find(*comp);
        let probe = outer.segments[0].start();
        let host = (0..faces.len())
            .filter(|&f| uf.find(faces[f].1) != comp && point_in_polygon(probe, &polys[f]))
            .min_by(|&a, &b| areas[a].total_cmp(&areas[b]));
        if let Some(f) = host {
            holes[f].push(outer.clone());
        }
    }

    faces
        .into_iter()
        .zip(holes)
        .map(|((outer, _), holes)| Profile { outer, holes })
        .collect()
}

/// Merge consecutive pieces of the same spline into one spline segment
/// (also across the loop's start).
fn merge_spline_pieces(mut segs: Vec<(Segment, Option<u32>)>) -> Vec<Segment> {
    if segs.len() > 1 {
        // Rotate so the loop does not start in the middle of a spline run.
        let n = segs.len();
        if let Some(k) = (0..n).find(|&k| segs[k].1.is_none() || segs[k].1 != segs[(k + n - 1) % n].1) {
            segs.rotate_left(k);
        }
    }
    let mut out: Vec<Segment> = Vec::new();
    let mut run: Option<(u32, Vec<[f64; 2]>)> = None;
    let flush = |run: &mut Option<(u32, Vec<[f64; 2]>)>, out: &mut Vec<Segment>| {
        if let Some((_, points)) = run.take() {
            out.push(if points.len() == 2 { Segment::Line { a: points[0], b: points[1] } } else { Segment::Spline { points } });
        }
    };
    for (seg, tag) in segs {
        match (tag, &mut run) {
            (Some(t), Some((rt, pts))) if *rt == t => pts.push(seg.end()),
            (Some(t), _) => {
                flush(&mut run, &mut out);
                run = Some((t, vec![seg.start(), seg.end()]));
            }
            (None, _) => {
                flush(&mut run, &mut out);
                out.push(seg);
            }
        }
    }
    flush(&mut run, &mut out);
    out
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
