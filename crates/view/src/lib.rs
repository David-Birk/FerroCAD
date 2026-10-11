//! Viewport math that does not depend on any GUI toolkit: an orbit camera
//! (Z up, like Fusion), screen ↔ world conversion, depth-sorted flat-shaded
//! triangles and picking helpers.
//!
//! The first renderer simply hands the sorted triangles to egui's painter.
//! A wgpu pipeline with a real depth buffer can later replace that without
//! touching this crate's camera or picking.

use ferrocad_model::geom::{add, cross, dot, normalize, scale, sub, Vec3};
use ferrocad_model::TriMesh;

/// Screen rectangle of the viewport in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Viewport {
    fn center(&self) -> [f64; 2] {
        [self.x + self.width * 0.5, self.y + self.height * 0.5]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Rotation around Z, radians.
    pub yaw: f64,
    /// Elevation above the XY plane, radians.
    pub pitch: f64,
    pub distance: f64,
    /// Vertical field of view, radians.
    pub fov_y: f64,
}

impl Default for Camera {
    fn default() -> Self {
        let mut c = Camera {
            target: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            distance: 200.0,
            fov_y: 35f64.to_radians(),
        };
        c.set_view(ViewPreset::Home);
        c
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewPreset {
    Home,
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
}

const PITCH_LIMIT: f64 = 89.9 * std::f64::consts::PI / 180.0;

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (cp, sp) = (self.pitch.cos(), self.pitch.sin());
        let (cy, sy) = (self.yaw.cos(), self.yaw.sin());
        add(self.target, scale([cp * cy, cp * sy, sp], self.distance))
    }

    /// (right, up, forward) unit vectors.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = normalize(sub(self.target, self.eye()));
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        let up = cross(right, forward);
        (right, up, forward)
    }

    pub fn set_view(&mut self, v: ViewPreset) {
        use std::f64::consts::FRAC_PI_2 as Q;
        let (yaw, pitch) = match v {
            ViewPreset::Home => (-Q + 0.6, 0.55),
            ViewPreset::Front => (-Q, 0.0),
            ViewPreset::Back => (Q, 0.0),
            ViewPreset::Top => (-Q, PITCH_LIMIT),
            ViewPreset::Bottom => (-Q, -PITCH_LIMIT),
            ViewPreset::Left => (std::f64::consts::PI, 0.0),
            ViewPreset::Right => (0.0, 0.0),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Look straight at a plane along its normal (entering sketch mode).
    pub fn look_at_plane(&mut self, normal: Vec3) {
        let n = normalize(normal);
        self.pitch = n[2].clamp(-1.0, 1.0).asin().clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.yaw = if n[0].abs() > 1e-9 || n[1].abs() > 1e-9 {
            n[1].atan2(n[0])
        } else {
            -std::f64::consts::FRAC_PI_2 // XY plane: +X right, +Y up
        };
    }

    fn pixel_scale(&self, vp: &Viewport) -> f64 {
        (self.fov_y * 0.5).tan() / (vp.height * 0.5)
    }

    /// World point → (screen x, screen y, depth). `None` behind the camera.
    pub fn project(&self, p: Vec3, vp: &Viewport) -> Option<[f64; 3]> {
        let (right, up, forward) = self.basis();
        let d = sub(p, self.eye());
        let z = dot(d, forward);
        if z < self.distance * 1e-3 {
            return None;
        }
        let k = 1.0 / (z * self.pixel_scale(vp));
        let c = vp.center();
        Some([c[0] + dot(d, right) * k, c[1] - dot(d, up) * k, z])
    }

    /// Ray through a screen point: (origin, unit direction).
    pub fn ray(&self, screen: [f64; 2], vp: &Viewport) -> (Vec3, Vec3) {
        let (right, up, forward) = self.basis();
        let c = vp.center();
        let s = self.pixel_scale(vp);
        let x = (screen[0] - c[0]) * s;
        let y = -(screen[1] - c[1]) * s;
        let dir = normalize(add(forward, add(scale(right, x), scale(up, y))));
        (self.eye(), dir)
    }

    /// Orbit by a mouse delta in pixels.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Pan so the content follows the mouse.
    pub fn pan(&mut self, dx: f64, dy: f64, vp: &Viewport) {
        let (right, up, _) = self.basis();
        let s = self.distance * self.pixel_scale(vp);
        self.target = add(self.target, add(scale(right, -dx * s), scale(up, dy * s)));
    }

    /// Zoom towards the point under the cursor. `factor` < 1 zooms in.
    pub fn zoom_at(&mut self, factor: f64, screen: [f64; 2], vp: &Viewport) {
        let factor = factor.clamp(0.2, 5.0);
        let (o, dir) = self.ray(screen, vp);
        // point on the focal plane under the cursor stays fixed
        let (_, _, forward) = self.basis();
        let t = self.distance / dot(dir, forward).max(1e-6);
        let anchor = add(o, scale(dir, t));
        self.target = add(anchor, scale(sub(self.target, anchor), factor));
        self.distance = (self.distance * factor).clamp(1e-3, 1e7);
    }

    /// Frame an axis-aligned bounding box.
    pub fn fit(&mut self, min: Vec3, max: Vec3) {
        self.target = scale(add(min, max), 0.5);
        let radius = (0.5 * ferrocad_model::geom::distance(min, max)).max(1.0);
        self.distance = radius / (self.fov_y * 0.5).sin() * 1.15;
    }
}

/// A screen-space triangle ready to be filled.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenTri {
    pub points: [[f32; 2]; 3],
    pub color: [u8; 3],
    pub depth: f64,
}

/// Project, cull back faces, shade with a headlight and sort far → near
/// (painter's algorithm).
pub fn shade_mesh(mesh: &TriMesh, cam: &Camera, vp: &Viewport, base: [u8; 3]) -> Vec<ScreenTri> {
    let (_, _, forward) = cam.basis();
    let eye = cam.eye();
    let projected: Vec<Option<[f64; 3]>> = mesh.positions.iter().map(|p| cam.project(*p, vp)).collect();
    let mut out = Vec::with_capacity(mesh.indices.len() / 3);
    for tri in mesh.indices.chunks_exact(3) {
        let [i, j, k] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (Some(a), Some(b), Some(c)) = (projected[i], projected[j], projected[k]) else {
            continue;
        };
        let (pa, pb, pc) = (mesh.positions[i], mesh.positions[j], mesh.positions[k]);
        let n = normalize(cross(sub(pb, pa), sub(pc, pa)));
        let center = scale(add(pa, add(pb, pc)), 1.0 / 3.0);
        let to_eye = normalize(sub(eye, center));
        let facing = dot(n, to_eye);
        if facing <= 0.0 {
            continue; // back face
        }
        // headlight + a little sky light from above
        let light = 0.25 + 0.6 * facing + 0.15 * (0.5 + 0.5 * n[2]);
        let light = light.clamp(0.0, 1.0);
        let color = base.map(|ch| (ch as f64 * light).round().clamp(0.0, 255.0) as u8);
        let depth = dot(sub(center, eye), forward);
        out.push(ScreenTri {
            points: [a, b, c].map(|p| [p[0] as f32, p[1] as f32]),
            color,
            depth,
        });
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// Distance from `p` to segment `ab` in 2D.
pub fn point_segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + t * ab[0], a[1] + t * ab[1]];
    (p[0] - q[0]).hypot(p[1] - q[1])
}

/// Screen distance from `p` to a projected 3D polyline.
pub fn polyline_screen_distance(p: [f64; 2], poly: &[Vec3], cam: &Camera, vp: &Viewport) -> f64 {
    let pts: Vec<[f64; 2]> =
        poly.iter().filter_map(|q| cam.project(*q, vp)).map(|s| [s[0], s[1]]).collect();
    match pts.len() {
        0 => f64::INFINITY,
        1 => (p[0] - pts[0][0]).hypot(p[1] - pts[0][1]),
        _ => pts.windows(2).map(|w| point_segment_distance(p, w[0], w[1])).fold(f64::INFINITY, f64::min),
    }
}

/// Bounding box of all meshes, if any.
pub fn bounds<'a>(meshes: impl IntoIterator<Item = &'a TriMesh>) -> Option<(Vec3, Vec3)> {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut any = false;
    for m in meshes {
        for p in &m.positions {
            any = true;
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
    }
    any.then_some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport { x: 0.0, y: 0.0, width: 800.0, height: 600.0 }
    }

    #[test]
    fn target_projects_to_center() {
        let cam = Camera::default();
        let s = cam.project(cam.target, &vp()).unwrap();
        assert!((s[0] - 400.0).abs() < 1e-9 && (s[1] - 300.0).abs() < 1e-9);
    }

    #[test]
    fn ray_hits_what_was_projected() {
        let mut cam = Camera::default();
        cam.orbit(37.0, -12.0);
        let p = [12.0, -7.0, 3.0];
        let s = cam.project(p, &vp()).unwrap();
        let (o, d) = cam.ray([s[0], s[1]], &vp());
        // distance from p to the ray
        let op = sub(p, o);
        let along = dot(op, d);
        let closest = add(o, scale(d, along));
        assert!(ferrocad_model::geom::distance(closest, p) < 1e-9);
    }

    #[test]
    fn front_view_is_upright() {
        let mut cam = Camera::default();
        cam.set_view(ViewPreset::Front);
        let top = cam.project([0.0, 0.0, 10.0], &vp()).unwrap();
        let right = cam.project([10.0, 0.0, 0.0], &vp()).unwrap();
        assert!(top[1] < 300.0, "+Z is up on screen");
        assert!(right[0] > 400.0, "+X is right on screen");
    }

    #[test]
    fn top_view_sees_xy_plane() {
        let mut cam = Camera::default();
        cam.set_view(ViewPreset::Top);
        let x = cam.project([10.0, 0.0, 0.0], &vp()).unwrap();
        let y = cam.project([0.0, 10.0, 0.0], &vp()).unwrap();
        assert!(x[0] > 400.0, "+X right");
        assert!(y[1] < 300.0, "+Y up");
    }

    #[test]
    fn pan_moves_content_with_mouse() {
        let mut cam = Camera::default();
        let p = [5.0, 5.0, 0.0];
        let before = cam.project(p, &vp()).unwrap();
        cam.pan(30.0, -20.0, &vp());
        let after = cam.project(p, &vp()).unwrap();
        // exact for points on the focal plane, close enough here
        assert!((after[0] - before[0] - 30.0).abs() < 3.0);
        assert!((after[1] - before[1] + 20.0).abs() < 3.0);
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut cam = Camera::default();
        let cursor = [500.0, 250.0];
        let (o, d) = cam.ray(cursor, &vp());
        let (_, _, f) = cam.basis();
        let anchor = add(o, scale(d, cam.distance / dot(d, f)));
        cam.zoom_at(0.5, cursor, &vp());
        let s = cam.project(anchor, &vp()).unwrap();
        assert!((s[0] - cursor[0]).abs() < 1e-6 && (s[1] - cursor[1]).abs() < 1e-6);
    }

    #[test]
    fn back_faces_are_culled_and_sorted() {
        // Two facing-up triangles at different heights, one facing down.
        let mesh = TriMesh {
            positions: vec![
                [0., 0., 0.], [10., 0., 0.], [0., 10., 0.],
                [0., 0., 5.], [10., 0., 5.], [0., 10., 5.],
                [0., 0., 9.], [0., 10., 9.], [10., 0., 9.],
            ],
            normals: vec![],
            indices: vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
        };
        let mut cam = Camera::default();
        cam.set_view(ViewPreset::Top);
        let tris = shade_mesh(&mesh, &cam, &vp(), [200, 200, 200]);
        assert_eq!(tris.len(), 2, "downward triangle culled");
        assert!(tris[0].depth > tris[1].depth, "far first");
    }

    #[test]
    fn look_at_plane_faces_it() {
        let mut cam = Camera::default();
        cam.look_at_plane([0.0, -1.0, 0.0]);
        let (_, _, f) = cam.basis();
        assert!((f[1] - 1.0).abs() < 1e-9, "looking along +Y towards the XZ plane");
    }

    #[test]
    fn segment_distance() {
        assert_eq!(point_segment_distance([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]), 3.0);
        assert_eq!(point_segment_distance([13.0, 4.0], [0.0, 0.0], [10.0, 0.0]), 5.0);
    }
}
