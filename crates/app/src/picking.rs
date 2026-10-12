//! Hit-testing in the 3D viewport, shared by selection and all commands.

use crate::app::FerroApp;
use crate::sketch_ui::entity_polyline;
use egui::Pos2;
use ferrocad_model::geom::{cross, distance, dot, normalize, scale, sub, Vec3};
use ferrocad_model::kernel::polyline_midpoint;
use ferrocad_model::{EdgeRef, FaceRef, FeatureId, FeatureKind, ProfileRef};
use ferrocad_sketch::profile::profile_at;
use ferrocad_sketch::{find_profiles, EntityId};
use ferrocad_view::{face_component, point_segment_distance, ray_mesh, Viewport};

pub const PICK_PX: f64 = 8.0;

/// A face under the cursor.
pub struct FaceHit {
    pub body: usize,
    pub face: FaceRef,
    pub planar: bool,
    pub t: f64,
}

impl FerroApp {
    fn ray(&self, pos: Pos2, vp: &Viewport) -> (Vec3, Vec3) {
        let (o, d) = self.camera.ray([pos.x as f64, pos.y as f64], vp);
        (o, normalize(d))
    }

    /// Nearest visible body face under the cursor.
    pub fn pick_face(&self, pos: Pos2, vp: &Viewport) -> Option<FaceHit> {
        let (o, d) = self.ray(pos, vp);
        let mut best: Option<FaceHit> = None;
        for (bi, body) in self.bodies.iter().enumerate().filter(|(_, b)| b.visible) {
            let Some((tri, t)) = ray_mesh(&body.mesh, o, d) else { continue };
            if best.as_ref().is_some_and(|b| b.t <= t) {
                continue;
            }
            let m = &body.mesh;
            let [a, b, c] = [0, 1, 2].map(|k| m.positions[m.indices[tri * 3 + k] as usize]);
            let mut n = normalize(cross(sub(b, a), sub(c, a)));
            if dot(n, d) > 0.0 {
                n = scale(n, -1.0);
            }
            let triangles = face_component(m, tri);
            let planar = triangles.iter().all(|&k| {
                let [p, q, r] = [0, 1, 2].map(|j| m.positions[m.indices[k * 3 + j] as usize]);
                let nk = cross(sub(q, p), sub(r, p));
                let len = dot(nk, nk).sqrt();
                len < 1e-12 || dot(scale(nk, 1.0 / len), n).abs() > 0.9995
            });
            let point = ferrocad_model::geom::add(o, scale(d, t));
            best = Some(FaceHit { body: bi, face: FaceRef { point, normal: n }, planar, t });
        }
        best
    }

    /// The displayed triangles of the face a reference points at.
    pub fn face_triangles(&self, face: &FaceRef) -> Option<(usize, Vec<usize>)> {
        let n = normalize(face.normal);
        let origin = ferrocad_model::geom::add(face.point, scale(n, 1e-3 * (1.0 + self.camera.distance)));
        let dir = scale(n, -1.0);
        self.bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| b.visible)
            .filter_map(|(bi, b)| ray_mesh(&b.mesh, origin, dir).map(|(tri, t)| (bi, tri, t)))
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(bi, tri, _)| (bi, face_component(&self.bodies[bi].mesh, tri)))
    }

    /// Nearest body under the cursor.
    pub fn pick_body(&self, pos: Pos2, vp: &Viewport) -> Option<usize> {
        self.pick_face(pos, vp).map(|f| f.body)
    }

    /// Nearest visible (not hidden behind a face) edge within a few pixels.
    pub fn pick_edge(&self, pos: Pos2, vp: &Viewport) -> Option<(EdgeRef, Vec<Vec3>)> {
        let p = [pos.x as f64, pos.y as f64];
        let face_t = self.pick_face(pos, vp).map(|f| f.t);
        let eye = self.camera.eye();
        let mut best: Option<(f64, Vec<Vec3>)> = None;
        for body in self.bodies.iter().filter(|b| b.visible) {
            for e in &body.edges {
                // Closest segment on screen.
                let mut closest: Option<(f64, Vec3)> = None;
                for w in e.windows(2) {
                    let (Some(a), Some(b)) = (self.camera.project(w[0], vp), self.camera.project(w[1], vp)) else {
                        continue;
                    };
                    let d = point_segment_distance(p, [a[0], a[1]], [b[0], b[1]]);
                    if closest.is_none_or(|c| d < c.0) {
                        let mid = ferrocad_model::geom::add(w[0], scale(sub(w[1], w[0]), 0.5));
                        closest = Some((d, mid));
                    }
                }
                let Some((d, point)) = closest else { continue };
                if d > PICK_PX || best.as_ref().is_some_and(|b| d >= b.0) {
                    continue;
                }
                // Hidden behind a face?
                if face_t.is_some_and(|t| t < distance(eye, point) - 0.02 * self.camera.distance) {
                    continue;
                }
                best = Some((d, e.clone()));
            }
        }
        best.and_then(|(_, e)| polyline_midpoint(&e).map(|m| (EdgeRef { point: m }, e)))
    }

    /// The edge polyline a reference points at.
    pub fn edge_polyline(&self, r: &EdgeRef) -> Option<Vec<Vec3>> {
        self.bodies
            .iter()
            .flat_map(|b| b.edges.iter())
            .filter_map(|e| polyline_midpoint(e).map(|m| (distance(m, r.point), e)))
            .filter(|(d, _)| *d < 1e-3)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, e)| e.clone())
    }

    /// Visible sketches before the marker (id, plane, sketch).
    pub fn visible_sketches(&self) -> Vec<FeatureId> {
        self.doc.features()[..self.doc.marker()]
            .iter()
            .filter(|f| matches!(f.kind, FeatureKind::Sketch(_)) && !f.suppressed)
            .map(|f| f.id)
            .collect()
    }

    /// The sketch profile under the cursor (nearest sketch plane first).
    pub fn pick_profile(&self, pos: Pos2, vp: &Viewport, sketches: &[FeatureId]) -> Option<ProfileRef> {
        let (o, d) = self.ray(pos, vp);
        sketches
            .iter()
            .filter_map(|&id| {
                let sf = self.doc.sketch(id)?;
                let hit = sf.plane.intersect_ray(o, d)?;
                let t = dot(sub(hit, o), d);
                let uv = sf.plane.to_local(hit);
                (t > 0.0 && profile_at(&find_profiles(&sf.sketch), uv).is_some())
                    .then_some((t, ProfileRef { sketch: id, point: uv }))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p)
    }

    /// The sketch curve (line/arc/circle) under the cursor.
    pub fn pick_curve(&self, pos: Pos2, vp: &Viewport, sketches: &[FeatureId]) -> Option<(FeatureId, EntityId)> {
        let p = [pos.x as f64, pos.y as f64];
        let mut best: Option<(f64, FeatureId, EntityId)> = None;
        for &id in sketches {
            let Some(sf) = self.doc.sketch(id) else { continue };
            for (eid, e) in sf.sketch.entities() {
                let pts: Vec<[f64; 3]> = entity_polyline(&sf.sketch, e)
                    .iter()
                    .filter_map(|uv| self.camera.project(sf.plane.to_world(*uv), vp))
                    .collect();
                let d = pts
                    .windows(2)
                    .map(|w| point_segment_distance(p, [w[0][0], w[0][1]], [w[1][0], w[1][1]]))
                    .fold(f64::INFINITY, f64::min);
                if d < PICK_PX && best.is_none_or(|b| d < b.0) {
                    best = Some((d, id, eid));
                }
            }
        }
        best.map(|(_, s, e)| (s, e))
    }

    /// Screen distance from the cursor to an infinite 3D line.
    pub fn line_screen_distance(&self, pos: Pos2, vp: &Viewport, origin: Vec3, dir: Vec3) -> f64 {
        let reach = self.camera.distance * 3.0;
        let a = ferrocad_model::geom::add(origin, scale(normalize(dir), -reach));
        let b = ferrocad_model::geom::add(origin, scale(normalize(dir), reach));
        match (self.camera.project(a, vp), self.camera.project(b, vp)) {
            (Some(a), Some(b)) => point_segment_distance([pos.x as f64, pos.y as f64], [a[0], a[1]], [b[0], b[1]]),
            _ => f64::INFINITY,
        }
    }
}
