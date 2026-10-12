//! Triangulation of planar polygons with holes (ear clipping), for filling
//! sketch profiles.

type P = [f64; 2];

fn cross(o: P, a: P, b: P) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn signed_area(pts: &[P]) -> f64 {
    (0..pts.len()).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        a[0] * b[1] - b[0] * a[1]
    })
    .sum::<f64>()
        / 2.0
}

fn inside_triangle(p: P, a: P, b: P, c: P) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Merge `hole` (clockwise) into `outer` (counter-clockwise) with a bridge
/// from the hole's rightmost vertex to a visible vertex of `outer`.
fn bridge(outer: &mut Vec<P>, hole: &[P]) {
    let hi = (0..hole.len()).max_by(|&a, &b| hole[a][0].total_cmp(&hole[b][0])).unwrap_or(0);
    let h = hole[hi];
    // Nearest crossing of the ray from h towards +x with an outer edge.
    let mut best: Option<(f64, usize)> = None;
    for i in 0..outer.len() {
        let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
        if (a[1] > h[1]) == (b[1] > h[1]) {
            continue;
        }
        let x = a[0] + (h[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
        if x >= h[0] && best.is_none_or(|(bx, _)| x < bx) {
            let j = if a[0] > b[0] { i } else { (i + 1) % outer.len() };
            best = Some((x, j));
        }
    }
    let Some((x, mut j)) = best else { return };
    // A reflex vertex inside the triangle (h, hit, outer[j]) would block the
    // bridge: take the one closest in angle to the ray instead.
    let hit = [x, h[1]];
    let mut best_angle = f64::INFINITY;
    for (k, &p) in outer.iter().enumerate() {
        let (prev, next) = (outer[(k + outer.len() - 1) % outer.len()], outer[(k + 1) % outer.len()]);
        let reflex = cross(prev, p, next) < 0.0;
        if k != j && reflex && p[0] >= h[0] {
            let (t1, t2, t3) = if hit[1] < outer[j][1] { (h, hit, outer[j]) } else { (h, outer[j], hit) };
            if inside_triangle(p, t1, t2, t3) {
                let angle = ((p[1] - h[1]).abs()).atan2(p[0] - h[0]);
                if angle < best_angle {
                    best_angle = angle;
                    j = k;
                }
            }
        }
    }
    let mut merged: Vec<P> = outer[..=j].to_vec();
    merged.extend(hole[hi..].iter().chain(&hole[..=hi]));
    merged.push(outer[j]);
    merged.extend_from_slice(&outer[j + 1..]);
    *outer = merged;
}

/// Triangles covering the polygon `outer` minus `holes` (any orientation).
pub fn triangulate(outer: &[P], holes: &[Vec<P>]) -> Vec<[P; 3]> {
    if outer.len() < 3 {
        return Vec::new();
    }
    let mut poly = outer.to_vec();
    if signed_area(&poly) < 0.0 {
        poly.reverse();
    }
    let mut holes: Vec<Vec<P>> = holes.iter().filter(|h| h.len() >= 3).cloned().collect();
    for h in &mut holes {
        if signed_area(h) > 0.0 {
            h.reverse();
        }
    }
    // Rightmost holes first, so later bridges don't cross earlier ones.
    holes.sort_by(|a, b| {
        let mx = |h: &Vec<P>| h.iter().map(|p| p[0]).fold(f64::MIN, f64::max);
        mx(b).total_cmp(&mx(a))
    });
    for h in &holes {
        bridge(&mut poly, h);
    }

    let mut idx: Vec<usize> = (0..poly.len()).collect();
    let mut out = Vec::with_capacity(poly.len());
    let mut guard = 0;
    while idx.len() > 3 && guard < poly.len() * poly.len() + 10 {
        guard += 1;
        let n = idx.len();
        let mut clipped = false;
        for i in 0..n {
            let (a, b, c) = (poly[idx[(i + n - 1) % n]], poly[idx[i]], poly[idx[(i + 1) % n]]);
            if cross(a, b, c) <= 1e-12 {
                continue;
            }
            let blocked = idx.iter().any(|&k| {
                let p = poly[k];
                p != a && p != b && p != c && inside_triangle(p, a, b, c)
            });
            if !blocked {
                out.push([a, b, c]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            // Degenerate rest (collinear points): drop a vertex and go on.
            idx.remove(0);
        }
    }
    if idx.len() == 3 {
        let t = [poly[idx[0]], poly[idx[1]], poly[idx[2]]];
        if cross(t[0], t[1], t[2]).abs() > 1e-12 {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(tris: &[[P; 3]]) -> f64 {
        tris.iter().map(|t| cross(t[0], t[1], t[2]).abs() / 2.0).sum()
    }

    #[test]
    fn square_with_two_holes_and_concave_l() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = |x: f64, y: f64| vec![[x, y], [x + 2.0, y], [x + 2.0, y + 2.0], [x, y + 2.0]];
        let tris = triangulate(&sq, &[hole(2.0, 2.0), hole(6.0, 5.0)]);
        assert!((area(&tris) - (100.0 - 8.0)).abs() < 1e-9, "{}", area(&tris));
        // No triangle covers a hole's centre.
        for c in [[3.0, 3.0], [7.0, 6.0]] {
            assert!(!tris.iter().any(|t| {
                let (a, b, d) = (t[0], t[1], t[2]);
                let s = cross(a, b, d).signum();
                cross(a, b, c) * s > 0.0 && cross(b, d, c) * s > 0.0 && cross(d, a, c) * s > 0.0
            }));
        }
        // Clockwise concave L.
        let mut l = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]];
        l.reverse();
        assert!((area(&triangulate(&l, &[])) - 20.0).abs() < 1e-9);
        // A circle-ish ring.
        let ring = |r: f64| (0..48).map(|k| { let a = k as f64 / 48.0 * std::f64::consts::TAU; [r * a.cos(), r * a.sin()] }).collect::<Vec<P>>();
        let tris = triangulate(&ring(5.0), &[ring(3.0)]);
        let exact = signed_area(&ring(5.0)) - signed_area(&ring(3.0));
        assert!((area(&tris) - exact).abs() < 1e-6, "{} vs {exact}", area(&tris));
    }
}
