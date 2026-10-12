use crate::constraint::Constraint;

/// Index of a scalar unknown (an x, y or radius).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstraintId(pub usize);

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: ParamId,
    pub y: ParamId,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entity {
    Line { a: PointId, b: PointId },
    Circle { center: PointId, radius: ParamId },
    /// Runs counter-clockwise from `start` to `end`. Its radius is implicit:
    /// the solver keeps `|end - center| == |start - center|`.
    Arc { center: PointId, start: PointId, end: PointId },
    /// Smooth curve through its fit points (centripetal Catmull–Rom).
    Spline { points: Vec<PointId> },
    /// A standalone sketch point (Fusion's Point tool), e.g. to place holes.
    Point { p: PointId },
}

impl Entity {
    pub fn points(&self) -> Vec<PointId> {
        match *self {
            Entity::Line { a, b } => vec![a, b],
            Entity::Circle { center, .. } => vec![center],
            Entity::Arc { center, start, end } => vec![center, start, end],
            Entity::Spline { ref points } => points.clone(),
            Entity::Point { p } => vec![p],
        }
    }
}

/// Points along a centripetal Catmull–Rom spline through `fit`, `per_span`
/// samples per span, including both ends.
pub fn spline_samples(fit: &[[f64; 2]], per_span: usize) -> Vec<[f64; 2]> {
    let n = fit.len();
    if n < 2 {
        return fit.to_vec();
    }
    let closed = n > 3 && dist(fit[0], fit[n - 1]) < 1e-9;
    let get = |i: isize| -> [f64; 2] {
        if closed {
            let m = (n - 1) as isize;
            fit[i.rem_euclid(m) as usize]
        } else if i < 0 {
            // Reflect the first point to make a phantom start.
            [2.0 * fit[0][0] - fit[1][0], 2.0 * fit[0][1] - fit[1][1]]
        } else if i as usize >= n {
            [2.0 * fit[n - 1][0] - fit[n - 2][0], 2.0 * fit[n - 1][1] - fit[n - 2][1]]
        } else {
            fit[i as usize]
        }
    };
    let mut out = vec![fit[0]];
    for i in 0..(n - 1) as isize {
        let (p0, p1, p2, p3) = (get(i - 1), get(i), get(i + 1), get(i + 2));
        let knot = |a: [f64; 2], b: [f64; 2]| dist(a, b).sqrt().max(1e-9);
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        for k in 1..=per_span {
            let t = t1 + (t2 - t1) * k as f64 / per_span as f64;
            let lerp = |a: [f64; 2], b: [f64; 2], ta: f64, tb: f64| {
                let w = (t - ta) / (tb - ta);
                [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
            };
            let a1 = lerp(p0, p1, t0, t1);
            let a2 = lerp(p1, p2, t1, t2);
            let a3 = lerp(p2, p3, t2, t3);
            let b1 = lerp(a1, a2, t0, t2);
            let b2 = lerp(a2, a3, t1, t3);
            out.push(lerp(b1, b2, t1, t2));
        }
    }
    if closed {
        let last = out.len() - 1;
        out[last] = out[0];
    }
    out
}

#[derive(Clone, Debug)]
pub(crate) struct EntitySlot {
    pub entity: Entity,
    pub construction: bool,
    pub alive: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Sketch {
    pub(crate) params: Vec<f64>,
    pub(crate) fixed: Vec<bool>,
    pub(crate) points: Vec<Point>,
    pub(crate) entities: Vec<EntitySlot>,
    pub(crate) constraints: Vec<Option<Constraint>>,
}

impl Sketch {
    pub fn new() -> Self {
        Self::default()
    }

    // ---------------------------------------------------------------- params

    pub(crate) fn add_param(&mut self, value: f64) -> ParamId {
        self.params.push(value);
        self.fixed.push(false);
        ParamId(self.params.len() - 1)
    }

    pub fn param(&self, id: ParamId) -> f64 {
        self.params[id.0]
    }

    pub fn set_param(&mut self, id: ParamId, value: f64) {
        self.params[id.0] = value;
    }

    pub fn params(&self) -> &[f64] {
        &self.params
    }

    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    // ---------------------------------------------------------------- points

    pub fn add_point(&mut self, pos: [f64; 2]) -> PointId {
        let x = self.add_param(pos[0]);
        let y = self.add_param(pos[1]);
        self.points.push(Point { x, y });
        PointId(self.points.len() - 1)
    }

    pub fn point(&self, id: PointId) -> Point {
        self.points[id.0]
    }

    pub fn point_pos(&self, id: PointId) -> [f64; 2] {
        let p = self.points[id.0];
        [self.params[p.x.0], self.params[p.y.0]]
    }

    pub fn set_point_pos(&mut self, id: PointId, pos: [f64; 2]) {
        let p = self.points[id.0];
        self.params[p.x.0] = pos[0];
        self.params[p.y.0] = pos[1];
    }

    /// Lock a point in place (Fusion's "Fix/Unfix"). Fixed parameters are
    /// removed from the solve instead of being added as equations.
    pub fn set_point_fixed(&mut self, id: PointId, fixed: bool) {
        let p = self.points[id.0];
        self.fixed[p.x.0] = fixed;
        self.fixed[p.y.0] = fixed;
    }

    /// Fix or free a single parameter (e.g. a projected circle's radius).
    pub fn set_param_fixed(&mut self, id: ParamId, fixed: bool) {
        self.fixed[id.0] = fixed;
    }

    pub fn is_point_fixed(&self, id: PointId) -> bool {
        let p = self.points[id.0];
        self.fixed[p.x.0] && self.fixed[p.y.0]
    }

    pub fn point_ids(&self) -> impl Iterator<Item = PointId> {
        (0..self.points.len()).map(PointId)
    }

    /// Points used by live entities (curve ends, centres, fit points and
    /// standalone points), sorted. Points of deleted curves are left out.
    pub fn used_points(&self) -> Vec<PointId> {
        let mut pts: Vec<PointId> = self.entities().flat_map(|(_, e)| e.points()).collect();
        pts.sort();
        pts.dedup();
        pts
    }

    // -------------------------------------------------------------- entities

    fn push_entity(&mut self, entity: Entity) -> EntityId {
        self.entities.push(EntitySlot { entity, construction: false, alive: true });
        EntityId(self.entities.len() - 1)
    }

    /// Standalone sketch point.
    pub fn add_point_entity(&mut self, pos: [f64; 2]) -> EntityId {
        let p = self.add_point(pos);
        self.push_entity(Entity::Point { p })
    }

    /// The live standalone-point entity at `p`, if `p` is one.
    pub fn point_entity(&self, p: PointId) -> Option<EntityId> {
        self.entities().find(|(_, e)| matches!(e, Entity::Point { p: q } if *q == p)).map(|(id, _)| id)
    }

    /// Closed polygon of lines sharing their corner points, all fixed – for
    /// exploded text, which would only burden the solver otherwise. Points
    /// closer than `1e-9` to their predecessor are dropped; fewer than three
    /// distinct points give nothing.
    pub fn add_fixed_polygon(&mut self, pts: &[[f64; 2]]) -> Vec<EntityId> {
        let mut clean: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
        for &p in pts {
            if clean.last().is_none_or(|q| dist(*q, p) > 1e-9) {
                clean.push(p);
            }
        }
        while clean.len() > 1 && dist(clean[0], clean[clean.len() - 1]) <= 1e-9 {
            clean.pop();
        }
        if clean.len() < 3 {
            return Vec::new();
        }
        let ids: Vec<PointId> = clean.iter().map(|p| self.add_point(*p)).collect();
        for &p in &ids {
            self.set_point_fixed(p, true);
        }
        (0..ids.len()).map(|i| self.add_line_between(ids[i], ids[(i + 1) % ids.len()])).collect()
    }

    pub fn add_line_between(&mut self, a: PointId, b: PointId) -> EntityId {
        self.push_entity(Entity::Line { a, b })
    }

    pub fn add_line(&mut self, a: [f64; 2], b: [f64; 2]) -> EntityId {
        let a = self.add_point(a);
        let b = self.add_point(b);
        self.add_line_between(a, b)
    }

    /// Spline through fit points (at least two).
    pub fn add_spline(&mut self, fit: &[[f64; 2]]) -> EntityId {
        let points = fit.iter().map(|p| self.add_point(*p)).collect();
        self.push_entity(Entity::Spline { points })
    }

    /// Spline through existing points (shared with other curves).
    pub fn add_spline_through(&mut self, points: Vec<PointId>) -> EntityId {
        self.push_entity(Entity::Spline { points })
    }

    /// Sample points of a spline entity.
    pub fn spline_points(&self, id: EntityId, per_span: usize) -> Vec<[f64; 2]> {
        match self.entity(id) {
            Entity::Spline { points } => {
                let fit: Vec<[f64; 2]> = points.iter().map(|p| self.point_pos(*p)).collect();
                spline_samples(&fit, per_span)
            }
            _ => Vec::new(),
        }
    }

    pub fn add_circle(&mut self, center: [f64; 2], radius: f64) -> EntityId {
        let center = self.add_point(center);
        let radius = self.add_param(radius);
        self.push_entity(Entity::Circle { center, radius })
    }

    /// Arc from `start` counter-clockwise to `end`. `end` is projected onto
    /// the circle through `start` so the arc is consistent from the start.
    pub fn add_arc(&mut self, center: [f64; 2], start: [f64; 2], end: [f64; 2]) -> EntityId {
        let r = dist(center, start);
        let a = (end[1] - center[1]).atan2(end[0] - center[0]);
        let end = [center[0] + r * a.cos(), center[1] + r * a.sin()];
        let center = self.add_point(center);
        let start = self.add_point(start);
        let end = self.add_point(end);
        self.push_entity(Entity::Arc { center, start, end })
    }

    /// Axis-aligned rectangle made of four lines that share corner points,
    /// with horizontal/vertical constraints – like Fusion's 2-point rectangle.
    pub fn add_rectangle(&mut self, c1: [f64; 2], c2: [f64; 2]) -> [EntityId; 4] {
        let p0 = self.add_point([c1[0], c1[1]]);
        let p1 = self.add_point([c2[0], c1[1]]);
        let p2 = self.add_point([c2[0], c2[1]]);
        let p3 = self.add_point([c1[0], c2[1]]);
        let bottom = self.add_line_between(p0, p1);
        let right = self.add_line_between(p1, p2);
        let top = self.add_line_between(p2, p3);
        let left = self.add_line_between(p3, p0);
        self.constrain(Constraint::Horizontal(bottom));
        self.constrain(Constraint::Horizontal(top));
        self.constrain(Constraint::Vertical(left));
        self.constrain(Constraint::Vertical(right));
        [bottom, right, top, left]
    }

    pub fn entity(&self, id: EntityId) -> &Entity {
        &self.entities[id.0].entity
    }

    pub fn is_alive(&self, id: EntityId) -> bool {
        self.entities[id.0].alive
    }

    pub fn is_construction(&self, id: EntityId) -> bool {
        self.entities[id.0].construction
    }

    pub fn set_construction(&mut self, id: EntityId, construction: bool) {
        self.entities[id.0].construction = construction;
    }

    /// All live entities.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alive)
            .map(|(i, s)| (EntityId(i), &s.entity))
    }

    /// Delete an entity and every constraint that mentions it or one of its
    /// points (when no other live entity still uses that point).
    pub fn remove_entity(&mut self, id: EntityId) {
        if !self.entities[id.0].alive {
            return;
        }
        self.entities[id.0].alive = false;
        let orphaned: Vec<PointId> = self.entities[id.0]
            .entity
            .points()
            .into_iter()
            .filter(|p| !self.entities().any(|(_, e)| e.points().contains(p)))
            .collect();
        for slot in self.constraints.iter_mut() {
            if let Some(c) = slot {
                if c.entities().contains(&id) || c.points().iter().any(|p| orphaned.contains(p)) {
                    *slot = None;
                }
            }
        }
        for p in orphaned {
            self.set_point_fixed(p, false);
        }
    }

    /// Current radius of a circle or arc.
    pub fn radius(&self, id: EntityId) -> Option<f64> {
        match *self.entity(id) {
            Entity::Circle { radius, .. } => Some(self.param(radius)),
            Entity::Arc { center, start, .. } => {
                Some(dist(self.point_pos(center), self.point_pos(start)))
            }
            Entity::Line { .. } | Entity::Spline { .. } | Entity::Point { .. } => None,
        }
    }

    // ----------------------------------------------------------- constraints

    pub fn constrain(&mut self, c: Constraint) -> ConstraintId {
        self.constraints.push(Some(c));
        ConstraintId(self.constraints.len() - 1)
    }

    pub fn constraint(&self, id: ConstraintId) -> Option<&Constraint> {
        self.constraints.get(id.0).and_then(|c| c.as_ref())
    }

    pub fn constraint_mut(&mut self, id: ConstraintId) -> Option<&mut Constraint> {
        self.constraints.get_mut(id.0).and_then(|c| c.as_mut())
    }

    pub fn remove_constraint(&mut self, id: ConstraintId) {
        if let Some(slot) = self.constraints.get_mut(id.0) {
            *slot = None;
        }
    }

    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintId, &Constraint)> {
        self.constraints
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|c| (ConstraintId(i), c)))
    }

    /// Set the driving value of a dimensional constraint (length, radius,
    /// angle in radians …). Returns `false` for geometric constraints.
    pub fn set_dimension(&mut self, id: ConstraintId, value: f64) -> bool {
        match self.constraint_mut(id).and_then(|c| c.value_mut()) {
            Some(v) => {
                *v = value;
                true
            }
            None => false,
        }
    }

    // --------------------------------------------------------------- helpers

    pub(crate) fn pt(&self, p: &[f64], id: PointId) -> [f64; 2] {
        let pt = self.points[id.0];
        [p[pt.x.0], p[pt.y.0]]
    }

    pub(crate) fn line(&self, p: &[f64], id: EntityId) -> Option<([f64; 2], [f64; 2])> {
        match *self.entity(id) {
            Entity::Line { a, b } => Some((self.pt(p, a), self.pt(p, b))),
            _ => None,
        }
    }

    /// Center and radius of a circle or arc, evaluated on `p`.
    pub(crate) fn circle(&self, p: &[f64], id: EntityId) -> Option<([f64; 2], f64)> {
        match *self.entity(id) {
            Entity::Circle { center, radius } => Some((self.pt(p, center), p[radius.0])),
            Entity::Arc { center, start, .. } => {
                let c = self.pt(p, center);
                Some((c, dist(c, self.pt(p, start))))
            }
            Entity::Line { .. } | Entity::Spline { .. } | Entity::Point { .. } => None,
        }
    }
}

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}
