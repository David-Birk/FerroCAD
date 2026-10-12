//! Application bar (file, undo/redo, document name) and the Fusion-style
//! ribbon: a workspace tab plus groups of large tool buttons, each group with
//! a dropdown that lists its commands.

use crate::app::{Command, FerroApp, Mode};
use crate::features_ui::FeatureType as F;
use crate::icons::{self, Icon};
use crate::sketch_ui::{ConstraintKind, SketchCommand, Tool};
use crate::theme;
use egui::{RichText, Ui};

/// A ribbon entry: command, icon, label, tooltip (with shortcut).
struct Entry(Command, Icon, &'static str, &'static str);

const SOLID_CREATE: &[Entry] = &[
    Entry(Command::CreateSketch, Icon::Sketch, "Sketch", "Create Sketch – pick a plane or flat face (S)"),
    Entry(Command::Extrude, Icon::Extrude, "Extrude", "Extrude a sketch profile (E)"),
    Entry(Command::Revolve, Icon::Revolve, "Revolve", "Revolve a sketch profile around an axis"),
    Entry(Command::Feature(F::Loft), Icon::Loft, "Loft", "Loft through two or more profiles"),
    Entry(Command::Feature(F::Hole), Icon::Hole, "Hole", "Drill a hole into a flat face (H)"),
];
const SOLID_CREATE_MORE: &[Entry] = &[
    Entry(Command::Feature(F::Sweep), Icon::Sweep, "Sweep", "Sweep a profile along a sketch path"),
    Entry(Command::Feature(F::RectPattern), Icon::RectPattern, "Rectangular Pattern", "Copy bodies along one or two directions"),
    Entry(Command::Feature(F::CircPattern), Icon::CircPattern, "Circular Pattern", "Copy bodies around an axis"),
    Entry(Command::Feature(F::Mirror), Icon::Mirror, "Mirror", "Mirror bodies at a plane"),
];
const SOLID_MODIFY: &[Entry] = &[
    Entry(Command::Feature(F::PressPull), Icon::PressPull, "Press Pull", "Push or pull a flat face (Q)"),
    Entry(Command::Fillet, Icon::Fillet, "Fillet", "Round edges (F)"),
    Entry(Command::Chamfer, Icon::Chamfer, "Chamfer", "Bevel edges"),
    Entry(Command::Feature(F::Shell), Icon::Shell, "Shell", "Hollow a body, removing faces"),
    Entry(Command::Feature(F::Combine), Icon::Combine, "Combine", "Join, cut or intersect bodies"),
    Entry(Command::Parameters, Icon::Parameters, "Parameters", "Change user parameters"),
];
const SOLID_MODIFY_MORE: &[Entry] = &[
    Entry(Command::Feature(F::Draft), Icon::Draft, "Draft", "Tilt faces for mould release"),
    Entry(Command::Feature(F::Split), Icon::Split, "Split Body", "Split a body at a plane"),
    Entry(Command::Feature(F::Move), Icon::Move, "Move/Copy", "Move, rotate or copy bodies (M)"),
    Entry(Command::RemoveBodies, Icon::Delete, "Remove", "Remove the selected bodies (Del)"),
];
const SOLID_CONSTRUCT: &[Entry] = &[
    Entry(Command::Feature(F::Plane), Icon::Plane, "Plane", "Offset plane / plane at angle"),
    Entry(Command::Feature(F::AxisEdge), Icon::Axis, "Axis", "Axis through an edge (straight or circular)"),
];
const SOLID_CONSTRUCT_MORE: &[Entry] = &[
    Entry(Command::Feature(F::Midplane), Icon::Midplane, "Midplane", "Plane halfway between two parallel planes or faces"),
    Entry(Command::Feature(F::AxisFace), Icon::Axis, "Axis Normal to Face", "Axis perpendicular to a face at a point"),
];
const SOLID_ASSEMBLE: &[Entry] = &[
    Entry(Command::NewComponent, Icon::Body, "Component", "New component from the selected bodies"),
    Entry(Command::Feature(F::Joint), Icon::Joint, "Joint", "Place bodies at each other (rigid, revolute, slider)"),
    Entry(Command::Interference, Icon::Interference, "Interference", "Find overlapping bodies"),
];
const SOLID_ASSEMBLE_MORE: &[Entry] = &[];
const SOLID_INSPECT: &[Entry] = &[
    Entry(Command::Measure, Icon::Measure, "Measure", "Measure distances, lengths, diameters (I)"),
    Entry(Command::Section, Icon::Section, "Section", "Section analysis: cut the view open"),
];
const SOLID_INSPECT_MORE: &[Entry] =
    &[Entry(Command::Properties, Icon::Properties, "Properties", "Volume, area and centre of mass")];

const SKETCH_CREATE: &[Entry] = &[
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Line)), Icon::Line, "Line", "Line (L) – click points, Esc to stop"),
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Rectangle)), Icon::Rectangle, "Rectangle", "2-point rectangle (R)"),
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Circle)), Icon::Circle, "Circle", "Center-diameter circle (C)"),
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Arc)), Icon::Arc, "Arc", "Center-point arc (A)"),
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Dimension)), Icon::Dimension, "Dimension", "Sketch dimension (D)"),
];
const fn tool(t: Tool) -> Command {
    Command::Sketch(SketchCommand::Tool(t))
}

const SKETCH_CREATE_MORE: &[Entry] = &[
    Entry(tool(Tool::CenterRectangle), Icon::CenterRect, "Center Rectangle", "Rectangle from its centre"),
    Entry(tool(Tool::Arc3), Icon::Arc3, "3-Point Arc", "Arc through start, end and a point"),
    Entry(tool(Tool::Polygon), Icon::Polygon, "Polygon", "Regular polygon (sides in the palette)"),
    Entry(tool(Tool::Spline), Icon::Spline, "Spline", "Fit-point spline – Enter or double-click to finish"),
    Entry(tool(Tool::Slot), Icon::Slot, "Slot", "Centre-to-centre slot"),
    Entry(tool(Tool::Project), Icon::Project, "Project", "Bring body edges into the sketch (P)"),
];
const SKETCH_MODIFY_MORE: &[Entry] = &[
    Entry(tool(Tool::CircPattern), Icon::CircPattern, "Circular Pattern", "Copy the selected curves around a point"),
    Entry(tool(Tool::RectPattern), Icon::RectPattern, "Rectangular Pattern", "Copy the selected curves in a row"),
];
const SKETCH_MODIFY: &[Entry] = &[
    Entry(Command::Sketch(SketchCommand::Tool(Tool::Select)), Icon::Select, "Select", "Select and drag geometry (Esc)"),
    Entry(tool(Tool::Trim), Icon::Trim, "Trim", "Remove a curve piece between intersections (T)"),
    Entry(tool(Tool::Extend), Icon::Extend, "Extend", "Extend a line to the next curve"),
    Entry(tool(Tool::Fillet), Icon::Fillet, "Fillet", "Round a sketch corner"),
    Entry(tool(Tool::Offset), Icon::Offset, "Offset", "Offset a chain of curves (O)"),
    Entry(tool(Tool::Mirror), Icon::Mirror, "Mirror", "Mirror the selected curves at a line"),
    Entry(Command::Sketch(SketchCommand::Construction), Icon::Construction, "Construction", "Toggle construction (X)"),
    Entry(Command::Sketch(SketchCommand::Delete), Icon::Delete, "Delete", "Delete selection (Del)"),
];

/// Every command with icon, name and description – for the command search.
pub fn all_commands(app: &FerroApp) -> Vec<(Command, Icon, &'static str, &'static str)> {
    let mut out: Vec<(Command, Icon, &'static str, &'static str)> = Vec::new();
    let tables: &[&[Entry]] = &[
        SOLID_CREATE,
        SOLID_CREATE_MORE,
        SOLID_MODIFY,
        SOLID_MODIFY_MORE,
        SOLID_CONSTRUCT,
        SOLID_CONSTRUCT_MORE,
        SOLID_ASSEMBLE,
        SOLID_INSPECT,
        SOLID_INSPECT_MORE,
        SKETCH_CREATE,
        SKETCH_CREATE_MORE,
        SKETCH_MODIFY,
        SKETCH_MODIFY_MORE,
    ];
    for table in tables {
        for Entry(c, i, l, t) in table.iter() {
            out.push((*c, *i, *l, *t));
        }
    }
    for k in ConstraintKind::ALL {
        out.push((Command::Sketch(SketchCommand::Constrain(k)), k.icon(), k.label(), k.tooltip()));
    }
    out.extend([
        (Command::Fillet, Icon::Fillet, "Fillet", "Round edges (F)"),
        (Command::FinishSketch, Icon::Finish, "Finish Sketch", "Leave the sketch"),
        (Command::Undo, Icon::Undo, "Undo", "Ctrl+Z"),
        (Command::Redo, Icon::Redo, "Redo", "Ctrl+Shift+Z"),
        (Command::New, Icon::New, "New Document", ""),
        (Command::Open, Icon::Open, "Open…", ""),
        (Command::Save, Icon::Save, "Save", "Ctrl+S"),
        (Command::Home, Icon::Home, "Home View", "F6"),
        (Command::Fit, Icon::Fit, "Fit", "Shift+F"),
        (Command::LookAt, Icon::LookAt, "Look At", "Look at the sketch plane"),
        (Command::ToggleGrid, Icon::Grid, "Toggle Grid", ""),
        (Command::ToggleEdges, Icon::Edges, "Toggle Edges", ""),
        (Command::SketchOnFace, Icon::Sketch, "Sketch on Selected Face", ""),
        (Command::ImportStep, Icon::Open, "Import STEP", "Insert bodies from a STEP file"),
    ]);
    // Unique by command, available now.
    let mut seen = Vec::new();
    out.retain(|(c, ..)| {
        let new = !seen.contains(c);
        seen.push(*c);
        new && app.can_run(*c)
    });
    out
}

/// Fuzzy score of `query` against `text`: all query characters in order;
/// prefix and word-start matches score higher. None if no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut prev_match = false;
    for (i, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            let word_start = i == 0 || !t[i - 1].is_alphanumeric();
            score += if word_start { 10 } else if prev_match { 5 } else { 1 };
            qi += 1;
            prev_match = true;
        } else {
            prev_match = false;
        }
    }
    (qi == q.len()).then_some(score - t.len() as i32 / 8)
}

impl FerroApp {
    pub fn app_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ferrocad").strong().color(theme::ORANGE));
            ui.add_space(6.0);
            ui.menu_button("File", |ui| self.file_menu(ui));
            ui.add_space(4.0);
            for (cmd, icon, tip) in [
                (Command::New, Icon::New, "New document"),
                (Command::Open, Icon::Open, "Open…"),
                (Command::Save, Icon::Save, "Save (Ctrl+S)"),
            ] {
                if icons::icon_button(ui, icon, 24.0, tip, self.can_run(cmd), false).clicked() {
                    self.run(cmd);
                }
            }
            ui.separator();
            let undo_tip = self.history.undo_label().map(|l| format!("Undo {l} (Ctrl+Z)")).unwrap_or_default();
            let redo_tip = self.history.redo_label().map(|l| format!("Redo {l} (Ctrl+Shift+Z)")).unwrap_or_default();
            if icons::icon_button(ui, Icon::Undo, 24.0, &undo_tip, self.can_run(Command::Undo), false).clicked() {
                self.run(Command::Undo);
            }
            if icons::icon_button(ui, Icon::Redo, 24.0, &redo_tip, self.can_run(Command::Redo), false).clicked() {
                self.run(Command::Redo);
            }

            // Document name in the middle of the bar.
            let name = std::path::Path::new(&self.doc_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.doc_path.clone());
            let rect = ui.max_rect();
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(13.0),
                theme::TEXT_WEAK,
            );
        });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        ui.set_min_width(240.0);
        let item = |ui: &mut Ui, text: &str, shortcut: &str| {
            ui.add(egui::Button::new(text).shortcut_text(shortcut).min_size(egui::vec2(220.0, 0.0))).clicked()
        };
        if item(ui, "New", "") {
            self.run(Command::New);
            ui.close();
        }
        if item(ui, "Open…", "") {
            ui.close();
            self.run(Command::Open);
        }
        if item(ui, "Save", "Ctrl+S") {
            ui.close();
            self.run(Command::Save);
        }
        if item(ui, "Save As…", "") {
            ui.close();
            self.save_as();
        }
        ui.separator();
        if item(ui, "Import STEP…", "") {
            ui.close();
            self.run(Command::ImportStep);
        }
        if item(ui, "Export STEP…", "") {
            ui.close();
            self.export_with_dialog(false);
        }
        if item(ui, "Export STL…", "") {
            ui.close();
            self.export_with_dialog(true);
        }
        ui.separator();
        ui.collapsing("Paths (if no file dialog is available)", |ui| {
            ui.label("Document");
            ui.add(egui::TextEdit::singleline(&mut self.doc_path).desired_width(220.0));
            ui.horizontal(|ui| {
                if ui.button("Open path").clicked() {
                    self.open_document_from_path();
                    ui.close();
                }
                if ui.button("Save to path").clicked() {
                    self.save_document_to_path();
                    ui.close();
                }
            });
            ui.label("Export");
            ui.add(egui::TextEdit::singleline(&mut self.export_path).desired_width(220.0));
            ui.horizontal(|ui| {
                if ui.button("STEP").clicked() {
                    self.export(false);
                    ui.close();
                }
                if ui.button("STL").clicked() {
                    self.export(true);
                    ui.close();
                }
            });
        });
    }

    pub fn ribbon(&mut self, ui: &mut Ui) {
        let sketching = matches!(self.mode, Mode::Sketch(_));
        // Workspace tabs.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 18.0;
            tab(ui, "SOLID", !sketching);
            if sketching {
                tab(ui, "SKETCH", true);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if sketching {
                self.group(ui, "CREATE", SKETCH_CREATE, SKETCH_CREATE_MORE);
                divider(ui);
                self.group(ui, "MODIFY", SKETCH_MODIFY, SKETCH_MODIFY_MORE);
                divider(ui);
                self.constraints_group(ui);
                divider(ui);
                self.finish_group(ui);
            } else {
                self.group(ui, "CREATE", SOLID_CREATE, SOLID_CREATE_MORE);
                divider(ui);
                self.group(ui, "MODIFY", SOLID_MODIFY, SOLID_MODIFY_MORE);
                divider(ui);
                self.group(ui, "CONSTRUCT", SOLID_CONSTRUCT, SOLID_CONSTRUCT_MORE);
                divider(ui);
                self.group(ui, "ASSEMBLE", SOLID_ASSEMBLE, SOLID_ASSEMBLE_MORE);
                divider(ui);
                self.group(ui, "INSPECT", SOLID_INSPECT, SOLID_INSPECT_MORE);
            }
        });
    }

    fn is_active(&self, cmd: Command) -> bool {
        match (&self.mode, cmd) {
            (Mode::Sketch(s), Command::Sketch(SketchCommand::Tool(t))) => s.tool == t,
            (Mode::Sketch(s), Command::Sketch(SketchCommand::Constrain(k))) => s.tool == Tool::Constrain(k),
            (Mode::PickPlane, Command::CreateSketch) => true,
            (Mode::Extrude(_), Command::Extrude) => true,
            (Mode::Revolve(_), Command::Revolve) => true,
            (Mode::Fillet(f), Command::Fillet) => !f.chamfer,
            (Mode::Fillet(f), Command::Chamfer) => f.chamfer,
            (Mode::Feature(f), Command::Feature(t)) => f.ty == t,
            (Mode::Measure(_), Command::Measure) => true,
            (_, Command::Section) => self.section.is_some(),
            (_, Command::Parameters) => self.show_params,
            _ => false,
        }
    }

    fn group(&mut self, ui: &mut Ui, title: &str, entries: &[Entry], more: &[Entry]) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for Entry(cmd, icon, label, tip) in entries {
                    let enabled = self.can_run(*cmd);
                    let active = self.is_active(*cmd);
                    if icons::ribbon_button(ui, *icon, label, tip, enabled, active).clicked() {
                        self.run(*cmd);
                    }
                }
            });
            let all = entries.iter().chain(more).map(|Entry(c, i, l, t)| (*c, *i, *l, *t)).collect();
            self.group_menu(ui, title, all);
        });
    }

    /// The "CREATE ▾" dropdown under a group, listing all its commands.
    fn group_menu(&mut self, ui: &mut Ui, title: &str, entries: Vec<(Command, Icon, &str, &str)>) {
        let text = RichText::new(format!("{title} ▾")).small().color(theme::TEXT_WEAK);
        ui.menu_button(text, |ui| {
            ui.set_min_width(240.0);
            for (cmd, icon, label, tip) in entries {
                let enabled = self.can_run(cmd);
                let clicked = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            icons::paint(ui.painter(), rect, icon, theme::ICON, enabled);
                            ui.add(egui::Button::new(label).frame(false)).on_hover_text(tip).clicked()
                        })
                        .inner
                    })
                    .inner;
                if clicked {
                    self.run(cmd);
                    ui.close();
                }
            }
        });
    }

    fn constraints_group(&mut self, ui: &mut Ui) {
        let Mode::Sketch(sess) = &self.mode else { return };
        let current = sess.tool;
        let mut clicked = None;
        ui.vertical(|ui| {
            // Two rows of small icons, like Fusion's constraint panel.
            let kinds = ConstraintKind::ALL;
            for row in kinds.chunks(5) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for &k in row {
                        let active = current == Tool::Constrain(k);
                        if icons::icon_button(ui, k.icon(), 25.0, k.tooltip(), true, active).clicked() {
                            clicked = Some(k);
                        }
                    }
                });
            }
            let entries = ConstraintKind::ALL
                .iter()
                .map(|k| (Command::Sketch(SketchCommand::Constrain(*k)), k.icon(), k.label(), k.tooltip()))
                .collect();
            self.group_menu(ui, "CONSTRAINTS", entries);
        });
        if let Some(k) = clicked {
            self.run(Command::Sketch(SketchCommand::Constrain(k)));
        }
    }

    fn finish_group(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            if icons::ribbon_button(ui, Icon::Finish, "Finish Sketch", "Finish the sketch and return to the model", true, false)
                .clicked()
            {
                self.run(Command::FinishSketch);
            }
            ui.label(RichText::new("FINISH SKETCH").small().color(theme::TEXT_WEAK));
        });
    }
}

fn tab(ui: &mut Ui, text: &str, active: bool) {
    let color = if active { theme::TEXT } else { theme::TEXT_WEAK };
    let r = ui.label(RichText::new(text).small().strong().color(color));
    if active {
        let y = r.rect.max.y + 1.0;
        ui.painter().line_segment(
            [egui::pos2(r.rect.min.x, y), egui::pos2(r.rect.max.x, y)],
            egui::Stroke::new(2.0, theme::ACCENT),
        );
    }
}

fn divider(ui: &mut Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 70.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BORDER);
    ui.add_space(4.0);
}
