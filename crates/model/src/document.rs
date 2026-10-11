//! The document and its rebuild – the heart of parametric modelling.
//!
//! Rebuilding replays the timeline from the start up to the rollback
//! marker. Each feature reads parameters, the solved sketches and the bodies
//! created so far. A failing feature is marked as an error and skipped; the
//! features after it still run, like Fusion's red timeline icons.

use crate::feature::{Feature, FeatureId, FeatureKind, Operation, SketchFeature};
use crate::geom::distance;
use crate::kernel::{polyline_midpoint, BooleanOp, EdgeRef, Kernel, PlacedProfile};
use crate::params::ParamTable;
use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::{find_profiles, Constraint};
use std::collections::HashMap;

/// How far (mm) an edge midpoint may move before a fillet loses track of it.
pub const EDGE_MATCH_TOLERANCE: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureStatus {
    Ok,
    Error(String),
    Suppressed,
    /// After the rollback marker – not evaluated.
    RolledBack,
}

#[derive(Clone, Debug)]
pub struct Body<S> {
    pub name: String,
    pub solid: S,
    pub created_by: FeatureId,
}

/// Result of a rebuild.
#[derive(Clone, Debug)]
pub struct Model<S> {
    pub bodies: Vec<Body<S>>,
    pub status: HashMap<FeatureId, FeatureStatus>,
}

impl<S> Model<S> {
    pub fn status(&self, id: FeatureId) -> &FeatureStatus {
        self.status.get(&id).unwrap_or(&FeatureStatus::RolledBack)
    }

    pub fn errors(&self) -> impl Iterator<Item = (FeatureId, &str)> {
        self.status.iter().filter_map(|(id, s)| match s {
            FeatureStatus::Error(e) => Some((*id, e.as_str())),
            _ => None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub params: ParamTable,
    features: Vec<Feature>,
    /// Number of features that are evaluated (the timeline marker).
    marker: usize,
    next_id: u64,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    pub fn index_of(&self, id: FeatureId) -> Option<usize> {
        self.features.iter().position(|f| f.id == id)
    }

    pub fn feature(&self, id: FeatureId) -> Option<&Feature> {
        self.features.iter().find(|f| f.id == id)
    }

    pub fn feature_mut(&mut self, id: FeatureId) -> Option<&mut Feature> {
        self.features.iter_mut().find(|f| f.id == id)
    }

    pub fn sketch(&self, id: FeatureId) -> Option<&SketchFeature> {
        match &self.feature(id)?.kind {
            FeatureKind::Sketch(s) => Some(s),
            _ => None,
        }
    }

    pub fn sketch_mut(&mut self, id: FeatureId) -> Option<&mut SketchFeature> {
        match &mut self.feature_mut(id)?.kind {
            FeatureKind::Sketch(s) => Some(s),
            _ => None,
        }
    }

    /// Next free default name, e.g. "Sketch3".
    pub fn next_name(&self, prefix: &str) -> String {
        let n = self.features.iter().filter(|f| f.name.starts_with(prefix)).count();
        format!("{prefix}{}", n + 1)
    }

    /// Insert a feature at the marker (Fusion inserts where the marker is)
    /// and advance the marker past it.
    pub fn add(&mut self, name: impl Into<String>, kind: FeatureKind) -> FeatureId {
        self.next_id += 1;
        let id = FeatureId(self.next_id);
        let feature = Feature { id, name: name.into(), suppressed: false, kind };
        self.features.insert(self.marker, feature);
        self.marker += 1;
        id
    }

    /// Delete a feature. Refuses while other features depend on it.
    pub fn remove(&mut self, id: FeatureId) -> Result<(), String> {
        let users: Vec<&str> = self
            .features
            .iter()
            .filter(|f| matches!(&f.kind, FeatureKind::Extrude(e) if e.sketch == id))
            .map(|f| f.name.as_str())
            .collect();
        if !users.is_empty() {
            return Err(format!("used by {}", users.join(", ")));
        }
        let Some(i) = self.index_of(id) else { return Ok(()) };
        self.features.remove(i);
        if i < self.marker {
            self.marker -= 1;
        }
        Ok(())
    }

    /// Move a feature to a new timeline position. Refuses to move an
    /// extrude before its sketch (or a sketch after its extrudes).
    pub fn move_feature(&mut self, id: FeatureId, to: usize) -> Result<(), String> {
        let from = self.index_of(id).ok_or("no such feature")?;
        let to = to.min(self.features.len() - 1);
        let f = self.features.remove(from);
        self.features.insert(to, f);
        let ok = self.features.iter().enumerate().all(|(i, f)| match &f.kind {
            FeatureKind::Extrude(e) => self.index_of(e.sketch).is_some_and(|s| s < i),
            _ => true,
        });
        if !ok {
            let f = self.features.remove(to);
            self.features.insert(from, f);
            return Err("an extrude must come after its sketch".into());
        }
        Ok(())
    }

    pub fn marker(&self) -> usize {
        self.marker
    }

    pub fn set_marker(&mut self, marker: usize) {
        self.marker = marker.min(self.features.len());
    }

    /// Replay the timeline with kernel `k`.
    pub fn rebuild<K: Kernel>(&mut self, k: &K) -> Model<K::Solid> {
        let mut model = Model { bodies: Vec::new(), status: HashMap::new() };
        let mut body_counter = 0;

        for i in 0..self.features.len() {
            let id = self.features[i].id;
            let status = if i >= self.marker {
                FeatureStatus::RolledBack
            } else if self.features[i].suppressed {
                FeatureStatus::Suppressed
            } else {
                match self.evaluate(i, k, &mut model, &mut body_counter) {
                    Ok(()) => FeatureStatus::Ok,
                    Err(e) => FeatureStatus::Error(e),
                }
            };
            model.status.insert(id, status);
        }
        model
    }

    fn evaluate<K: Kernel>(
        &mut self,
        i: usize,
        k: &K,
        model: &mut Model<K::Solid>,
        body_counter: &mut usize,
    ) -> Result<(), String> {
        let id = self.features[i].id;
        // Split borrows: parameters are read while the feature is mutated.
        let params = &self.params;
        match &mut self.features[i].kind {
            FeatureKind::Sketch(s) => {
                for (cid, expr) in &s.driven {
                    let mut v = params.eval(expr).map_err(|e| e.to_string())?;
                    if matches!(s.sketch.constraint(*cid), Some(Constraint::Angle(..))) {
                        v = v.to_radians();
                    }
                    s.sketch.set_dimension(*cid, v);
                }
                s.sketch.solve().map(|_| ()).map_err(|e| e.to_string())
            }

            FeatureKind::Extrude(e) => {
                let e = e.clone();
                let distance = params.eval(&e.distance).map_err(|e| e.to_string())?;
                if distance.abs() < 1e-9 {
                    return Err("distance is zero".into());
                }
                let si = self.index_of(e.sketch).ok_or("sketch was deleted")?;
                if si >= i {
                    return Err("sketch comes after this extrude".into());
                }
                if self.features[si].suppressed {
                    return Err("sketch is suppressed".into());
                }
                let FeatureKind::Sketch(s) = &self.features[si].kind else {
                    return Err("referenced feature is not a sketch".into());
                };
                let profiles = find_profiles(&s.sketch);
                let pi = profile_at(&profiles, e.profile_point)
                    .ok_or("profile no longer exists – edit the extrude to pick it again")?;
                let placed = PlacedProfile::new(&profiles[pi], &s.plane);
                let tool = k.extrude(&placed, distance, e.symmetric).map_err(|e| e.0)?;
                apply_operation(k, model, e.operation, tool, id, body_counter)
            }

            FeatureKind::Fillet(f) => {
                let f = f.clone();
                let radius = params.eval(&f.radius).map_err(|e| e.to_string())?;
                if radius <= 0.0 {
                    return Err("radius must be positive".into());
                }
                if model.bodies.is_empty() {
                    return Err("no body to fillet".into());
                }
                // Hand every edge reference to the body that owns the
                // closest edge.
                let midpoints: Vec<(usize, [f64; 3])> = model
                    .bodies
                    .iter()
                    .enumerate()
                    .flat_map(|(bi, b)| {
                        k.edges(&b.solid)
                            .into_iter()
                            .filter_map(|pl| polyline_midpoint(&pl))
                            .map(move |m| (bi, m))
                    })
                    .collect();
                let mut per_body: Vec<Vec<EdgeRef>> = vec![Vec::new(); model.bodies.len()];
                for r in &f.edges {
                    let best = midpoints
                        .iter()
                        .map(|&(bi, m)| (bi, distance(m, r.point)))
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    match best {
                        Some((bi, d)) if d < EDGE_MATCH_TOLERANCE => per_body[bi].push(*r),
                        _ => {
                            return Err("an edge moved or disappeared – re-pick the edges".into())
                        }
                    }
                }
                for (bi, refs) in per_body.into_iter().enumerate() {
                    if !refs.is_empty() {
                        let solid = &model.bodies[bi].solid;
                        model.bodies[bi].solid =
                            k.fillet(solid, &refs, radius, f.chamfer).map_err(|e| e.0)?;
                    }
                }
                Ok(())
            }
        }
    }
}

fn apply_operation<K: Kernel>(
    k: &K,
    model: &mut Model<K::Solid>,
    op: Operation,
    tool: K::Solid,
    created_by: FeatureId,
    body_counter: &mut usize,
) -> Result<(), String> {
    let mut new_body = |model: &mut Model<K::Solid>, solid| {
        *body_counter += 1;
        model.bodies.push(Body { name: format!("Body{body_counter}"), solid, created_by });
    };
    match op {
        Operation::NewBody => new_body(model, tool),
        Operation::Join => match model.bodies.last_mut() {
            Some(b) => b.solid = k.boolean(BooleanOp::Union, &b.solid, &tool).map_err(|e| e.0)?,
            None => new_body(model, tool),
        },
        Operation::Cut => {
            if model.bodies.is_empty() {
                return Err("nothing to cut".into());
            }
            for b in &mut model.bodies {
                b.solid = k.boolean(BooleanOp::Subtract, &b.solid, &tool).map_err(|e| e.0)?;
            }
        }
        Operation::Intersect => {
            let b = model.bodies.last_mut().ok_or("nothing to intersect")?;
            b.solid = k.boolean(BooleanOp::Intersect, &b.solid, &tool).map_err(|e| e.0)?;
        }
    }
    Ok(())
}
