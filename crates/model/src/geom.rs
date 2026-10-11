//! Tiny 3D vector helpers and sketch planes.

pub type Vec3 = [f64; 3];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn length(a: Vec3) -> f64 {
    dot(a, a).sqrt()
}

pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        a
    }
}

pub fn distance(a: Vec3, b: Vec3) -> f64 {
    length(sub(a, b))
}

/// A sketch plane: 2D sketch coordinates (u, v) map to
/// `origin + u * x_axis + v * y_axis`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
}

impl Plane {
    pub const XY: Plane = Plane { origin: [0.0; 3], x_axis: [1.0, 0.0, 0.0], y_axis: [0.0, 1.0, 0.0] };
    /// Front plane in a Z-up world: normal points towards −Y (the viewer).
    pub const XZ: Plane = Plane { origin: [0.0; 3], x_axis: [1.0, 0.0, 0.0], y_axis: [0.0, 0.0, 1.0] };
    pub const YZ: Plane = Plane { origin: [0.0; 3], x_axis: [0.0, 1.0, 0.0], y_axis: [0.0, 0.0, 1.0] };

    /// Plane through `origin` with the given normal; picks a stable in-plane
    /// x axis.
    pub fn from_normal(origin: Vec3, normal: Vec3) -> Plane {
        let n = normalize(normal);
        let helper = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let x_axis = normalize(cross(helper, n));
        let y_axis = cross(n, x_axis);
        Plane { origin, x_axis, y_axis }
    }

    pub fn normal(&self) -> Vec3 {
        normalize(cross(self.x_axis, self.y_axis))
    }

    pub fn to_world(&self, uv: [f64; 2]) -> Vec3 {
        add(self.origin, add(scale(self.x_axis, uv[0]), scale(self.y_axis, uv[1])))
    }

    /// Project a world point onto the plane's (u, v) coordinates.
    pub fn to_local(&self, p: Vec3) -> [f64; 2] {
        let d = sub(p, self.origin);
        [dot(d, self.x_axis), dot(d, self.y_axis)]
    }

    /// Where a ray hits the plane, if it is not parallel to it.
    pub fn intersect_ray(&self, origin: Vec3, dir: Vec3) -> Option<Vec3> {
        let n = self.normal();
        let denom = dot(n, dir);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = dot(n, sub(self.origin, origin)) / denom;
        Some(add(origin, scale(dir, t)))
    }
}
