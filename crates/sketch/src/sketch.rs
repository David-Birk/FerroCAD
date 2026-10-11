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

#[derive(Clone, Debug)]
pub enum Entity {
    Line { a: PointId, b: PointId },
    Circle { center: PointId, radius: ParamId },
    /// Runs counter-clockwise from `start` to `end`. Its radius is implicit:
    /// the solver keeps `|end - center| == |start - center|`.
    Arc { center: PointId, start: PointId, end: PointId },
}

impl Entity {
    pub fn points(&self) -> Vec<PointId> {
        match *self {
            Entity::Line { a, b } => vec![a, b],
            Entity::Circle { center, .. } => vec![center],
            Entity::Arc { center, start, end } => vec![center, start, end],
        }
    }
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

    pub fn is_point_fixed(&self, id: PointId) -> bool {
        let p = self.points[id.0];
        self.fixed[p.x.0] && self.fixed[p.y.0]
    }

    pub fn point_ids(&self) -> impl Iterator<Item = PointId> {
        (0..self.points.len()).map(PointId)
    }

    // -------------------------------------------------------------- entities

    fn push_entity(&mut self, entity: Entity) -> EntityId {
        self.entities.push(EntitySlot { entity, construction: false, alive: true });
        EntityId(self.entities.len() - 1)
    }

    pub fn add_line_between(&mut self, a: PointId, b: PointId) -> EntityId {
        self.push_entity(Entity::Line { a, b })
    }

    pub fn add_line(&mut self, a: [f64; 2], b: [f64; 2]) -> EntityId {
        let a = self.add_point(a);
        let b = self.add_point(b);
        self.add_line_between(a, b)
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
            Entity::Line { .. } => None,
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
            Entity::Line { .. } => None,
        }
    }
}

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}
