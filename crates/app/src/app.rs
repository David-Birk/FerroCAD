//! Application state and the Fusion-style layout: application bar and ribbon
//! on top, browser on the left, sketch palette on the right while sketching,
//! timeline and status bar at the bottom, command dialogs floating over the
//! viewport.

use crate::dialogs::{ExtrudeSession, FilletSession, RevolveSession};
use crate::features_ui::{FeatureSession, FeatureType, Pick};
use crate::inspect::{MeasureSession, Section};
use crate::sketch_ui::{SketchCommand, SketchSession};
use crate::theme;
use ferrocad_kernel_occt::OcctKernel;
use ferrocad_model::geom::Vec3;
use ferrocad_model::{
    io, Document, FeatureId, FeatureKind, History, Kernel, Model, Plane, SketchFeature, TriMesh,
};
use ferrocad_view::{Camera, ViewPreset};
use std::collections::HashSet;

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
    /// "Create Sketch": waiting for the user to pick an origin plane.
    PickPlane,
    Sketch(Box<SketchSession>),
    Extrude(ExtrudeSession),
    Revolve(RevolveSession),
    Fillet(FilletSession),
    /// Any feature with a generic dialog (see `features_ui`).
    Feature(Box<FeatureSession>),
    Measure(MeasureSession),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModeKind {
    Model,
    PickPlane,
    Sketch,
    Extrude,
    Revolve,
    Fillet,
    Feature,
    Measure,
}

/// Everything the ribbon, menus and keyboard shortcuts can trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    Undo,
    Redo,
    CreateSketch,
    Extrude,
    Revolve,
    Fillet,
    Chamfer,
    Parameters,
    Home,
    Fit,
    LookAt,
    ToggleGrid,
    ToggleEdges,
    FinishSketch,
    Sketch(SketchCommand),
    Feature(FeatureType),
    /// Remove the selected bodies (adds a Remove feature).
    RemoveBodies,
    Properties,
    Measure,
    Section,
    /// Start a sketch on the selected face.
    SketchOnFace,
    /// Insert the bodies of a STEP file (file dialog).
    ImportStep,
    /// Find overlapping bodies.
    Interference,
    /// Group the selected bodies into a new component.
    NewComponent,
}

/// Bodies computed for a dialog's live preview.
pub struct Preview {
    pub key: String,
    pub bodies: Vec<BodyDisplay>,
    pub error: Option<String>,
}

struct CameraAnim {
    from: Camera,
    to: Camera,
    t: f32,
}

pub struct FerroApp {
    pub doc: Document,
    pub kernel: OcctKernel,
    pub model: Model<Solid>,
    pub bodies: Vec<BodyDisplay>,
    pub camera: Camera,
    cam_anim: Option<CameraAnim>,
    pub mode: Mode,
    pub message: Option<(String, bool)>,
    /// When the current info message appeared (info messages time out).
    message_seen: Option<(String, f64)>,
    pub show_params: bool,
    pub new_param: (String, String),
    pub param_edits: Vec<(String, String)>,
    /// Edit buffers of the model parameters table (feature, slot).
    pub model_param_edits: std::collections::HashMap<(FeatureId, usize), String>,
    pub export_path: String,
    pub history: History,
    /// Path of the open `.fcad` document (editable in the File menu).
    pub doc_path: String,
    /// While sketching: the document as of the last undo step, so sketch
    /// edits can be recorded without cloning the document every frame.
    pub sketch_baseline: Option<Document>,
    pub preview: Option<Preview>,
    /// Sketches hidden in the viewport (consumed sketches are hidden automatically).
    pub hidden_sketches: HashSet<FeatureId>,
    pub show_grid: bool,
    pub show_edges: bool,
    pub show_origin: bool,
    /// Screen rect of the 3D viewport in the last frame.
    pub viewport_rect: egui::Rect,
    /// Model-mode selection (faces, edges, bodies).
    pub selection: Vec<Pick>,
    pub hidden_construction: HashSet<FeatureId>,
    pub section: Option<Section>,
    /// Bodies are drawn with wgpu (depth buffer) instead of the painter.
    pub gpu: bool,
    /// Edge length of drawn origin/construction planes, set on rebuild.
    pub plane_extent: f64,
    /// Bumped on every rebuild; part of the GPU scene cache key.
    pub scene_rev: u64,
    pub scene_cache: Option<(String, std::sync::Arc<crate::render3d::SceneData>)>,
    /// Depth-tested translucent triangles for this frame (GPU mode).
    pub overlay: crate::render3d::Overlay,
    /// The command search box (S), if open: query and highlighted row.
    pub search: Option<(String, usize)>,
    /// Overlaps found by the interference check (shown until the next rebuild).
    pub interference: Vec<TriMesh>,
    /// Joint being animated: (feature, original expression, start time).
    pub animation: Option<(FeatureId, String, f64)>,
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
            model: Model::empty(),
            bodies: Vec::new(),
            camera: Camera::default(),
            cam_anim: None,
            mode: Mode::Model,
            message: None,
            message_seen: None,
            show_params: false,
            new_param: (String::new(), String::new()),
            param_edits: Vec::new(),
            model_param_edits: Default::default(),
            export_path: "ferrocad-export.step".into(),
            history: History::default(),
            doc_path: format!("untitled.{}", io::EXTENSION),
            sketch_baseline: None,
            preview: None,
            hidden_sketches: HashSet::new(),
            show_grid: true,
            show_edges: true,
            show_origin: false,
            viewport_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            selection: Vec::new(),
            hidden_construction: HashSet::new(),
            section: None,
            gpu: false,
            plane_extent: crate::features_ui::plane_size_for(None),
            scene_rev: 0,
            scene_cache: None,
            overlay: Default::default(),
            search: None,
            interference: Vec::new(),
            animation: None,
        };
        app.camera.set_view(ViewPreset::Home);
        app.rebuild();
        app
    }

    pub fn mode_kind(&self) -> ModeKind {
        match self.mode {
            Mode::Model => ModeKind::Model,
            Mode::PickPlane => ModeKind::PickPlane,
            Mode::Sketch(_) => ModeKind::Sketch,
            Mode::Extrude(_) => ModeKind::Extrude,
            Mode::Revolve(_) => ModeKind::Revolve,
            Mode::Fillet(_) => ModeKind::Fillet,
            Mode::Feature(_) => ModeKind::Feature,
            Mode::Measure(_) => ModeKind::Measure,
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    fn display_bodies(&self, model: &Model<Solid>, keep_visibility: bool) -> Vec<BodyDisplay> {
        model
            .bodies
            .iter()
            .map(|b| BodyDisplay {
                name: b.name.clone(),
                mesh: self.kernel.tessellate(&b.solid, 0.05).unwrap_or_default(),
                edges: self.kernel.edges(&b.solid),
                visible: !keep_visibility
                    || self.bodies.iter().find(|d| d.name == b.name).is_none_or(|d| d.visible),
            })
            .collect()
    }

    /// Replay the timeline and re-tessellate every body.
    pub fn rebuild(&mut self) {
        self.model = self.doc.rebuild(&self.kernel);
        self.bodies = self.display_bodies(&self.model, true);
        self.plane_extent = crate::features_ui::plane_size_for(ferrocad_view::bounds(self.bodies.iter().map(|b| &b.mesh)));
        self.scene_rev += 1;
        self.model_param_edits.clear();
        self.interference.clear();
        self.preview = None;
        if let Some((id, err)) = self.model.errors().next() {
            let name = self.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
            self.message = Some((format!("{name}: {err}"), true));
        }
    }

    /// Rebuild a modified copy of the document for a dialog's live preview.
    /// Does nothing while `key` (a description of the dialog inputs) is unchanged.
    pub fn update_preview(&mut self, key: String, apply: impl FnOnce(&mut Document)) {
        if self.preview.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        let mut doc = self.doc.clone();
        apply(&mut doc);
        let model = doc.rebuild(&self.kernel);
        let error = model.errors().next().map(|(_, e)| e.to_string());
        let bodies = self.display_bodies(&model, false);
        self.preview = Some(Preview { key, bodies, error });
    }

    // ------------------------------------------------------------ camera

    /// Move the camera smoothly to wherever `f` puts it.
    pub fn animate_camera(&mut self, f: impl FnOnce(&mut Camera)) {
        let mut to = self.camera;
        f(&mut to);
        let tau = std::f64::consts::TAU;
        let mut d = (to.yaw - self.camera.yaw).rem_euclid(tau);
        if d > std::f64::consts::PI {
            d -= tau;
        }
        to.yaw = self.camera.yaw + d;
        self.cam_anim = Some(CameraAnim { from: self.camera, to, t: 0.0 });
    }

    pub fn stop_camera_animation(&mut self) {
        self.cam_anim = None;
    }

    /// Advance the camera animation; true while it is running.
    pub fn step_camera(&mut self, dt: f32) -> bool {
        let Some(a) = &mut self.cam_anim else { return false };
        a.t = (a.t + dt / 0.4).min(1.0);
        let s = (a.t * a.t * (3.0 - 2.0 * a.t)) as f64;
        let lerp = |x: f64, y: f64| x + (y - x) * s;
        let (f, t) = (a.from, a.to);
        self.camera.yaw = lerp(f.yaw, t.yaw);
        self.camera.pitch = lerp(f.pitch, t.pitch);
        self.camera.distance = (lerp(f.distance.ln(), t.distance.ln())).exp();
        for i in 0..3 {
            self.camera.target[i] = lerp(f.target[i], t.target[i]);
        }
        if a.t >= 1.0 {
            self.camera = t;
            self.cam_anim = None;
        }
        true
    }

    /// Bounds of the visible bodies and, while sketching, the sketch.
    fn scene_bounds(&self) -> (Vec3, Vec3) {
        let mut b = ferrocad_view::bounds(self.bodies.iter().filter(|b| b.visible).map(|b| &b.mesh));
        if let Mode::Sketch(sess) = &self.mode {
            if let Some(sf) = self.doc.sketch(sess.feature) {
                for (_, e) in sf.sketch.entities() {
                    for uv in crate::sketch_ui::entity_polyline(&sf.sketch, e) {
                        let p = sf.plane.to_world(uv);
                        let (mut lo, mut hi) = b.unwrap_or((p, p));
                        for i in 0..3 {
                            lo[i] = lo[i].min(p[i]);
                            hi[i] = hi[i].max(p[i]);
                        }
                        b = Some((lo, hi));
                    }
                }
            }
        }
        b.unwrap_or(([-50.0, -50.0, 0.0], [50.0, 50.0, 0.0]))
    }

    pub fn fit_view(&mut self) {
        let (min, max) = self.scene_bounds();
        self.animate_camera(|c| c.fit(min, max));
    }

    pub fn set_view(&mut self, preset: ViewPreset) {
        let (min, max) = self.scene_bounds();
        self.animate_camera(|c| {
            c.set_view(preset);
            if preset == ViewPreset::Home {
                c.fit(min, max);
            }
        });
    }

    pub fn look_along(&mut self, normal: Vec3) {
        self.animate_camera(|c| c.look_at_plane(normal));
    }

    // ------------------------------------------------------------ features

    /// Start a sketch on an origin plane, construction plane or face. Sketches
    /// on faces and construction planes follow them when the model changes.
    pub fn start_sketch_on(&mut self, r: ferrocad_model::PlaneRef) {
        use ferrocad_model::PlaneRef;
        let Some((plane, _)) = self.plane_of(&r) else { return };
        let (attach, label) = match r {
            PlaneRef::Origin(o) => (None, format!("the {} plane", o.label())),
            PlaneRef::Face(_) => (Some(r), "a face".to_string()),
            PlaneRef::Construction(id) => {
                let name = self.doc.feature(id).map(|f| f.name.clone()).unwrap_or_default();
                (Some(r), name)
            }
        };
        self.start_sketch_attached(plane, attach, &label);
    }

    pub fn start_sketch(&mut self, plane: Plane, label: &str) {
        self.start_sketch_attached(plane, None, label);
    }

    fn start_sketch_attached(&mut self, plane: Plane, attach: Option<ferrocad_model::PlaneRef>, label: &str) {
        self.checkpoint("New sketch");
        let name = self.doc.next_name("Sketch");
        let mut sf = SketchFeature::new(plane);
        sf.attach = attach;
        let id = self.doc.add(name.clone(), FeatureKind::Sketch(sf));
        self.look_along(plane.normal());
        self.mode = Mode::Sketch(Box::new(SketchSession::new(id)));
        self.info(format!("{name} on {label}"));
    }

    pub fn edit_feature(&mut self, id: FeatureId) {
        let Some(f) = self.doc.feature(id) else { return };
        match &f.kind {
            FeatureKind::Sketch(s) => {
                let normal = s.plane.normal();
                self.mode = Mode::Sketch(Box::new(SketchSession::new(id)));
                self.look_along(normal);
            }
            FeatureKind::Extrude(e) => self.mode = Mode::Extrude(ExtrudeSession::edit(id, e)),
            FeatureKind::Revolve(r) => self.mode = Mode::Revolve(RevolveSession::edit(id, r)),
            FeatureKind::Fillet(fl) => self.mode = Mode::Fillet(FilletSession::edit(id, fl)),
            other => {
                if let Some(sess) = FeatureSession::edit(id, other) {
                    self.mode = Mode::Feature(Box::new(sess));
                }
            }
        }
    }

    /// Record an undo step before mutating the document.
    pub fn checkpoint(&mut self, label: &str) {
        // Any real edit ends an animation (restoring the joint first).
        if self.animation.is_some() {
            self.stop_animation();
        }
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
        self.animation = None;
        self.sketch_baseline = None;
        let keep_sketch = match &mut self.mode {
            Mode::Sketch(sess) if self.doc.sketch(sess.feature).is_some() => {
                sess.reset_transient();
                true
            }
            _ => false,
        };
        if !keep_sketch {
            self.mode = Mode::Model;
        }
        self.param_edits.clear();
        self.model_param_edits.clear();
        self.rebuild();
    }

    fn new_document(&mut self) {
        self.checkpoint("New document");
        self.doc = Document::new();
        self.mode = Mode::Model;
        self.param_edits.clear();
        self.hidden_sketches.clear();
        self.doc_path = format!("untitled.{}", io::EXTENSION);
        self.rebuild();
        self.info("new document (Undo brings the old one back)");
    }

    /// Native file dialog for .fcad files (None if cancelled or unavailable).
    fn fcad_dialog(&self, save: bool) -> Option<std::path::PathBuf> {
        let current = std::path::Path::new(self.doc_path.trim());
        let mut d = rfd::FileDialog::new().add_filter("ferrocad document", &[io::EXTENSION]);
        if let Some(dir) = current.parent().filter(|p| p.is_dir()) {
            d = d.set_directory(dir);
        }
        if save {
            d.set_file_name(current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                .save_file()
        } else {
            d.pick_file()
        }
    }

    pub fn import_step_with_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("STEP", &["step", "stp", "STEP", "STP"]).pick_file() else {
            return;
        };
        self.import_step(&path);
    }

    /// Add an Import feature for a STEP file (one body per solid).
    pub fn import_step(&mut self, path: &std::path::Path) {
        self.checkpoint("Import STEP");
        let name = self.doc.next_name("Import");
        let path_text = path.display().to_string();
        self.doc.add(name, FeatureKind::Import(ferrocad_model::ImportFeature { path: path_text }));
        self.message = None;
        self.rebuild();
        if self.message.is_none() {
            self.info(format!("imported {}", path.display()));
            self.fit_view();
        }
    }

    /// Drive a revolute (angle) or slider (offset) joint over time.
    pub fn start_animation(&mut self, id: FeatureId) {
        self.stop_animation();
        let Some(FeatureKind::Joint(j)) = self.doc.feature(id).map(|f| &f.kind) else { return };
        let original = match j.kind {
            ferrocad_model::JointKind::Slider => j.offset.clone(),
            _ => j.angle.clone(),
        };
        self.animation = Some((id, original, f64::NAN));
        self.info("animating the joint – Esc stops");
    }

    pub fn stop_animation(&mut self) {
        if let Some((id, original, _)) = self.animation.take() {
            if let Some(FeatureKind::Joint(j)) = self.doc.feature_mut(id).map(|f| &mut f.kind) {
                match j.kind {
                    ferrocad_model::JointKind::Slider => j.offset = original,
                    _ => j.angle = original,
                }
            }
            self.rebuild();
        }
    }

    /// One animation frame: 60°/s for revolute joints, ±20 mm for sliders.
    fn step_animation(&mut self, ctx: &egui::Context) {
        let Some((id, original, start)) = &mut self.animation else { return };
        let now = ctx.input(|i| i.time);
        if start.is_nan() {
            *start = now;
        }
        let t = now - *start;
        let (id, base) = (*id, self.doc.params.eval(original).unwrap_or(0.0));
        let params = self.doc.params.clone();
        let Some(FeatureKind::Joint(j)) = self.doc.feature_mut(id).map(|f| &mut f.kind) else {
            self.animation = None;
            return;
        };
        // With limits: swing between them; without: turn / oscillate freely.
        let limits = j.limits.as_ref().and_then(|(lo, hi)| Some((params.eval(lo).ok()?, params.eval(hi).ok()?)));
        let swing = |lo: f64, hi: f64| lo + (hi - lo) * (0.5 - 0.5 * (t * 1.5).cos());
        let value = match (j.kind, limits) {
            (_, Some((lo, hi))) => swing(lo, hi),
            (ferrocad_model::JointKind::Slider, None) => base + 20.0 * (t * 1.5).sin(),
            (_, None) => base + (60.0 * t) % 360.0,
        };
        match j.kind {
            ferrocad_model::JointKind::Slider => j.offset = format!("{value:.4}"),
            _ => j.angle = format!("{value:.4}"),
        }
        // Rebuild without touching the message or undo history.
        let message = self.message.clone();
        self.rebuild();
        self.message = message;
        ctx.request_repaint();
    }

    /// Set (or with `None` reset) a body's appearance.
    pub fn set_appearance(&mut self, body: &str, material: Option<&str>) {
        self.set_appearances(&[body.to_string()], material);
    }

    /// Set the appearance of several bodies as one undo step.
    pub fn set_appearances(&mut self, bodies: &[String], material: Option<&str>) {
        self.checkpoint("Appearance");
        let list = &mut self.doc.view.appearances;
        list.retain(|(b, _)| !bodies.contains(b));
        if let Some(m) = material {
            list.extend(bodies.iter().map(|b| (b.clone(), m.to_string())));
        }
    }

    pub fn open_with_dialog(&mut self) {
        if let Some(path) = self.fcad_dialog(false) {
            self.doc_path = path.display().to_string();
            self.open_document();
        }
    }

    pub fn save_as(&mut self) {
        if let Some(path) = self.fcad_dialog(true) {
            self.doc_path = path.display().to_string();
            self.save_document();
        }
    }

    pub fn export_with_dialog(&mut self, stl: bool) {
        let (name, ext) = if stl { ("STL mesh", "stl") } else { ("STEP", "step") };
        let stem = std::path::Path::new(&self.doc_path).file_stem().map(|s| s.to_string_lossy().into_owned());
        let path = rfd::FileDialog::new()
            .add_filter(name, &[ext])
            .set_file_name(format!("{}.{ext}", stem.unwrap_or_else(|| "export".into())))
            .save_file();
        if let Some(path) = path {
            self.export_path = path.display().to_string();
            self.export(stl);
        }
    }

    fn save_document(&mut self) {
        self.stop_animation();
        let mut path = std::path::PathBuf::from(self.doc_path.trim());
        if path.extension().is_none() {
            path.set_extension(io::EXTENSION);
        }
        self.doc.view = ferrocad_model::ViewState {
            hidden_bodies: self.bodies.iter().filter(|b| !b.visible).map(|b| b.name.clone()).collect(),
            hidden_features: self.hidden_sketches.iter().chain(&self.hidden_construction).copied().collect(),
            appearances: self.doc.view.appearances.clone(),
            active_component: self.doc.view.active_component.clone(),
        };
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
                let view = self.doc.view.clone();
                if view == ferrocad_model::ViewState::default() {
                    // Older files: sketches that feed a feature start hidden.
                    self.hidden_sketches =
                        self.doc.features().iter().flat_map(|f| f.kind.consumed_sketches()).collect();
                    self.hidden_construction.clear();
                } else {
                    let is_sketch =
                        |id: &FeatureId| self.doc.feature(*id).is_some_and(|f| matches!(f.kind, FeatureKind::Sketch(_)));
                    self.hidden_sketches = view.hidden_features.iter().copied().filter(is_sketch).collect();
                    self.hidden_construction = view.hidden_features.iter().copied().filter(|f| !is_sketch(f)).collect();
                }
                self.selection.clear();
                self.bodies.clear();
                self.rebuild();
                for b in &mut self.bodies {
                    b.visible = !view.hidden_bodies.contains(&b.name);
                }
                self.fit_view();
                if self.message.is_none() {
                    self.info(format!("opened {}", path.display()));
                }
            }
            Err(e) => self.error(format!("could not open {}: {e}", path.display())),
        }
    }

    pub fn open_document_from_path(&mut self) {
        self.open_document();
    }

    pub fn save_document_to_path(&mut self) {
        self.save_document();
    }

    pub fn export(&mut self, stl: bool) {
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

    // ------------------------------------------------------------ commands

    /// Whether a command can run in the current mode.
    pub fn can_run(&self, cmd: Command) -> bool {
        let kind = self.mode_kind();
        let idle = matches!(kind, ModeKind::Model | ModeKind::PickPlane);
        match cmd {
            Command::Undo => self.history.undo_label().is_some(),
            Command::Redo => self.history.redo_label().is_some(),
            Command::CreateSketch
            | Command::Extrude
            | Command::Revolve
            | Command::Fillet
            | Command::Chamfer
            | Command::Feature(_)
            | Command::Measure => idle || kind == ModeKind::Measure,
            Command::RemoveBodies => idle && self.selection.iter().any(|p| matches!(p, Pick::Body(_))),
            Command::SketchOnFace => idle && self.selection.iter().any(|p| matches!(p, Pick::Face(_))),
            Command::NewComponent => idle && self.selection.iter().any(|p| matches!(p, Pick::Body(_))),
            Command::New | Command::Open => kind != ModeKind::Sketch,
            Command::ImportStep => idle,
            Command::FinishSketch | Command::Sketch(_) | Command::LookAt => kind == ModeKind::Sketch,
            _ => true,
        }
    }

    pub fn run(&mut self, cmd: Command) {
        if !self.can_run(cmd) {
            return;
        }
        match cmd {
            Command::New => self.new_document(),
            Command::Open => self.open_with_dialog(),
            Command::Save => {
                // An untitled document asks where to save it.
                if self.doc_path.starts_with("untitled.") {
                    self.save_as();
                } else {
                    self.save_document();
                }
            }
            Command::Undo => self.undo(),
            Command::Redo => self.redo(),
            Command::CreateSketch => {
                self.mode = Mode::PickPlane;
                self.info("Create Sketch: select a plane or a flat face");
            }
            Command::Extrude => {
                let mut sess = ExtrudeSession::new(&self.doc);
                // A selected sketch profile is the one to extrude.
                if let Some(Pick::Profile(p)) = self.selection.iter().find(|p| matches!(p, Pick::Profile(_))) {
                    sess.sketch = Some(p.sketch);
                    sess.profile_point = Some(p.point);
                    let on_face = self.doc.sketch(p.sketch).is_some_and(|s| matches!(s.attach, Some(ferrocad_model::PlaneRef::Face(_))));
                    sess.operation = if on_face { ferrocad_model::Operation::Join } else { ferrocad_model::Operation::NewBody };
                }
                self.mode = Mode::Extrude(sess);
                self.message = None;
            }
            Command::Revolve => {
                self.mode = Mode::Revolve(RevolveSession::new(&self.doc));
                self.message = None;
            }
            Command::Fillet | Command::Chamfer => {
                let mut sess = FilletSession::new(cmd == Command::Chamfer);
                sess.edges = self.selected_edges();
                self.mode = Mode::Fillet(sess);
                self.selection.clear();
                self.message = None;
            }
            Command::Feature(ty) => {
                let sess = FeatureSession::new(ty, &self.selection);
                self.mode = Mode::Feature(Box::new(sess));
                self.selection.clear();
                self.message = None;
            }
            Command::RemoveBodies => {
                let bodies: Vec<String> = self
                    .selection
                    .iter()
                    .filter_map(|p| if let Pick::Body(n) = p { Some(n.clone()) } else { None })
                    .collect();
                self.checkpoint("Remove body");
                let name = self.doc.next_name("Remove");
                self.doc.add(name, FeatureKind::Remove(ferrocad_model::RemoveFeature { bodies }));
                self.selection.clear();
                self.message = None;
                self.rebuild();
            }
            Command::Properties => {
                let names: Vec<String> = self
                    .selection
                    .iter()
                    .filter_map(|p| if let Pick::Body(n) = p { Some(n.clone()) } else { None })
                    .collect();
                self.show_properties(&names);
            }
            Command::ImportStep => self.import_step_with_dialog(),
            Command::Interference => self.check_interference(),
            Command::NewComponent => {
                // Selected bodies (components contribute their bodies).
                let mut bodies: Vec<String> = Vec::new();
                for p in &self.selection {
                    if let Pick::Body(n) = p {
                        match self.model.components.iter().find(|(c, _)| c == n) {
                            Some((_, list)) => bodies.extend(list.iter().cloned()),
                            None => bodies.push(n.clone()),
                        }
                    }
                }
                let mut k = self.model.components.len() + 1;
                let name = loop {
                    let candidate = format!("Component{k}");
                    let used = self.doc.features().iter().any(|f| matches!(&f.kind, FeatureKind::Component(c) if c.name == candidate));
                    if !used {
                        break candidate;
                    }
                    k += 1;
                };
                self.checkpoint("New component");
                let fname = self.doc.next_name("Component");
                self.doc.add(fname, FeatureKind::Component(ferrocad_model::ComponentFeature { name: name.clone(), bodies }));
                self.selection = vec![Pick::Body(name.clone())];
                self.message = None;
                self.rebuild();
                if self.message.is_none() {
                    self.info(format!("{name} created – clicking any of its bodies now selects it"));
                }
            }
            Command::SketchOnFace => {
                let face = self.selection.iter().find_map(|p| if let Pick::Face(f) = p { Some(*f) } else { None });
                if let Some(f) = face {
                    self.selection.clear();
                    self.start_sketch_on(ferrocad_model::PlaneRef::Face(f));
                }
            }
            Command::Measure => {
                self.mode = Mode::Measure(MeasureSession::default());
                self.selection.clear();
                self.message = None;
            }
            Command::Section => {
                self.section = match self.section {
                    Some(_) => None,
                    None => {
                        let (min, max) = self.scene_bounds();
                        Some(Section { axis: 1, offset: 0.5 * (min[1] + max[1]), flip: false })
                    }
                };
            }
            Command::Parameters => {
                self.show_params = !self.show_params;
                self.param_edits.clear();
            }
            Command::Home => self.set_view(ViewPreset::Home),
            Command::Fit => self.fit_view(),
            Command::LookAt => {
                if let Mode::Sketch(sess) = &self.mode {
                    if let Some(sf) = self.doc.sketch(sess.feature) {
                        let n = sf.plane.normal();
                        self.look_along(n);
                    }
                }
            }
            Command::ToggleGrid => self.show_grid = !self.show_grid,
            Command::ToggleEdges => self.show_edges = !self.show_edges,
            Command::FinishSketch => self.finish_sketch(),
            Command::Sketch(c) => self.sketch_command(c),
        }
    }

    pub fn finish_sketch(&mut self) {
        if !matches!(self.mode, Mode::Sketch(_)) {
            return;
        }
        self.mode = Mode::Model;
        self.rebuild();
        if self.message.as_ref().is_some_and(|(_, err)| !err) {
            self.message = None;
        }
    }

    pub fn cancel(&mut self) {
        match self.mode {
            Mode::PickPlane
            | Mode::Extrude(_)
            | Mode::Revolve(_)
            | Mode::Fillet(_)
            | Mode::Feature(_)
            | Mode::Measure(_) => {
                self.mode = Mode::Model;
                self.message = None;
            }
            Mode::Model => self.selection.clear(),
            Mode::Sketch(_) => {}
        }
    }

    /// Edges in the model selection.
    pub fn selected_edges(&self) -> Vec<ferrocad_model::EdgeRef> {
        self.selection.iter().filter_map(|p| if let Pick::Edge(e) = p { Some(*e) } else { None }).collect()
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let in_sketch = matches!(self.mode, Mode::Sketch(_));
        // Nothing in progress in the sketch: Enter finishes it, like Fusion.
        let sketch_idle = matches!(&self.mode, Mode::Sketch(s) if s.is_idle());
        let mut commands = Vec::new();
        ctx.input(|i| {
            let cmd = i.modifiers.command;
            let shift = i.modifiers.shift;
            let key = |k| i.key_pressed(k);
            if cmd {
                if key(egui::Key::Z) {
                    commands.push(if shift { Command::Redo } else { Command::Undo });
                }
                if key(egui::Key::Y) {
                    commands.push(Command::Redo);
                }
                if key(egui::Key::S) {
                    commands.push(Command::Save);
                }
                return;
            }
            if i.modifiers.alt {
                return;
            }
            if key(egui::Key::F6) {
                commands.push(Command::Home);
            }
            if key(egui::Key::F) && shift {
                commands.push(Command::Fit);
            }
            if key(egui::Key::S) && !shift {
                self.search = Some((String::new(), 0));
                return;
            }
            if in_sketch {
                if sketch_idle && key(egui::Key::Enter) {
                    commands.push(Command::FinishSketch);
                    return;
                }
                commands.extend(crate::sketch_ui::sketch_shortcut(i).map(Command::Sketch));
            } else {
                if key(egui::Key::E) {
                    commands.push(Command::Extrude);
                }
                if key(egui::Key::F) && !shift {
                    commands.push(Command::Fillet);
                }
                if key(egui::Key::Q) {
                    commands.push(Command::Feature(FeatureType::PressPull));
                }
                if key(egui::Key::H) {
                    commands.push(Command::Feature(FeatureType::Hole));
                }
                if key(egui::Key::M) {
                    commands.push(Command::Feature(FeatureType::Move));
                }
                if key(egui::Key::I) {
                    commands.push(Command::Measure);
                }

                if key(egui::Key::Delete) {
                    commands.push(Command::RemoveBodies);
                }
                if key(egui::Key::Escape) {
                    self.cancel();
                }
            }
        });
        for c in commands {
            self.run(c);
        }
    }

    /// Hint for the status bar when no message is shown.
    fn hint(&self) -> String {
        match &self.mode {
            Mode::Model => "Shift+middle-drag: orbit · Middle-drag: pan · Wheel: zoom · Right-click: menu".into(),
            Mode::PickPlane => "Select a plane or a flat face to sketch on (Esc to cancel)".into(),
            Mode::Sketch(sess) => sess.hint().into(),
            Mode::Extrude(_) => "Click inside a profile, set the distance, then OK (Enter)".into(),
            Mode::Revolve(_) => "Click inside a profile, choose the axis, then OK (Enter)".into(),
            Mode::Fillet(_) => "Click edges to add or remove them, set the radius, then OK (Enter)".into(),
            Mode::Feature(f) => match f.active_kind() {
                Some(k) => format!("{}: pick in the viewport for the highlighted input · Enter OK · Esc cancel", f.ty.title())
                    + if matches!(k, crate::features_ui::SelKind::Plane) { " (planes or flat faces)" } else { "" },
                None => f.ty.title().into(),
            },
            Mode::Measure(_) => "Measure: click vertices, edges or faces · Esc to finish".into(),
        }
    }

    /// Fusion's "S" box: type to find any command, Enter runs it.
    fn search_box(&mut self, ctx: &egui::Context) {
        let Some((mut query, mut selected)) = self.search.take() else { return };
        let mut results: Vec<(i32, Command, crate::icons::Icon, &str, &str)> = crate::ribbon::all_commands(self)
            .into_iter()
            .filter_map(|(c, i, l, t)| crate::ribbon::fuzzy_score(&query, l).map(|s| (s, c, i, l, t)))
            .collect();
        results.sort_by_key(|r| std::cmp::Reverse(r.0));
        results.truncate(12);
        let (up, down, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if down {
            selected = (selected + 1).min(results.len().saturating_sub(1));
        }
        if up {
            selected = selected.saturating_sub(1);
        }
        selected = selected.min(results.len().saturating_sub(1));
        let mut run = None;
        let at = ctx.pointer_hover_pos().unwrap_or(self.viewport_rect.center());
        egui::Area::new(egui::Id::new("command_search"))
            .fixed_pos(at + egui::vec2(-160.0, -20.0))
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                theme::dialog_frame().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(320.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text("Search commands…")
                            .desired_width(f32::INFINITY),
                    );
                    r.request_focus();
                    ui.add_space(4.0);
                    for (k, (_, c, icon, label, tip)) in results.iter().enumerate() {
                        let resp = ui
                            .horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                                crate::icons::paint(ui.painter(), rect, *icon, theme::ICON, true);
                                let text = egui::RichText::new(*label).color(if k == selected { theme::TEXT } else { theme::TEXT_WEAK });
                                let b = ui.add(egui::Button::new(text).frame(k == selected).fill(theme::ACCENT_DIM));
                                ui.weak(*tip);
                                b
                            })
                            .inner;
                        if resp.clicked() {
                            run = Some(*c);
                        }
                    }
                    if results.is_empty() {
                        ui.weak("No matching command");
                    }
                });
            });
        if enter {
            run = run.or(results.get(selected).map(|r| r.1));
        }
        match run {
            Some(c) => self.run(c),
            None if !esc => self.search = Some((query, selected)),
            None => {}
        }
    }

    fn expire_message(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        match &self.message {
            Some((text, is_error)) => {
                if self.message_seen.as_ref().is_none_or(|(t, _)| t != text) {
                    self.message_seen = Some((text.clone(), now));
                } else if let Some((_, since)) = &self.message_seen {
                    if !is_error && now - since > 6.0 {
                        self.message = None;
                    } else if !is_error {
                        ctx.request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
            }
            None => self.message_seen = None,
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            match &self.message {
                Some((text, true)) => {
                    ui.colored_label(theme::ERROR, format!("⚠ {text}"));
                }
                Some((text, false)) => {
                    ui.label(text);
                }
                None => {
                    ui.weak(self.hint());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak("mm");
                if let Some(c) = &self.doc.view.active_component {
                    ui.separator();
                    ui.colored_label(theme::ACCENT, format!("Active: {c}"));
                }
                if let Mode::Sketch(sess) = &self.mode {
                    if let Some(a) = &sess.analysis {
                        ui.separator();
                        if a.is_fully_constrained() {
                            ui.colored_label(theme::TEXT, "Fully constrained");
                        } else {
                            ui.colored_label(theme::ACCENT, format!("{} DOF", a.dof));
                        }
                    }
                }
            });
        });
    }
}

impl eframe::App for FerroApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.animation.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.stop_animation();
        }
        self.step_animation(&ctx);
        self.shortcuts(&ctx);
        self.expire_message(&ctx);
        if matches!(self.mode, Mode::Sketch(_)) && self.sketch_baseline.is_none() {
            self.sketch_baseline = Some(self.doc.clone());
        }
        if matches!(self.mode, Mode::Model | Mode::PickPlane | Mode::Sketch(_) | Mode::Measure(_)) {
            self.preview = None;
        }

        let bar = egui::Frame::new().fill(theme::BAR).inner_margin(egui::Margin::symmetric(8, 3));
        let panel = |margin: i8| egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::same(margin));

        egui::Panel::top("appbar").frame(bar).show(ui, |ui| self.app_bar(ui));
        egui::Panel::top("ribbon")
            .frame(panel(0).inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 2 }))
            .show(ui, |ui| self.ribbon(ui));
        egui::Panel::bottom("status").frame(bar).exact_size(24.0).show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("timeline").frame(panel(4)).resizable(false).show(ui, |ui| self.timeline(ui));
        egui::Panel::left("browser").frame(panel(8)).resizable(true).default_size(240.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.browser(ui));
        });
        if matches!(self.mode, Mode::Sketch(_)) {
            egui::Panel::right("sketch_palette")
                .frame(panel(10))
                .resizable(true)
                .default_size(280.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.sketch_palette(ui));
                });
        }

        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.viewport(ui));

        // Floating command dialogs.
        if self.show_params {
            self.parameters_window(&ctx);
        }
        match self.mode_kind() {
            ModeKind::Extrude => self.extrude_window(&ctx),
            ModeKind::Revolve => self.revolve_window(&ctx),
            ModeKind::Fillet => self.fillet_window(&ctx),
            ModeKind::Sketch => self.sketch_overlays(&ctx),
            ModeKind::Feature => self.feature_window(&ctx),
            ModeKind::Measure => self.measure_window(&ctx),
            ModeKind::Model | ModeKind::PickPlane => {}
        }
        self.section_window(&ctx);
        self.search_box(&ctx);
    }
}
