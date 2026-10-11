//! Application state and the Fusion-style layout:
//! toolbar on top, browser on the left, timeline at the bottom, viewport in
//! the middle, tool dialogs as floating windows.

use crate::dialogs::{ExtrudeSession, FilletSession, RevolveSession};
use crate::sketch_ui::SketchSession;
use ferrocad_kernel_occt::OcctKernel;
use ferrocad_model::geom::Vec3;
use ferrocad_model::{
    io, Document, FeatureId, FeatureKind, History, Kernel, Model, Plane, SketchFeature, TriMesh,
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
    Revolve(RevolveSession),
    Fillet(FilletSession),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModeKind {
    Model,
    Sketch,
    Extrude,
    Revolve,
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
    pub history: History,
    /// Path of the open `.fcad` document (editable in the File menu).
    pub doc_path: String,
    /// While sketching: the document as of the last undo step, so sketch
    /// edits can be recorded without cloning the document every frame.
    pub sketch_baseline: Option<Document>,
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
            history: History::default(),
            doc_path: format!("untitled.{}", io::EXTENSION),
            sketch_baseline: None,
        };
        app.rebuild();
        app
    }

    pub fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::Model => ModeKind::Model,
            Mode::Sketch(_) => ModeKind::Sketch,
            Mode::Extrude(_) => ModeKind::Extrude,
            Mode::Revolve(_) => ModeKind::Revolve,
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
        self.checkpoint("New sketch");
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
            FeatureKind::Revolve(r) => self.mode = Mode::Revolve(RevolveSession::edit(id, r)),
            FeatureKind::Fillet(fl) => self.mode = Mode::Fillet(FilletSession::edit(id, fl)),
        }
    }

    /// Record an undo step before mutating the document.
    pub fn checkpoint(&mut self, label: &str) {
        self.history.checkpoint(label, &self.doc);
        self.sketch_baseline = None;
    }

    /// Record an undo step from a snapshot taken earlier.
    pub fn record(&mut self, label: &str, before: Document) {
        self.history.record(label, before);
        self.sketch_baseline = None;
    }

    /// Record a sketch edit that already happened (see `sketch_baseline`).
    pub fn commit_sketch_edit(&mut self, label: &str) {
        if let Some(before) = self.sketch_baseline.take() {
            self.history.record(label, before);
        }
    }

    pub fn undo(&mut self) {
        match self.history.undo(&mut self.doc) {
            Some(label) => {
                self.after_history_change();
                self.info(format!("Undo: {label}"));
            }
            None => self.info("nothing to undo"),
        }
    }

    pub fn redo(&mut self) {
        match self.history.redo(&mut self.doc) {
            Some(label) => {
                self.after_history_change();
                self.info(format!("Redo: {label}"));
            }
            None => self.info("nothing to redo"),
        }
    }

    /// The document was swapped out under the UI: drop stale tool state.
    fn after_history_change(&mut self) {
        self.sketch_baseline = None;
        let keep_sketch = match &mut self.mode {
            Mode::Sketch(sess) if self.doc.sketch(sess.feature).is_some() => {
                sess.analysis = None;
                sess.selection.clear();
                sess.clicks.clear();
                sess.dim_edits.clear();
                sess.dragging = None;
                true
            }
            _ => false,
        };
        if !keep_sketch {
            self.mode = Mode::Model;
        }
        self.param_edits.clear();
        self.rebuild();
    }

    fn new_document(&mut self) {
        self.checkpoint("New document");
        self.doc = Document::new();
        self.mode = Mode::Model;
        self.param_edits.clear();
        self.doc_path = format!("untitled.{}", io::EXTENSION);
        self.rebuild();
        self.info("new document (Undo brings the old one back)");
    }

    fn save_document(&mut self) {
        let mut path = std::path::PathBuf::from(self.doc_path.trim());
        if path.extension().is_none() {
            path.set_extension(io::EXTENSION);
        }
        match std::fs::write(&path, io::save(&self.doc)) {
            Ok(()) => {
                self.doc_path = path.display().to_string();
                self.info(format!("saved {}", path.display()));
            }
            Err(e) => self.error(format!("could not save {}: {e}", path.display())),
        }
    }

    fn open_document(&mut self) {
        let path = std::path::PathBuf::from(self.doc_path.trim());
        let loaded = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| io::load(&text).map_err(|e| e.to_string()));
        match loaded {
            Ok(doc) => {
                self.checkpoint("Open document");
                self.doc = doc;
                self.mode = Mode::Model;
                self.param_edits.clear();
                self.message = None;
                self.rebuild();
                self.fit_view();
                if self.message.is_none() {
                    self.info(format!("opened {}", path.display()));
                }
            }
            Err(e) => self.error(format!("could not open {}: {e}", path.display())),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo, save) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            let z = cmd && i.key_pressed(egui::Key::Z);
            (
                z && !i.modifiers.shift,
                (z && i.modifiers.shift) || (cmd && i.key_pressed(egui::Key::Y)),
                cmd && i.key_pressed(egui::Key::S),
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if save {
            self.save_document();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("File", |ui| {
                ui.label("Document");
                ui.text_edit_singleline(&mut self.doc_path);
                ui.horizontal(|ui| {
                    if ui.button("New").clicked() {
                        self.new_document();
                        ui.close();
                    }
                    if ui.button("Open").clicked() {
                        self.open_document();
                        ui.close();
                    }
                    if ui.button("Save (Ctrl+S)").clicked() {
                        self.save_document();
                        ui.close();
                    }
                });
                ui.separator();
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
            let undo_tip = self.history.undo_label().map(|l| format!("Undo {l} (Ctrl+Z)"));
            if ui
                .add_enabled(undo_tip.is_some(), egui::Button::new("↶"))
                .on_hover_text(undo_tip.unwrap_or_default())
                .clicked()
            {
                self.undo();
            }
            let redo_tip = self.history.redo_label().map(|l| format!("Redo {l} (Ctrl+Shift+Z)"));
            if ui
                .add_enabled(redo_tip.is_some(), egui::Button::new("↷"))
                .on_hover_text(redo_tip.unwrap_or_default())
                .clicked()
            {
                self.redo();
            }
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
                if ui.button("⟳ Revolve").clicked() {
                    self.mode = Mode::Revolve(RevolveSession::new(&self.doc));
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
        self.shortcuts(&ctx);
        if matches!(self.mode, Mode::Sketch(_)) && self.sketch_baseline.is_none() {
            self.sketch_baseline = Some(self.doc.clone());
        }

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
            ModeKind::Revolve => self.revolve_window(&ctx),
            ModeKind::Fillet => self.fillet_window(&ctx),
            ModeKind::Model => {}
        }
    }
}
