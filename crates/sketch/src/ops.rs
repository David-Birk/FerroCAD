//! Editing operations on sketch geometry: trim, extend, corner fillet,
//! offset, mirror, copies, and compound shapes (polygon, slot, centre
//! rectangle, three-point arc). They change geometry and add the constraints
//! Fusion would add; the caller solves afterwards.

use crate::constraint::Constraint;
use crate::sketch::{dist, ConstraintId, Entity, EntityId, PointId, Sketch};
use std::f64::consts::{PI, TAU};

const EPS: f64 = 1e-7;

type V = [f64; 2];
/// A cut along a curve: (parameter, cutting curve).
type Cut = (f64, EntityId);

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: V, s: f64) -> V {
    [a[0] * s, a[1] * s]
}
fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: V, b: V) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn norm(a: V) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: V) -> V {
    mul(a, 1.0 / norm(a).max(1e-300))
}
fn angle_of(c: V, p: V) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0]).rem_euclid(TAU)
}

/// A curve in closed form, parameterised: lines by t ∈ [0, 1], circles by
/// angle, arcs by the angle travelled from their start (0 … sweep).
#[derive(Clone, Copy, Debug)]
enum Curve {
    Line(V, V),
    Circle(V, f64),
    Arc { c: V, r: f64, start: f64, sweep: f64 },
}

impl Curve {
    fn point(&self, t: f64) -> V {
        match *self {
            Curve::Line(a, b) => add(a, mul(sub(b, a), t)),
            Curve::Circle(c, r) => [c[0] + r * t.cos(), c[1] + r * t.sin()],
            Curve::Arc { c, r, start, .. } => [c[0] + r * (start + t).cos(), c[1] + r * (start + t).sin()],
        }
    }

    /// Parameter of the point on the curve nearest `p` (lines unclamped).
    fn param(&self, p: V) -> f64 {
        match *self {
            Curve::Line(a, b) => {
                let d = sub(b, a);
                dot(sub(p, a), d) / dot(d, d).max(1e-300)
            }
            Curve::Circle(c, _) => angle_of(c, p),
            Curve::Arc { c, start, .. } => (angle_of(c, p) - start).rem_euclid(TAU),
        }
    }

    fn end(&self) -> f64 {
        match *self {
            Curve::Line(..) => 1.0,
            Curve::Circle(..) => TAU,
            Curve::Arc { sweep, .. } => sweep,
        }
    }

    fn contains(&self, t: f64, unbounded_line: bool) -> bool {
        match *self {
            Curve::Line(..) => unbounded_line || (-EPS..=1.0 + EPS).contains(&t),
            Curve::Circle(..) => true,
            Curve::Arc { sweep, .. } => t <= sweep + 1e-9 || t >= TAU - 1e-9,
        }
    }

    fn circle(&self) -> Option<(V, f64)> {
        match *self {
            Curve::Circle(c, r) | Curve::Arc { c, r, .. } => Some((c, r)),
            Curve::Line(..) => None,
        }
    }
}

/// Raw intersection points of two curves (as full lines/circles).
fn raw_intersections(a: &Curve, b: &Curve) -> Vec<V> {
    match (a.circle(), b.circle()) {
        (None, None) => {
            let (Curve::Line(p, q), Curve::Line(r, s)) = (*a, *b) else { return vec![] };
            let (d1, d2) = (sub(q, p), sub(s, r));
            let den = cross(d1, d2);
            if den.abs() < 1e-12 {
                return vec![];
            }
            let t = cross(sub(r, p), d2) / den;
            vec![add(p, mul(d1, t))]
        }
        (None, Some((c, r))) | (Some((c, r)), None) => {
            let line = if a.circle().is_none() { a } else { b };
            let Curve::Line(p, q) = *line else { return vec![] };
            let d = sub(q, p);
            let f = sub(p, c);
            let (qa, qb, qc) = (dot(d, d), 2.0 * dot(f, d), dot(f, f) - r * r);
            let disc = qb * qb - 4.0 * qa * qc;
            if disc < -1e-12 || qa < 1e-300 {
                return vec![];
            }
            let s = disc.max(0.0).sqrt();
            let mut v = vec![add(p, mul(d, (-qb - s) / (2.0 * qa)))];
            if s > 1e-12 {
                v.push(add(p, mul(d, (-qb + s) / (2.0 * qa))));
            }
            v
        }
        (Some((c1, r1)), Some((c2, r2))) => {
            let d = dist(c1, c2);
            if d < 1e-12 || d > r1 + r2 + 1e-9 || d < (r1 - r2).abs() - 1e-9 {
                return vec![];
            }
            let a_len = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a_len * a_len).max(0.0).sqrt();
            let u = unit(sub(c2, c1));
            let m = add(c1, mul(u, a_len));
            let n = [-u[1], u[0]];
            let mut v = vec![add(m, mul(n, h))];
            if h > 1e-12 {
                v.push(sub(m, mul(n, h)));
            }
            v
        }
    }
}

impl Sketch {
    /// Closed-form pieces of an entity (a spline gives its sample segments).
    fn curves_of(&self, e: EntityId) -> Vec<Curve> {
        match self.entity(e) {
            Entity::Spline { .. } => self.spline_points(e, 12).windows(2).map(|w| Curve::Line(w[0], w[1])).collect(),
            Entity::Point { .. } => vec![],
            _ => vec![self.curve(e)],
        }
    }

    fn curve(&self, e: EntityId) -> Curve {
        match *self.entity(e) {
            Entity::Spline { .. } => {
                let pts = self.spline_points(e, 12);
                Curve::Line(pts[0], pts[pts.len() - 1])
            }
            Entity::Line { a, b } => Curve::Line(self.point_pos(a), self.point_pos(b)),
            // Degenerate; callers reject points before asking for a curve.
            Entity::Point { p } => Curve::Line(self.point_pos(p), self.point_pos(p)),
            Entity::Circle { center, radius } => Curve::Circle(self.point_pos(center), self.param(radius)),
            Entity::Arc { center, start, end } => {
                let c = self.point_pos(center);
                let s = angle_of(c, self.point_pos(start));
                let e = angle_of(c, self.point_pos(end));
                let mut sweep = (e - s).rem_euclid(TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                Curve::Arc { c, r: dist(c, self.point_pos(start)), start: s, sweep }
            }
        }
    }

    /// Parameters along `e` where other curves cross or touch it, with the
    /// curve responsible. Sorted, without `e`'s own end points.
    fn cuts(&self, e: EntityId) -> Vec<(f64, EntityId)> {
        let me = self.curve(e);
        let mut out: Vec<(f64, EntityId)> = Vec::new();
        for (other, _) in self.entities() {
            if other == e {
                continue;
            }
            for oc in self.curves_of(other) {
            for p in raw_intersections(&me, &oc) {
                let t = me.param(p);
                let to = oc.param(p);
                if !me.contains(t, false) || !oc.contains(to, false) {
                    continue;
                }
                let interior = match me {
                    Curve::Circle(..) => true,
                    _ => t > EPS && t < me.end() - EPS,
                };
                if interior && !out.iter().any(|(u, _)| (u - t).abs() < 1e-6) {
                    out.push((t, other));
                }
            }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn set_entity(&mut self, e: EntityId, entity: Entity) {
        self.entities[e.0].entity = entity;
    }

    fn push_entity_pub(&mut self, entity: Entity) -> EntityId {
        self.entities.push(crate::sketch::EntitySlot { entity, construction: false, alive: true });
        EntityId(self.entities.len() - 1)
    }

    /// Remove constraints on points no live entity uses any more.
    fn drop_orphans(&mut self, pts: &[PointId]) {
        let orphaned: Vec<PointId> =
            pts.iter().copied().filter(|p| !self.entities().any(|(_, e)| e.points().contains(p))).collect();
        for slot in self.constraints.iter_mut() {
            if slot.as_ref().is_some_and(|c| c.points().iter().any(|p| orphaned.contains(p))) {
                *slot = None;
            }
        }
        for p in orphaned {
            self.set_point_fixed(p, false);
        }
    }

    /// Size constraints of `e` that a change of its extent invalidates.
    fn drop_size_constraints(&mut self, e: EntityId) {
        for slot in self.constraints.iter_mut() {
            let stale = matches!(slot, Some(Constraint::Length(l, _)) | Some(Constraint::Equal(l, _)) if *l == e)
                || matches!(slot, Some(Constraint::Equal(_, l)) if *l == e);
            if stale {
                *slot = None;
            }
        }
    }

    /// Keep the new end point on the curve that cut it.
    fn stick_to(&mut self, p: PointId, curve: EntityId) {
        let c = match self.entity(curve) {
            Entity::Line { .. } => Constraint::PointOnLine(p, curve),
            _ => Constraint::PointOnCircle(p, curve),
        };
        self.constrain(c);
    }

    // ------------------------------------------------------------------ trim

    /// Remove the piece of `e` between the intersections around `at`. A curve
    /// without intersections is deleted.
    pub fn trim(&mut self, e: EntityId, at: V) -> Result<(), String> {
        if !self.is_alive(e) {
            return Err("curve was deleted".into());
        }
        if matches!(self.entity(e), Entity::Spline { .. }) {
            return self.trim_spline(e, at);
        }
        if matches!(self.entity(e), Entity::Point { .. }) {
            return Err("points cannot be trimmed – delete them instead".into());
        }
        let curve = self.curve(e);
        let cuts = self.cuts(e);
        let t = curve.param(at);
        match (self.entity(e).clone(), curve) {
            (_, Curve::Circle(..)) if cuts.len() < 2 => self.remove_entity(e),
            (Entity::Circle { center, .. }, Curve::Circle(c, r)) => {
                // Keep the arc from the cut after `at` round to the cut before it.
                let i = cuts.iter().position(|(u, _)| *u > t).unwrap_or(0);
                let (after, before) = (cuts[i], cuts[(i + cuts.len() - 1) % cuts.len()]);
                let s = self.add_point(Curve::Circle(c, r).point(after.0));
                let en = self.add_point(Curve::Circle(c, r).point(before.0));
                self.remove_entity(e);
                // The centre point stays: it may carry constraints.
                self.push_entity_pub(Entity::Arc { center, start: s, end: en });
                self.stick_to(s, after.1);
                self.stick_to(en, before.1);
            }
            _ if cuts.is_empty() => self.remove_entity(e),
            (Entity::Line { a, b }, Curve::Line(..)) => {
                let t = t.clamp(0.0, 1.0);
                let lo = cuts.iter().rev().find(|(u, _)| *u < t).copied();
                let hi = cuts.iter().find(|(u, _)| *u > t).copied();
                self.drop_size_constraints(e);
                match (lo, hi) {
                    (None, Some((u, by))) => {
                        let p = self.add_point(curve.point(u));
                        self.set_entity(e, Entity::Line { a: p, b });
                        self.stick_to(p, by);
                        self.drop_orphans(&[a]);
                    }
                    (Some((u, by)), None) => {
                        let p = self.add_point(curve.point(u));
                        self.set_entity(e, Entity::Line { a, b: p });
                        self.stick_to(p, by);
                        self.drop_orphans(&[b]);
                    }
                    (Some((u0, by0)), Some((u1, by1))) => {
                        let p0 = self.add_point(curve.point(u0));
                        let p1 = self.add_point(curve.point(u1));
                        self.set_entity(e, Entity::Line { a, b: p0 });
                        self.push_entity_pub(Entity::Line { a: p1, b });
                        self.stick_to(p0, by0);
                        self.stick_to(p1, by1);
                    }
                    (None, None) => self.remove_entity(e),
                }
            }
            (Entity::Arc { center, start, end }, Curve::Arc { .. }) => {
                let lo = cuts.iter().rev().find(|(u, _)| *u < t).copied();
                let hi = cuts.iter().find(|(u, _)| *u > t).copied();
                self.drop_size_constraints(e);
                match (lo, hi) {
                    (None, Some((u, by))) => {
                        let p = self.add_point(curve.point(u));
                        self.set_entity(e, Entity::Arc { center, start: p, end });
                        self.stick_to(p, by);
                        self.drop_orphans(&[start]);
                    }
                    (Some((u, by)), None) => {
                        let p = self.add_point(curve.point(u));
                        self.set_entity(e, Entity::Arc { center, start, end: p });
                        self.stick_to(p, by);
                        self.drop_orphans(&[end]);
                    }
                    (Some((u0, by0)), Some((u1, by1))) => {
                        let p0 = self.add_point(curve.point(u0));
                        let p1 = self.add_point(curve.point(u1));
                        self.set_entity(e, Entity::Arc { center, start, end: p0 });
                        self.push_entity_pub(Entity::Arc { center, start: p1, end });
                        self.stick_to(p0, by0);
                        self.stick_to(p1, by1);
                    }
                    (None, None) => self.remove_entity(e),
                }
            }
            _ => return Err("cannot trim this curve".into()),
        }
        Ok(())
    }

    /// Trim an open spline: the kept pieces become splines through the
    /// remaining fit points and the cut points.
    fn trim_spline(&mut self, e: EntityId, at: V) -> Result<(), String> {
        const PER_SPAN: usize = 12;
        let Entity::Spline { points: fit } = self.entity(e).clone() else { return Err("not a spline".into()) };
        if fit.first() == fit.last() {
            return Err("closed splines cannot be trimmed yet".into());
        }
        let samples = self.spline_points(e, PER_SPAN);
        let nseg = samples.len() - 1;
        // Cuts as (global parameter = segment index + local t, cutting curve).
        let mut cuts: Vec<(f64, EntityId)> = Vec::new();
        for (other, _) in self.entities() {
            if other == e {
                continue;
            }
            for oc in self.curves_of(other) {
                for (k, w) in samples.windows(2).enumerate() {
                    let seg = Curve::Line(w[0], w[1]);
                    for p in raw_intersections(&seg, &oc) {
                        let t = seg.param(p);
                        if !(-EPS..=1.0 + EPS).contains(&t) || !oc.contains(oc.param(p), false) {
                            continue;
                        }
                        let g = k as f64 + t.clamp(0.0, 1.0);
                        if g > 1e-6 && g < nseg as f64 - 1e-6 && !cuts.iter().any(|(u, _)| (u - g).abs() < 1e-6) {
                            cuts.push((g, other));
                        }
                    }
                }
            }
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        if cuts.is_empty() {
            self.remove_entity(e);
            return Ok(());
        }
        // Parameter of the click: nearest point on the sample polyline.
        let t_at = samples
            .windows(2)
            .enumerate()
            .map(|(k, w)| {
                let d = sub(w[1], w[0]);
                let t = (dot(sub(at, w[0]), d) / dot(d, d).max(1e-300)).clamp(0.0, 1.0);
                (dist(at, add(w[0], mul(d, t))), k as f64 + t)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t)| t)
            .unwrap_or(0.0);
        let point_at = |g: f64| {
            let k = (g.floor() as usize).min(nseg - 1);
            let t = g - k as f64;
            add(samples[k], mul(sub(samples[k + 1], samples[k]), t))
        };
        let fit_param = |i: usize| (i * PER_SPAN) as f64;
        let lo = cuts.iter().rev().find(|(u, _)| *u < t_at).copied();
        let hi = cuts.iter().find(|(u, _)| *u > t_at).copied();
        // Pieces to keep: (from, to) in global parameters, with the cut at each end.
        let mut pieces: Vec<(Option<Cut>, Option<Cut>)> = Vec::new();
        if let Some(l) = lo {
            pieces.push((None, Some(l)));
        }
        if let Some(h) = hi {
            pieces.push((Some(h), None));
        }
        let mut created = false;
        for (from, to) in pieces {
            let (g0, g1) = (from.map_or(0.0, |c| c.0), to.map_or(nseg as f64, |c| c.0));
            let mut pts: Vec<PointId> = Vec::new();
            if let Some((g, by)) = from {
                let p = self.add_point(point_at(g));
                self.stick_to_any(p, by);
                pts.push(p);
            }
            for (i, &f) in fit.iter().enumerate() {
                let g = fit_param(i);
                if g >= g0 - 1e-9 && g <= g1 + 1e-9 && !(from.is_some() && (g - g0).abs() < 1e-6) && !(to.is_some() && (g - g1).abs() < 1e-6) {
                    pts.push(f);
                }
            }
            if let Some((g, by)) = to {
                let p = self.add_point(point_at(g));
                self.stick_to_any(p, by);
                pts.push(p);
            }
            if pts.len() >= 2 {
                if created {
                    self.push_entity_pub(Entity::Spline { points: pts });
                } else {
                    self.set_entity(e, Entity::Spline { points: pts });
                    created = true;
                }
            }
        }
        if !created {
            self.remove_entity(e);
        }
        let used: Vec<PointId> = fit.clone();
        self.drop_orphans(&used);
        Ok(())
    }

    /// Like `stick_to`, but nothing for splines (no point-on-spline constraint).
    fn stick_to_any(&mut self, p: PointId, curve: EntityId) {
        if !matches!(self.entity(curve), Entity::Spline { .. }) {
            self.stick_to(p, curve);
        }
    }

    // ---------------------------------------------------------------- extend

    /// Extend the end of line `e` nearest `near` to the next curve it meets.
    pub fn extend(&mut self, e: EntityId, near: V) -> Result<(), String> {
        let Entity::Line { a, b } = *self.entity(e) else {
            return Err("only lines can be extended".into());
        };
        let (pa, pb) = (self.point_pos(a), self.point_pos(b));
        let at_b = dist(near, pb) <= dist(near, pa);
        let line = Curve::Line(pa, pb);
        let mut best: Option<(f64, V, EntityId)> = None;
        for (other, _) in self.entities() {
            if other == e {
                continue;
            }
            for oc in self.curves_of(other) {
            for p in raw_intersections(&line, &oc) {
                let t = line.param(p);
                if !oc.contains(oc.param(p), false) {
                    continue;
                }
                let beyond = if at_b { t - 1.0 } else { -t };
                if beyond > EPS && best.is_none_or(|bb| beyond < bb.0) {
                    best = Some((beyond, p, other));
                }
            }
            }
        }
        let (_, p, other) = best.ok_or("nothing to extend to")?;
        let end = if at_b { b } else { a };
        if self.entities().filter(|(_, en)| en.points().contains(&end)).count() > 1 {
            return Err("the end is connected to other curves".into());
        }
        self.set_point_pos(end, p);
        self.drop_size_constraints(e);
        self.stick_to(end, other);
        Ok(())
    }

    // ----------------------------------------------------------- fillet

    /// Round the corner where exactly two lines meet at `corner`. Returns the
    /// new arc and its radius dimension.
    pub fn fillet_corner(&mut self, corner: PointId, radius: f64) -> Result<(EntityId, ConstraintId), String> {
        let lines: Vec<(EntityId, PointId)> = self
            .entities()
            .filter_map(|(id, e)| match *e {
                Entity::Line { a, b } if a == corner => Some((id, b)),
                Entity::Line { a, b } if b == corner => Some((id, a)),
                _ => None,
            })
            .collect();
        let [(l1, far1), (l2, far2)] = lines[..] else {
            return Err("pick a corner where exactly two lines meet".into());
        };
        if radius <= 0.0 {
            return Err("radius must be positive".into());
        }
        let p = self.point_pos(corner);
        let (f1, f2) = (self.point_pos(far1), self.point_pos(far2));
        let (u1, u2) = (unit(sub(f1, p)), unit(sub(f2, p)));
        let phi = dot(u1, u2).clamp(-1.0, 1.0).acos();
        if phi < 1e-6 || (PI - phi) < 1e-6 {
            return Err("the lines are parallel".into());
        }
        let d = radius / (phi / 2.0).tan();
        if d >= dist(p, f1) - 1e-9 || d >= dist(p, f2) - 1e-9 {
            return Err("radius too large for these lines".into());
        }
        let t1 = add(p, mul(u1, d));
        let t2 = add(p, mul(u2, d));
        let c = add(p, mul(unit(add(u1, u2)), radius / (phi / 2.0).sin()));
        let (q1, q2, qc) = (self.add_point(t1), self.add_point(t2), self.add_point(c));
        let reconnect = |sk: &mut Sketch, l: EntityId, q: PointId| {
            if let Entity::Line { a, b } = *sk.entity(l) {
                let e = if a == corner { Entity::Line { a: q, b } } else { Entity::Line { a, b: q } };
                sk.set_entity(l, e);
            }
        };
        reconnect(self, l1, q1);
        reconnect(self, l2, q2);
        self.drop_size_constraints(l1);
        self.drop_size_constraints(l2);
        // Short arc between the tangent points, counter-clockwise.
        let (start, end) = if cross(sub(t1, c), sub(t2, c)) > 0.0 { (q1, q2) } else { (q2, q1) };
        let arc = self.push_entity_pub(Entity::Arc { center: qc, start, end });
        self.constrain(Constraint::Tangent(l1, arc));
        self.constrain(Constraint::Tangent(l2, arc));
        let dim = self.constrain(Constraint::Radius(arc, radius));
        self.drop_orphans(&[corner]);
        Ok((arc, dim))
    }

    // ----------------------------------------------------------- copies

    /// Copy entities with a point mapping. Points in `keep` map to themselves.
    /// `reverse_arcs` swaps arc ends (needed for reflections).
    fn copy_mapped(
        &mut self,
        ids: &[EntityId],
        f: impl Fn(V) -> V,
        keep: &dyn Fn(PointId) -> bool,
        reverse_arcs: bool,
    ) -> (Vec<EntityId>, Vec<(PointId, PointId)>) {
        let mut map: Vec<(PointId, PointId)> = Vec::new();
        let mut out = Vec::new();
        let get = |sk: &mut Sketch, p: PointId, map: &mut Vec<(PointId, PointId)>| -> PointId {
            if keep(p) {
                return p;
            }
            if let Some((_, q)) = map.iter().find(|(o, _)| *o == p) {
                return *q;
            }
            let q = sk.add_point(f(sk.point_pos(p)));
            map.push((p, q));
            q
        };
        for &id in ids {
            if !self.is_alive(id) {
                continue;
            }
            let construction = self.is_construction(id);
            if let Entity::Spline { points } = self.entity(id).clone() {
                let points = points.iter().map(|p| get(self, *p, &mut map)).collect();
                let new = self.push_entity_pub(Entity::Spline { points });
                self.set_construction(new, construction);
                out.push(new);
                continue;
            }
            let e = match *self.entity(id) {
                Entity::Line { a, b } => Entity::Line { a: get(self, a, &mut map), b: get(self, b, &mut map) },
                Entity::Circle { center, radius } => {
                    let r = self.param(radius);
                    let c = get(self, center, &mut map);
                    let rp = self.add_param(r);
                    Entity::Circle { center: c, radius: rp }
                }
                Entity::Arc { center, start, end } => {
                    let (c, s, en) = (get(self, center, &mut map), get(self, start, &mut map), get(self, end, &mut map));
                    if reverse_arcs {
                        Entity::Arc { center: c, start: en, end: s }
                    } else {
                        Entity::Arc { center: c, start: s, end: en }
                    }
                }
                Entity::Point { p } => Entity::Point { p: get(self, p, &mut map) },
                Entity::Spline { .. } => unreachable!("handled above"),
            };
            let new = self.push_entity_pub(e);
            self.set_construction(new, construction);
            out.push(new);
        }
        (out, map)
    }

    /// Mirror entities at a line, linked by symmetric constraints.
    pub fn mirror(&mut self, ids: &[EntityId], axis: EntityId) -> Result<Vec<EntityId>, String> {
        let Entity::Line { a, b } = *self.entity(axis) else {
            return Err("the mirror line must be a line".into());
        };
        let (la, lb) = (self.point_pos(a), self.point_pos(b));
        let d = unit(sub(lb, la));
        let reflect = move |p: V| {
            let v = sub(p, la);
            let along = mul(d, dot(v, d));
            add(la, sub(mul(along, 2.0), v))
        };
        let on_axis = |sk: &Sketch, p: PointId| {
            let q = sk.point_pos(p);
            cross(d, sub(q, la)).abs() < 1e-9
        };
        let snapshot = self.clone();
        let ids: Vec<EntityId> = ids.iter().copied().filter(|&i| i != axis).collect();
        let (out, map) = self.copy_mapped(&ids, reflect, &|p| on_axis(&snapshot, p), true);
        for (p, q) in map {
            self.constrain(Constraint::Symmetric(p, q, axis));
        }
        for (orig, copy) in ids.iter().zip(&out) {
            if matches!(self.entity(*orig), Entity::Circle { .. } | Entity::Arc { .. }) {
                self.constrain(Constraint::Equal(*orig, *copy));
            }
        }
        Ok(out)
    }

    /// Unconstrained copies, one per transform.
    pub fn copy_transformed(&mut self, ids: &[EntityId], transforms: &[&dyn Fn(V) -> V]) -> Vec<EntityId> {
        let mut out = Vec::new();
        for f in transforms {
            out.extend(self.copy_mapped(ids, f, &|_| false, false).0);
        }
        out
    }

    // ------------------------------------------------------------ offset

    /// Offset a connected chain of lines, a circle or an arc by `distance`
    /// towards the side of `toward`.
    pub fn offset(&mut self, ids: &[EntityId], distance: f64, toward: V) -> Result<Vec<EntityId>, String> {
        if ids.is_empty() {
            return Err("select curves to offset".into());
        }
        if distance <= 0.0 {
            return Err("distance must be positive".into());
        }
        if ids.iter().any(|e| matches!(self.entity(*e), Entity::Point { .. })) {
            return Err("points cannot be offset".into());
        }
        if let [single] = ids {
            if let Some((c, r)) = self.curve(*single).circle() {
                let outward = dist(toward, c) > r;
                let nr = if outward { r + distance } else { r - distance };
                if nr <= 1e-9 {
                    return Err("offset is larger than the radius".into());
                }
                let k = nr / r;
                let center = match *self.entity(*single) {
                    Entity::Circle { center, .. } | Entity::Arc { center, .. } => center,
                    _ => unreachable!(),
                };
                let scale_from = move |p: V| add(c, mul(sub(p, c), k));
                let (out, _) = self.copy_mapped(&[*single], scale_from, &|p| p == center, false);
                if let Entity::Circle { radius, .. } = *self.entity(out[0]) {
                    self.params[radius.0] = nr;
                }
                // The copy shares the centre point, so it is concentric already.
                return Ok(out);
            }
        }
        // Order the lines into a chain.
        let mut segs: Vec<(EntityId, PointId, PointId)> = Vec::new();
        for &id in ids {
            match *self.entity(id) {
                Entity::Line { a, b } => segs.push((id, a, b)),
                _ => return Err("offset supports a chain of lines, a circle or an arc".into()),
            }
        }
        let count = |p: PointId, segs: &[(EntityId, PointId, PointId)]| {
            segs.iter().filter(|(_, a, b)| *a == p || *b == p).count()
        };
        let start = segs.iter().position(|(_, a, b)| count(*a, &segs) == 1 || count(*b, &segs) == 1).unwrap_or(0);
        let mut chain: Vec<(EntityId, PointId, PointId)> = Vec::new();
        let mut first = segs.remove(start);
        if count(first.1, &segs) > 0 && count(first.2, &segs) == 0 && !segs.is_empty() {
            first = (first.0, first.2, first.1);
        }
        chain.push(first);
        while !segs.is_empty() {
            let end = chain.last().expect("non-empty").2;
            let j = segs
                .iter()
                .position(|(_, a, b)| *a == end || *b == end)
                .ok_or("the lines must form one connected chain")?;
            let (id, a, b) = segs.remove(j);
            chain.push(if a == end { (id, a, b) } else { (id, b, a) });
        }
        let closed = chain.len() > 2 && chain.first().expect("non-empty").1 == chain.last().expect("non-empty").2;
        // Side: for a loop, inside or outside; for an open chain, the side of
        // the segment nearest to `toward`.
        let verts: Vec<V> = chain.iter().map(|(_, a, _)| self.point_pos(*a)).collect();
        let left = if closed {
            let area: f64 = (0..verts.len()).map(|i| cross(verts[i], verts[(i + 1) % verts.len()])).sum();
            let mut inside = false;
            for i in 0..verts.len() {
                let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
                if (a[1] > toward[1]) != (b[1] > toward[1])
                    && toward[0] < a[0] + (toward[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                {
                    inside = !inside;
                }
            }
            // Left of a counter-clockwise loop is its inside.
            inside == (area > 0.0)
        } else {
            let (_, a, b) = chain
                .iter()
                .min_by(|x, y| {
                    let d = |(_, a, b): &&(EntityId, PointId, PointId)| {
                        let (pa, pb) = (self.point_pos(*a), self.point_pos(*b));
                        let t = (dot(sub(toward, pa), sub(pb, pa)) / dot(sub(pb, pa), sub(pb, pa)).max(1e-300)).clamp(0.0, 1.0);
                        dist(toward, add(pa, mul(sub(pb, pa), t)))
                    };
                    d(x).total_cmp(&d(y))
                })
                .expect("non-empty");
            let (pa, pb) = (self.point_pos(*a), self.point_pos(*b));
            cross(sub(pb, pa), sub(toward, pa)) > 0.0
        };
        let s = if left { distance } else { -distance };
        let shifted: Vec<(V, V)> = chain
            .iter()
            .map(|(_, a, b)| {
                let (pa, pb) = (self.point_pos(*a), self.point_pos(*b));
                let n = unit([-(pb[1] - pa[1]), pb[0] - pa[0]]);
                (add(pa, mul(n, s)), add(pb, mul(n, s)))
            })
            .collect();
        let n = shifted.len();
        let meet = |i: usize, j: usize| -> V {
            let (a, b) = shifted[i];
            let (c, d) = shifted[j];
            raw_intersections(&Curve::Line(a, b), &Curve::Line(c, d)).first().copied().unwrap_or(b)
        };
        let mut corners: Vec<V> = Vec::new();
        if closed {
            for i in 0..n {
                corners.push(meet((i + n - 1) % n, i));
            }
        } else {
            corners.push(shifted[0].0);
            for i in 1..n {
                corners.push(meet(i - 1, i));
            }
            corners.push(shifted[n - 1].1);
        }
        let pts: Vec<PointId> = corners.iter().map(|c| self.add_point(*c)).collect();
        let mut out = Vec::new();
        for i in 0..n {
            let (a, b) = if closed { (pts[i], pts[(i + 1) % n]) } else { (pts[i], pts[i + 1]) };
            let l = self.add_line_between(a, b);
            self.constrain(Constraint::Parallel(chain[i].0, l));
            out.push(l);
        }
        Ok(out)
    }

    // ------------------------------------------------------- compound shapes

    /// Regular polygon with `n` sides inscribed in a construction circle.
    pub fn add_polygon(&mut self, center: V, corner: V, n: usize) -> Result<Vec<EntityId>, String> {
        if !(3..=64).contains(&n) {
            return Err("a polygon needs 3 to 64 sides".into());
        }
        let r = dist(center, corner);
        if r < 1e-9 {
            return Err("polygon has no size".into());
        }
        let a0 = angle_of(center, corner);
        let circle = self.add_circle(center, r);
        self.set_construction(circle, true);
        let pts: Vec<PointId> = (0..n)
            .map(|i| {
                let a = a0 + TAU * i as f64 / n as f64;
                self.add_point([center[0] + r * a.cos(), center[1] + r * a.sin()])
            })
            .collect();
        let mut lines = Vec::new();
        for i in 0..n {
            lines.push(self.add_line_between(pts[i], pts[(i + 1) % n]));
            self.constrain(Constraint::PointOnCircle(pts[i], circle));
        }
        for i in 1..n {
            self.constrain(Constraint::Equal(lines[0], lines[i]));
        }
        Ok(lines)
    }

    /// Centre-to-centre slot of the given width.
    pub fn add_slot(&mut self, c1: V, c2: V, width: f64) -> Result<Vec<EntityId>, String> {
        let len = dist(c1, c2);
        if len < 1e-9 || width <= 0.0 {
            return Err("slot needs a length and a width".into());
        }
        let r = width / 2.0;
        let u = unit(sub(c2, c1));
        let n = [-u[1], u[0]];
        let corner = |c: V, s: f64| add(c, mul(n, s * r));
        let (pa, pb, pc, pd) = (corner(c1, 1.0), corner(c2, 1.0), corner(c2, -1.0), corner(c1, -1.0));
        let (a, b, c, d) = (self.add_point(pa), self.add_point(pb), self.add_point(pc), self.add_point(pd));
        let (k1, k2) = (self.add_point(c1), self.add_point(c2));
        let top = self.add_line_between(a, b);
        let bottom = self.add_line_between(c, d);
        let right = self.push_entity_pub(Entity::Arc { center: k2, start: c, end: b });
        let left = self.push_entity_pub(Entity::Arc { center: k1, start: a, end: d });
        for (l, arc) in [(top, right), (bottom, right), (top, left), (bottom, left)] {
            self.constrain(Constraint::Tangent(l, arc));
        }
        self.constrain(Constraint::Equal(left, right));
        Ok(vec![top, right, bottom, left])
    }

    /// Rectangle around `center` with a corner at `corner`, plus a
    /// construction diagonal whose midpoint is the centre.
    pub fn add_center_rectangle(&mut self, center: V, corner: V) -> [EntityId; 4] {
        let other = sub(mul(center, 2.0), corner);
        let lines = self.add_rectangle(other, corner);
        let (p0, p2) = match (self.entity(lines[0]).clone(), self.entity(lines[2]).clone()) {
            (Entity::Line { a, .. }, Entity::Line { a: c, .. }) => (a, c),
            _ => unreachable!(),
        };
        let diag = self.add_line_between(p0, p2);
        self.set_construction(diag, true);
        let mid = self.add_point(center);
        self.constrain(Constraint::Midpoint(mid, diag));
        lines
    }

    /// Arc through three points (start, any point on it, end).
    pub fn add_arc_3pt(&mut self, start: V, mid: V, end: V) -> Result<EntityId, String> {
        let (a, b) = (sub(mid, start), sub(end, start));
        let d = 2.0 * cross(a, b);
        if d.abs() < 1e-12 {
            return Err("the three points are on a line".into());
        }
        let (aa, bb) = (dot(a, a), dot(b, b));
        let c = add(start, [(b[1] * aa - a[1] * bb) / d, (a[0] * bb - b[0] * aa) / d]);
        // Counter-clockwise from start through mid to end?
        let ccw = cross(sub(mid, start), sub(end, mid)) > 0.0;
        Ok(if ccw { self.add_arc(c, start, end) } else { self.add_arc(c, end, start) })
    }

    /// Centre of rotation / scaling helpers for patterns in the UI.
    pub fn rotate_about(center: V, angle: f64) -> impl Fn(V) -> V {
        let (s, c) = angle.sin_cos();
        move |p: V| {
            let v = sub(p, center);
            add(center, [c * v[0] - s * v[1], s * v[0] + c * v[1]])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_pts(sk: &Sketch, l: EntityId) -> (V, V) {
        match *sk.entity(l) {
            Entity::Line { a, b } => (sk.point_pos(a), sk.point_pos(b)),
            _ => panic!("not a line"),
        }
    }

    fn close(a: V, b: V) -> bool {
        dist(a, b) < 1e-6
    }

    #[test]
    fn trim_line_middle_end_and_isolated() {
        let mut sk = Sketch::new();
        let h = sk.add_line([0.0, 0.0], [10.0, 0.0]);
        sk.add_line([3.0, -1.0], [3.0, 1.0]);
        sk.add_line([7.0, -1.0], [7.0, 1.0]);
        // Middle piece: the line splits in two.
        sk.trim(h, [5.0, 0.0]).unwrap();
        assert!(sk.solve().is_ok());
        let (a, b) = line_pts(&sk, h);
        assert!(close(a, [0.0, 0.0]) && close(b, [3.0, 0.0]), "{a:?} {b:?}");
        assert_eq!(sk.entities().count(), 4);
        // The left piece now only touches the vertical line at its end, so
        // trimming it removes it.
        sk.trim(h, [1.0, 0.0]).unwrap();
        assert!(!sk.is_alive(h));
        // A curve without intersections disappears.
        let lone = sk.add_line([0.0, 50.0], [5.0, 50.0]);
        sk.trim(lone, [2.0, 50.0]).unwrap();
        assert!(!sk.is_alive(lone));
    }

    #[test]
    fn trim_circle_becomes_arc() {
        let mut sk = Sketch::new();
        let c = sk.add_circle([0.0, 0.0], 5.0);
        sk.add_line([-10.0, 0.0], [10.0, 0.0]);
        sk.trim(c, [0.0, 5.0]).unwrap(); // remove the upper half
        assert!(!sk.is_alive(c));
        let arc = sk.entities().find(|(_, e)| matches!(e, Entity::Arc { .. })).map(|(id, _)| id).unwrap();
        let Curve::Arc { start, sweep, .. } = sk.curve(arc) else { panic!() };
        assert!((sweep - PI).abs() < 1e-9 && (start - PI).abs() < 1e-9, "{start} {sweep}");
        assert!(sk.solve().is_ok());
    }

    #[test]
    fn extend_line_to_next_curve() {
        let mut sk = Sketch::new();
        let l = sk.add_line([0.0, 0.0], [4.0, 0.0]);
        sk.add_circle([10.0, 0.0], 2.0);
        sk.extend(l, [3.9, 0.0]).unwrap();
        assert!(close(line_pts(&sk, l).1, [8.0, 0.0]));
        assert!(sk.solve().is_ok());
        assert!(sk.extend(l, [0.0, 0.0]).is_err(), "nothing behind the start");
    }

    #[test]
    fn fillet_rectangle_corner() {
        let mut sk = Sketch::new();
        let [bottom, right, ..] = sk.add_rectangle([0.0, 0.0], [20.0, 10.0]);
        let corner = match *sk.entity(bottom) {
            Entity::Line { b, .. } => b,
            _ => unreachable!(),
        };
        let (arc, dim) = sk.fillet_corner(corner, 3.0).unwrap();
        assert!(sk.solve().is_ok());
        assert!((sk.radius(arc).unwrap() - 3.0).abs() < 1e-9);
        assert!(close(line_pts(&sk, bottom).1, [17.0, 0.0]));
        assert!(close(line_pts(&sk, right).0, [20.0, 3.0]));
        // The radius is a real dimension.
        assert!(sk.set_dimension(dim, 4.0));
        sk.solve().unwrap();
        assert!((sk.radius(arc).unwrap() - 4.0).abs() < 1e-6);
        // Still one closed profile.
        assert_eq!(crate::find_profiles(&sk).len(), 1);
        assert!(sk.fillet_corner(corner, 1.0).is_err());
    }

    #[test]
    fn offset_closed_chain_and_circle() {
        let mut sk = Sketch::new();
        let rect = sk.add_rectangle([0.0, 0.0], [10.0, 10.0]);
        let out = sk.offset(&rect, 2.0, [20.0, 5.0]).unwrap();
        assert_eq!(out.len(), 4);
        let xs: Vec<f64> = out.iter().flat_map(|l| { let (a, b) = line_pts(&sk, *l); [a[0], b[0]] }).collect();
        assert!(xs.iter().any(|x| (x - 12.0).abs() < 1e-9) && xs.iter().any(|x| (x + 2.0).abs() < 1e-9), "{xs:?}");
        assert!(sk.solve().is_ok());

        let c = sk.add_circle([50.0, 0.0], 5.0);
        let inner = sk.offset(&[c], 1.5, [50.0, 1.0]).unwrap();
        assert!((sk.radius(inner[0]).unwrap() - 3.5).abs() < 1e-9);
    }

    #[test]
    fn mirror_is_symmetric_and_follows() {
        let mut sk = Sketch::new();
        let axis = sk.add_line([0.0, -10.0], [0.0, 10.0]);
        sk.set_construction(axis, true);
        let (a, b) = match *sk.entity(axis) {
            Entity::Line { a, b } => (a, b),
            _ => unreachable!(),
        };
        sk.set_point_fixed(a, true);
        sk.set_point_fixed(b, true);
        let l = sk.add_line([2.0, 0.0], [5.0, 3.0]);
        let c = sk.add_circle([4.0, -4.0], 1.0);
        let out = sk.mirror(&[l, c], axis).unwrap();
        assert!(sk.solve().is_ok());
        let (p, q) = line_pts(&sk, out[0]);
        assert!(close(p, [-2.0, 0.0]) && close(q, [-5.0, 3.0]));
        // Moving the original moves the copy.
        let pa = match *sk.entity(l) {
            Entity::Line { a, .. } => a,
            _ => unreachable!(),
        };
        sk.drag_point(pa, [3.0, 1.0]).unwrap();
        assert!(close(line_pts(&sk, out[0]).0, [-3.0, 1.0]), "{:?}", line_pts(&sk, out[0]));
    }

    #[test]
    fn compound_shapes() {
        let mut sk = Sketch::new();
        let hex = sk.add_polygon([0.0, 0.0], [10.0, 0.0], 6).unwrap();
        assert_eq!(hex.len(), 6);
        assert!(sk.solve().is_ok());
        let slot = sk.add_slot([0.0, 30.0], [20.0, 30.0], 6.0).unwrap();
        assert_eq!(slot.len(), 4);
        assert!(sk.solve().is_ok());
        let profiles = crate::find_profiles(&sk);
        assert_eq!(profiles.len(), 2, "hexagon and slot are closed");
        sk.add_center_rectangle([100.0, 0.0], [104.0, 3.0]);
        assert!(sk.solve().is_ok());
        let arc = sk.add_arc_3pt([0.0, -50.0], [5.0, -45.0], [10.0, -50.0]).unwrap();
        assert!((sk.radius(arc).unwrap() - 5.0).abs() < 1e-9);
        assert!(sk.add_arc_3pt([0.0, 0.0], [1.0, 1.0], [2.0, 2.0]).is_err());
    }

    #[test]
    fn pattern_copies() {
        let mut sk = Sketch::new();
        let c = sk.add_circle([10.0, 0.0], 1.0);
        let r1 = Sketch::rotate_about([0.0, 0.0], PI / 2.0);
        let r2 = Sketch::rotate_about([0.0, 0.0], PI);
        let out = sk.copy_transformed(&[c], &[&r1, &r2]);
        assert_eq!(out.len(), 2);
        let centre = |sk: &Sketch, e: EntityId| match *sk.entity(e) {
            Entity::Circle { center, .. } => sk.point_pos(center),
            _ => unreachable!(),
        };
        assert!(close(centre(&sk, out[0]), [0.0, 10.0]) && close(centre(&sk, out[1]), [-10.0, 0.0]));
    }
}

#[cfg(test)]
mod redundancy {
    use super::*;

    #[test]
    fn shapes_are_not_over_constrained() {
        let check = |name: &str, f: &dyn Fn(&mut Sketch)| {
            let mut sk = Sketch::new();
            f(&mut sk);
            sk.solve().unwrap();
            let a = sk.analyze();
            assert_eq!(a.redundant, 0, "{name}: {} redundant", a.redundant);
        };
        check("polygon", &|sk| {
            sk.add_polygon([0.0, 0.0], [10.0, 0.0], 6).unwrap();
        });
        check("slot", &|sk| {
            sk.add_slot([0.0, 0.0], [20.0, 0.0], 6.0).unwrap();
        });
        check("fillet", &|sk| {
            let [bottom, ..] = sk.add_rectangle([0.0, 0.0], [20.0, 10.0]);
            let corner = match *sk.entity(bottom) {
                Entity::Line { b, .. } => b,
                _ => unreachable!(),
            };
            sk.fillet_corner(corner, 3.0).unwrap();
        });
        check("offset circle", &|sk| {
            let c = sk.add_circle([0.0, 0.0], 5.0);
            sk.offset(&[c], 2.0, [10.0, 0.0]).unwrap();
        });
        check("centre rectangle", &|sk| {
            sk.add_center_rectangle([0.0, 0.0], [5.0, 3.0]);
        });
    }
}

#[cfg(test)]
mod spline_trim {
    use super::*;

    #[test]
    fn trim_spline_end_and_middle() {
        let mut sk = Sketch::new();
        let s = sk.add_spline(&[[0.0, 0.0], [10.0, 5.0], [20.0, 0.0], [30.0, 5.0], [40.0, 0.0]]);
        sk.add_line([12.0, -10.0], [12.0, 20.0]);
        sk.add_line([28.0, -10.0], [28.0, 20.0]);
        // Middle piece between the two lines goes away: two splines remain.
        sk.trim(s, [20.0, 0.5]).unwrap();
        let splines: Vec<EntityId> =
            sk.entities().filter(|(_, e)| matches!(e, Entity::Spline { .. })).map(|(id, _)| id).collect();
        assert_eq!(splines.len(), 2);
        sk.solve().unwrap();
        for id in &splines {
            let pts = sk.spline_points(*id, 12);
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            // Every piece ends on one of the cutting lines.
            assert!(
                [a[0], b[0]].iter().any(|x| (x - 12.0).abs() < 1e-6 || (x - 28.0).abs() < 1e-6),
                "{a:?} {b:?}"
            );
        }
        // Trimming the left piece's end beyond the line removes it.
        sk.trim(splines[0], [2.0, 1.0]).unwrap();
        assert_eq!(sk.entities().filter(|(_, e)| matches!(e, Entity::Spline { .. })).count(), 1);
    }
}
