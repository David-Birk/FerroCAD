# ferrocad

*[Deutsche Version](docs/README.de.md)*

Native parametric CAD for Linux, written in Rust. The workflow and UI follow Fusion 360:
**sketch → constraints/dimensions → solid features**, all driven by a timeline and parameters.

> **Note:** ferrocad is a vibe-coded hobby project, not a professional product, and it is not
> affiliated with Autodesk or FreeCAD. Details in the [disclaimer](docs/DISCLAIMER.md).

## Contents

- [Features](#features)
- [Building (Arch Linux)](#building-arch-linux)
- [Usage](#usage)
- [File format](#file-format-fcad)
- [Architecture](#architecture)
- [Known limitations](#known-limitations)
- [Contributing](#contributing)
- [License](#license)

## Features

**UI:** menu bar (File, Undo/Redo) · ribbon with SOLID and SKETCH tabs (Create, Modify, Construct,
Inspect; every group has a ▾ menu) · browser on the left · sketch palette on the right · ViewCube ·
navigation bar · timeline · status bar with hints for the active tool.

### Sketching

- **Create a sketch:** click an origin plane, a construction plane or a **flat face of a body**.
  Sketches on faces and construction planes follow the model when it changes.
- **Tools:** line (L), rectangle (R), centre rectangle, circle (C), arc (A), 3-point arc, polygon,
  slot, **spline** (click fit points; Enter or double-click finishes), project (P) body edges (purple;
  lines, circles and arcs follow the model).
- **Type values while drawing:** after the first click, input fields appear at the geometry
  (length/angle, width/height, diameter). Type a value, **Tab** to the next field, **Enter** creates the
  shape. Typed values become real dimensions, parameters included (`width / 2`).
- **Units:** values always show their unit (`30 mm`, `45°`) – in dimensions, input fields, dialogs and
  the parameter table. A plain number gets its unit when you leave the field; `cm`, `m`, `in`,
  `deg`/`°` and `rad` can be typed as well.
- **Dimension (D):** click a line and place it, a second line for an angle, two points (to the side =
  vertical, above/below = horizontal), a circle or arc. Double-click a dimension to edit it.
- **Constraints:** horizontal, vertical, coincident, parallel, perpendicular, equal, tangent,
  concentric, midpoint, fix – pick the tool, click the geometry. The constraint badges can be clicked
  and deleted with Del. Blue = still free, white = fully defined.
- **Modify:** trim (T), extend, sketch fillet, offset (O, whole chain), mirror (linked symmetrically),
  rectangular and circular pattern, construction (X), delete.
- **Selection box:** drag in empty space – to the right selects only what lies fully inside (window),
  to the left everything it touches (crossing). Shift/Ctrl adds to the selection.

### Solids

- **Create:** extrude (E; distance, *to object* (up to a plane or flat face, tilted ones too) or
  *all*; one side / two sides / symmetric, flip, taper angle, thin wall inside/outside/centred; joins
  by default on faces), revolve (axis: sketch axis, sketch line, world or construction axis), loft,
  sweep, hole (H; on a face or on sketch points – circle centres, line ends, points – simple /
  counterbore / countersink), rectangular and circular pattern of bodies or of features (repeat holes,
  cuts, bosses), mirror.
- **Modify:** press pull (Q) a face, fillet (F), chamfer (equal distance or two distances), draft
  (tilt faces around a neutral plane), shell, combine, split body, move/copy (M), remove (Del),
  parameters (user and model parameters: every dimension and feature value in one table).
- **Appearance:** right-click a body in the browser → *Appearance* (steel, aluminium, brass, plastics,
  wood …); the material's density gives the mass.
- **Drag arrows:** extrude and press pull have an arrow in the model to drag the distance.
- **Live preview:** extrude, revolve, fillet and every dialog with a selection show a preview.
  Enter = OK, Esc = cancel.

### Construct

- Offset/angled plane, midplane between two parallel planes or faces, axis through an edge, axis
  normal to a face.

### Assemble

- **Component** from the selected bodies (own browser folder; clicking one of its bodies selects the
  whole component, and joints/move/patterns move it as one).
- **Joint:** place a body at a round edge or a face point of another – rigid, revolute or slider;
  angle and offset are parameters, with optional limits. Right-click in the timeline → *Animate* moves
  the joint (Esc stops).
- **Interference check** shows overlaps in red.

### Inspect

- Measure (I: points, edges, diameters, face area, angles between faces/edges), section view,
  properties (material, mass, volume, surface area, centre of mass).

### File and timeline

- **Timeline:** double-click = edit, right-click = suppress / roll back / move / delete; drag the
  orange marker to roll back.
- **File:** New, Open…, Save (Ctrl+S), Save As…, **STEP import** (one body per solid, as a feature in
  the timeline), STEP and STL export – with native file dialogs (XDG portal).
- **Undo/Redo:** Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y – including single sketch steps.

## Building (Arch Linux)

```sh
sudo pacman -S --needed cmake clang opencascade   # Rust via rustup, not the "rust" package
cargo run --release
cargo run --release -- part.fcad                   # open a file directly
```

`opencascade-rs` needs OCCT **7.8 or newer within 7.x** (Arch ships 7.9). Without the system library,
OCCT can be built from source once (20–40 min):

```sh
cargo run --release --features builtin-occt
```

Tests: `cargo test --workspace` (solver, regions, timeline, file format, the real OCCT kernel and UI
tests that play through whole workflows with simulated clicks).

## Usage

**Navigation:**

- Drag with the right mouse button = orbit
- Middle mouse button = pan (Shift+middle = orbit)
- Mouse wheel = zoom to the cursor
- ViewCube: click faces, edges and corners
- F6 = home, Shift+F = fit
- Right-click = context menu for the selection

**Selection:** click faces and edges in the model (Shift = several), bodies in the browser. A matching
command takes over the selection.

**Command search (S):** like in Fusion – type a command, Enter runs it.

**Shortcuts:**

| Key | Command | Key | Command |
|---|---|---|---|
| S | Command search | E | Extrude |
| L | Line | H | Hole |
| R | Rectangle | Q | Press pull |
| C | Circle | F | Fillet |
| A | Arc | M | Move/copy |
| P | Project | I | Measure |
| D | Dimension | Del | Remove/delete |
| T | Trim | Ctrl+S | Save |
| O | Offset | Ctrl+Z | Undo |
| X | Construction | Ctrl+Shift+Z / Ctrl+Y | Redo |
| Enter | OK | Esc | Cancel |

## File format `.fcad`

Readable JSON: parameters, sketches (points, curves, constraints, parametric dimensions) and the
feature timeline. Bodies are rebuilt on load, as in Fusion. Files work well with git; damaged files
give an error message instead of a crash. Hidden bodies, sketches and construction geometry are saved
too.

## Architecture

```
crates/
├── sketch/       2D constraint solver, regions/profiles, sketch operations (no dependencies)
├── model/        parameters, expressions, feature timeline, kernel trait, .fcad format
├── view/         camera, projection, picking, section (GUI-independent)
├── kernel-occt/  kernel implementation with OpenCascade (opencascade-rs)
└── app/          egui/eframe UI (wgpu with depth buffer)
vendor/           patched copies of opencascade-rs (OCCT 7.9, crash-safe operations)
```

## Known limitations

1. **Topological naming:** edges are found again by their midpoint (1 mm tolerance), faces by point +
   normal. Big changes can lose a selection → the feature shows an error; re-pick the edges/faces.
2. Closed splines can't be trimmed yet; overlapping *collinear* lines aren't merged in regions.
   Revolve profiles must not cross the axis.
3. Highlighted edges, sketches and outlines are drawn on top of the bodies (no depth test); face
   highlights and planes are depth-tested.
4. The solver uses a numeric Jacobian and dense linear algebra – fine for hundreds of unknowns; large
   sketches will need analytic derivatives and a sparse factorisation later.
5. Components don't have their own origin or timeline yet; contact sets, threads, drawings and sheet
   metal are still missing.

## Contributing

Thoughts, improvements and feedback are welcome – just open an issue or a pull request. How the
project is made is explained in the [disclaimer](docs/DISCLAIMER.md).

## License

GPL-3.0-or-later. OpenCascade (OCCT) and opencascade-rs are LGPL-2.1.
