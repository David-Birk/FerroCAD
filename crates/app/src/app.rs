//! Application state and the Fusion-style layout:
//! toolbar on top, browser on the left, timeline at the bottom, viewport in
//! the middle, tool dialogs as floating windows.

use crate::dialogs::{ExtrudeSession, FilletSession};
use crate::sketch_ui::SketchSession;
use ferrocad_kernel_occt::OcctKernel;
use ferrocad_model::geom::Vec3;
use ferrocad_model::{
    Document, FeatureId, FeatureKind, Kernel, Model, Plane, SketchFeature, TriMesh,
};
use ferrocad_view::{Camera, ViewPreset};

pub type Solid = <OcctKernel as Kernel>::Solid;

/// Display data derived from a body after each rebuild.
pub struct BodyDisplay {
    pub name: String,
    pub mesh: TriMesh,
    pub edges: Vec<Vec<Vec3>>,
    pub visible: bool,
}

pub enum Mode {
    Model,
    Sketch(SketchSession),
    Extrude(ExtrudeSession),
    Fillet(FilletSession),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModeKind {
    Model,
    Sketch,
    Extrude,
    Fillet,
}

pub struct FerroApp {
    pub doc: Document,
    pub kernel: OcctKernel,
    pub model: Model<Solid>,
    pub bodies: Vec<BodyDisplay>,
    pub camera: Camera,
    pub mode: Mode,
    pub message: Option<(String, bool)>,
    pub show_params: bool,
    pub new_param: (String, String),
    pub param_edits: Vec<(String, String)>,
    pub export_path: String,
}

impl FerroApp {
    pub fn new() -> Self {
        let mut doc = Document::new();
        // A friendly default so the parameter workflow is visible at once.
        let _ = doc.params.set("width", "40 mm");
        let _ = doc.params.set("height", "10 mm");
        let mut app = Self {
            doc,
            kernel: OcctKernel::new(),
            model: Model { bodies: Vec::new(), status: Default::default() },
            bodies: Vec::new(),
            camera: Camera::default(),
            mode: Mode::Model,
            message: None,
            show_params: false,
            new_param: (String::new(), String::new()),
            param_edits: Vec::new(),
            export_path: "ferrocad-export.step".into(),
        };
        app.rebuild();
        app
    }

    pub fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::Model => ModeKind::Model,
            Mode::Sketch(_) => ModeKind::Sketch,
            Mode::Extrude(_) => ModeKind::Extrude,
            Mode::Fillet(_) => ModeKind::Fillet,
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    /// Replay the timeline and re-tessellate every body.
    pub fn rebuild(&mut self) {
        let visibility: Vec<(String, bool)> =
            self.bodies.iter().map(|b| (b.name.clone(), b.visible)).collect();
        self.model = self.doc.rebuild(&self.kernel);
        self.bodies = self
            .model
            .bodies
            .iter()
            .map(|b| BodyDisplay {
                name: b.name.clone(),
                mesh: self.kernel.tessellate(&b.solid, 0.05).unwrap_or_default(),
                edges: self.kernel.edges(&b.solid),
                visible: visibility.iter().find(|(n, _)| *n == b.name).is_none_or(|(_, v)| *v),
            })
            .collect();
        if let Some((id, err)) = self.model.errors().next() {
            let name = self.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
            self.message = Some((format!("{name}: {err}"), true));
        }
    }

    pub fn fit_view(&mut self) {
        if let Some((min, max)) = ferrocad_view::bounds(self.bodies.iter().map(|b| &b.mesh)) {
            self.camera.fit(min, max);
        } else {
            self.camera.fit([-50.0, -50.0, 0.0], [50.0, 50.0, 0.0]);
        }
    }

    pub fn start_sketch(&mut self, plane: Plane, label: &str) {
        let name = self.doc.next_name("Sketch");
        let id = self.doc.add(name.clone(), FeatureKind::Sketch(SketchFeature::new(plane)));
        self.camera.look_at_plane(plane.normal());
        self.mode = Mode::Sketch(SketchSession::new(id));
        self.info(format!("{name} on {label} – draw, constrain, then Finish sketch"));
    }

    pub fn edit_feature(&mut self, id: FeatureId) {
        let Some(f) = self.doc.feature(id) else { return };
        match &f.kind {
            FeatureKind::Sketch(s) => {
                self.camera.look_at_plane(s.plane.normal());
                self.mode = Mode::Sketch(SketchSession::new(id));
            }
            FeatureKind::Extrude(e) => self.mode = Mode::Extrude(ExtrudeSession::edit(id, e)),
            FeatureKind::Fillet(fl) => self.mode = Mode::Fillet(FilletSession::edit(id, fl)),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("File", |ui| {
                ui.label("Export path");
                ui.text_edit_singleline(&mut self.export_path);
                if ui.button("Export STEP (all bodies)").clicked() {
                    self.export(false);
                    ui.close();
                }
                if ui.button("Export STL (first visible body)").clicked() {
                    self.export(true);
                    ui.close();
                }
            });
            ui.separator();

            let in_sketch = matches!(self.mode, Mode::Sketch(_));
            ui.add_enabled_ui(!in_sketch, |ui| {
                ui.menu_button("✏ Sketch", |ui| {
                    for (label, plane) in
                        [("XY (top)", Plane::XY), ("XZ (front)", Plane::XZ), ("YZ (right)", Plane::YZ)]
                    {
                        if ui.button(label).clicked() {
                            self.start_sketch(plane, label);
                            ui.close();
                        }
                    }
                });
                if ui.button("⬆ Extrude").clicked() {
                    self.mode = Mode::Extrude(ExtrudeSession::new(&self.doc));
                }
                if ui.button("◔ Fillet").clicked() {
                    self.mode = Mode::Fillet(FilletSession::new(false));
                }
                if ui.button("◢ Chamfer").clicked() {
                    self.mode = Mode::Fillet(FilletSession::new(true));
                }
            });
            ui.separator();
            if ui.button("ƒ Parameters").clicked() {
                self.show_params = !self.show_params;
                self.param_edits.clear();
            }
            ui.separator();
            for (label, preset) in [
                ("Home", ViewPreset::Home),
                ("Front", ViewPreset::Front),
                ("Top", ViewPreset::Top),
                ("Right", ViewPreset::Right),
            ] {
                if ui.button(label).clicked() {
                    self.camera.set_view(preset);
                }
            }
            if ui.button("Fit").clicked() {
                self.fit_view();
            }
        });
    }

    fn export(&mut self, stl: bool) {
        let path = std::path::PathBuf::from(&self.export_path);
        let visible: Vec<Solid> = self
            .model
            .bodies
            .iter()
            .zip(&self.bodies)
            .filter(|(_, d)| d.visible)
            .map(|(b, _)| b.solid.clone())
            .collect();
        if visible.is_empty() {
            self.error("nothing to export");
            return;
        }
        let result = if stl {
            self.kernel.export_stl(&visible[0], &path.with_extension("stl"))
        } else {
            self.kernel.export_step(&visible, &path.with_extension("step"))
        };
        match result {
            Ok(()) => self.info(format!("exported {}", path.display())),
            Err(e) => self.error(format!("export failed: {e}")),
        }
    }

    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.heading("Browser");
        ui.separator();
        egui::CollapsingHeader::new("Bodies").default_open(true).show(ui, |ui| {
            if self.bodies.is_empty() {
                ui.weak("none yet");
            }
            for b in &mut self.bodies {
                ui.checkbox(&mut b.visible, b.name.as_str());
            }
        });
        let sketches: Vec<(FeatureId, String)> = self
            .doc
            .features()
            .iter()
            .filter(|f| matches!(f.kind, FeatureKind::Sketch(_)))
            .map(|f| (f.id, f.name.clone()))
            .collect();
        egui::CollapsingHeader::new("Sketches").default_open(true).show(ui, |ui| {
            for (id, name) in sketches {
                if ui.button(name).on_hover_text("Edit sketch").clicked() {
                    self.edit_feature(id);
                }
            }
        });
        egui::CollapsingHeader::new("Parameters").default_open(true).show(ui, |ui| {
            let rows: Vec<(String, String)> =
                self.doc.params.iter().map(|p| (p.name.clone(), p.expression.clone())).collect();
            for (name, expr) in rows {
                let value = self.doc.params.value(&name).map(|v| format!("{v:.3}")).unwrap_or("?".into());
                ui.label(format!("{name} = {expr}  ({value})"));
            }
            if ui.button("Edit…").clicked() {
                self.show_params = true;
            }
        });
    }
}

impl eframe::App for FerroApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| match &self.message {
                Some((text, true)) => {
                    ui.colored_label(egui::Color32::from_rgb(255, 110, 100), text);
                }
                Some((text, false)) => {
                    ui.label(text);
                }
                None => {
                    ui.weak("Right-drag: orbit · Middle-drag: pan · Wheel: zoom");
                }
            });
        });

        egui::Panel::bottom("timeline").resizable(false).show(ui, |ui| self.timeline(ui));

        egui::Panel::left("browser").resizable(true).default_size(220.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.browser(ui));
        });

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));

        // Floating tool windows.
        if self.show_params {
            self.parameters_window(&ctx);
        }
        match self.mode_kind() {
            ModeKind::Sketch => self.sketch_palette(&ctx),
            ModeKind::Extrude => self.extrude_window(&ctx),
            ModeKind::Fillet => self.fillet_window(&ctx),
            ModeKind::Model => {}
        }
    }
}
