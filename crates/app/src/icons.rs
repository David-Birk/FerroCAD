//! Vector icons drawn with egui's painter, plus the icon button widgets used by
//! the ribbon, browser, timeline and palettes. Coordinates are in a unit
//! square (y down) mapped onto the target rect.

use crate::theme;
use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui};
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    // documents & editing
    New,
    Open,
    Save,
    Undo,
    Redo,
    // solid
    Sketch,
    Extrude,
    Revolve,
    Fillet,
    Chamfer,
    Parameters,
    PressPull,
    Shell,
    Draft,
    Midplane,
    Hole,
    Mirror,
    RectPattern,
    CircPattern,
    Combine,
    Split,
    Move,
    Loft,
    Sweep,
    Axis,
    Measure,
    Section,
    Properties,
    Joint,
    Interference,
    // sketch create / modify
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    Dimension,
    Construction,
    Delete,
    Trim,
    Extend,
    Offset,
    Polygon,
    Slot,
    CenterRect,
    Arc3,
    Project,
    Spline,
    Finish,
    Cancel,
    // constraints
    Horizontal,
    Vertical,
    Coincident,
    Parallel,
    Perpendicular,
    Equal,
    Tangent,
    Concentric,
    Midpoint,
    Fix,
    // navigation & display
    Home,
    Fit,
    Grid,
    Edges,
    LookAt,
    // timeline
    First,
    Prev,
    Next,
    Last,
    // browser
    Body,
    Plane,
    Origin,
    Folder,
    Document,
    EyeOpen,
    EyeClosed,
}

struct Pen<'a> {
    painter: &'a Painter,
    rect: Rect,
    color: Color32,
    accent: Color32,
    width: f32,
}

impl Pen<'_> {
    fn pt(&self, (x, y): (f32, f32)) -> Pos2 {
        egui::pos2(self.rect.min.x + x * self.rect.width(), self.rect.min.y + y * self.rect.height())
    }

    fn stroke(&self, color: Color32) -> Stroke {
        Stroke::new(self.width, color)
    }

    fn line(&self, pts: &[(f32, f32)], color: Color32) {
        let pts: Vec<Pos2> = pts.iter().map(|p| self.pt(*p)).collect();
        self.painter.add(egui::Shape::line(pts, self.stroke(color)));
    }

    fn thick(&self, pts: &[(f32, f32)], color: Color32, factor: f32) {
        let pts: Vec<Pos2> = pts.iter().map(|p| self.pt(*p)).collect();
        self.painter.add(egui::Shape::line(pts, Stroke::new(self.width * factor, color)));
    }

    fn closed(&self, pts: &[(f32, f32)], color: Color32) {
        let pts: Vec<Pos2> = pts.iter().map(|p| self.pt(*p)).collect();
        self.painter.add(egui::Shape::closed_line(pts, self.stroke(color)));
    }

    /// Filled convex polygon with an outline.
    fn poly(&self, pts: &[(f32, f32)], fill: Color32, outline: Option<Color32>) {
        let pts: Vec<Pos2> = pts.iter().map(|p| self.pt(*p)).collect();
        let stroke = outline.map(|c| self.stroke(c)).unwrap_or(Stroke::NONE);
        self.painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
    }

    fn circle(&self, c: (f32, f32), r: f32, color: Color32) {
        self.painter.circle_stroke(self.pt(c), r * self.rect.width(), self.stroke(color));
    }

    fn dot(&self, c: (f32, f32), r: f32, color: Color32) {
        self.painter.circle_filled(self.pt(c), r * self.rect.width(), color);
    }

    /// Arc points; angles in degrees, counter-clockwise on screen.
    fn arc_pts(c: (f32, f32), r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
        let n = 24;
        (0..=n)
            .map(|i| {
                let t = (from + (to - from) * i as f32 / n as f32) * PI / 180.0;
                (c.0 + r * t.cos(), c.1 - r * t.sin())
            })
            .collect()
    }

    fn arc(&self, c: (f32, f32), r: f32, from: f32, to: f32, color: Color32) {
        self.line(&Self::arc_pts(c, r, from, to), color);
    }

    /// Polyline with an arrow head at its last point.
    fn arrow(&self, pts: &[(f32, f32)], color: Color32) {
        self.line(pts, color);
        let n = pts.len();
        let (a, b) = (self.pt(pts[n - 2]), self.pt(pts[n - 1]));
        let dir = (b - a).normalized();
        let side = egui::vec2(-dir.y, dir.x);
        let size = self.rect.width() * 0.2;
        let head = vec![b, b - dir * size + side * size * 0.55, b - dir * size - side * size * 0.55];
        self.painter.add(egui::Shape::convex_polygon(head, color, Stroke::NONE));
    }

    fn text(&self, at: (f32, f32), text: &str, size: f32, color: Color32) {
        self.painter.text(
            self.pt(at),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(size * self.rect.height()),
            color,
        );
    }
}

fn faded(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

/// Default accent colour for an icon.
fn accent_for(icon: Icon) -> Color32 {
    use Icon::*;
    match icon {
        Sketch | Line | Rectangle | Circle | Arc | Construction | Plane | Axis | Polygon | Slot | CenterRect | Arc3 | Spline => {
            theme::ORANGE
        }
        Finish => theme::GREEN,
        Delete | Cancel => theme::ERROR,
        _ => theme::ACCENT,
    }
}

/// Paint `icon` into `rect`. A disabled icon is drawn in a single grey.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32, enabled: bool) {
    use Icon::*;
    let accent = if enabled { accent_for(icon) } else { color };
    let p = Pen { painter, rect, color, accent, width: (rect.width() * 0.07).max(1.1) };
    let (c, a) = (p.color, p.accent);
    match icon {
        New => {
            p.closed(&[(0.25, 0.1), (0.6, 0.1), (0.78, 0.28), (0.78, 0.9), (0.25, 0.9)], c);
            p.line(&[(0.6, 0.1), (0.6, 0.28), (0.78, 0.28)], c);
            p.line(&[(0.52, 0.48), (0.52, 0.76)], a);
            p.line(&[(0.38, 0.62), (0.66, 0.62)], a);
        }
        Open | Folder => {
            let fill = if icon == Open { faded(a, 90) } else { faded(c, 50) };
            p.poly(&[(0.1, 0.28), (0.38, 0.28), (0.46, 0.37), (0.9, 0.37), (0.9, 0.82), (0.1, 0.82)], fill, None);
            p.closed(&[(0.1, 0.28), (0.38, 0.28), (0.46, 0.37), (0.9, 0.37), (0.9, 0.82), (0.1, 0.82)], c);
        }
        Save => {
            p.closed(&[(0.16, 0.14), (0.72, 0.14), (0.86, 0.28), (0.86, 0.86), (0.16, 0.86)], c);
            p.poly(&[(0.3, 0.14), (0.66, 0.14), (0.66, 0.36), (0.3, 0.36)], faded(a, 200), None);
            p.closed(&[(0.3, 0.56), (0.72, 0.56), (0.72, 0.86), (0.3, 0.86)], c);
        }
        Undo => p.arrow(&Pen::arc_pts((0.52, 0.6), 0.3, -50.0, 180.0), c),
        Redo => p.arrow(&Pen::arc_pts((0.48, 0.6), 0.3, 230.0, 0.0), c),

        Sketch => {
            p.closed(&[(0.12, 0.38), (0.66, 0.38), (0.66, 0.88), (0.12, 0.88)], c);
            p.thick(&[(0.42, 0.62), (0.86, 0.16)], a, 2.2);
            p.line(&[(0.36, 0.7), (0.42, 0.62)], a);
        }
        Extrude => {
            let base = [(0.1, 0.8), (0.56, 0.8), (0.86, 0.64), (0.4, 0.64)];
            let top: Vec<(f32, f32)> = base.iter().map(|(x, y)| (*x, y - 0.28)).collect();
            p.poly(&top, faded(a, 80), Some(c));
            p.closed(&base, c);
            for (b, t) in base.iter().zip(&top) {
                p.line(&[*b, *t], c);
            }
            p.arrow(&[(0.48, 0.44), (0.48, 0.04)], a);
        }
        Revolve => {
            let ell: Vec<(f32, f32)> = (0..=32)
                .map(|i| {
                    let t = i as f32 / 32.0 * 2.0 * PI;
                    (0.5 + 0.38 * t.cos(), 0.66 + 0.15 * t.sin())
                })
                .collect();
            p.line(&ell, c);
            p.poly(&[(0.56, 0.2), (0.78, 0.2), (0.78, 0.6), (0.56, 0.6)], faded(a, 90), Some(a));
            p.line(&[(0.5, 0.06), (0.5, 0.95)], c);
            p.arrow(&[(0.2, 0.74), (0.13, 0.67)], a);
        }
        Fillet => {
            p.line(&[(0.15, 0.9), (0.15, 0.55)], c);
            p.line(&[(0.45, 0.25), (0.9, 0.25)], c);
            p.thick(&Pen::arc_pts((0.45, 0.55), 0.3, 180.0, 90.0), a, 1.6);
            p.line(&[(0.15, 0.9), (0.9, 0.9), (0.9, 0.25)], faded(c, 110));
        }
        Chamfer => {
            p.line(&[(0.15, 0.9), (0.15, 0.55)], c);
            p.line(&[(0.45, 0.25), (0.9, 0.25)], c);
            p.thick(&[(0.15, 0.55), (0.45, 0.25)], a, 1.6);
            p.line(&[(0.15, 0.9), (0.9, 0.9), (0.9, 0.25)], faded(c, 110));
        }
        Parameters => {
            p.text((0.42, 0.5), "ƒ", 0.78, c);
            p.text((0.72, 0.6), "x", 0.5, a);
        }

        PressPull => {
            p.poly(&[(0.12, 0.62), (0.58, 0.62), (0.88, 0.46), (0.42, 0.46)], faded(a, 140), Some(c));
            p.closed(&[(0.12, 0.62), (0.12, 0.88), (0.58, 0.88), (0.58, 0.62)], c);
            p.line(&[(0.58, 0.88), (0.88, 0.72), (0.88, 0.46)], c);
            p.arrow(&[(0.5, 0.5), (0.5, 0.08)], a);
        }
        Shell => {
            p.closed(&[(0.12, 0.2), (0.12, 0.86), (0.88, 0.86), (0.88, 0.2)], c);
            p.poly(&[(0.12, 0.2), (0.24, 0.2), (0.24, 0.74), (0.12, 0.74)], faded(a, 160), None);
            p.poly(&[(0.76, 0.2), (0.88, 0.2), (0.88, 0.74), (0.76, 0.74)], faded(a, 160), None);
            p.poly(&[(0.12, 0.74), (0.88, 0.74), (0.88, 0.86), (0.12, 0.86)], faded(a, 160), None);
        }
        Draft => {
            p.poly(&[(0.3, 0.18), (0.7, 0.18), (0.86, 0.86), (0.14, 0.86)], faded(a, 120), Some(c));
            p.line(&[(0.3, 0.18), (0.3, 0.86)], faded(c, 110));
            p.line(&[(0.06, 0.86), (0.94, 0.86)], theme::ORANGE);
            p.arrow(&[(0.5, 0.7), (0.5, 0.04)], a);
        }
        Midplane => {
            p.line(&[(0.08, 0.88), (0.6, 0.88), (0.92, 0.6)], c);
            p.line(&[(0.08, 0.36), (0.6, 0.36), (0.92, 0.08)], c);
            p.poly(&[(0.08, 0.62), (0.6, 0.62), (0.92, 0.34), (0.4, 0.34)], faded(a, 110), Some(a));
        }
        Hole => {
            p.poly(&[(0.1, 0.4), (0.6, 0.4), (0.9, 0.24), (0.4, 0.24)], faded(c, 60), Some(c));
            p.line(&[(0.1, 0.4), (0.1, 0.8), (0.6, 0.8), (0.9, 0.64), (0.9, 0.24)], c);
            p.line(&[(0.6, 0.4), (0.6, 0.8)], c);
            let ell: Vec<(f32, f32)> =
                (0..=24).map(|i| { let t = i as f32 / 24.0 * 2.0 * PI; (0.5 + 0.13 * t.cos(), 0.32 + 0.05 * t.sin()) }).collect();
            p.thick(&ell, a, 1.3);
            p.line(&[(0.37, 0.32), (0.37, 0.62)], faded(a, 160));
            p.line(&[(0.63, 0.32), (0.63, 0.62)], faded(a, 160));
        }
        Mirror => {
            p.poly(&[(0.12, 0.3), (0.4, 0.2), (0.4, 0.8), (0.12, 0.7)], faded(c, 80), Some(c));
            p.poly(&[(0.88, 0.3), (0.6, 0.2), (0.6, 0.8), (0.88, 0.7)], faded(a, 110), Some(a));
            p.line(&[(0.5, 0.06), (0.5, 0.94)], theme::ORANGE);
        }
        RectPattern => {
            for (x, y) in [(0.12, 0.12), (0.56, 0.12), (0.12, 0.56), (0.56, 0.56)] {
                let col = if (x, y) == (0.12, 0.12) { c } else { a };
                p.poly(&[(x, y), (x + 0.32, y), (x + 0.32, y + 0.32), (x, y + 0.32)], faded(col, 120), Some(col));
            }
        }
        CircPattern => {
            p.circle((0.5, 0.5), 0.3, faded(c, 120));
            for k in 0..6 {
                let t = k as f32 / 6.0 * 2.0 * PI;
                p.dot((0.5 + 0.3 * t.cos(), 0.5 + 0.3 * t.sin()), 0.09, if k == 0 { c } else { a });
            }
        }
        Combine => {
            p.circle((0.38, 0.5), 0.26, c);
            p.circle((0.62, 0.5), 0.26, a);
        }
        Split => {
            p.closed(&[(0.12, 0.2), (0.88, 0.2), (0.88, 0.8), (0.12, 0.8)], c);
            p.thick(&[(0.3, 0.06), (0.7, 0.94)], theme::ORANGE, 1.4);
        }
        Move => {
            p.arrow(&[(0.5, 0.5), (0.5, 0.08)], a);
            p.arrow(&[(0.5, 0.5), (0.5, 0.92)], a);
            p.arrow(&[(0.5, 0.5), (0.08, 0.5)], a);
            p.arrow(&[(0.5, 0.5), (0.92, 0.5)], a);
        }
        Loft => {
            p.closed(&[(0.12, 0.72), (0.62, 0.72), (0.88, 0.86), (0.38, 0.86)], c);
            let top: Vec<(f32, f32)> =
                (0..=24).map(|i| { let t = i as f32 / 24.0 * 2.0 * PI; (0.5 + 0.18 * t.cos(), 0.2 + 0.07 * t.sin()) }).collect();
            p.line(&top, a);
            p.line(&[(0.12, 0.72), (0.32, 0.2)], faded(a, 170));
            p.line(&[(0.88, 0.86), (0.68, 0.2)], faded(a, 170));
        }
        Sweep => {
            p.line(&Pen::arc_pts((0.15, 0.15), 0.6, -90.0, 0.0), theme::ORANGE);
            p.circle((0.15, 0.75), 0.12, c);
            p.thick(&Pen::arc_pts((0.15, 0.15), 0.72, -90.0, 0.0), faded(a, 160), 0.8);
            p.thick(&Pen::arc_pts((0.15, 0.15), 0.48, -90.0, 0.0), faded(a, 160), 0.8);
        }
        Axis => {
            for i in 0..5 {
                let t0 = 0.08 + i as f32 * 0.18;
                p.line(&[(t0, 1.0 - t0), (t0 + 0.1, 0.9 - t0)], a);
            }
            p.dot((0.5, 0.5), 0.07, c);
        }
        Measure => {
            p.poly(&[(0.08, 0.62), (0.62, 0.08), (0.92, 0.38), (0.38, 0.92)], faded(a, 90), Some(c));
            for k in 1..5 {
                let t = k as f32 * 0.11;
                p.line(&[(0.08 + t + 0.0, 0.62 - t), (0.16 + t, 0.70 - t)], c);
            }
        }
        Section => {
            p.poly(&[(0.12, 0.3), (0.5, 0.12), (0.5, 0.88), (0.12, 0.7)], faded(c, 70), Some(c));
            p.poly(&[(0.5, 0.12), (0.88, 0.3), (0.88, 0.7), (0.5, 0.88)], faded(theme::ORANGE, 150), Some(theme::ORANGE));
        }
        Properties => {
            p.circle((0.5, 0.5), 0.38, c);
            p.text((0.5, 0.52), "i", 0.55, a);
        }

        Joint => {
            p.poly(&[(0.08, 0.55), (0.45, 0.55), (0.45, 0.9), (0.08, 0.9)], faded(c, 90), Some(c));
            p.poly(&[(0.55, 0.1), (0.92, 0.1), (0.92, 0.45), (0.55, 0.45)], faded(a, 110), Some(a));
            p.circle((0.5, 0.5), 0.12, theme::ORANGE);
            p.dot((0.5, 0.5), 0.05, theme::ORANGE);
        }
        Interference => {
            p.circle((0.38, 0.5), 0.26, c);
            p.circle((0.62, 0.5), 0.26, c);
            p.poly(&[(0.5, 0.3), (0.58, 0.5), (0.5, 0.7), (0.42, 0.5)], theme::ERROR, None);
        }

        Select => {
            let pts = [(0.28, 0.1), (0.28, 0.8), (0.44, 0.65), (0.56, 0.9), (0.65, 0.86), (0.54, 0.61), (0.76, 0.61)];
            p.closed(&pts, c);
            p.poly(&[(0.28, 0.1), (0.28, 0.8), (0.44, 0.65), (0.54, 0.61), (0.76, 0.61)], faded(c, 70), None);
        }
        Line => {
            p.line(&[(0.18, 0.82), (0.82, 0.18)], c);
            p.dot((0.18, 0.82), 0.08, a);
            p.dot((0.82, 0.18), 0.08, a);
        }
        Rectangle => {
            p.closed(&[(0.14, 0.26), (0.86, 0.26), (0.86, 0.76), (0.14, 0.76)], c);
            p.dot((0.14, 0.76), 0.08, a);
            p.dot((0.86, 0.26), 0.08, a);
        }
        Circle => {
            p.circle((0.5, 0.5), 0.36, c);
            p.line(&[(0.5, 0.5), (0.75, 0.25)], a);
            p.dot((0.5, 0.5), 0.07, a);
        }
        Arc => {
            p.arc((0.5, 0.72), 0.38, 15.0, 165.0, c);
            p.dot((0.5, 0.72), 0.06, a);
            p.dot((0.87, 0.62), 0.07, a);
            p.dot((0.13, 0.62), 0.07, a);
        }
        Dimension => {
            p.line(&[(0.14, 0.3), (0.14, 0.86)], c);
            p.line(&[(0.86, 0.3), (0.86, 0.86)], c);
            p.arrow(&[(0.5, 0.5), (0.15, 0.5)], a);
            p.arrow(&[(0.5, 0.5), (0.85, 0.5)], a);
            p.text((0.5, 0.25), "12", 0.32, c);
        }
        Construction => {
            for i in 0..4 {
                let t0 = 0.12 + i as f32 * 0.2;
                let t1 = t0 + 0.12;
                p.line(&[(t0, 1.0 - t0), (t1, 1.0 - t1)], theme::ORANGE);
            }
        }
        Delete => {
            p.line(&[(0.18, 0.24), (0.82, 0.24)], c);
            p.line(&[(0.4, 0.24), (0.4, 0.12), (0.6, 0.12), (0.6, 0.24)], c);
            p.closed(&[(0.26, 0.24), (0.32, 0.9), (0.68, 0.9), (0.74, 0.24)], c);
            p.line(&[(0.43, 0.38), (0.45, 0.78)], a);
            p.line(&[(0.57, 0.38), (0.55, 0.78)], a);
        }
        Trim => {
            p.line(&[(0.1, 0.5), (0.4, 0.5)], c);
            p.line(&[(0.62, 0.5), (0.9, 0.5)], faded(c, 70));
            p.line(&[(0.4, 0.15), (0.4, 0.85)], c);
            p.thick(&[(0.56, 0.36), (0.8, 0.64)], a, 1.3);
            p.thick(&[(0.8, 0.36), (0.56, 0.64)], a, 1.3);
        }
        Extend => {
            p.line(&[(0.1, 0.5), (0.45, 0.5)], c);
            p.arrow(&[(0.45, 0.5), (0.78, 0.5)], a);
            p.line(&[(0.86, 0.15), (0.86, 0.85)], c);
        }
        Offset => {
            p.line(&[(0.15, 0.85), (0.15, 0.4), (0.6, 0.4)], c);
            p.line(&[(0.35, 0.85), (0.35, 0.6), (0.6, 0.6)], a);
            p.arc((0.6, 0.15), 0.25, 270.0, 360.0, c);
        }
        Polygon => {
            let pts: Vec<(f32, f32)> =
                (0..6).map(|k| { let t = k as f32 / 6.0 * 2.0 * PI; (0.5 + 0.38 * t.cos(), 0.5 + 0.38 * t.sin()) }).collect();
            p.closed(&pts, c);
            p.dot((0.5, 0.5), 0.06, a);
            for q in pts {
                p.dot(q, 0.05, a);
            }
        }
        Slot => {
            p.line(&[(0.3, 0.32), (0.7, 0.32)], c);
            p.line(&[(0.3, 0.68), (0.7, 0.68)], c);
            p.arc((0.3, 0.5), 0.18, 90.0, 270.0, c);
            p.arc((0.7, 0.5), 0.18, -90.0, 90.0, c);
            p.dot((0.3, 0.5), 0.05, a);
            p.dot((0.7, 0.5), 0.05, a);
        }
        CenterRect => {
            p.closed(&[(0.14, 0.26), (0.86, 0.26), (0.86, 0.76), (0.14, 0.76)], c);
            p.dot((0.5, 0.51), 0.07, a);
            p.dot((0.86, 0.26), 0.07, a);
        }
        Arc3 => {
            p.arc((0.5, 0.72), 0.38, 15.0, 165.0, c);
            p.dot((0.87, 0.62), 0.07, a);
            p.dot((0.13, 0.62), 0.07, a);
            p.dot((0.5, 0.34), 0.07, a);
        }
        Spline => {
            let pts: Vec<(f32, f32)> =
                (0..=24).map(|i| { let t = i as f32 / 24.0; (0.1 + 0.8 * t, 0.5 - 0.3 * (t * 2.0 * PI).sin()) }).collect();
            p.line(&pts, c);
            for q in [(0.1, 0.5), (0.3, 0.2), (0.7, 0.8), (0.9, 0.5)] {
                p.dot(q, 0.06, a);
            }
        }
        Project => {
            p.poly(&[(0.1, 0.8), (0.62, 0.8), (0.9, 0.6), (0.38, 0.6)], faded(theme::ORANGE, 90), Some(theme::ORANGE));
            p.closed(&[(0.3, 0.15), (0.6, 0.15), (0.6, 0.4), (0.3, 0.4)], c);
            p.arrow(&[(0.45, 0.4), (0.45, 0.68)], a);
        }
        Finish => p.thick(&[(0.16, 0.52), (0.4, 0.76), (0.86, 0.24)], a, 1.8),
        Cancel => {
            p.thick(&[(0.22, 0.22), (0.78, 0.78)], a, 1.5);
            p.thick(&[(0.78, 0.22), (0.22, 0.78)], a, 1.5);
        }

        Horizontal => {
            p.thick(&[(0.14, 0.55), (0.86, 0.55)], a, 1.5);
            p.dot((0.14, 0.55), 0.07, c);
            p.dot((0.86, 0.55), 0.07, c);
            p.text((0.5, 0.26), "H", 0.36, c);
        }
        Vertical => {
            p.thick(&[(0.45, 0.14), (0.45, 0.86)], a, 1.5);
            p.dot((0.45, 0.14), 0.07, c);
            p.dot((0.45, 0.86), 0.07, c);
            p.text((0.76, 0.5), "V", 0.36, c);
        }
        Coincident => {
            p.line(&[(0.12, 0.85), (0.5, 0.42)], c);
            p.line(&[(0.88, 0.82), (0.5, 0.42)], c);
            p.dot((0.5, 0.42), 0.12, a);
        }
        Parallel => {
            p.thick(&[(0.18, 0.86), (0.5, 0.14)], a, 1.4);
            p.thick(&[(0.5, 0.86), (0.82, 0.14)], a, 1.4);
        }
        Perpendicular => {
            p.thick(&[(0.14, 0.84), (0.86, 0.84)], c, 1.4);
            p.thick(&[(0.5, 0.84), (0.5, 0.14)], a, 1.4);
            p.line(&[(0.5, 0.66), (0.66, 0.66), (0.66, 0.84)], c);
        }
        Equal => {
            p.thick(&[(0.18, 0.38), (0.82, 0.38)], a, 1.5);
            p.thick(&[(0.18, 0.62), (0.82, 0.62)], a, 1.5);
        }
        Tangent => {
            p.circle((0.5, 0.6), 0.3, c);
            p.thick(&[(0.08, 0.3), (0.92, 0.3)], a, 1.4);
        }
        Concentric => {
            p.circle((0.5, 0.5), 0.38, c);
            p.circle((0.5, 0.5), 0.2, a);
            p.dot((0.5, 0.5), 0.05, c);
        }
        Midpoint => {
            p.line(&[(0.1, 0.72), (0.9, 0.28)], c);
            p.poly(&[(0.5, 0.36), (0.62, 0.58), (0.38, 0.58)], a, None);
        }
        Fix => {
            p.arc((0.5, 0.46), 0.18, 0.0, 180.0, c);
            p.line(&[(0.32, 0.46), (0.32, 0.5)], c);
            p.line(&[(0.68, 0.46), (0.68, 0.5)], c);
            p.poly(&[(0.22, 0.5), (0.78, 0.5), (0.78, 0.88), (0.22, 0.88)], a, None);
        }

        Home => {
            p.line(&[(0.1, 0.52), (0.5, 0.14), (0.9, 0.52)], c);
            p.line(&[(0.22, 0.42), (0.22, 0.86), (0.78, 0.86), (0.78, 0.42)], c);
            p.poly(&[(0.42, 0.6), (0.58, 0.6), (0.58, 0.86), (0.42, 0.86)], a, None);
        }
        Fit => {
            for (x, y, dx, dy) in [(0.12, 0.12, 1.0, 1.0), (0.88, 0.12, -1.0, 1.0), (0.12, 0.88, 1.0, -1.0), (0.88, 0.88, -1.0, -1.0)] {
                p.line(&[(x + 0.22 * dx, y), (x, y), (x, y + 0.22 * dy)], c);
            }
            p.poly(&[(0.36, 0.36), (0.64, 0.36), (0.64, 0.64), (0.36, 0.64)], faded(a, 160), None);
        }
        Grid => {
            for t in [0.15, 0.38, 0.62, 0.85] {
                p.line(&[(t, 0.12), (t, 0.88)], c);
                p.line(&[(0.12, t), (0.88, t)], c);
            }
        }
        Edges => {
            let f = [(0.14, 0.34), (0.62, 0.34), (0.62, 0.86), (0.14, 0.86)];
            let b: Vec<(f32, f32)> = f.iter().map(|(x, y)| (x + 0.24, y - 0.2)).collect();
            p.poly(&f, faded(c, 40), None);
            p.closed(&f, a);
            p.closed(&b, a);
            for (u, v) in f.iter().zip(&b) {
                p.line(&[*u, *v], a);
            }
        }
        LookAt => {
            p.poly(&[(0.1, 0.62), (0.62, 0.62), (0.9, 0.88), (0.38, 0.88)], faded(theme::ORANGE, 110), Some(theme::ORANGE));
            p.arrow(&[(0.5, 0.06), (0.5, 0.68)], a);
        }

        First => {
            p.thick(&[(0.2, 0.22), (0.2, 0.78)], c, 1.4);
            p.poly(&[(0.82, 0.22), (0.82, 0.78), (0.3, 0.5)], c, None);
        }
        Prev => p.poly(&[(0.72, 0.2), (0.72, 0.8), (0.24, 0.5)], c, None),
        Next => p.poly(&[(0.28, 0.2), (0.28, 0.8), (0.76, 0.5)], c, None),
        Last => {
            p.thick(&[(0.8, 0.22), (0.8, 0.78)], c, 1.4);
            p.poly(&[(0.18, 0.22), (0.18, 0.78), (0.7, 0.5)], c, None);
        }

        Body => {
            let top = [(0.14, 0.3), (0.52, 0.14), (0.88, 0.3), (0.5, 0.46)];
            p.poly(&top, faded(a, 140), Some(c));
            p.poly(&[(0.14, 0.3), (0.5, 0.46), (0.5, 0.88), (0.14, 0.72)], faded(c, 90), Some(c));
            p.poly(&[(0.5, 0.46), (0.88, 0.3), (0.88, 0.72), (0.5, 0.88)], faded(c, 40), Some(c));
        }
        Plane => {
            p.poly(&[(0.08, 0.7), (0.6, 0.7), (0.92, 0.3), (0.4, 0.3)], faded(a, 90), Some(a));
        }
        Origin => {
            let o = (0.34, 0.66);
            p.line(&[o, (0.9, 0.66)], Color32::from_rgb(220, 80, 80));
            p.line(&[o, (0.34, 0.1)], Color32::from_rgb(90, 130, 240));
            p.line(&[o, (0.66, 0.4)], Color32::from_rgb(90, 200, 90));
            p.dot(o, 0.08, c);
        }
        Document => {
            p.closed(&[(0.22, 0.1), (0.62, 0.1), (0.8, 0.28), (0.8, 0.9), (0.22, 0.9)], c);
            for y in [0.42, 0.58, 0.74] {
                p.line(&[(0.34, y), (0.68, y)], faded(c, 140));
            }
        }
        EyeOpen | EyeClosed => {
            let lower: Vec<(f32, f32)> =
                (0..=16).map(|i| (0.08 + 0.84 * i as f32 / 16.0, 0.5 + 0.24 * (PI * i as f32 / 16.0).sin())).collect();
            p.line(&lower, c);
            if icon == EyeOpen {
                let upper: Vec<(f32, f32)> =
                    (0..=16).map(|i| (0.08 + 0.84 * i as f32 / 16.0, 0.5 - 0.24 * (PI * i as f32 / 16.0).sin())).collect();
                p.line(&upper, c);
                p.dot((0.5, 0.5), 0.12, c);
            } else {
                p.line(&[(0.18, 0.82), (0.82, 0.18)], c);
            }
        }
    }
}

/// Square icon-only button (toolbar, timeline, browser).
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, tooltip: &str, enabled: bool, selected: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 4.0, theme::ACCENT_DIM);
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        } else if enabled && response.hovered() {
            painter.rect_filled(rect, 4.0, theme::HOVER);
        }
        let color = if enabled { theme::ICON } else { Color32::from_gray(95) };
        paint(painter, rect.shrink(size * 0.16), icon, color, enabled);
    }
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Large ribbon button: icon above a small label.
pub fn ribbon_button(ui: &mut Ui, icon: Icon, label: &str, tooltip: &str, enabled: bool, selected: bool) -> Response {
    let font = egui::FontId::proportional(11.0);
    let text_color = if enabled { theme::TEXT } else { Color32::from_gray(105) };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text_color);
    let width = (galley.size().x + 12.0).max(50.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 52.0), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 4.0, theme::ACCENT_DIM);
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, theme::ACCENT), egui::StrokeKind::Inside);
        } else if enabled && response.hovered() {
            painter.rect_filled(rect, 4.0, theme::HOVER);
        }
        let icon_rect = Rect::from_center_size(egui::pos2(rect.center().x, rect.min.y + 18.0), egui::vec2(26.0, 26.0));
        let color = if enabled { theme::ICON } else { Color32::from_gray(95) };
        paint(painter, icon_rect, icon, color, enabled);
        let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.max.y - galley.size().y - 3.0);
        painter.galley(text_pos, galley, text_color);
    }
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

/// Label with a small icon in front (browser rows, dialog headers).
pub fn icon_label(ui: &mut Ui, icon: Icon, text: impl Into<egui::WidgetText>) -> Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
        paint(ui.painter(), rect, icon, theme::ICON, true);
        ui.label(text)
    })
    .inner
}
