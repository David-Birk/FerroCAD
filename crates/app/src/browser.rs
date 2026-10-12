//! The browser on the left: a document tree with origin planes, bodies,
//! sketches and user parameters, with visibility toggles like Fusion's.

use crate::app::{Command, FerroApp};
use crate::features_ui::{FeatureType, Pick};
use crate::icons::{self, Icon};
use crate::theme;
use egui::{RichText, Ui};
use ferrocad_model::{FeatureId, FeatureKind, Plane};

enum Action {
    EditSketch(FeatureId),
    ToggleSketch(FeatureId),
    SketchOn(Plane, &'static str),
    ToggleBody(usize),
    SelectBody(String, bool),
    Run(Command),
    Edit(FeatureId),
    ToggleConstruction(FeatureId),
    SketchOnPlane(FeatureId),
    Appearance(String, String),
    AppearanceAll(Vec<String>, String),
    SetVisible(Vec<String>, bool),
    Activate(Option<String>),
}

/// One tree row: optional eye toggle, icon, label. Returns the label response.
fn row(ui: &mut Ui, visible: Option<bool>, icon: Icon, label: impl Into<egui::WidgetText>) -> (Option<egui::Response>, egui::Response) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let eye = visible.map(|v| {
            let icon = if v { Icon::EyeOpen } else { Icon::EyeClosed };
            icons::icon_button(ui, icon, 18.0, if v { "Hide" } else { "Show" }, true, false)
        });
        if visible.is_none() {
            ui.add_space(22.0);
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        icons::paint(ui.painter(), rect, icon, theme::ICON, true);
        let label = ui.add(egui::Label::new(label).sense(egui::Sense::click()).selectable(false));
        (eye, label)
    })
    .inner
}

/// Collapsible tree node with an icon in its header.
fn node(ui: &mut Ui, id: &str, icon: Icon, title: impl Into<egui::WidgetText>, open: bool, body: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open)
        .show_header(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            icons::paint(ui.painter(), rect, icon, theme::ICON, true);
            ui.label(title);
        })
        .body(body);
}

impl FerroApp {
    pub fn browser(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("BROWSER").small().strong().color(theme::TEXT_WEAK));
        ui.add_space(2.0);
        let mut action: Option<Action> = None;
        let can_sketch = self.can_run(Command::CreateSketch);

        let doc_name = std::path::Path::new(&self.doc_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        node(ui, "doc", Icon::Document, RichText::new(doc_name).strong(), true, |ui| {
            node(ui, "settings", Icon::Folder, "Document Settings", false, |ui| {
                row(ui, None, Icon::Dimension, "Units: mm");
            });

            node(ui, "origin", Icon::Origin, "Origin", false, |ui| {
                let (eye, _) = row(ui, Some(self.show_origin), Icon::Origin, "Origin planes");
                if eye.is_some_and(|r| r.clicked()) {
                    self.show_origin = !self.show_origin;
                }
                for (plane, label) in [(Plane::XY, "XY"), (Plane::XZ, "XZ"), (Plane::YZ, "YZ")] {
                    let (_, r) = row(ui, None, Icon::Plane, label);
                    let r = r.on_hover_text(if can_sketch { "Click to sketch on this plane" } else { "" });
                    if r.clicked() && can_sketch {
                        action = Some(Action::SketchOn(plane, label));
                    }
                }
            });

            let components = self.model.components.clone();
            if !components.is_empty() {
                node(ui, "components", Icon::Folder, format!("Components ({})", components.len()), true, |ui| {
                    for (name, members) in &components {
                        let visible = self.bodies.iter().any(|b| members.contains(&b.name) && b.visible);
                        let selected = self.selection.contains(&Pick::Body(name.clone()));
                        let active = self.doc.view.active_component.as_deref() == Some(name.as_str());
                        let shown = if active { format!("{name}  ● active") } else { name.clone() };
                        let text = if selected {
                            RichText::new(shown).color(crate::sketch_ui::COL_SELECTED)
                        } else if active {
                            RichText::new(shown).strong().color(theme::ACCENT)
                        } else {
                            RichText::new(shown).strong()
                        };
                        let (eye, label) = row(ui, Some(visible), Icon::Body, text);
                        if eye.is_some_and(|r| r.clicked()) {
                            action = Some(Action::SetVisible(members.clone(), !visible));
                        }
                        let label = label.on_hover_text(format!("{} – click to select · right-click for more", members.join(", ")));
                        if label.clicked() {
                            let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                            action = Some(Action::SelectBody(name.clone(), additive));
                        }
                        label.context_menu(|ui| {
                            if active {
                                if ui.button("Activate Root").on_hover_text("New bodies go to the root again").clicked() {
                                    action = Some(Action::Activate(None));
                                    ui.close();
                                }
                            } else if ui.button("Activate").on_hover_text("New bodies go into this component").clicked() {
                                action = Some(Action::Activate(Some(name.clone())));
                                ui.close();
                            }
                            ui.separator();
                            for (cmd, text) in [
                                (Command::Feature(FeatureType::Joint), "Joint"),
                                (Command::Feature(FeatureType::Move), "Move/Copy"),
                                (Command::Feature(FeatureType::Mirror), "Mirror"),
                                (Command::Feature(FeatureType::CircPattern), "Circular Pattern"),
                                (Command::RemoveBodies, "Remove"),
                            ] {
                                if ui.button(text).clicked() {
                                    self.selection = vec![Pick::Body(name.clone())];
                                    action = Some(Action::Run(cmd));
                                    ui.close();
                                }
                            }
                            ui.menu_button("Appearance", |ui| {
                                for (material, rgb, _) in theme::MATERIALS {
                                    let clicked = ui
                                        .horizontal(|ui| {
                                            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                                            ui.painter().rect_filled(r, 3.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                                            ui.add(egui::Button::new(*material).frame(false)).clicked()
                                        })
                                        .inner;
                                    if clicked {
                                        action = Some(Action::AppearanceAll(members.clone(), material.to_string()));
                                        ui.close();
                                    }
                                }
                            });
                        });
                        for m in members {
                            ui.horizontal(|ui| {
                                ui.add_space(18.0);
                                ui.weak(format!("└ {m}"));
                            });
                        }
                    }
                });
            }

            let loose: Vec<usize> =
                (0..self.bodies.len()).filter(|&i| self.model.component_of(&self.bodies[i].name).is_none()).collect();
            node(ui, "bodies", Icon::Folder, format!("Bodies ({})", loose.len()), true, |ui| {
                if loose.is_empty() {
                    ui.weak("    none");
                }
                for (bi, b) in self.bodies.iter().enumerate().filter(|(i, _)| loose.contains(i)) {
                    let selected = self.selection.contains(&Pick::Body(b.name.clone()));
                    let text = if selected {
                        RichText::new(b.name.as_str()).color(crate::sketch_ui::COL_SELECTED)
                    } else {
                        RichText::new(b.name.as_str())
                    };
                    let (eye, label) = row(ui, Some(b.visible), Icon::Body, text);
                    if eye.is_some_and(|r| r.clicked()) {
                        action = Some(Action::ToggleBody(bi));
                    }
                    let label = label.on_hover_text("Click to select (Shift adds) · right-click for more");
                    if label.clicked() {
                        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                        action = Some(Action::SelectBody(b.name.clone(), additive));
                    }
                    label.context_menu(|ui| {
                        if !selected {
                            action = Some(Action::SelectBody(b.name.clone(), false));
                        }
                        for (cmd, text) in [
                            (Command::Feature(FeatureType::Move), "Move/Copy"),
                            (Command::Feature(FeatureType::Mirror), "Mirror"),
                            (Command::Feature(FeatureType::RectPattern), "Rectangular Pattern"),
                            (Command::Feature(FeatureType::CircPattern), "Circular Pattern"),
                            (Command::Feature(FeatureType::Split), "Split Body"),
                            (Command::Feature(FeatureType::Combine), "Combine"),
                            (Command::Properties, "Properties"),
                            (Command::RemoveBodies, "Remove"),
                        ] {
                            if ui.button(text).clicked() {
                                // Select first, then run (handled below).
                                if !selected {
                                    self.selection = vec![Pick::Body(b.name.clone())];
                                }
                                action = Some(Action::Run(cmd));
                                ui.close();
                            }
                        }
                        if ui.button(if b.visible { "Hide" } else { "Show" }).clicked() {
                            action = Some(Action::ToggleBody(bi));
                            ui.close();
                        }
                        ui.menu_button("Appearance", |ui| {
                            for (name, rgb, _) in theme::MATERIALS {
                                let clicked = ui
                                    .horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                                        ui.painter().rect_filled(r, 3.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                                        ui.add(egui::Button::new(*name).frame(false)).clicked()
                                    })
                                    .inner;
                                if clicked {
                                    action = Some(Action::Appearance(b.name.clone(), name.to_string()));
                                    ui.close();
                                }
                            }
                        });
                    });
                }
            });

            let sketches: Vec<(FeatureId, String, bool)> = self.doc.features()[..self.doc.marker()]
                .iter()
                .filter(|f| matches!(f.kind, FeatureKind::Sketch(_)))
                .map(|f| (f.id, f.name.clone(), !self.hidden_sketches.contains(&f.id)))
                .collect();
            node(ui, "sketches", Icon::Folder, format!("Sketches ({})", sketches.len()), true, |ui| {
                if sketches.is_empty() {
                    ui.weak("    none yet");
                }
                for (id, name, visible) in sketches {
                    let (eye, label) = row(ui, Some(visible), Icon::Sketch, name);
                    if eye.is_some_and(|r| r.clicked()) {
                        action = Some(Action::ToggleSketch(id));
                    }
                    let label = label.on_hover_text("Double-click to edit");
                    if label.double_clicked() {
                        action = Some(Action::EditSketch(id));
                    }
                    label.context_menu(|ui| {
                        if ui.button("Edit Sketch").clicked() {
                            action = Some(Action::EditSketch(id));
                            ui.close();
                        }
                        if ui.button(if visible { "Hide" } else { "Show" }).clicked() {
                            action = Some(Action::ToggleSketch(id));
                            ui.close();
                        }
                    });
                }
            });

            let construction: Vec<(FeatureId, String, bool, bool)> = self.doc.features()[..self.doc.marker()]
                .iter()
                .filter(|f| matches!(f.kind, FeatureKind::Plane(_) | FeatureKind::Axis(_)))
                .map(|f| {
                    (f.id, f.name.clone(), matches!(f.kind, FeatureKind::Plane(_)), !self.hidden_construction.contains(&f.id))
                })
                .collect();
            if !construction.is_empty() {
                node(ui, "construction", Icon::Folder, format!("Construction ({})", construction.len()), true, |ui| {
                    for (id, name, is_plane, visible) in construction {
                        let icon = if is_plane { Icon::Plane } else { Icon::Axis };
                        let (eye, label) = row(ui, Some(visible), icon, name);
                        if eye.is_some_and(|r| r.clicked()) {
                            action = Some(Action::ToggleConstruction(id));
                        }
                        let label = label.on_hover_text("Double-click to edit · right-click for more");
                        if label.double_clicked() {
                            action = Some(Action::Edit(id));
                        }
                        label.context_menu(|ui| {
                            if is_plane && ui.add_enabled(can_sketch, egui::Button::new("Create Sketch Here")).clicked() {
                                action = Some(Action::SketchOnPlane(id));
                                ui.close();
                            }
                            if ui.add_enabled(can_sketch, egui::Button::new("Edit")).clicked() {
                                action = Some(Action::Edit(id));
                                ui.close();
                            }
                        });
                    }
                });
            }

            node(ui, "params", Icon::Parameters, "User Parameters", true, |ui| {
                let rows: Vec<(String, String)> =
                    self.doc.params.iter().map(|p| (p.name.clone(), p.expression.clone())).collect();
                egui::Grid::new("browser_params").num_columns(2).show(ui, |ui| {
                    for (name, expr) in rows {
                        ui.horizontal(|ui| {
                            ui.add_space(22.0);
                            ui.label(&name);
                        });
                        let value = self.doc.params.value(&name).map(|v| format!("{v:.3}")).unwrap_or("?".into());
                        ui.weak(expr).on_hover_text(format!("= {value}"));
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    if ui.small_button("Change Parameters…").clicked() {
                        self.show_params = true;
                        self.param_edits.clear();
                    }
                });
            });
        });

        match action {
            Some(Action::EditSketch(id)) if can_sketch => self.edit_feature(id),
            Some(Action::ToggleSketch(id)) => {
                if !self.hidden_sketches.remove(&id) {
                    self.hidden_sketches.insert(id);
                }
            }
            Some(Action::SketchOn(plane, label)) => self.start_sketch(plane, &format!("the {label} plane")),
            Some(Action::ToggleBody(bi)) => {
                if let Some(b) = self.bodies.get_mut(bi) {
                    b.visible = !b.visible;
                }
            }
            Some(Action::SelectBody(name, additive)) => {
                let p = Pick::Body(name);
                if additive {
                    if let Some(i) = self.selection.iter().position(|q| *q == p) {
                        self.selection.remove(i);
                    } else {
                        self.selection.push(p);
                    }
                } else {
                    self.selection = vec![p];
                }
            }
            Some(Action::Run(cmd)) => self.run(cmd),
            Some(Action::Appearance(body, material)) => self.set_appearance(&body, Some(&material)),
            Some(Action::AppearanceAll(bodies, material)) => self.set_appearances(&bodies, Some(&material)),
            Some(Action::Activate(c)) => {
                match &c {
                    Some(name) => self.info(format!("{name} is active – new bodies go into it")),
                    None => self.info("root is active"),
                }
                self.doc.view.active_component = c;
            }
            Some(Action::SetVisible(bodies, visible)) => {
                for b in self.bodies.iter_mut().filter(|b| bodies.contains(&b.name)) {
                    b.visible = visible;
                }
            }
            Some(Action::SketchOnPlane(id)) => self.start_sketch_on(ferrocad_model::PlaneRef::Construction(id)),
            Some(Action::Edit(id)) if can_sketch => self.edit_feature(id),
            Some(Action::ToggleConstruction(id))
                if !self.hidden_construction.remove(&id) => {
                    self.hidden_construction.insert(id);
                }
            _ => {}
        }
    }
}
