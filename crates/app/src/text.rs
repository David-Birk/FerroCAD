//! Sketch text: glyph outlines of a bundled font, flattened to closed
//! polylines and added to the sketch as fixed lines ("exploded" text). The
//! region finder turns the letters into profiles, counters (the inside of
//! "o", "A", "8") become holes.

use ferrocad_sketch::{EntityId, Sketch};
use ttf_parser::{Face, OutlineBuilder};

/// Ubuntu Light, shipped with egui (Ubuntu Font Licence 1.0). Bundled, so
/// text looks the same everywhere and needs no system font lookup.
const FONT: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;

/// Largest distance between a glyph curve and its line segments, relative
/// to the text height.
pub const TOLERANCE: f64 = 0.005;

/// Collects the contours of one glyph, flattening curves on the fly.
struct Flattener {
    /// Font units → sketch units.
    scale: f64,
    /// Where the glyph's origin lands in the sketch.
    origin: [f64; 2],
    /// Chord tolerance in sketch units.
    tol: f64,
    current: Vec<[f64; 2]>,
    contours: Vec<Vec<[f64; 2]>>,
}

impl Flattener {
    fn map(&self, x: f32, y: f32) -> [f64; 2] {
        [self.origin[0] + x as f64 * self.scale, self.origin[1] + y as f64 * self.scale]
    }

    fn last(&self) -> [f64; 2] {
        self.current.last().copied().unwrap_or(self.origin)
    }

    /// Segments needed so a curve with second difference `d` stays within
    /// the tolerance (chord error ≤ k·d/n²).
    fn segments(&self, d: f64, k: f64) -> usize {
        ((k * d / self.tol).sqrt().ceil() as usize).clamp(1, 64)
    }

    fn flush(&mut self) {
        let c = std::mem::take(&mut self.current);
        if c.len() >= 3 {
            self.contours.push(c);
        }
    }
}

fn second_diff(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (a[0] - 2.0 * b[0] + c[0]).hypot(a[1] - 2.0 * b[1] + c[1])
}

impl OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        let p = self.map(x, y);
        self.current.push(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.current.push(p);
    }

    /// TrueType outlines.
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.last(), self.map(x1, y1), self.map(x, y));
        let n = self.segments(second_diff(p0, p1, p2), 0.25);
        for k in 1..=n {
            let t = k as f64 / n as f64;
            let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
            self.current.push([a * p0[0] + b * p1[0] + c * p2[0], a * p0[1] + b * p1[1] + c * p2[1]]);
        }
    }

    /// CFF (PostScript) outlines.
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.last(), self.map(x1, y1), self.map(x2, y2), self.map(x, y));
        let d = second_diff(p0, p1, p2).max(second_diff(p1, p2, p3));
        let n = self.segments(d, 0.75);
        for k in 1..=n {
            let t = k as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.current.push([
                a * p0[0] + b * p1[0] + c * p2[0] + e * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + e * p3[1],
            ]);
        }
    }

    fn close(&mut self) {
        self.flush();
    }
}

/// Closed outlines of `text` (one polyline per glyph contour, the closing
/// point not repeated). The baseline starts at the origin and runs along +x;
/// `height` is the height of capital letters.
pub fn outlines(text: &str, height: f64) -> Result<Vec<Vec<[f64; 2]>>, String> {
    if !(height.is_finite() && height > 0.0) {
        return Err("text height must be positive".into());
    }
    let face = Face::parse(FONT, 0).map_err(|e| format!("font: {e}"))?;
    let em = face.units_per_em() as f64;
    let cap = face.capital_height().filter(|h| *h > 0).map(f64::from).unwrap_or(0.7 * em);
    let scale = height / cap;
    let mut fl = Flattener { scale, origin: [0.0, 0.0], tol: TOLERANCE * height, current: Vec::new(), contours: Vec::new() };
    let mut x = 0.0;
    for ch in text.chars() {
        let glyph = face.glyph_index(ch).or_else(|| face.glyph_index('?'));
        let advance = glyph.and_then(|g| face.glyph_hor_advance(g)).map(f64::from).unwrap_or(0.5 * em);
        if let Some(g) = glyph {
            fl.origin = [x, 0.0];
            face.outline_glyph(g, &mut fl);
            fl.flush();
        }
        x += advance * scale;
    }
    Ok(fl.contours)
}

/// Add `text` as fixed lines with its baseline starting at `at`. Returns the
/// new lines.
pub fn add_text(sk: &mut Sketch, at: [f64; 2], text: &str, height: f64) -> Result<Vec<EntityId>, String> {
    let contours = outlines(text, height)?;
    let mut out = Vec::new();
    for c in contours {
        let moved: Vec<[f64; 2]> = c.iter().map(|p| [p[0] + at[0], p[1] + at[1]]).collect();
        out.extend(sk.add_fixed_polygon(&moved));
    }
    if out.is_empty() {
        return Err("type some text first".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrocad_sketch::find_profiles;
    use ferrocad_sketch::profile::profile_at;

    fn bounds(sk: &Sketch) -> ([f64; 2], [f64; 2]) {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for p in sk.used_points() {
            let q = sk.point_pos(p);
            for i in 0..2 {
                lo[i] = lo[i].min(q[i]);
                hi[i] = hi[i].max(q[i]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn letters_with_counters_become_profiles_with_holes() {
        let mut sk = Sketch::new();
        let lines = add_text(&mut sk, [10.0, 5.0], "Ao8", 10.0).unwrap();
        assert!(lines.len() > 30 && lines.len() < 400, "{} lines", lines.len());
        // Every glyph point is fixed: nothing for the solver to do.
        assert!(sk.used_points().iter().all(|p| sk.is_point_fixed(*p)));
        assert_eq!(sk.analyze().dof, 0);
        sk.solve().unwrap();

        let profiles = find_profiles(&sk);
        // Each letter body plus one disk per counter: A (1 hole) + its
        // triangle, o (1 hole) + its inside, 8 (2 holes) + its two insides.
        let mut holes: Vec<usize> = profiles.iter().map(|p| p.holes.len()).collect();
        holes.sort();
        assert_eq!(holes, vec![0, 0, 0, 0, 1, 1, 2], "{holes:?}");

        // Capital height = requested height, baseline at the click.
        let a_only = {
            let mut s = Sketch::new();
            add_text(&mut s, [10.0, 5.0], "A", 10.0).unwrap();
            bounds(&s)
        };
        assert!((a_only.0[1] - 5.0).abs() < 0.05, "{a_only:?}");
        assert!((a_only.1[1] - a_only.0[1] - 10.0).abs() < 0.2, "{a_only:?}");
        let (lo, hi) = bounds(&sk);
        assert!(hi[1] - lo[1] > 9.8 && hi[1] - lo[1] < 10.6, "{lo:?} {hi:?}");
        assert!(lo[0] >= 10.0 && hi[0] - lo[0] > 15.0, "runs along x: {lo:?} {hi:?}");

        // Clicking the stroke of the "A" (left leg, near the baseline) picks
        // the letter, which has one hole.
        let leg = (0..40)
            .map(|k| [lo[0] + 0.1 * k as f64, 5.3])
            .find_map(|q| profile_at(&profiles, q))
            .expect("a point on the A's leg");
        assert_eq!(profiles[leg].holes.len(), 1);
    }

    #[test]
    fn spaces_advance_and_bad_input_is_an_error() {
        let one = outlines("I", 5.0).unwrap();
        let two = outlines("I I", 5.0).unwrap();
        assert_eq!(two.len(), 2 * one.len());
        let shift = two[one.len()][0][0] - one[0][0][0];
        assert!(shift > 2.0, "{shift}");
        assert!(outlines("A", 0.0).is_err());
        assert!(add_text(&mut Sketch::new(), [0.0, 0.0], "   ", 5.0).is_err());
    }

    #[test]
    fn cubic_and_quadratic_curves_are_flattened_within_tolerance() {
        let mut fl = Flattener { scale: 1.0, origin: [0.0, 0.0], tol: 0.01, current: Vec::new(), contours: Vec::new() };
        // Quarter circle approximations: quadratic and cubic.
        fl.move_to(1.0, 0.0);
        fl.quad_to(1.0, 1.0, 0.0, 1.0);
        fl.line_to(0.0, 0.0);
        fl.close();
        let k = 0.552_284_8;
        fl.move_to(1.0, 0.0);
        fl.curve_to(1.0, k, k, 1.0, 0.0, 1.0);
        fl.line_to(0.0, 0.0);
        fl.close();
        assert_eq!(fl.contours.len(), 2);
        for c in &fl.contours {
            assert!(c.len() > 5, "{} points", c.len());
            // Chord midpoints stay close to the curve: the cubic is within
            // 0.03 % of a circle, the quadratic within its parabola bound.
            for w in c[..c.len() - 1].windows(2) {
                let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
                let r = m[0].hypot(m[1]);
                assert!(r > 0.6 && r < 1.0 + 1e-3, "{r}");
            }
        }
        let cubic = &fl.contours[1];
        for w in cubic[..cubic.len() - 1].windows(2) {
            let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            assert!(1.0 - m[0].hypot(m[1]) < 0.011, "chord sag beyond tolerance");
        }
    }
}
