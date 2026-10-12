//! The document and its rebuild – the heart of parametric modelling.
//!
//! Rebuilding replays the timeline from the start up to the rollback
//! marker. Each feature reads parameters, the solved sketches and the bodies
//! created so far. A failing feature is marked as an error and skipped; the
//! features after it still run, like Fusion's red timeline icons.

use crate::feature::*;
use crate::geom::{add, cross, distance, dot, length, normalize, scale, sub, Plane, Vec3};
use crate::kernel::{
    polyline_midpoint, BooleanOp, EdgeRef, FaceInfo, FaceRef, Kernel, Loop3, PlacedProfile, Seg3, Transform,
};
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
    /// Construction planes by feature.
    pub planes: HashMap<FeatureId, Plane>,
    /// Construction axes by feature: (point, direction).
    pub axes: HashMap<FeatureId, (Vec3, Vec3)>,
    /// Components: (name, body names).
    pub components: Vec<(String, Vec<String>)>,
    /// Tool solids features added to or cut from bodies (for feature patterns).
    pub tools: HashMap<FeatureId, Vec<FeatureTool<S>>>,
}

/// A solid a feature joined, cut or intersected with `bodies` (or made a new
/// body from), kept so feature patterns can repeat it.
#[derive(Clone, Debug)]
pub struct FeatureTool<S> {
    pub solid: S,
    pub operation: Operation,
    pub bodies: Vec<String>,
}

/// Length standing in for "through everything".
const THROUGH_ALL: f64 = 1e4;

impl<S> Model<S> {
    pub fn empty() -> Self {
        Model {
            bodies: Vec::new(),
            status: HashMap::new(),
            planes: HashMap::new(),
            axes: HashMap::new(),
            components: Vec::new(),
            tools: HashMap::new(),
        }
    }

    pub fn body(&self, name: &str) -> Option<&Body<S>> {
        self.bodies.iter().find(|b| b.name == name)
    }

    /// The component a body belongs to.
    pub fn component_of(&self, body: &str) -> Option<&str> {
        self.components.iter().find(|(_, b)| b.iter().any(|n| n == body)).map(|(c, _)| c.as_str())
    }

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
    /// What is hidden in the viewport (saved with the file, not part of the recipe).
    pub view: ViewState,
    pub(crate) features: Vec<Feature>,
    /// Number of features that are evaluated (the timeline marker).
    pub(crate) marker: usize,
    pub(crate) next_id: u64,
}

/// Display state stored in the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub hidden_bodies: Vec<String>,
    /// Appearance (material name) per body name.
    pub appearances: Vec<(String, String)>,
    /// Component new features go into (Fusion's "Activate").
    pub active_component: Option<String>,
    /// Hidden sketches and construction geometry.
    pub hidden_features: Vec<FeatureId>,
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
        let component = self.view.active_component.clone();
        let feature = Feature { id, name: name.into(), suppressed: false, kind, component };
        self.features.insert(self.marker, feature);
        self.marker += 1;
        id
    }

    /// Delete a feature. Refuses while other features depend on it.
    pub fn remove(&mut self, id: FeatureId) -> Result<(), String> {
        let users: Vec<&str> = self
            .features
            .iter()
            .filter(|f| f.kind.references().contains(&id))
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
        let ok = self.features.iter().enumerate().all(|(i, f)| {
            f.kind.references().iter().all(|r| self.index_of(*r).is_some_and(|s| s < i))
        });
        if !ok {
            let f = self.features.remove(to);
            self.features.insert(from, f);
            return Err("a feature must come after the features it uses".into());
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
        let mut model = Model::empty();
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
            // Bodies made while a component was active belong to it.
            if let Some(comp) = self.features[i].component.clone() {
                let new: Vec<String> = model
                    .bodies
                    .iter()
                    .filter(|b| b.created_by == id && model.component_of(&b.name).is_none())
                    .map(|b| b.name.clone())
                    .collect();
                if !new.is_empty() {
                    match model.components.iter_mut().find(|(c, _)| *c == comp) {
                        Some((_, list)) => list.extend(new),
                        None => model.components.push((comp, new)),
                    }
                }
            }
            model.status.insert(id, status);
            // Removed bodies leave their component; empty components vanish.
            let alive: Vec<String> = model.bodies.iter().map(|b| b.name.clone()).collect();
            for (_, list) in &mut model.components {
                list.retain(|b| alive.contains(b));
            }
            model.components.retain(|(_, list)| !list.is_empty());
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
        // Attached sketches follow their plane.
        if let FeatureKind::Sketch(SketchFeature { attach: Some(a), .. }) = &self.features[i].kind {
            let a = *a;
            let plane = resolve_plane(k, model, &a)?;
            if let FeatureKind::Sketch(s) = &mut self.features[i].kind {
                s.plane = plane;
            }
        }
        // Projected edges follow the bodies built so far.
        if matches!(&self.features[i].kind, FeatureKind::Sketch(s) if !s.projections.is_empty()) {
            let edges: Vec<Vec<Vec3>> = model.bodies.iter().flat_map(|b| k.edges(&b.solid)).collect();
            if let FeatureKind::Sketch(s) = &mut self.features[i].kind {
                crate::project::update_projections(s, &edges)?;
            }
        }
        // Split borrows: parameters are read while the feature is mutated.
        let params = &self.params;
        let eval = |e: &str| params.eval(e).map_err(|e| e.to_string());
        let kerr = |e: crate::kernel::KernelError| e.0;
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
                let (_, placed) = self.placed_profile(i, e.sketch, e.profile_point)?;
                let taper = eval(&e.taper)?;
                if taper.abs() >= 89.0 {
                    return Err("taper must be between -89° and 89°".into());
                }
                let tapered = taper.abs() > 1e-12;
                let n = placed.normal;
                // Half-space beyond a tilted "to" target, cut off at the end.
                let mut trim: Option<PlacedProfile> = None;
                let (fwd, back) = match &e.extent {
                    ExtrudeExtent::Distance => {
                        let distance = eval(&e.distance)?;
                        if distance.abs() < 1e-9 {
                            return Err("distance is zero".into());
                        }
                        match &e.distance2 {
                            Some(d2) => (distance, eval(d2)?),
                            None if e.symmetric => (distance / 2.0, distance / 2.0),
                            None => (distance, 0.0),
                        }
                    }
                    ExtrudeExtent::All => {
                        if matches!(e.operation, Operation::NewBody | Operation::Join) {
                            return Err("\"All\" only works with Cut or Intersect".into());
                        }
                        if tapered {
                            return Err("\"All\" can't be tapered".into());
                        }
                        let flip = eval(&e.distance).is_ok_and(|d| d < 0.0);
                        match (e.symmetric || e.distance2.is_some(), flip) {
                            (true, _) => (THROUGH_ALL, THROUGH_ALL),
                            (false, false) => (THROUGH_ALL, 0.0),
                            (false, true) => (-THROUGH_ALL, 0.0),
                        }
                    }
                    ExtrudeExtent::To(target) => {
                        if e.symmetric || e.distance2.is_some() {
                            return Err("extrude \"To\" goes one way only".into());
                        }
                        let target = resolve_plane(k, model, target)?;
                        let tn = target.normal();
                        let denom = dot(n, tn);
                        if denom.abs() < 1e-6 {
                            return Err("the target is parallel to the extrude direction".into());
                        }
                        // Distance along the normal from each profile point to the target.
                        let pts = loop_points(&placed.outer);
                        let reach: Vec<f64> = pts.iter().map(|p| dot(sub(target.origin, *p), tn) / denom).collect();
                        let (lo, hi) = reach.iter().fold((f64::MAX, f64::MIN), |(lo, hi), t| (lo.min(*t), hi.max(*t)));
                        if lo * hi <= 1e-12 {
                            return Err("the target touches or crosses the profile".into());
                        }
                        if length(cross(n, tn)) < 1e-6 {
                            (reach[0], 0.0)
                        } else {
                            // Go past the farthest point, then cut off beyond the target.
                            let far = if hi > 0.0 { hi } else { lo };
                            let center = scale(pts.iter().fold([0.0; 3], |c, p| add(c, *p)), 1.0 / pts.len() as f64);
                            let on_target = sub(center, scale(tn, dot(sub(center, target.origin), tn)));
                            let away = if dot(sub(target.origin, center), tn) > 0.0 { tn } else { scale(tn, -1.0) };
                            let size = 2.0 * far.abs() + 1000.0;
                            let corner = |a: f64, b: f64| add(on_target, add(scale(target.x_axis, a * size), scale(target.y_axis, b * size)));
                            let pts = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
                            let segments = (0..4).map(|j| Seg3::Line { a: pts[j], b: pts[(j + 1) % 4] }).collect();
                            trim = Some(PlacedProfile { outer: Loop3 { segments }, holes: vec![], normal: away });
                            (far * 1.05 + far.signum(), 0.0)
                        }
                    }
                };
                let mut tool = match (&e.thin, e.extent, &e.distance2, tapered) {
                    (Some(thin), ..) => {
                        if tapered {
                            return Err("thin extrudes can't be tapered".into());
                        }
                        let t = eval(&thin.thickness)?;
                        if t <= 0.0 {
                            return Err("wall thickness must be positive".into());
                        }
                        k.extrude_thin(&placed, fwd, back, t, thin.side).map_err(kerr)?
                    }
                    (None, ExtrudeExtent::Distance, None, false) => {
                        let distance = if e.symmetric { fwd * 2.0 } else { fwd };
                        k.extrude(&placed, distance, e.symmetric).map_err(kerr)?
                    }
                    _ => k.extrude_tapered(&placed, fwd, back, taper.to_radians()).map_err(kerr)?,
                };
                if let Some(half_space) = trim {
                    let block = k.extrude(&half_space, 2.0 * THROUGH_ALL, false).map_err(kerr)?;
                    tool = k.boolean(BooleanOp::Subtract, &tool, &block).map_err(kerr)?;
                }
                apply_operation(k, model, e.operation, tool, id, body_counter)
            }

            FeatureKind::Revolve(r) => {
                let r = r.clone();
                let degrees = params.eval(&r.angle).map_err(|e| e.to_string())?;
                if degrees.abs() < 1e-9 || degrees.abs() > 360.0 + 1e-9 {
                    return Err("angle must be between 0 and 360°".into());
                }
                let (s, placed) = self.placed_profile(i, r.sketch, r.profile_point)?;
                let (origin, dir) = match r.axis {
                    RevolveAxis::SketchX => (s.plane.origin, s.plane.x_axis),
                    RevolveAxis::SketchY => (s.plane.origin, s.plane.y_axis),
                    RevolveAxis::Axis(a) => resolve_axis(model, &a)?,
                    RevolveAxis::Line(l) => {
                        let ferrocad_sketch::Entity::Line { a, b } = *s.sketch.entity(l) else {
                            return Err("axis is not a line".into());
                        };
                        if !s.sketch.is_alive(l) {
                            return Err("axis line was deleted".into());
                        }
                        let a = s.plane.to_world(s.sketch.point_pos(a));
                        let b = s.plane.to_world(s.sketch.point_pos(b));
                        (a, sub(b, a))
                    }
                };
                if distance(dir, [0.0; 3]) < 1e-9 {
                    return Err("axis has zero length".into());
                }
                let tool = k
                    .revolve(&placed, origin, normalize(dir), degrees.to_radians())
                    .map_err(|e| e.0)?;
                apply_operation(k, model, r.operation, tool, id, body_counter)
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
                            match (&f.distance2, f.chamfer) {
                                (Some(d2), true) => {
                                    let d2 = eval(d2)?;
                                    if d2 <= 0.0 {
                                        return Err("distance 2 must be positive".into());
                                    }
                                    k.chamfer_two(solid, &refs, radius, d2).map_err(kerr)?
                                }
                                _ => k.fillet(solid, &refs, radius, f.chamfer).map_err(kerr)?,
                            };
                    }
                }
                Ok(())
            }

            FeatureKind::Plane(p) => {
                let p = p.clone();
                let mut base = resolve_plane(k, model, &p.base)?;
                if let Some(b2) = &p.base2 {
                    let other = resolve_plane(k, model, b2)?;
                    let n = base.normal();
                    if length(cross(n, other.normal())) > 1e-6 {
                        return Err("midplane needs two parallel planes".into());
                    }
                    base.origin = add(base.origin, scale(n, dot(sub(other.origin, base.origin), n) / 2.0));
                }
                let offset = eval(&p.offset)?;
                let angle = eval(&p.angle)?.to_radians();
                let n = base.normal();
                let y_axis = add(scale(base.y_axis, angle.cos()), scale(n, angle.sin()));
                let turned = Plane { origin: base.origin, x_axis: base.x_axis, y_axis };
                let plane = Plane { origin: add(base.origin, scale(turned.normal(), offset)), ..turned };
                model.planes.insert(id, plane);
                Ok(())
            }

            FeatureKind::Axis(a) => {
                let axis = match a.source {
                    AxisSource::FaceNormal(f) => {
                        let (_, info) = find_face(k, model, &f)?;
                        (info.point, info.normal)
                    }
                    AxisSource::Edge(e) => edge_axis(k, model, &e)?,
                };
                model.axes.insert(id, axis);
                Ok(())
            }

            FeatureKind::PressPull(pp) => {
                let pp = pp.clone();
                let d = eval(&pp.distance)?;
                if d.abs() < 1e-9 {
                    return Err("distance is zero".into());
                }
                let (bi, _) = find_face(k, model, &pp.face)?;
                let solid = &model.bodies[bi].solid;
                let tool = k.extrude_face(solid, &pp.face, d).map_err(kerr)?;
                let op = if d > 0.0 { BooleanOp::Union } else { BooleanOp::Subtract };
                model.bodies[bi].solid = k.boolean(op, solid, &tool).map_err(kerr)?;
                let operation = if d > 0.0 { Operation::Join } else { Operation::Cut };
                let name = model.bodies[bi].name.clone();
                record_tool(model, id, tool, operation, vec![name]);
                Ok(())
            }

            FeatureKind::Thread(t) => {
                let t = t.clone();
                let (bi, _) = find_face(k, model, &t.face)?;
                let cyl = k.cylinder(&model.bodies[bi].solid, &t.face).ok_or("pick a cylindrical face")?;
                let external = cyl.outward;
                let size = if t.size.trim().eq_ignore_ascii_case("auto") {
                    crate::thread::size_for(2.0 * cyl.radius, external).to_string()
                } else {
                    t.size.clone()
                };
                let (major, pitch) = crate::thread::metric(&size).ok_or_else(|| format!("unknown thread size {size}"))?;
                let depth = crate::thread::groove_depth(pitch);
                let eps = 0.05 * pitch;
                // Shafts: the cylinder is the major diameter. Bores: cut out to it.
                let (base, apex) = if external { (cyl.radius + eps, cyl.radius - depth) } else { (cyl.radius - eps, major / 2.0) };
                if apex <= 0.0 || (!external && apex <= cyl.radius) {
                    return Err(format!("{size} doesn't fit this {:.2} mm cylinder", 2.0 * cyl.radius));
                }
                let total = cyl.to - cyl.from;
                let length = match &t.length {
                    Some(l) => {
                        let l = eval(l)?;
                        if l <= 0.0 {
                            return Err("thread length must be positive".into());
                        }
                        l.min(total)
                    }
                    None => total,
                };
                // Start at the end nearer the picked point and run past it a little.
                let at = dot(sub(t.face.point, cyl.origin), cyl.axis);
                let (from, to) = if at - cyl.from <= cyl.to - at {
                    (cyl.from - pitch, cyl.from + length)
                } else {
                    (cyl.to - length, cyl.to + pitch)
                };
                let groove = k.thread_groove(cyl.origin, cyl.axis, base, apex, pitch, from, to).map_err(kerr)?;
                let mut solid = model.bodies[bi].solid.clone();
                // A bore: plug it, cut the thread, drill again (OCCT misses the
                // groove crossing an existing bore wall).
                let plug = if external {
                    None
                } else {
                    let center = add(cyl.origin, scale(cyl.axis, cyl.from));
                    let profile = PlacedProfile {
                        outer: Loop3 { segments: vec![Seg3::Circle { center, normal: cyl.axis, radius: cyl.radius }] },
                        holes: vec![],
                        normal: cyl.axis,
                    };
                    let plug = k.extrude(&profile, cyl.to - cyl.from, false).map_err(kerr)?;
                    solid = k.boolean(BooleanOp::Union, &solid, &plug).map_err(kerr)?;
                    Some(plug)
                };
                solid = k.boolean(BooleanOp::Subtract, &solid, &groove).map_err(kerr)?;
                if let Some(plug) = plug {
                    solid = k.boolean(BooleanOp::Subtract, &solid, &plug).map_err(kerr)?;
                }
                model.bodies[bi].solid = solid;
                let name = model.bodies[bi].name.clone();
                record_tool(model, id, groove, Operation::Cut, vec![name]);
                Ok(())
            }

            FeatureKind::Draft(d) => {
                let d = d.clone();
                let angle = eval(&d.angle)?;
                if angle.abs() >= 89.0 {
                    return Err("angle must be between -89° and 89°".into());
                }
                let first = d.faces.first().ok_or("pick at least one face to draft")?;
                let (bi, _) = find_face(k, model, first)?;
                for f in &d.faces[1..] {
                    if find_face(k, model, f)?.0 != bi {
                        return Err("all faces must belong to the same body".into());
                    }
                }
                let neutral = resolve_plane(k, model, &d.pull)?;
                let pull = if d.flip { scale(neutral.normal(), -1.0) } else { neutral.normal() };
                let solid = &model.bodies[bi].solid;
                model.bodies[bi].solid =
                    k.draft(solid, &d.faces, neutral.origin, pull, angle.to_radians()).map_err(kerr)?;
                Ok(())
            }

            FeatureKind::Shell(sh) => {
                let sh = sh.clone();
                let t = eval(&sh.thickness)?;
                if t <= 0.0 {
                    return Err("thickness must be positive".into());
                }
                let first = sh.faces.first().ok_or("pick at least one face to remove")?;
                let (bi, _) = find_face(k, model, first)?;
                for f in &sh.faces[1..] {
                    if find_face(k, model, f)?.0 != bi {
                        return Err("all faces must belong to the same body".into());
                    }
                }
                model.bodies[bi].solid = k.shell(&model.bodies[bi].solid, &sh.faces, t).map_err(kerr)?;
                Ok(())
            }

            FeatureKind::Hole(h) => {
                let h = h.clone();
                // Tapped holes are drilled at the tap drill size.
                let tap = match &h.thread {
                    Some(size) => Some(crate::thread::metric(size).ok_or_else(|| format!("unknown thread size {size}"))?),
                    None => None,
                };
                let diameter = match tap {
                    Some((major, pitch)) => major - pitch,
                    None => eval(&h.diameter)?,
                };
                if diameter <= 0.0 {
                    return Err("diameter must be positive".into());
                }
                let depth = if h.through { 1e4 } else { eval(&h.depth)? };
                if depth <= 0.0 {
                    return Err("depth must be positive".into());
                }
                // Hole sites: point on the surface and outward normal.
                let (sites, targets): (Vec<(Vec3, Vec3)>, Vec<usize>) = match &h.placement {
                    HolePlacement::Face(face) => {
                        let (bi, info) = find_face(k, model, face)?;
                        if !info.planar {
                            return Err("holes need a flat face".into());
                        }
                        let n = normalize(info.normal);
                        let on_face = sub(face.point, scale(n, dot(sub(face.point, info.point), n)));
                        let targets = if h.through { (0..model.bodies.len()).collect() } else { vec![bi] };
                        (vec![(on_face, n)], targets)
                    }
                    HolePlacement::Points { sketch, points, flip } => {
                        let s = self.sketch_before(i, *sketch)?;
                        if points.is_empty() {
                            return Err("pick at least one sketch point".into());
                        }
                        let count = s.sketch.point_ids().count();
                        let n = if *flip { scale(s.plane.normal(), -1.0) } else { s.plane.normal() };
                        let mut sites = Vec::new();
                        for p in points {
                            if p.0 >= count {
                                return Err("a hole point no longer exists – re-pick the points".into());
                            }
                            sites.push((s.plane.to_world(s.sketch.point_pos(*p)), n));
                        }
                        (sites, (0..model.bodies.len()).collect())
                    }
                };
                if model.bodies.is_empty() {
                    return Err("no body to drill into".into());
                }
                let mut tools = Vec::new();
                for &(on_face, n) in &sites {
                    // Start a little above the face so the cut is clean.
                    let eps = 0.01;
                    let center = add(on_face, scale(n, eps));
                    let profile = PlacedProfile {
                        outer: Loop3 { segments: vec![Seg3::Circle { center, normal: n, radius: diameter / 2.0 }] },
                        holes: vec![],
                        normal: n,
                    };
                    tools.push(k.extrude(&profile, -(depth + eps), false).map_err(kerr)?);
                    match &h.head {
                        HoleHead::None => {}
                        HoleHead::Counterbore { diameter: hd, depth: hdepth } => {
                            let (hd, hdepth) = (eval(hd)?, eval(hdepth)?);
                            if hd <= diameter || hdepth <= 0.0 {
                                return Err("counterbore must be wider than the hole and have a depth".into());
                            }
                            let head = PlacedProfile {
                                outer: Loop3 { segments: vec![Seg3::Circle { center, normal: n, radius: hd / 2.0 }] },
                                holes: vec![],
                                normal: n,
                            };
                            tools.push(k.extrude(&head, -(hdepth + eps), false).map_err(kerr)?);
                        }
                        HoleHead::Countersink { diameter: hd, angle } => {
                            let (hd, angle) = (eval(hd)?, eval(angle)?);
                            if hd <= diameter || !(10.0..170.0).contains(&angle) {
                                return Err("countersink must be wider than the hole, angle 10–170°".into());
                            }
                            // Cone: revolve a right triangle around the hole axis.
                            let half = (angle / 2.0).to_radians();
                            let r = hd / 2.0 + eps * half.tan();
                            let h_cone = r / half.tan();
                            let u = normalize(cross(n, if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
                            let at = |radial: f64, axial: f64| add(center, add(scale(u, radial), scale(n, axial)));
                            let (p0, p1, p2) = (at(0.0, 0.0), at(r, 0.0), at(0.0, -h_cone));
                            let segments = vec![Seg3::Line { a: p0, b: p1 }, Seg3::Line { a: p1, b: p2 }, Seg3::Line { a: p2, b: p0 }];
                            let profile = PlacedProfile { outer: Loop3 { segments }, holes: vec![], normal: cross(u, n) };
                            tools.push(k.revolve(&profile, center, n, std::f64::consts::TAU).map_err(kerr)?);
                        }
                    }
                }
                // Threads are cut before drilling: OCCT misses grooves that
                // cross an existing bore wall.
                let mut grooves = Vec::new();
                if let Some((major, pitch)) = tap {
                    let eps = 0.05 * pitch;
                    for &(on_face, n) in &sites {
                        let axis = scale(n, -1.0);
                        // Through holes: thread as far as the bodies reach.
                        let length = if h.through {
                            targets
                                .iter()
                                .filter_map(|b| k.bounds(&model.bodies[*b].solid))
                                .flat_map(|(lo, hi)| {
                                    (0..8).map(move |c| {
                                        let p = [if c & 1 == 0 { lo[0] } else { hi[0] }, if c & 2 == 0 { lo[1] } else { hi[1] }, if c & 4 == 0 { lo[2] } else { hi[2] }];
                                        dot(sub(p, on_face), axis)
                                    })
                                })
                                .fold(0.0, f64::max)
                        } else {
                            depth
                        };
                        if length > 0.0 {
                            let groove = k
                                .thread_groove(on_face, axis, diameter / 2.0 - eps, major / 2.0, pitch, -pitch, length)
                                .map_err(kerr)?;
                            grooves.push(groove);
                        }
                    }
                }
                grooves.append(&mut tools);
                let tools = grooves;
                let names: Vec<String> = targets.iter().map(|b| model.bodies[*b].name.clone()).collect();
                for b in targets {
                    for tool in &tools {
                        model.bodies[b].solid =
                            k.boolean(BooleanOp::Subtract, &model.bodies[b].solid, tool).map_err(kerr)?;
                    }
                }
                for tool in tools {
                    record_tool(model, id, tool, Operation::Cut, names.clone());
                }
                Ok(())
            }

            FeatureKind::Mirror(m) => {
                let m = m.clone();
                let plane = resolve_plane(k, model, &m.plane)?;
                let t = Transform::Mirror { origin: plane.origin, normal: plane.normal() };
                for name in &expand(model, &m.bodies) {
                    let bi = body_index(model, name)?;
                    let image = k.transform(&model.bodies[bi].solid, &t).map_err(kerr)?;
                    if m.join {
                        let joined = k.boolean(BooleanOp::Union, &model.bodies[bi].solid, &image).map_err(kerr)?;
                        model.bodies[bi].solid = joined;
                    } else {
                        push_body(model, body_counter, image, id);
                    }
                }
                Ok(())
            }

            FeatureKind::Pattern(p) => {
                let p = p.clone();
                let count = |e: &str| -> Result<usize, String> {
                    let n = eval(e)?.round();
                    if !(1.0..=500.0).contains(&n) {
                        return Err("count must be between 1 and 500".into());
                    }
                    Ok(n as usize)
                };
                let mut transforms = Vec::new();
                match &p.kind {
                    PatternKind::Rectangular { direction, count: c1, spacing, direction2, count2, spacing2 } => {
                        let d1 = normalize(resolve_axis(model, direction)?.1);
                        let d2 = normalize(resolve_axis(model, direction2)?.1);
                        let (n1, n2) = (count(c1)?, count(count2)?);
                        let (s1, s2) = (eval(spacing)?, if n2 > 1 { eval(spacing2)? } else { 0.0 });
                        for a in 0..n1 {
                            for b in 0..n2 {
                                if a + b > 0 {
                                    let v = add(scale(d1, s1 * a as f64), scale(d2, s2 * b as f64));
                                    transforms.push(Transform::Translate(v));
                                }
                            }
                        }
                    }
                    PatternKind::Circular { axis, count: c, angle } => {
                        let (origin, dir) = resolve_axis(model, axis)?;
                        let n = count(c)?;
                        let total = eval(angle)?;
                        let full = (total.abs() - 360.0).abs() < 1e-9;
                        let step = if full || n < 2 { total / n as f64 } else { total / (n - 1) as f64 };
                        for a in 1..n {
                            let angle = (step * a as f64).to_radians();
                            transforms.push(Transform::Rotate { origin, axis: normalize(dir), angle });
                        }
                    }
                }
                if transforms.is_empty() {
                    return Err("the pattern has only one instance".into());
                }
                if !p.features.is_empty() {
                    let mut sources = Vec::new();
                    for f in &p.features {
                        let tools = model.tools.get(f).cloned().ok_or(
                            "a patterned feature has no geometry (pattern extrudes, revolves, holes, lofts, sweeps or press pulls)",
                        )?;
                        sources.extend(tools);
                    }
                    for t in &transforms {
                        for src in &sources {
                            let copy = k.transform(&src.solid, t).map_err(kerr)?;
                            match src.operation {
                                Operation::NewBody => push_body(model, body_counter, copy, id),
                                // Copies of cuts may land on any body.
                                Operation::Cut => {
                                    for b in &mut model.bodies {
                                        b.solid = k.boolean(BooleanOp::Subtract, &b.solid, &copy).map_err(kerr)?;
                                    }
                                }
                                op => {
                                    let bop = if op == Operation::Join { BooleanOp::Union } else { BooleanOp::Intersect };
                                    for name in &src.bodies {
                                        let bi = body_index(model, name)?;
                                        model.bodies[bi].solid = k.boolean(bop, &model.bodies[bi].solid, &copy).map_err(kerr)?;
                                    }
                                }
                            }
                        }
                    }
                    return Ok(());
                }
                for name in &expand(model, &p.bodies) {
                    let bi = body_index(model, name)?;
                    let source = model.bodies[bi].solid.clone();
                    let mut acc = source.clone();
                    for t in &transforms {
                        let copy = k.transform(&source, t).map_err(kerr)?;
                        if p.join {
                            acc = k.boolean(BooleanOp::Union, &acc, &copy).map_err(kerr)?;
                        } else {
                            push_body(model, body_counter, copy, id);
                        }
                    }
                    model.bodies[bi].solid = acc;
                }
                Ok(())
            }

            FeatureKind::Combine(c) => {
                let c = c.clone();
                let ti = body_index(model, &c.target)?;
                if c.tools.is_empty() {
                    return Err("pick at least one tool body".into());
                }
                let op = match c.op {
                    CombineOp::Join => BooleanOp::Union,
                    CombineOp::Cut => BooleanOp::Subtract,
                    CombineOp::Intersect => BooleanOp::Intersect,
                };
                for name in &c.tools {
                    let bi = body_index(model, name)?;
                    if bi == ti {
                        return Err("a body cannot be combined with itself".into());
                    }
                    model.bodies[ti].solid =
                        k.boolean(op, &model.bodies[ti].solid, &model.bodies[bi].solid).map_err(kerr)?;
                }
                if k.is_empty(&model.bodies[ti].solid) {
                    return Err("nothing is left of the target body".into());
                }
                if !c.keep_tools {
                    model.bodies.retain(|b| !c.tools.contains(&b.name));
                }
                Ok(())
            }

            FeatureKind::Split(sp) => {
                let sp = sp.clone();
                let bi = body_index(model, &sp.body)?;
                let plane = resolve_plane(k, model, &sp.plane)?;
                let l = 1e4;
                let corners = [[-l, -l], [l, -l], [l, l], [-l, l]].map(|uv| plane.to_world(uv));
                let segments = (0..4).map(|j| Seg3::Line { a: corners[j], b: corners[(j + 1) % 4] }).collect();
                let profile = PlacedProfile { outer: Loop3 { segments }, holes: vec![], normal: plane.normal() };
                let half = k.extrude(&profile, l, false).map_err(kerr)?;
                let solid = &model.bodies[bi].solid;
                let above = k.boolean(BooleanOp::Intersect, solid, &half).map_err(kerr)?;
                let below = k.boolean(BooleanOp::Subtract, solid, &half).map_err(kerr)?;
                if k.is_empty(&above) || k.is_empty(&below) {
                    return Err("the plane does not cut the body".into());
                }
                model.bodies[bi].solid = below;
                push_body(model, body_counter, above, id);
                Ok(())
            }

            FeatureKind::Move(mv) => {
                let mv = mv.clone();
                let v = [eval(&mv.translate[0])?, eval(&mv.translate[1])?, eval(&mv.translate[2])?];
                let angle = eval(&mv.angle)?.to_radians();
                let rotation = match mv.axis {
                    Some(a) if angle.abs() > 1e-12 => Some(resolve_axis(model, &a)?),
                    _ => None,
                };
                for name in &expand(model, &mv.bodies) {
                    let bi = body_index(model, name)?;
                    let mut solid = model.bodies[bi].solid.clone();
                    if length(v) > 1e-12 {
                        solid = k.transform(&solid, &Transform::Translate(v)).map_err(kerr)?;
                    }
                    if let Some((origin, axis)) = rotation {
                        let t = Transform::Rotate { origin, axis: normalize(axis), angle };
                        solid = k.transform(&solid, &t).map_err(kerr)?;
                    }
                    if mv.copy {
                        push_body(model, body_counter, solid, id);
                    } else {
                        model.bodies[bi].solid = solid;
                    }
                }
                Ok(())
            }

            FeatureKind::Loft(lf) => {
                let lf = lf.clone();
                if lf.profiles.len() < 2 {
                    return Err("a loft needs at least two profiles".into());
                }
                let mut sections = Vec::new();
                for p in &lf.profiles {
                    let (_, placed) = self.placed_profile(i, p.sketch, p.point)?;
                    sections.push(placed.outer);
                }
                let tool = k.loft(&sections).map_err(kerr)?;
                apply_operation(k, model, lf.operation, tool, id, body_counter)
            }

            FeatureKind::Sweep(sw) => {
                let sw = sw.clone();
                let (_, placed) = self.placed_profile(i, sw.profile.sketch, sw.profile.point)?;
                let path_sketch = self.sketch_before(i, sw.path_sketch)?;
                let path = path_segments(path_sketch, &sw.path)?;
                let tool = k.sweep(&placed, &path).map_err(kerr)?;
                apply_operation(k, model, sw.operation, tool, id, body_counter)
            }

            FeatureKind::Joint(j) => {
                let j = j.clone();
                let (p1, z1) = joint_frame(k, model, &j.from)?;
                let (p2, z2) = joint_frame(k, model, &j.to)?;
                let mut angle = eval(&j.angle)?;
                let mut offset = eval(&j.offset)?;
                match j.kind {
                    JointKind::Revolute => angle = j.clamp(angle, &eval)?,
                    JointKind::Slider => offset = j.clamp(offset, &eval)?,
                    JointKind::Rigid => {}
                }
                let angle = angle.to_radians();
                let target = if j.flip { z2 } else { scale(z2, -1.0) };
                // 1. turn the moving part so its z points at `target`, about p1
                let mut steps: Vec<Transform> = Vec::new();
                let axis = cross(z1, target);
                let c = dot(z1, target).clamp(-1.0, 1.0);
                if length(axis) > 1e-9 {
                    steps.push(Transform::Rotate { origin: p1, axis: normalize(axis), angle: c.acos() });
                } else if c < 0.0 {
                    // opposite: any perpendicular axis
                    let perp = normalize(cross(z1, if z1[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
                    steps.push(Transform::Rotate { origin: p1, axis: perp, angle: std::f64::consts::PI });
                }
                // 2. move the origins together (plus the offset along the axis)
                steps.push(Transform::Translate(add(sub(p2, p1), scale(z2, offset))));
                // 3. the joint angle around the fixed part's axis
                if angle.abs() > 1e-12 {
                    steps.push(Transform::Rotate { origin: add(p2, scale(z2, offset)), axis: z2, angle });
                }
                for name in &expand(model, &j.moving) {
                    let bi = body_index(model, name)?;
                    let mut solid = model.bodies[bi].solid.clone();
                    for t in &steps {
                        solid = k.transform(&solid, t).map_err(kerr)?;
                    }
                    model.bodies[bi].solid = solid;
                }
                Ok(())
            }

            FeatureKind::Component(c) => {
                let c = c.clone();
                if c.bodies.is_empty() {
                    return Err("a component needs at least one body".into());
                }
                if model.components.iter().any(|(n, _)| *n == c.name) || model.bodies.iter().any(|b| b.name == c.name) {
                    return Err(format!("the name {} is already used", c.name));
                }
                for b in &c.bodies {
                    body_index(model, b)?;
                }
                // A body belongs to one component: take it out of any other.
                for (_, list) in &mut model.components {
                    list.retain(|b| !c.bodies.contains(b));
                }
                model.components.retain(|(_, list)| !list.is_empty());
                model.components.push((c.name, c.bodies));
                Ok(())
            }

            FeatureKind::Import(im) => {
                let solids = k.import_step(std::path::Path::new(&im.path)).map_err(kerr)?;
                if solids.is_empty() {
                    return Err("the file contains no solids".into());
                }
                for s in solids {
                    push_body(model, body_counter, s, id);
                }
                Ok(())
            }

            FeatureKind::Remove(r) => {
                for name in expand(model, &r.bodies) {
                    let bi = body_index(model, &name)?;
                    model.bodies.remove(bi);
                }
                Ok(())
            }
        }
    }
}

impl Document {
    /// The sketch feature `sketch`, which must come before position `user`.
    fn sketch_before(&self, user: usize, sketch: FeatureId) -> Result<&SketchFeature, String> {
        let si = self.index_of(sketch).ok_or("sketch was deleted")?;
        if si >= user {
            return Err("the sketch comes after this feature".into());
        }
        if self.features[si].suppressed {
            return Err("sketch is suppressed".into());
        }
        match &self.features[si].kind {
            FeatureKind::Sketch(s) => Ok(s),
            _ => Err("referenced feature is not a sketch".into()),
        }
    }

    /// The sketch feature at `sketch` (which must come before timeline
    /// position `user`) and its profile containing `point`, placed in 3D.
    fn placed_profile(
        &self,
        user: usize,
        sketch: FeatureId,
        point: [f64; 2],
    ) -> Result<(&SketchFeature, PlacedProfile), String> {
        let s = self.sketch_before(user, sketch)?;
        let profiles = find_profiles(&s.sketch);
        let pi = profile_at(&profiles, point)
            .ok_or("profile no longer exists – edit the feature to pick it again")?;
        Ok((s, PlacedProfile::new(&profiles[pi], &s.plane)))
    }
}

fn record_tool<S>(model: &mut Model<S>, feature: FeatureId, solid: S, operation: Operation, bodies: Vec<String>) {
    model.tools.entry(feature).or_default().push(FeatureTool { solid, operation, bodies });
}

/// Points on a loop, enough to bound it (ends, arc middles, circle extremes).
fn loop_points(l: &Loop3) -> Vec<Vec3> {
    let mut out = Vec::new();
    for s in &l.segments {
        match s {
            Seg3::Line { a, b } => out.extend([*a, *b]),
            Seg3::Arc { start, mid, end } => out.extend([*start, *mid, *end]),
            Seg3::Circle { center, normal, radius } => {
                let u = normalize(cross(*normal, if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
                let w = cross(*normal, u);
                for d in [u, w, scale(u, -1.0), scale(w, -1.0)] {
                    out.push(add(*center, scale(d, *radius)));
                }
            }
            Seg3::Spline { points } => out.extend(points.iter().copied()),
        }
    }
    out
}

fn push_body<S>(model: &mut Model<S>, body_counter: &mut usize, solid: S, created_by: FeatureId) {
    *body_counter += 1;
    model.bodies.push(Body { name: format!("Body{body_counter}"), solid, created_by });
}

/// Body names with component names replaced by their bodies.
fn expand<S>(model: &Model<S>, names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for n in names {
        match model.components.iter().find(|(c, _)| c == n) {
            Some((_, bodies)) => out.extend(bodies.iter().cloned()),
            None => out.push(n.clone()),
        }
    }
    out
}

fn body_index<S>(model: &Model<S>, name: &str) -> Result<usize, String> {
    model.bodies.iter().position(|b| b.name == name).ok_or_else(|| format!("{name} no longer exists"))
}

/// The body owning the face matched by `face`, and the face itself.
pub fn find_face<K: Kernel>(k: &K, model: &Model<K::Solid>, face: &FaceRef) -> Result<(usize, FaceInfo), String> {
    model
        .bodies
        .iter()
        .enumerate()
        .filter_map(|(i, b)| k.find_face(&b.solid, face).map(|f| (i, f)))
        .min_by(|a, b| distance(a.1.point, face.point).total_cmp(&distance(b.1.point, face.point)))
        .ok_or_else(|| "the face no longer exists – re-pick it".to_string())
}

fn resolve_plane<K: Kernel>(k: &K, model: &Model<K::Solid>, p: &PlaneRef) -> Result<Plane, String> {
    match p {
        PlaneRef::Origin(o) => Ok(o.plane()),
        PlaneRef::Construction(id) => {
            model.planes.get(id).copied().ok_or_else(|| "the construction plane is missing or failed".into())
        }
        PlaneRef::Face(f) => {
            let (_, info) = find_face(k, model, f)?;
            if !info.planar {
                return Err("the face is not flat".into());
            }
            Ok(Plane::at_face(info.point, info.normal))
        }
    }
}

/// Origin and unit z axis of a joint origin.
fn joint_frame<K: Kernel>(k: &K, model: &Model<K::Solid>, o: &JointOrigin) -> Result<(Vec3, Vec3), String> {
    match o {
        JointOrigin::Face(f) => {
            let (_, info) = find_face(k, model, f)?;
            // Keep the picked spot, moved onto the face as it is now.
            let n = normalize(info.normal);
            let p = sub(f.point, scale(n, dot(sub(f.point, info.point), n)));
            Ok((p, n))
        }
        JointOrigin::Edge(e) => {
            let poly = model
                .bodies
                .iter()
                .flat_map(|b| k.edges(&b.solid))
                .filter_map(|pl| polyline_midpoint(&pl).map(|m| (distance(m, e.point), pl)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .filter(|(d, _)| *d < EDGE_MATCH_TOLERANCE)
                .map(|(_, pl)| pl)
                .ok_or("a joint edge moved or disappeared – re-pick it")?;
            let (o, dir) = polyline_axis(&poly).ok_or("the joint edge is neither straight nor circular")?;
            let straight = poly.len() == 2 || distance(o, poly[0]) < 1e-9;
            if straight {
                Ok((polyline_midpoint(&poly).unwrap_or(o), normalize(dir)))
            } else {
                Ok((o, normalize(dir)))
            }
        }
    }
}

fn resolve_axis<S>(model: &Model<S>, a: &AxisRef) -> Result<(Vec3, Vec3), String> {
    match a {
        AxisRef::World(i) => {
            let mut d = [0.0; 3];
            d[(*i).min(2)] = 1.0;
            Ok(([0.0; 3], d))
        }
        AxisRef::Construction(id) => {
            model.axes.get(id).copied().ok_or_else(|| "the construction axis is missing or failed".into())
        }
    }
}

/// Axis of the edge matched by `e`: along a straight edge, or through the
/// centre of a circular one.
fn edge_axis<K: Kernel>(k: &K, model: &Model<K::Solid>, e: &EdgeRef) -> Result<(Vec3, Vec3), String> {
    let poly = model
        .bodies
        .iter()
        .flat_map(|b| k.edges(&b.solid))
        .filter_map(|pl| polyline_midpoint(&pl).map(|m| (distance(m, e.point), pl)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .filter(|(d, _)| *d < EDGE_MATCH_TOLERANCE)
        .map(|(_, pl)| pl)
        .ok_or("the edge moved or disappeared – re-pick it")?;
    polyline_axis(&poly).ok_or_else(|| "the edge is neither straight nor circular".into())
}

/// Axis of a polyline: its direction if straight, else the axis of the
/// circle through three of its points.
pub fn polyline_axis(poly: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let (&first, &last) = (poly.first()?, poly.last()?);
    let span = sub(last, first);
    let len = length(span);
    let straight = len > 1e-9
        && poly.iter().all(|p| {
            let d = sub(*p, first);
            length(cross(d, span)) / len < 1e-6 * (1.0 + len)
        });
    if straight {
        return Some((first, scale(span, 1.0 / len)));
    }
    let n = poly.len();
    let (p0, p1, p2) = if distance(first, last) < 1e-9 {
        (poly[0], poly[n / 3], poly[2 * n / 3])
    } else {
        (first, poly[n / 2], last)
    };
    let (a, b) = (sub(p1, p0), sub(p2, p0));
    let axb = cross(a, b);
    let d = 2.0 * dot(axb, axb);
    if d < 1e-18 {
        return None;
    }
    let num = add(scale(cross(b, axb), dot(a, a)), scale(cross(axb, a), dot(b, b)));
    let center = add(p0, scale(num, 1.0 / d));
    Some((center, normalize(axb)))
}

/// Lines and arcs of a sketch chained into a connected, consistently
/// oriented path.
fn path_segments(sf: &SketchFeature, ids: &[ferrocad_sketch::EntityId]) -> Result<Vec<Seg3>, String> {
    use ferrocad_sketch::{Entity, Segment};
    let sk = &sf.sketch;
    let mut pieces: Vec<(Vec3, Vec3, Seg3)> = Vec::new();
    for &id in ids {
        if !sk.is_alive(id) {
            return Err("a path curve was deleted".into());
        }
        let w = |uv: [f64; 2]| sf.plane.to_world(uv);
        match *sk.entity(id) {
            Entity::Line { a, b } => {
                let (a, b) = (w(sk.point_pos(a)), w(sk.point_pos(b)));
                pieces.push((a, b, Seg3::Line { a, b }));
            }
            Entity::Arc { center, start, end } => {
                let seg = Segment::Arc {
                    center: sk.point_pos(center),
                    from: sk.point_pos(start),
                    to: sk.point_pos(end),
                    ccw: true,
                };
                let (s, m, e) = (w(sk.point_pos(start)), w(seg.midpoint()), w(sk.point_pos(end)));
                pieces.push((s, e, Seg3::Arc { start: s, mid: m, end: e }));
            }
            Entity::Circle { .. } => return Err("a sweep path cannot contain circles".into()),
            Entity::Point { .. } => continue,
            Entity::Spline { .. } => {
                let pts: Vec<Vec3> = sk.spline_points(id, 12).into_iter().map(w).collect();
                let (s, e) = (pts[0], pts[pts.len() - 1]);
                pieces.push((s, e, Seg3::Spline { points: pts }));
            }
        }
    }
    if pieces.is_empty() {
        return Err("pick the path curves".into());
    }
    let close = |a: Vec3, b: Vec3| distance(a, b) < 1e-6;
    let flip = |(a, b, s): (Vec3, Vec3, Seg3)| {
        let s = match s {
            Seg3::Line { a, b } => Seg3::Line { a: b, b: a },
            Seg3::Arc { start, mid, end } => Seg3::Arc { start: end, mid, end: start },
            Seg3::Spline { mut points } => {
                points.reverse();
                Seg3::Spline { points }
            }
            other => other,
        };
        (b, a, s)
    };
    // Start at a free end if the path is open.
    let all = pieces.clone();
    let free = |p: Vec3| all.iter().filter(|(a, b, _)| close(*a, p) || close(*b, p)).count() == 1;
    let start = (0..pieces.len()).find(|&j| free(pieces[j].0) || free(pieces[j].1)).unwrap_or(0);
    let mut first = pieces.remove(start);
    if !free(first.0) && free(first.1) {
        first = flip(first);
    }
    let mut end = first.1;
    let mut chain = vec![first.2];
    while !pieces.is_empty() {
        let j = pieces
            .iter()
            .position(|(a, b, _)| close(*a, end) || close(*b, end))
            .ok_or("the path curves are not connected")?;
        let mut p = pieces.remove(j);
        if !close(p.0, end) {
            p = flip(p);
        }
        end = p.1;
        chain.push(p.2);
    }
    Ok(chain)
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
        let name = format!("Body{body_counter}");
        model.bodies.push(Body { name: name.clone(), solid, created_by });
        (Operation::NewBody, name)
    };
    let mut touched = Vec::new();
    let mut operation = op;
    match op {
        Operation::NewBody => {
            let (o, name) = new_body(model, tool.clone());
            touched.push(name);
            operation = o;
        }
        Operation::Join => match model.bodies.last_mut() {
            Some(b) => {
                b.solid = k.boolean(BooleanOp::Union, &b.solid, &tool).map_err(|e| e.0)?;
                touched.push(b.name.clone());
            }
            None => {
                let (o, name) = new_body(model, tool.clone());
                touched.push(name);
                operation = o;
            }
        },
        Operation::Cut => {
            if model.bodies.is_empty() {
                return Err("nothing to cut".into());
            }
            for b in &mut model.bodies {
                b.solid = k.boolean(BooleanOp::Subtract, &b.solid, &tool).map_err(|e| e.0)?;
                touched.push(b.name.clone());
            }
        }
        Operation::Intersect => {
            let b = model.bodies.last_mut().ok_or("nothing to intersect")?;
            b.solid = k.boolean(BooleanOp::Intersect, &b.solid, &tool).map_err(|e| e.0)?;
            touched.push(b.name.clone());
        }
    }
    record_tool(model, created_by, tool, operation, touched);
    Ok(())
}
