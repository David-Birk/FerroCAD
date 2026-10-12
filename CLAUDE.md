# ferrocad – context for Claude Code

Native parametric CAD for Linux in Rust, modelled on Fusion 360 (workflow and UI): sketch →
constraints/dimensions → solid features, driven by a feature timeline and user parameters.

The user (David) runs Arch Linux, knows Fusion 360 well, and writes German or English. The README is
in German; code and comments are in English.

## Build status

Everything builds on the user's machine (Arch, system OCCT 7.9.3, egui/eframe 0.36.2, wgpu) and
`cargo test --workspace` passes: headless crates, the real OCCT kernel (`kernel-occt` tests) and
end-to-end UI tests (`app/src/ui_tests.rs`, egui_kittest with simulated clicks/keys).

Build setup quirks:
- `.cargo/config.toml` sets `CMAKE_POLICY_VERSION_MINIMUM=3.5`; CMake 4 rejects the
  `cmake_minimum_required(3.1)` in opencascade-sys's OCCT detection project otherwise.
- `vendor/` holds local forks wired in via `[patch]` in `Cargo.toml`:
  - `opencascade-sys`: OCCT 7.9 made `TopoDS` a namespace → C++ shim `TopoDS_Cast`
    (`include/topo_ds.hxx`). **`include/ferro.hxx` + `src/ferro.rs`** add whole modelling operations
    (fillet, boolean, shell, transform, profile face, prism, revolve, loft, sweep, face lookup, mass
    properties) that catch OCCT exceptions and check `IsDone()`/validity, returning cxx `Result`s.
    The bindings compile as C++11.
  - `opencascade` (+ `kicad-parser`, its path dependency): adds `src/safe.rs`, the Rust API over
    `ferro.hxx`. **The kernel must only use `opencascade::safe` for modelling** – a raw
    opencascade-rs call that throws aborts the process.
- Use rustup, not Arch's `rust` package (they conflict).
- APIs that changed recently in egui 0.36: `eframe::App::ui(&mut self, ui, frame)`,
  `egui::Panel::top/bottom/left/right(id)…show(ui, …)`, `ui.close()`, `TextEdit::frame(Frame)`.
  wgpu 30: `depth_write_enabled`/`depth_compare` are `Option`s, vertex `buffers` are
  `&[Option<VertexBufferLayout>]`, `multiview_mask`, `PipelineLayoutDescriptor::immediate_size`.
  When an API doesn't match, read the real source in `~/.cargo/registry/src/*/` instead of guessing.

## Commands

```sh
sudo pacman -S --needed cmake clang opencascade        # OCCT must be 7.8+ but < 8; Rust via rustup
cargo test --workspace --release                        # everything incl. OCCT and UI tests
cargo test -p ferrocad-sketch -p ferrocad-model -p ferrocad-view   # fast headless subset
cargo run --release [-- file.fcad]                      # default member = crates/app
cargo run -p ferrocad-model --example demo -- demo.fcad # writes a demo part using many features
SHOTS=/tmp/shots cargo test --release -p ferrocad screenshots -- --ignored   # render UI states to PNGs
cargo clippy --workspace --all-targets                  # keep clean
```

## Architecture

```
crates/
├── sketch/       constraint solver, regions/profiles, editing ops (no deps)
│   ├── solver.rs    damped Gauss–Newton, DOF analysis
│   ├── region.rs    planar arrangement → profiles (splines enter as tagged line pieces
│   │                and are merged back into one Segment::Spline per loop)
│   └── ops.rs       trim, extend, corner fillet, offset, mirror (Symmetric constraint),
│                    copies, polygon, slot, centre rectangle, 3-point arc
├── model/        params, expressions, features, timeline rebuild, Kernel trait,
│                 undo history, .fcad JSON I/O (own JSON parser, no serde)
│   └── examples/demo.rs
├── view/         orbit camera (Z up), projection, ray/mesh picking, face components,
│                 mesh clipping (section view), software shading (fallback renderer)
├── kernel-occt/  `impl Kernel for OcctKernel` via `opencascade::safe` (Solid = Rc<Shape>)
└── app/          eframe/egui UI (Fusion-style dark theme)
    ├── app.rs        FerroApp state, Mode enum, Command enum + run(), shortcuts, camera
    │                 animation, live preview, undo/redo, file ops (rfd dialogs), layout
    ├── theme.rs      colours, egui style, fonts (`setup`), dialog frame, buttons
    ├── icons.rs      vector icons drawn with the painter + icon/ribbon button widgets
    ├── ribbon.rs     application bar, File menu, ribbon (SOLID / SKETCH tabs, groups + ▾ menus)
    ├── browser.rs    document tree: origin, bodies (select/hide/menu), sketches,
    │                 construction geometry, parameters
    ├── viewport.rs   navigation, body rendering dispatch, grid, plane picking for sketches,
    │                 model selection, construction display, ViewCube, nav bar, context menu
    ├── render3d.rs   wgpu pipeline (depth buffer, MSAA, section clipping in the shader)
    │                 as an egui_wgpu paint callback; painter fallback if no wgpu
    ├── picking.rs    face/edge/body/profile/curve hit-testing shared by all tools
    ├── sketch_ui.rs  sketch tools, typed value fields while drawing, snapping, constraint
    │                 tools + badges, dimensions with inline editor, sketch palette
    ├── dialogs.rs    extrude / revolve / fillet / parameters dialogs, shared dialog widgets
    ├── features_ui.rs generic command dialogs for all other features: declarative inputs
    │                 (selections + expressions), picking, highlighting, preview, editing
    ├── inspect.rs    measure, section analysis, body properties, interference
    ├── manipulators.rs drag arrows for extrude / press pull
    ├── timeline.rs   feature icon chips, draggable rollback marker, context menu
    └── ui_tests.rs   headless end-to-end tests (+ ignored `screenshots` test)
```

UI conventions:
- Every user action is a `Command` routed through `FerroApp::run`, so ribbon, context menus and
  shortcuts stay in sync. `can_run` decides availability.
- New solid/construction features go through `features_ui.rs`: add a `FeatureType`, its inputs in
  `FeatureSession::blank`, the mapping in `build` and `edit`. Picking/preview/dialog come for free.
- Icons come from `icons.rs` instead of Unicode glyphs (egui's bundled fonts lack most symbols).
- Model-mode selection (`FerroApp::selection`) pre-fills commands that start from it.
- Bodies are drawn by `render3d` with a real depth buffer. Translucent fills (face highlights,
  planes) go into `FerroApp::overlay` and are drawn depth-tested in the same pass; everything drawn
  with the egui painter (sketches, lines, dimensions) is an overlay without depth test.
- Tangent constraints between a line and an arc that share an end point use a perpendicularity
  residual (the distance form is degenerate at tangency and showed up as "redundant").

Dependency direction: `sketch` ← `model` ← `view`, and `kernel-occt` ← `app`. Keep `model` and
`view` free of egui and OCCT so they stay testable headless. Model tests use a mock kernel
(`crates/model/tests/common/mod.rs`).

## Key design decisions

- **Solver** (`sketch/src/solver.rs`): every coordinate and radius is a scalar parameter, and each
  constraint is a residual. Damped minimum-norm Gauss–Newton with a numeric Jacobian, so
  under-constrained geometry moves minimally. DOF and "fully determined" come from an RREF null-space
  analysis. A failed solve restores the previous state. An `Analysis` can be stale after edits; use
  `analysis_is_current` (lookups tolerate stale ones).
- **Ids are indices that never shift.** Deleted entities and constraints keep their slot
  (`alive=false` / `None`). `SketchFeature::driven` stores `ConstraintId`s; after sketch ops that
  delete constraints, drop stale `driven` entries (the UI does this when a sketch changes).
- **Splines** are fit-point centripetal Catmull–Rom curves (`spline_samples`); kernels get dense
  points (`Seg3::Spline`) and interpolate a B-spline (`GeomAPI_Interpolate`).
- **Profile selection is a point inside the region** (`ProfileRef`), not an index.
- **References into solids:** edges by midpoint (`EdgeRef`, `EDGE_MATCH_TOLERANCE = 1 mm`); faces by
  point + outward normal (`FaceRef`), found again along the line through the point in the normal
  direction, so they survive the face moving along its normal. Bodies by name (`Body1`, …).
- **Projected edges** (`model/src/project.rs`): lines, circles and arcs projected from body edges
  are stored as `SketchFeature::projections` and re-positioned on every rebuild (nearest edge of the
  same shape/direction). Other curves become fixed polylines.
- **View state** (hidden bodies/sketches/construction, body appearances) is saved in
  `Document::view`, outside the recipe. Materials (colour + density) live in `theme::MATERIALS`.
- **Joints** (`JointFeature`) move bodies so origin `from` (circular-edge centre or face point) mates
  with `to`: align z axes (opposed unless `flip`), translate, then turn by `angle` / shift by
  `offset` along the fixed axis.
- **Components** (`ComponentFeature`, `Model::components`): named groups of bodies. Body lists in
  mirror/pattern/move/joint/remove accept component names (`document::expand`). In the UI a component
  pick is `Pick::Body(component name)`; `SelKind::Body` picks components, `SelKind::SingleBody` doesn't.
  Joint animation rewrites the expression every frame without undo steps and restores it on stop.
- **STEP import** is a timeline feature (`ImportFeature`, path stored) giving one body per solid; the
  file is re-read on rebuild.
- **Drag handles** (`manipulators.rs`) rewrite the dialog's distance expression while dragging.
- **Attached sketches / planes:** `SketchFeature::attach` (`PlaneRef`: origin plane, face,
  construction plane) is resolved on every rebuild; construction planes/axes live in
  `Model::planes/axes`.
- **Rebuild** replays the timeline up to the marker. A failing feature becomes
  `FeatureStatus::Error` and later features still run. `FeatureKind::references()` drives delete and
  reorder checks.
- **Undo** stores whole-document snapshots (`model::History`). In the app, use
  `self.checkpoint(label)` before mutating the document, or `self.record(label, before)`.
  Sketch mode uses `sketch_baseline` plus `commit_sketch_edit()`.
- **Units:** mm and degrees in expressions; radians inside the solver.
- **.fcad** is pretty-printed JSON holding only the recipe. Solids are rebuilt on load. Loading
  validates every reference and must never panic.

## Known issues / next steps

1. Topological naming is naïve (edge midpoints, face point+normal, body names): big topology changes
   lose references → feature error, user re-picks.
2. Closed splines can't be trimmed and splines can't be extended; collinear overlapping lines are
   not merged in `region.rs`; revolve profiles that cross the axis give an OCCT error.
3. In GPU mode, face highlights and plane fills are depth-tested (`render3d::Overlay`); edge/body
   highlight lines, sketches and outlines are still painter overlays without depth.
4. Section view has no cap faces (only an outline).
5. Solver performance: analytic Jacobians and sparse linear algebra for large sketches.
6. Missing Fusion features: component origins/own timelines, joint limits, contact sets, threads,
   drawings, sheet metal, sketch text.

## Conventions

- Run `cargo test` for the headless crates after any change to `sketch` or `model`, and the UI tests
  after app changes. Add a test for every bug fix.
- Don't add serde or other heavy deps to `sketch` or `model` without asking. Being dependency-free
  was deliberate.
- License is GPL-3.0-or-later; OCCT and opencascade-rs are LGPL-2.1.
