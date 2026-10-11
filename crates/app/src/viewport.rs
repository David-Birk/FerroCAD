//! The 3D viewport: camera navigation, body rendering and mode dispatch.
//!
//! Rendering is a software painter's algorithm on top of egui's painter
//! (depth-sorted flat-shaded triangles). It is fine for parts with a few
//! ten thousand triangles; the next step is an egui_wgpu paint callback with
//! a real depth buffer.

use crate::app::{FerroApp, ModeKind};
use egui::{Color32, Mesh, Pos2, Stroke};
use ferrocad_model::geom::Vec3;
use ferrocad_view::{shade_mesh, Viewport};

pub const BACKGROUND: Color32 = Color32::from_rgb(38, 41, 46);
const BODY_COLOR: [u8; 3] = [150, 162, 178];
const EDGE_COLOR: Color32 = Color32::from_rgb(20, 22, 26);

enum Prim {
    Tri([Pos2; 3], Color32, f64),
    Seg([Pos2; 2], f64),
}

impl Prim {
    fn depth(&self) -> f64 {
        match self {
            Prim::Tri(_, _, d) | Prim::Seg(_, d) => *d,
        }
    }
}

impl FerroApp {
    pub fn to_screen(&self, p: Vec3, vp: &Viewport) -> Option<Pos2> {
        self.camera.project(p, vp).map(|s| egui::pos2(s[0] as f32, s[1] as f32))
    }

    pub fn viewport(&mut self, ui: &mut egui::Ui) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0.0_f32, BACKGROUND);
        let vp = Viewport {
            x: rect.min.x as f64,
            y: rect.min.y as f64,
            width: rect.width().max(1.0) as f64,
            height: rect.height().max(1.0) as f64,
        };

        // ---- navigation (Fusion-like: middle = pan, shift+middle/right = orbit)
        let shift = ui.input(|i| i.modifiers.shift);
        let delta = response.drag_delta();
        if response.dragged_by(egui::PointerButton::Secondary)
            || (shift && response.dragged_by(egui::PointerButton::Middle))
        {
            self.camera.orbit(delta.x as f64, delta.y as f64);
        } else if response.dragged_by(egui::PointerButton::Middle) {
            self.camera.pan(delta.x as f64, delta.y as f64, &vp);
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y) as f64;
            if scroll != 0.0 {
                let factor = (-scroll * 0.002).exp();
                self.camera.zoom_at(factor, [hover.x as f64, hover.y as f64], &vp);
            }
        }

        self.draw_grid(&painter, &vp);
        self.draw_bodies(&painter, &vp);

        // ---- mode-specific interaction and overlays
        match self.mode_kind() {
            ModeKind::Model => {}
            ModeKind::Sketch => self.sketch_viewport(ui, &response, &painter, &vp),
            ModeKind::Extrude => self.extrude_viewport(&response, &painter, &vp),
            ModeKind::Revolve => self.revolve_viewport(&response, &painter, &vp),
            ModeKind::Fillet => self.fillet_viewport(&response, &painter, &vp),
        }

        self.draw_axis_triad(&painter, rect);
    }

    fn draw_grid(&self, painter: &egui::Painter, vp: &Viewport) {
        let step = 10f64.powf((self.camera.distance / 8.0).log10().floor()).max(1e-3);
        let n = 20;
        let extent = step * n as f64;
        let minor = Stroke::new(1.0, Color32::from_rgb(50, 54, 60));
        let major = Stroke::new(1.0, Color32::from_rgb(62, 67, 74));
        // Centre the grid near the camera target so it never runs out.
        let cx = (self.camera.target[0] / step).round() * step;
        let cy = (self.camera.target[1] / step).round() * step;
        for i in -n..=n {
            let t = i as f64 * step;
            let stroke = if i % 5 == 0 { major } else { minor };
            let lines = [
                ([cx + t, cy - extent, 0.0], [cx + t, cy + extent, 0.0]),
                ([cx - extent, cy + t, 0.0], [cx + extent, cy + t, 0.0]),
            ];
            for (a, b) in lines {
                if let (Some(a), Some(b)) = (self.to_screen(a, vp), self.to_screen(b, vp)) {
                    painter.line_segment([a, b], stroke);
                }
            }
        }
        // world axes through the origin
        let len = step * 5.0;
        let o = [0.0; 3];
        for (dir, color) in [
            ([len, 0.0, 0.0], Color32::from_rgb(220, 80, 80)),
            ([0.0, len, 0.0], Color32::from_rgb(90, 200, 90)),
            ([0.0, 0.0, len], Color32::from_rgb(90, 130, 240)),
        ] {
            if let (Some(a), Some(b)) = (self.to_screen(o, vp), self.to_screen(dir, vp)) {
                painter.line_segment([a, b], Stroke::new(1.5, color));
            }
        }
    }

    fn draw_bodies(&self, painter: &egui::Painter, vp: &Viewport) {
        let mut prims: Vec<Prim> = Vec::new();
        // Pull edges slightly towards the camera so they win against the
        // faces they bound but still hide behind faces in front of them.
        let bias = self.camera.distance * 0.004;
        for body in self.bodies.iter().filter(|b| b.visible) {
            for t in shade_mesh(&body.mesh, &self.camera, vp, BODY_COLOR) {
                let pts = t.points.map(|p| egui::pos2(p[0], p[1]));
                let [r, g, b] = t.color;
                prims.push(Prim::Tri(pts, Color32::from_rgb(r, g, b), t.depth));
            }
            for poly in &body.edges {
                for w in poly.windows(2) {
                    if let (Some(a), Some(b)) =
                        (self.camera.project(w[0], vp), self.camera.project(w[1], vp))
                    {
                        let depth = 0.5 * (a[2] + b[2]) - bias;
                        let pa = egui::pos2(a[0] as f32, a[1] as f32);
                        let pb = egui::pos2(b[0] as f32, b[1] as f32);
                        prims.push(Prim::Seg([pa, pb], depth));
                    }
                }
            }
        }
        prims.sort_by(|a, b| b.depth().total_cmp(&a.depth()));

        let mut mesh = Mesh::default();
        let flush = |mesh: &mut Mesh| {
            if !mesh.indices.is_empty() {
                painter.add(std::mem::take(mesh));
            }
        };
        for p in prims {
            match p {
                Prim::Tri(pts, color, _) => {
                    let base = mesh.vertices.len() as u32;
                    for q in pts {
                        mesh.colored_vertex(q, color);
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                }
                Prim::Seg(pts, _) => {
                    flush(&mut mesh);
                    painter.line_segment(pts, Stroke::new(1.2, EDGE_COLOR));
                }
            }
        }
        flush(&mut mesh);
    }

    /// Small orientation triad in the bottom-left corner.
    fn draw_axis_triad(&self, painter: &egui::Painter, rect: egui::Rect) {
        let (right, up, _) = self.camera.basis();
        let origin = egui::pos2(rect.min.x + 45.0, rect.max.y - 45.0);
        for (axis, color, label) in [
            ([1.0, 0.0, 0.0], Color32::from_rgb(220, 80, 80), "X"),
            ([0.0, 1.0, 0.0], Color32::from_rgb(90, 200, 90), "Y"),
            ([0.0, 0.0, 1.0], Color32::from_rgb(90, 130, 240), "Z"),
        ] {
            let x = ferrocad_model::geom::dot(axis, right) as f32;
            let y = ferrocad_model::geom::dot(axis, up) as f32;
            let tip = origin + egui::vec2(x, -y) * 30.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, color));
            painter.text(tip, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), color);
        }
    }
}
