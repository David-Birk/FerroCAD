# ferrocad

Ein natives, parametrisches CAD für Linux in Rust – Workflow nach dem Vorbild von Fusion 360:
**Skizze → Bedingungen/Bemaßungen → Extrusion → Verrundung**, alles über eine Zeitleiste und
Parameter steuerbar.

## Architektur

```
crates/
├── sketch/       2D-Constraint-Solver (reines Rust, keine Abhängigkeiten)
├── model/        Parameter, Ausdrücke, Feature-Timeline, Kernel-Trait
├── view/         Kamera, Projektion, Picking, Software-Shading (GUI-unabhängig)
├── kernel-occt/  Kernel-Implementierung mit OpenCascade (opencascade-rs)
└── app/          egui/eframe-Oberfläche (wgpu)
```

| Crate | Status |
|---|---|
| `sketch`, `model`, `view` | getestet (`cargo test -p ferrocad-sketch -p ferrocad-model -p ferrocad-view`, 35 Tests) |
| `kernel-occt`, `app` | geschrieben gegen die Quellen von opencascade-rs (main) und egui 0.36.2, **noch nie kompiliert** – erster Build auf deinem Rechner |

## Bauen (Arch Linux)

```sh
sudo pacman -S --needed rust cmake clang opencascade
cargo run --release
```

`opencascade-rs` verlangt OCCT **7.8 oder neuer innerhalb von 7.x**. Falls Arch schon OCCT 8 ausliefert
(oder die Systembibliothek nicht gefunden wird), OCCT einmalig aus den Quellen mitbauen (20–40 min):

```sh
cargo run --release --features builtin-occt
```

## Bedienung

- **Navigation:** Rechte Maustaste ziehen = Orbit · Mittlere Maustaste = Pan (Shift+Mitte = Orbit) · Mausrad = Zoom zum Cursor
- **✏ Sketch** → Ebene wählen → Linie (Polylinie, Esc beendet), Rechteck, Kreis, Bogen (Mitte, Start, Ende)
  - Punkte rasten auf vorhandene Punkte ein; fast waagerechte/senkrechte Linien bekommen automatisch H/V
  - Auswählen (Shift = mehrere) → Bedingung oder **↔ Dimension** in der Palette
  - Bemaßungen akzeptieren Zahlen oder Ausdrücke wie `width / 2`; blau = noch frei, weiß = voll bestimmt
  - Punkte mit dem Auswahlwerkzeug ziehen
- **⬆ Extrude** → Skizze wählen, in eine Region klicken → Abstand (Ausdruck), Operation (Neuer Körper/Vereinigen/Schneiden/Schnitt)
- **◔ Fillet / ◢ Chamfer** → Kanten anklicken → Radius
- **ƒ Parameters** → Parameter ändern, alles baut sich neu auf
- **Zeitleiste:** Doppelklick = bearbeiten, Rechtsklick = unterdrücken / zurückrollen / verschieben / löschen
- **File** → STEP (alle sichtbaren Körper) oder STL exportieren

## Bekannte Grenzen und nächste Schritte

1. **Speichern/Laden** fehlt – nächster Schritt: `serde` + RON-Dateiformat.
2. **Fillet-Absturz:** opencascade-rs prüft `IsDone()` nicht; ein zu großer Radius wirft eine C++-Exception
   und beendet das Programm. Lösung: kleiner Upstream-PR (oder Fork), der `Result` zurückgibt.
3. **Topologisches Benennungsproblem:** Kanten für Fillets werden über ihren Mittelpunkt wiedergefunden
   (Toleranz 1 mm). Große Parameteränderungen verlieren die Auswahl → Fehler in der Zeitleiste, Kanten neu wählen.
4. **Regionen:** Verzweigte Linien (eine Linie quer durch ein Rechteck) werden noch nicht in Teilflächen zerlegt;
   geschlossene Schleifen und Löcher funktionieren.
5. **Rendering:** Painter-Algorithmus über egui (keine echte Tiefenpufferung). Gut für einfache Teile; nächster
   Schritt ist ein `egui_wgpu`-Paint-Callback mit Depth-Buffer.
6. Skizzen nur auf den Ursprungsebenen – Skizzieren auf Körperflächen, Revolve, Muster, Spiegeln, Undo/Redo
   und Baugruppen fehlen noch.
7. Der Solver nutzt eine numerische Jacobimatrix und dichte lineare Algebra – reicht für Hunderte Unbekannte;
   für große Skizzen später analytische Ableitungen und eine Sparse-Zerlegung.

Lizenz: GPL-3.0-or-later (opencascade-rs/OCCT sind LGPL-2.1).
