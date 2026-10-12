//! The 3D viewport: camera navigation, body rendering, grid, origin planes,
//! ViewCube, navigation bar, context menu and mode dispatch.
//!
//! Rendering is a software painter's algorithm on top of egui's painter
//! (depth-sorted flat-shaded triangles). It is fine for parts with a few
//! ten thousand triangles; the next step is an egui_wgpu paint callback with
//! a real depth buffer.

use crate::app::{BodyDisplay, Command, FerroApp, Mode, ModeKind};
use crate::features_ui::{FeatureType, Pick, SelKind};
use crate::icons::{self, Icon};
use crate::sketch_ui::{self, ConstraintKind, SketchCommand, Tool};
use crate::theme;
use egui::{Color32, Mesh, Pos2, Rect, Stroke};
use ferrocad_model::geom::{add, dot, normalize, scale, sub, Vec3};
use ferrocad_model::{FeatureKind, OriginPlane, Plane};
use ferrocad_view::{shade_mesh, Viewport};

const BODY_COLOR: [u8; 3] = [168, 178, 192];
const EDGE_COLOR: Color32 = Color32::from_rgb(24, 26, 30);
const SKETCH_COLOR: Color32 = Color32::from_rgb(120, 160, 210);

/// Grid spacing for the current zoom level (also used for snapping).
pub fn grid_step(distance: f64) -> f64 {
    10f64.powf((distance / 8.0).log10().floor()).max(1e-3)
}

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

fn axis_color(dir: Vec3) -> Color32 {
    let d = normalize(dir);
    if d[0].abs() > 0.9 {
        Color32::from_rgb(220, 80, 80)
    } else if d[1].abs() > 0.9 {
        Color32::from_rgb(90, 200, 90)
    } else if d[2].abs() > 0.9 {
        Color32::from_rgb(90, 130, 240)
    } else {
        Color32::from_gray(160)
    }
}

impl FerroApp {
    pub fn to_screen(&self, p: Vec3, vp: &Viewport) -> Option<Pos2> {
        self.camera.project(p, vp).map(|s| egui::pos2(s[0] as f32, s[1] as f32))
    }

    pub fn viewport(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = response.rect;
        self.viewport_rect = rect;
        paint_background(&painter, rect);
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
            self.stop_camera_animation();
            self.camera.orbit(delta.x as f64, delta.y as f64);
        } else if response.dragged_by(egui::PointerButton::Middle) {
            self.stop_camera_animation();
            self.camera.pan(delta.x as f64, delta.y as f64, &vp);
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y) as f64;
            if scroll != 0.0 {
                self.stop_camera_animation();
                let factor = (-scroll * 0.002).exp();
                self.camera.zoom_at(factor, [hover.x as f64, hover.y as f64], &vp);
            }
        }
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        if self.step_camera(dt) {
            ui.ctx().request_repaint();
        }

        let sketch_plane = match &self.mode {
            Mode::Sketch(s) => self.doc.sketch(s.feature).map(|sf| sf.plane),
            _ => None,
        };
        if self.show_grid {
            self.draw_grid(&painter, &vp, sketch_plane.unwrap_or(Plane::XY));
        }
        self.draw_bodies(&painter, &vp);
        self.draw_sketches(&painter, &vp);
        self.draw_construction(&painter, &vp);
        self.draw_interference(&painter, &vp);

        // ---- mode-specific interaction and overlays
        match self.mode_kind() {
            ModeKind::Model => {
                if self.show_origin {
                    for o in OriginPlane::ALL {
                        let fill = theme::ORANGE.gamma_multiply(0.12);
                        self.draw_plane(&painter, &vp, &o.plane(), false, fill, Stroke::new(1.0, theme::ORANGE));
                    }
                }
                self.model_selection(&response, &painter, &vp);
            }
            ModeKind::PickPlane => self.pick_sketch_plane(&response, &painter, &vp),
            ModeKind::Feature => self.feature_viewport(&response, &painter, &vp),
            ModeKind::Measure => self.measure_viewport(&response, &painter, &vp),
            ModeKind::Sketch => self.sketch_viewport(ui, &response, &painter, &vp),
            ModeKind::Extrude => self.extrude_viewport(&response, &painter, &vp),
            ModeKind::Revolve => self.revolve_viewport(&response, &painter, &vp),
            ModeKind::Fillet => self.fillet_viewport(&response, &painter, &vp),
        }

        self.manipulators(ui, &painter, &vp);
        self.view_cube(ui, &painter, rect);
        self.nav_bar(ui, rect);
        self.context_menu(&response);
    }

    fn draw_grid(&self, painter: &egui::Painter, vp: &Viewport, plane: Plane) {
        let step = grid_step(self.camera.distance);
        let n = 25;
        let extent = step * n as f64;
        let minor = Stroke::new(1.0, Color32::from_white_alpha(10));
        let major = Stroke::new(1.0, Color32::from_white_alpha(26));
        // Centre the grid near the camera target so it never runs out.
        let c = plane.to_local(self.camera.target);
        let (cu, cv) = ((c[0] / step).round() * step, (c[1] / step).round() * step);
        for i in -n..=n {
            let t = i as f64 * step;
            let k = ((cu + t) / step).round() as i64;
            let l = ((cv + t) / step).round() as i64;
            let lines = [
                ([cu + t, cv - extent], [cu + t, cv + extent], k),
                ([cu - extent, cv + t], [cu + extent, cv + t], l),
            ];
            for (a, b, index) in lines {
                let stroke = if index % 5 == 0 { major } else { minor };
                if let (Some(a), Some(b)) =
                    (self.to_screen(plane.to_world(a), vp), self.to_screen(plane.to_world(b), vp))
                {
                    painter.line_segment([a, b], stroke);
                }
            }
        }
        // Axes of the plane through its origin.
        for (dir, uv) in [(plane.x_axis, [1.0, 0.0]), (plane.y_axis, [0.0, 1.0])] {
            let a = plane.to_world([-extent * uv[0], -extent * uv[1]]);
            let b = plane.to_world([extent * uv[0], extent * uv[1]]);
            if let (Some(a), Some(b)) = (self.to_screen(a, vp), self.to_screen(b, vp)) {
                let c = axis_color(dir);
                painter.line_segment([a, b], Stroke::new(1.2, c.gamma_multiply(0.7)));
            }
        }
    }

    fn draw_bodies(&mut self, painter: &egui::Painter, vp: &Viewport) {
        if self.gpu {
            self.draw_bodies_gpu(painter, vp);
            return;
        }
        let bodies: &[BodyDisplay] = match &self.preview {
            Some(p) => &p.bodies,
            None => &self.bodies,
        };
        let mut prims: Vec<Prim> = Vec::new();
        // Pull edges slightly towards the camera so they win against the
        // faces they bound but still hide behind faces in front of them.
        let bias = self.camera.distance * 0.004;
        let section = self.section.map(|s| s.plane());
        let mut cut_outline: Vec<[Vec3; 2]> = Vec::new();
        for body in bodies.iter().filter(|b| b.visible) {
            let clipped;
            let mesh = match section {
                Some((o, n)) => {
                    let (m, outline) = ferrocad_view::clip_mesh(&body.mesh, o, n);
                    cut_outline.extend(outline);
                    clipped = m;
                    &clipped
                }
                None => &body.mesh,
            };
            let kept = |p: Vec3| section.is_none_or(|(o, n)| dot(sub(p, o), n) <= 1e-9);
            for t in shade_mesh(mesh, &self.camera, vp, self.body_color(&body.name)) {
                let pts = t.points.map(|p| egui::pos2(p[0], p[1]));
                let [r, g, b] = t.color;
                prims.push(Prim::Tri(pts, Color32::from_rgb(r, g, b), t.depth));
            }
            if !self.show_edges {
                continue;
            }
            for poly in &body.edges {
                for w in poly.windows(2) {
                    if !(kept(w[0]) && kept(w[1])) {
                        continue;
                    }
                    if let (Some(a), Some(b)) = (self.camera.project(w[0], vp), self.camera.project(w[1], vp)) {
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
        for [a, b] in cut_outline {
            if let (Some(a), Some(b)) = (self.to_screen(a, vp), self.to_screen(b, vp)) {
                painter.line_segment([a, b], Stroke::new(2.0, theme::ORANGE));
            }
        }
    }

    /// Display colour of a body from its appearance.
    pub fn body_color(&self, name: &str) -> [u8; 3] {
        self.doc
            .view
            .appearances
            .iter()
            .find(|(b, _)| b == name)
            .map(|(_, m)| theme::material_color(m))
            .unwrap_or(BODY_COLOR)
    }

    /// Bodies through the wgpu pipeline; geometry is uploaded only when it
    /// changes.
    fn draw_bodies_gpu(&mut self, painter: &egui::Painter, vp: &Viewport) {
        let bodies: &[BodyDisplay] = match &self.preview {
            Some(p) => &p.bodies,
            None => &self.bodies,
        };
        let visible: String = bodies.iter().map(|b| if b.visible { '1' } else { '0' }).collect();
        let key = format!(
            "{}|{}|{}|{}|{:?}",
            self.scene_rev,
            self.preview.as_ref().map(|p| p.key.as_str()).unwrap_or(""),
            visible,
            self.show_edges,
            self.doc.view.appearances
        );
        if self.scene_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let id = self.scene_cache.as_ref().map(|(_, d)| d.id + 1).unwrap_or(0);
            let data = crate::render3d::SceneData::build(id, bodies.iter(), self.show_edges, &|n| self.body_color(n));
            self.scene_cache = Some((key, std::sync::Arc::new(data)));
        }
        let Some((_, data)) = &self.scene_cache else { return };
        let bodies: &[BodyDisplay] = match &self.preview {
            Some(p) => &p.bodies,
            None => &self.bodies,
        };
        let radius = ferrocad_view::bounds(bodies.iter().filter(|b| b.visible).map(|b| &b.mesh))
            .map(|(min, max)| {
                let corners = [min, max];
                let mut r: f64 = 0.0;
                for i in 0..8 {
                    let c = [corners[i & 1][0], corners[(i >> 1) & 1][1], corners[(i >> 2) & 1][2]];
                    r = r.max(ferrocad_model::geom::distance(c, self.camera.target));
                }
                r
            })
            .unwrap_or(100.0);
        let section = self.section.map(|s| s.plane());
        let color = BODY_COLOR.map(|c| c as f32 / 255.0);
        let uniforms = crate::render3d::uniforms(&self.camera, vp, color, section, radius);
        let rect = egui::Rect::from_min_size(
            egui::pos2(vp.x as f32, vp.y as f32),
            egui::vec2(vp.width as f32, vp.height as f32),
        );
        if let Ok(mut v) = self.overlay.lock() {
            v.clear();
        }
        painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            crate::render3d::SceneCallback { data: data.clone(), uniforms, overlay: self.overlay.clone() },
        ));
        // Outline of the cut.
        if let Some((o, n)) = section {
            for b in bodies.iter().filter(|b| b.visible) {
                for [a, c] in ferrocad_view::clip_mesh(&b.mesh, o, n).1 {
                    if let (Some(a), Some(c)) = (self.to_screen(a, vp), self.to_screen(c, vp)) {
                        painter.line_segment([a, c], Stroke::new(2.0, theme::ORANGE));
                    }
                }
            }
        }
    }

    /// Finished, visible sketches before the timeline marker.
    fn draw_sketches(&self, painter: &egui::Painter, vp: &Viewport) {
        let editing = match &self.mode {
            Mode::Sketch(s) => Some(s.feature),
            _ => None,
        };
        for f in &self.doc.features()[..self.doc.marker()] {
            let FeatureKind::Sketch(sf) = &f.kind else { continue };
            if Some(f.id) == editing || self.hidden_sketches.contains(&f.id) || f.suppressed {
                continue;
            }
            for (id, e) in sf.sketch.entities() {
                if sf.sketch.is_construction(id) {
                    continue;
                }
                let pts: Vec<Pos2> = sketch_ui::entity_polyline(&sf.sketch, e)
                    .iter()
                    .filter_map(|uv| self.to_screen(sf.plane.to_world(*uv), vp))
                    .collect();
                painter.add(egui::Shape::line(pts, Stroke::new(1.3, SKETCH_COLOR)));
            }
        }
    }

    /// Create Sketch: pick an origin plane, construction plane or flat face.
    fn pick_sketch_plane(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        for o in OriginPlane::ALL {
            let fill = theme::ORANGE.gamma_multiply(0.16);
            self.draw_plane(painter, vp, &o.plane(), false, fill, Stroke::new(1.0, theme::ORANGE));
            if let Some(c) = self.to_screen(o.plane().to_world([1.0, 1.0].map(|v| v * self.camera.distance * 0.3)), vp) {
                painter.text(c, egui::Align2::CENTER_CENTER, o.label(), egui::FontId::proportional(12.0), theme::ORANGE);
            }
        }
        let Some(pos) = response.hover_pos() else { return };
        let hovered = self.hover_pick(SelKind::Plane, pos, vp);
        match &hovered {
            Some(p) => self.draw_pick(painter, vp, p, theme::ACCENT),
            None => {
                if self.pick_face(pos, vp).is_some_and(|f| !f.planar) {
                    painter.text(
                        pos + egui::vec2(14.0, 14.0),
                        egui::Align2::LEFT_TOP,
                        "only flat faces can be sketched on",
                        egui::FontId::proportional(12.0),
                        theme::ERROR,
                    );
                }
            }
        }
        if response.clicked() {
            if let Some(Pick::Plane(r)) = hovered {
                self.start_sketch_on(r);
            }
        }
    }

    /// Model mode: hover highlight and click selection of edges and faces.
    fn model_selection(&mut self, response: &egui::Response, painter: &egui::Painter, vp: &Viewport) {
        let hovered = response.hover_pos().and_then(|pos| {
            self.pick_edge(pos, vp)
                .map(|(e, _)| Pick::Edge(e))
                .or_else(|| self.pick_face(pos, vp).map(|f| Pick::Face(f.face)))
        });
        for p in &self.selection {
            self.draw_pick(painter, vp, p, crate::sketch_ui::COL_SELECTED);
        }
        if let Some(p) = &hovered {
            if !self.selection.contains(p) {
                self.draw_pick(painter, vp, p, theme::ACCENT.gamma_multiply(0.8));
            }
        }
        let additive = response.ctx.input(|i| i.modifiers.shift || i.modifiers.command);
        if response.clicked() {
            match hovered.clone() {
                Some(p) if additive => {
                    if let Some(i) = self.selection.iter().position(|q| *q == p) {
                        self.selection.remove(i);
                    } else {
                        self.selection.push(p);
                    }
                }
                Some(p) => self.selection = vec![p],
                None if !additive => self.selection.clear(),
                None => {}
            }
        }
        // Right-click on something unselected selects it, like Fusion.
        if response.secondary_clicked() {
            if let Some(p) = hovered {
                if !self.selection.contains(&p) {
                    self.selection = vec![p];
                }
            }
        }
    }

    /// Overlaps found by the interference check, in translucent red.
    fn draw_interference(&self, painter: &egui::Painter, vp: &Viewport) {
        let red = theme::ERROR.gamma_multiply(0.7);
        for mesh in &self.interference {
            for t in mesh.indices.as_chunks::<3>().0 {
                // Drawn without depth test: overlaps lie inside the bodies.
                let p = [0, 1, 2].map(|k| mesh.positions[t[k] as usize]);
                let pts: Vec<Pos2> = p.iter().filter_map(|q| self.to_screen(*q, vp)).collect();
                if pts.len() == 3 {
                    painter.add(egui::Shape::convex_polygon(pts, red, Stroke::NONE));
                }
            }
        }
    }

    /// Construction planes and axes.
    fn draw_construction(&self, painter: &egui::Painter, vp: &Viewport) {
        for (_, plane) in self.construction_planes() {
            let fill = theme::ORANGE.gamma_multiply(0.12);
            self.draw_plane(painter, vp, &plane, true, fill, Stroke::new(1.0, theme::ORANGE));
        }
        for (_, (o, d)) in self.construction_axes() {
            let reach = self.camera.distance;
            let a = self.to_screen(add(o, scale(normalize(d), -reach)), vp);
            let b = self.to_screen(add(o, scale(normalize(d), reach)), vp);
            if let (Some(a), Some(b)) = (a, b) {
                painter.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.5, theme::ORANGE), 10.0, 4.0));
            }
        }
    }

    // ------------------------------------------------------------ ViewCube

    fn view_cube(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect) {
        let center = egui::pos2(rect.max.x - 78.0, rect.min.y + 78.0);
        let half = 33.0_f32;
        let cube_rect = Rect::from_center_size(center, egui::vec2(120.0, 120.0));
        let resp = ui.interact(cube_rect, ui.id().with("viewcube"), egui::Sense::click());
        let (right, up, forward) = self.camera.basis();
        let to_eye = scale(forward, -1.0);
        let project = |v: Vec3| center + egui::vec2(dot(v, right) as f32, -dot(v, up) as f32) * half;

        // (normal, u, v, label): u/v span the face, as seen from outside.
        let faces: [(Vec3, Vec3, Vec3, &str); 6] = [
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], "TOP"),
            ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], "BOTTOM"),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], "FRONT"),
            ([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], "BACK"),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], "RIGHT"),
            ([-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], "LEFT"),
        ];
        let mut visible: Vec<(f64, usize)> =
            faces.iter().enumerate().map(|(i, f)| (dot(f.0, to_eye), i)).filter(|(d, _)| *d > 0.01).collect();
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Cells: 3×3 per face (centre = face, edges, corners), Fusion-style.
        let bounds = [-1.0, -0.62, 0.62, 1.0];
        let cell_quad = |f: &(Vec3, Vec3, Vec3, &str), i: usize, j: usize| -> [Pos2; 4] {
            let at = |a: f64, b: f64| project(add(f.0, add(scale(f.1, a), scale(f.2, b))));
            let (a0, a1, b0, b1) = (bounds[i], bounds[i + 1], bounds[j], bounds[j + 1]);
            [at(a0, b0), at(a1, b0), at(a1, b1), at(a0, b1)]
        };
        let inside = |q: &[Pos2; 4], p: Pos2| {
            let mut sign = 0.0;
            for k in 0..4 {
                let (a, b) = (q[k], q[(k + 1) % 4]);
                let cross = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
                if sign == 0.0 {
                    sign = cross.signum();
                } else if cross.signum() != sign && cross != 0.0 {
                    return false;
                }
            }
            true
        };
        // Hover: front-most face first.
        let mut hovered: Option<(usize, usize, usize)> = None;
        if let Some(p) = resp.hover_pos() {
            'outer: for &(_, fi) in visible.iter().rev() {
                for i in 0..3 {
                    for j in 0..3 {
                        if inside(&cell_quad(&faces[fi], i, j), p) {
                            hovered = Some((fi, i, j));
                            break 'outer;
                        }
                    }
                }
            }
        }
        for &(facing, fi) in &visible {
            let f = &faces[fi];
            let shade = (150.0 + 80.0 * facing) as u8;
            for i in 0..3 {
                for j in 0..3 {
                    let q = cell_quad(f, i, j);
                    let fill =
                        if hovered == Some((fi, i, j)) { theme::ACCENT } else { Color32::from_rgb(shade, shade, shade + 6) };
                    painter.add(egui::Shape::convex_polygon(q.to_vec(), fill, Stroke::new(0.5, Color32::from_gray(120))));
                }
            }
            let outline: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .iter()
                .map(|(a, b)| project(add(f.0, add(scale(f.1, *a), scale(f.2, *b)))))
                .collect();
            painter.add(egui::Shape::closed_line(outline, Stroke::new(1.0, Color32::from_gray(70))));
            if facing > 0.3 {
                let alpha = ((facing - 0.3) / 0.7 * 255.0) as u8;
                painter.text(
                    project(f.0),
                    egui::Align2::CENTER_CENTER,
                    f.3,
                    egui::FontId::proportional(10.5),
                    Color32::from_rgba_unmultiplied(40, 42, 46, alpha),
                );
            }
        }
        // Small axis triad at the cube's corner.
        let origin = project([-1.25, -1.25, -1.25]);
        for (axis, label) in [([1.0, 0.0, 0.0], "X"), ([0.0, 1.0, 0.0], "Y"), ([0.0, 0.0, 1.0], "Z")] {
            let tip = origin + egui::vec2(dot(axis, right) as f32, -dot(axis, up) as f32) * 26.0;
            painter.line_segment([origin, tip], Stroke::new(2.0, axis_color(axis)));
            painter.text(tip, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), axis_color(axis));
        }

        if resp.clicked() {
            if let Some((fi, i, j)) = hovered {
                let f = &faces[fi];
                let a = i as f64 - 1.0;
                let b = j as f64 - 1.0;
                let dir = add(f.0, add(scale(f.1, a), scale(f.2, b)));
                self.look_along(dir);
            }
        }

        // Home button next to the cube.
        let home_rect = Rect::from_min_size(cube_rect.min + egui::vec2(-6.0, -2.0), egui::vec2(22.0, 22.0));
        let home = ui.interact(home_rect, ui.id().with("home"), egui::Sense::click());
        if home.hovered() {
            painter.rect_filled(home_rect, 4.0, theme::HOVER);
        }
        icons::paint(painter, home_rect.shrink(3.0), Icon::Home, theme::ICON, true);
        if home.on_hover_text("Home view (F6)").clicked() {
            self.run(Command::Home);
        }
    }

    // ------------------------------------------------------- navigation bar

    fn nav_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let sketching = matches!(self.mode, Mode::Sketch(_));
        let mut cmd = None;
        egui::Area::new(ui.id().with("nav_bar"))
            .fixed_pos(egui::pos2(rect.center().x, rect.max.y - 10.0))
            .pivot(egui::Align2::CENTER_BOTTOM)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::WINDOW.gamma_multiply(0.95))
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let items = [
                                (Command::Home, Icon::Home, "Home view (F6)", true, false),
                                (Command::Fit, Icon::Fit, "Fit (Shift+F)", true, false),
                                (Command::LookAt, Icon::LookAt, "Look at sketch plane", sketching, false),
                                (Command::ToggleGrid, Icon::Grid, "Show grid", true, self.show_grid),
                                (Command::ToggleEdges, Icon::Edges, "Show edges", true, self.show_edges),
                            ];
                            for (c, icon, tip, enabled, selected) in items {
                                if icons::icon_button(ui, icon, 26.0, tip, enabled, selected).clicked() {
                                    cmd = Some(c);
                                }
                            }
                        });
                    });
            });
        if let Some(c) = cmd {
            self.run(c);
        }
    }

    // ------------------------------------------------------- context menu

    fn context_menu(&mut self, response: &egui::Response) {
        let mut cmd = None;
        let sketching = matches!(self.mode, Mode::Sketch(_));
        response.context_menu(|ui| {
            ui.set_min_width(200.0);
            let mut item = |ui: &mut egui::Ui, c: Command, icon: Icon, label: &str| {
                let enabled = self.can_run(c);
                let clicked = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                            icons::paint(ui.painter(), r, icon, theme::ICON, enabled);
                            ui.add(egui::Button::new(label).frame(false)).clicked()
                        })
                        .inner
                    })
                    .inner;
                if clicked {
                    cmd = Some(c);
                    ui.close();
                }
            };
            if sketching {
                item(ui, Command::FinishSketch, Icon::Finish, "Finish Sketch");
                ui.separator();
                let tool = |t| Command::Sketch(SketchCommand::Tool(t));
                item(ui, tool(Tool::Line), Icon::Line, "Line   L");
                item(ui, tool(Tool::Rectangle), Icon::Rectangle, "Rectangle   R");
                item(ui, tool(Tool::Circle), Icon::Circle, "Circle   C");
                item(ui, tool(Tool::Arc), Icon::Arc, "Arc   A");
                item(ui, tool(Tool::Dimension), Icon::Dimension, "Dimension   D");
                ui.separator();
                item(ui, Command::Sketch(SketchCommand::Constrain(ConstraintKind::Coincident)), Icon::Coincident, "Coincident");
                item(ui, Command::Sketch(SketchCommand::Construction), Icon::Construction, "Toggle construction   X");
                item(ui, Command::Sketch(SketchCommand::Delete), Icon::Delete, "Delete   Del");
                ui.separator();
                item(ui, Command::LookAt, Icon::LookAt, "Look At");
            } else {
                let has = |f: fn(&Pick) -> bool| self.selection.iter().any(f);
                let (face, edge, body) = (
                    has(|p| matches!(p, Pick::Face(_))),
                    has(|p| matches!(p, Pick::Edge(_))),
                    has(|p| matches!(p, Pick::Body(_))),
                );
                if face {
                    item(ui, Command::SketchOnFace, Icon::Sketch, "Create Sketch on Face");
                    item(ui, Command::Feature(FeatureType::PressPull), Icon::PressPull, "Press Pull   Q");
                    item(ui, Command::Feature(FeatureType::Hole), Icon::Hole, "Hole   H");
                    item(ui, Command::Feature(FeatureType::Shell), Icon::Shell, "Shell (remove faces)");
                    item(ui, Command::Feature(FeatureType::Draft), Icon::Draft, "Draft");
                    item(ui, Command::Feature(FeatureType::Plane), Icon::Plane, "Offset Plane");
                    item(ui, Command::Feature(FeatureType::AxisFace), Icon::Axis, "Axis Normal to Face");
                    ui.separator();
                }
                if edge {
                    item(ui, Command::Fillet, Icon::Fillet, "Fillet   F");
                    item(ui, Command::Chamfer, Icon::Chamfer, "Chamfer");
                    item(ui, Command::Feature(FeatureType::AxisEdge), Icon::Axis, "Axis Through Edge");
                    ui.separator();
                }
                if body {
                    item(ui, Command::NewComponent, Icon::Body, "New Component");
                    item(ui, Command::Feature(FeatureType::Joint), Icon::Joint, "Joint");
                    item(ui, Command::Feature(FeatureType::Move), Icon::Move, "Move/Copy   M");
                    item(ui, Command::Feature(FeatureType::Mirror), Icon::Mirror, "Mirror");
                    item(ui, Command::Feature(FeatureType::RectPattern), Icon::RectPattern, "Rectangular Pattern");
                    item(ui, Command::Feature(FeatureType::CircPattern), Icon::CircPattern, "Circular Pattern");
                    item(ui, Command::Feature(FeatureType::Split), Icon::Split, "Split Body");
                    item(ui, Command::Properties, Icon::Properties, "Properties");
                    item(ui, Command::RemoveBodies, Icon::Delete, "Remove   Del");
                    ui.separator();
                }
                item(ui, Command::CreateSketch, Icon::Sketch, "Create Sketch   S");
                item(ui, Command::Extrude, Icon::Extrude, "Extrude   E");
                item(ui, Command::Revolve, Icon::Revolve, "Revolve");
                item(ui, Command::Fillet, Icon::Fillet, "Fillet   F");
                item(ui, Command::Chamfer, Icon::Chamfer, "Chamfer");
                ui.separator();
                item(ui, Command::Measure, Icon::Measure, "Measure   I");
                item(ui, Command::Section, Icon::Section, "Section Analysis");
                item(ui, Command::Undo, Icon::Undo, "Undo   Ctrl+Z");
                item(ui, Command::Redo, Icon::Redo, "Redo   Ctrl+Shift+Z");
            }
            ui.separator();
            item(ui, Command::Home, Icon::Home, "Home view");
            item(ui, Command::Fit, Icon::Fit, "Fit");
        });
        if let Some(c) = cmd {
            self.run(c);
        }
    }
}

fn paint_background(painter: &egui::Painter, rect: Rect) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), theme::VIEW_TOP);
    mesh.colored_vertex(rect.right_top(), theme::VIEW_TOP);
    mesh.colored_vertex(rect.right_bottom(), theme::VIEW_BOTTOM);
    mesh.colored_vertex(rect.left_bottom(), theme::VIEW_BOTTOM);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}
