//! ISO metric coarse threads (ISO 261/262) and the thread profile used for
//! modelled threads.

/// (designation, major diameter, pitch) in mm.
pub const METRIC: [(&str, f64, f64); 12] = [
    ("M2", 2.0, 0.4),
    ("M2.5", 2.5, 0.45),
    ("M3", 3.0, 0.5),
    ("M4", 4.0, 0.7),
    ("M5", 5.0, 0.8),
    ("M6", 6.0, 1.0),
    ("M8", 8.0, 1.25),
    ("M10", 10.0, 1.5),
    ("M12", 12.0, 1.75),
    ("M14", 14.0, 2.0),
    ("M16", 16.0, 2.0),
    ("M20", 20.0, 2.5),
];

/// Major diameter and pitch of a designation like "M5".
pub fn metric(designation: &str) -> Option<(f64, f64)> {
    let d = designation.trim();
    METRIC.iter().find(|(n, ..)| n.eq_ignore_ascii_case(d)).map(|&(_, dia, pitch)| (dia, pitch))
}

/// Depth of the modelled V groove (5/8 of the fundamental triangle height,
/// the basic ISO profile).
pub fn groove_depth(pitch: f64) -> f64 {
    0.625 * 0.866_025_4 * pitch
}

/// Tap drill diameter for an internal thread: major diameter minus pitch.
pub fn tap_drill(designation: &str) -> Option<f64> {
    metric(designation).map(|(d, p)| d - p)
}

/// The size an existing cylinder is meant for: an outer surface is the
/// major diameter, a bore the tap drill.
pub fn size_for(diameter: f64, external: bool) -> &'static str {
    let fit = |&&(_, d, p): &&(&str, f64, f64)| if external { (d - diameter).abs() } else { (d - p - diameter).abs() };
    METRIC.iter().min_by(|a, b| fit(a).total_cmp(&fit(b))).map(|(n, ..)| *n).unwrap_or("M5")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(metric("m5"), Some((5.0, 0.8)));
        assert_eq!(metric("M7"), None);
        assert_eq!(tap_drill("M3"), Some(2.5));
        assert_eq!(size_for(4.95, true), "M5");
        assert_eq!(size_for(4.2, false), "M5");
        assert_eq!(size_for(2.5, false), "M3");
        assert!((groove_depth(1.0) - 0.5413).abs() < 1e-4);
    }
}
