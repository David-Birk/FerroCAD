//! Plain, fully public snapshot of a sketch for file formats. Ids are the
//! indices into these vectors, so constraint ids stored elsewhere (e.g.
//! parameter-driven dimensions) stay valid across save/load.

use crate::constraint::Constraint;
use crate::sketch::{Entity, EntitySlot, Point, PointId, Sketch};

#[derive(Clone, Debug, PartialEq)]
pub struct EntityData {
    pub entity: Entity,
    pub construction: bool,
    /// Deleted entities keep their slot so ids never shift.
    pub alive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchData {
    pub params: Vec<f64>,
    pub fixed: Vec<bool>,
    /// (x param index, y param index)
    pub points: Vec<(usize, usize)>,
    pub entities: Vec<EntityData>,
    /// `None` marks a deleted constraint.
    pub constraints: Vec<Option<Constraint>>,
}

impl Sketch {
    pub fn to_data(&self) -> SketchData {
        SketchData {
            params: self.params.clone(),
            fixed: self.fixed.clone(),
            points: self.points.iter().map(|p| (p.x.0, p.y.0)).collect(),
            entities: self
                .entities
                .iter()
                .map(|s| EntityData { entity: s.entity.clone(), construction: s.construction, alive: s.alive })
                .collect(),
            constraints: self.constraints.clone(),
        }
    }

    /// Rebuild a sketch, checking every index so a damaged file cannot
    /// cause a panic later.
    pub fn from_data(d: SketchData) -> Result<Sketch, String> {
        let np = d.params.len();
        if d.fixed.len() != np {
            return Err("fixed flags do not match parameters".into());
        }
        if d.params.iter().any(|v| !v.is_finite()) {
            return Err("non-finite parameter".into());
        }
        for &(x, y) in &d.points {
            if x >= np || y >= np {
                return Err("point refers to a missing parameter".into());
            }
        }
        let n_points = d.points.len();
        let point_ok = |p: PointId| p.0 < n_points;
        for e in &d.entities {
            let ok = match e.entity {
                Entity::Line { a, b } => point_ok(a) && point_ok(b) && a != b,
                Entity::Circle { center, radius } => point_ok(center) && radius.0 < np,
                Entity::Arc { center, start, end } => point_ok(center) && point_ok(start) && point_ok(end),
                Entity::Spline { ref points } => points.len() >= 2 && points.iter().all(|p| point_ok(*p)),
                Entity::Point { p } => point_ok(p),
            };
            if !ok {
                return Err("entity refers to a missing point".into());
            }
        }
        let n_entities = d.entities.len();
        for c in d.constraints.iter().flatten() {
            if c.points().iter().any(|p| !point_ok(*p)) || c.entities().iter().any(|e| e.0 >= n_entities)
            {
                return Err(format!("{} constraint refers to missing geometry", c.label()));
            }
        }
        Ok(Sketch {
            params: d.params,
            fixed: d.fixed,
            points: d
                .points
                .into_iter()
                .map(|(x, y)| Point { x: crate::ParamId(x), y: crate::ParamId(y) })
                .collect(),
            entities: d
                .entities
                .into_iter()
                .map(|e| EntitySlot { entity: e.entity, construction: e.construction, alive: e.alive })
                .collect(),
            constraints: d.constraints,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityId;

    #[test]
    fn roundtrip_preserves_everything() {
        let mut sk = Sketch::new();
        let r = sk.add_rectangle([0.0, 0.0], [10.0, 5.0]);
        let c = sk.add_circle([5.0, 2.5], 1.0);
        sk.set_construction(r[0], true);
        let d = sk.constrain(Constraint::Diameter(c, 2.0));
        sk.remove_constraint(d);
        let l = sk.add_line([20.0, 0.0], [30.0, 0.0]);
        sk.remove_entity(l);
        let back = Sketch::from_data(sk.to_data()).unwrap();
        assert_eq!(back.to_data(), sk.to_data());
        assert!(back.is_construction(r[0]));
        assert!(!back.is_alive(l));
    }

    #[test]
    fn rejects_dangling_references() {
        let mut sk = Sketch::new();
        sk.add_line([0.0, 0.0], [1.0, 0.0]);
        let mut d = sk.to_data();
        d.constraints.push(Some(Constraint::Horizontal(EntityId(7))));
        assert!(Sketch::from_data(d).is_err());

        let mut d = sk.to_data();
        d.points[0].0 = 99;
        assert!(Sketch::from_data(d).is_err());
    }
}
