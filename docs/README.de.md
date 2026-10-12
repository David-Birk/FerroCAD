# ferrocad

*[English version](../README.md)*

Ein natives, parametrisches CAD für Linux in Rust – Workflow und Oberfläche nach dem Vorbild von
Fusion 360: **Skizze → Bedingungen/Bemaßungen → Körper-Features**, alles über eine Zeitleiste und
Parameter steuerbar.

> **Hinweis:** ferrocad ist ein vibe-codetes Hobbyprojekt, kein professionelles Produkt und nicht mit
> Autodesk oder FreeCAD verbunden. Details im [Disclaimer](DISCLAIMER.md).

## Inhalt

- [Features](#features)
- [Bauen (Arch Linux)](#bauen-arch-linux)
- [Bedienung](#bedienung)
- [Dateiformat](#dateiformat-fcad)
- [Architektur](#architektur)
- [Bekannte Grenzen](#bekannte-grenzen)
- [Mitmachen](#mitmachen)
- [Lizenz](#lizenz)

## Features

**Oberfläche:** Menüleiste (Datei, Rückgängig/Wiederholen) · Ribbon mit den Bereichen SOLID bzw.
SKETCH (Erstellen, Ändern, Konstruieren, Prüfen, jede Gruppe mit ▾-Menü) · Browser links ·
Skizzenpalette rechts · ViewCube · Navigationsleiste · Zeitleiste · Statuszeile mit Hinweisen zum
aktiven Werkzeug.

### Skizzieren

- **Skizze erstellen:** Ursprungsebene, Konstruktionsebene oder eine **ebene Körperfläche** anklicken.
  Skizzen auf Flächen und Konstruktionsebenen wandern mit, wenn sich das Modell ändert.
- **Werkzeuge:** Linie (L), Rechteck (R), Mittelpunkt-Rechteck, Kreis (C), Bogen (A), 3-Punkt-Bogen,
  Polygon, Langloch, **Spline** (Stützpunkte klicken, Enter/Doppelklick beendet), **Punkt** (z. B. für
  Bohrungspositionen), **Text** (Anfang der Grundlinie klicken, Text und Höhe tippen; die Buchstaben
  werden Profile zum Extrudieren oder Ausschneiden), Projizieren (P) von Körperkanten (lila; Linien,
  Kreise und Bögen folgen dem Modell).
- **Werte direkt eintippen:** Nach dem ersten Klick erscheinen Eingabefelder an der Geometrie
  (Länge/Winkel, Breite/Höhe, Durchmesser) – Wert tippen, **Tab** zum nächsten Feld, **Enter** erzeugt
  die Form. Getippte Werte werden echte Bemaßungen, auch mit Parametern (`breite / 2`).
- **Einheiten:** Werte werden immer mit Einheit angezeigt (`30 mm`, `45°`) – in Bemaßungen,
  Eingabefeldern, Dialogen und der Parametertabelle. Eine reine Zahl bekommt beim Verlassen des Felds
  ihre Einheit; `cm`, `m`, `in`, `deg`/`°` und `rad` lassen sich ebenfalls tippen.
- **Bemaßen (D):** Linie anklicken und platzieren, zweite Linie für Winkel, zwei Punkte (seitlich =
  vertikal, ober-/unterhalb = horizontal), Kreis/Bogen. Doppelklick auf eine Bemaßung bearbeitet sie.
- **Bedingungen:** Horizontal, Vertikal, Koinzident, Parallel, Senkrecht, Gleich, Tangential,
  Konzentrisch, Mittelpunkt, Fixieren – Werkzeug wählen, Geometrie anklicken. Symbole an der Geometrie
  lassen sich anklicken und mit Entf löschen. Blau = noch frei, weiß = voll bestimmt.
- **Ändern:** Trimmen (T), Verlängern, Skizzen-Verrundung, Versatz (O, ganze Kette), Spiegeln
  (symmetrisch verknüpft), Rechteck- und Kreismuster, Konstruktion (X), Löschen.
- **Auswahlrechteck:** Im leeren Bereich ziehen – nach rechts wählt nur, was ganz im Rechteck liegt
  (Fenster), nach links alles, was es berührt (Kreuzen). Shift/Ctrl erweitert die Auswahl.

### Körper

- **Erstellen:** Extrusion (E; Abstand, *bis Objekt* (bis zu einer Ebene oder ebenen Fläche, auch
  schräg) oder *Alle*; eine Seite/zwei Seiten/symmetrisch, Flip, Verjüngungswinkel, dünne Wand
  innen/außen/mittig; auf Flächen standardmäßig Vereinigen), Drehung (Achse: Skizzenachse,
  Skizzenlinie, Welt- oder Konstruktionsachse), Loft, Sweep, Bohrung (H; auf einer Fläche oder auf
  Skizzenpunkten – Kreismittelpunkte, Linienenden, Punkte – einfach/Senkung/Kegelsenkung), Rechteck-
  und Kreismuster von Körpern oder von Features (Bohrungen, Ausschnitte, Aufsätze wiederholen),
  Spiegeln.
- **Ändern:** Drücken/Ziehen (Q) einer Fläche, Verrundung (F), Fase (gleicher Abstand oder zwei
  Abstände), Formschräge (Draft: Flächen um eine neutrale Ebene neigen), Wandstärke (Shell), Kombinieren,
  Körper teilen, Verschieben/Kopieren (M), Entfernen (Entf), Parameter (Benutzer- und
  Modellparameter: alle Bemaßungen und Feature-Werte in einer Tabelle).
- **Aussehen:** Rechtsklick auf einen Körper im Browser → *Appearance* (Stahl, Aluminium, Messing,
  Kunststoffe, Holz …); die Dichte des Materials ergibt die Masse.
- **Ziehpfeile:** Extrusion und Drücken/Ziehen haben einen Pfeil im Modell, an dem man den Abstand
  zieht.
- **Live-Vorschau:** Extrusion, Drehung, Verrundung und alle Dialoge mit Auswahl zeigen eine Vorschau.
  Enter = OK, Esc = Abbrechen.

### Konstruieren

- Versatz-/Winkelebene, Mittelebene zwischen zwei parallelen Ebenen/Flächen, Achse durch Kante, Achse
  senkrecht zur Fläche.

### Montieren

- **Komponente** aus ausgewählten Körpern (eigener Browser-Ordner; ein Klick auf einen ihrer Körper
  wählt die ganze Komponente, Gelenke/Verschieben/Muster bewegen sie gemeinsam).
- **Gelenk (Joint):** Körper an eine runde Kante oder Flächenstelle eines anderen setzen – starr, als
  Drehgelenk oder Schiebegelenk; Winkel/Versatz sind Parameter. Rechtsklick in der Zeitleiste →
  *Animieren* bewegt das Gelenk (Esc beendet).
- **Interferenzprüfung** zeigt Überschneidungen rot.

### Prüfen

- Messen (I: Punkte, Kanten, Durchmesser, Flächeninhalt, Winkel zwischen Flächen/Kanten),
  Schnittansicht, Eigenschaften (Material, Masse, Volumen, Oberfläche, Schwerpunkt).

### Datei und Zeitleiste

- **Zeitleiste:** Doppelklick = bearbeiten, Rechtsklick = unterdrücken / zurückrollen / verschieben /
  löschen, orangen Marker ziehen zum Zurückrollen.
- **Datei:** Neu, Öffnen…, Speichern (Ctrl+S), Speichern unter…, **STEP-Import** (ein Körper je Solid,
  als Feature in der Zeitleiste), STEP- und STL-Export – mit nativen Dateidialogen (XDG-Portal).
- **Rückgängig/Wiederholen:** Ctrl+Z, Ctrl+Shift+Z oder Ctrl+Y – auch einzelne Skizzenschritte.

## Bauen (Arch Linux)

```sh
sudo pacman -S --needed cmake clang opencascade   # Rust über rustup, nicht das Paket "rust"
cargo run --release
cargo run --release -- teil.fcad                   # Datei direkt öffnen
```

`opencascade-rs` verlangt OCCT **7.8 oder neuer innerhalb von 7.x** (Arch liefert 7.9). Ohne
Systembibliothek lässt sich OCCT einmalig aus den Quellen mitbauen (20–40 min):

```sh
cargo run --release --features builtin-occt
```

Tests: `cargo test --workspace` (Solver, Regionen, Timeline, Dateiformat, echter OCCT-Kernel und
Oberflächen-Tests, die ganze Arbeitsabläufe per simulierten Klicks durchspielen).

## Bedienung

**Navigation:**

- Rechte Maustaste ziehen = Orbit
- Mittlere Maustaste = Pan (Shift+Mitte = Orbit)
- Mausrad = Zoom zum Cursor
- ViewCube: Flächen, Kanten und Ecken anklicken
- F6 = Home, Shift+F = Einpassen
- Rechtsklick = Kontextmenü passend zur Auswahl

**Auswahl:** Flächen und Kanten im Modell anklicken (Shift = mehrere), Körper im Browser. Ein
passender Befehl übernimmt die Auswahl.

**Befehlssuche (S):** wie in Fusion – Befehl tippen, Enter führt ihn aus.

**Tastenkürzel:**

| Taste | Befehl | Taste | Befehl |
|---|---|---|---|
| S | Befehlssuche | E | Extrusion |
| L | Linie | H | Bohrung |
| R | Rechteck | Q | Drücken/Ziehen |
| C | Kreis | F | Verrundung |
| A | Bogen | M | Verschieben/Kopieren |
| P | Projizieren | I | Messen |
| D | Bemaßen | Entf | Entfernen/Löschen |
| T | Trimmen | Ctrl+S | Speichern |
| O | Versatz | Ctrl+Z | Rückgängig |
| X | Konstruktion | Ctrl+Shift+Z / Ctrl+Y | Wiederholen |
| Enter | OK | Esc | Abbrechen |

## Dateiformat `.fcad`

Lesbares JSON: Parameter, Skizzen (Punkte, Kurven, Bedingungen, parametrische Bemaßungen) und die
Feature-Zeitleiste. Körper werden beim Laden neu berechnet, wie bei Fusion. Dateien lassen sich gut
mit git versionieren; beschädigte Dateien liefern eine Fehlermeldung statt eines Absturzes.
Ausgeblendete Körper, Skizzen und Konstruktionselemente werden mitgespeichert.

## Architektur

```
crates/
├── sketch/       2D-Constraint-Solver, Regionen/Profile, Skizzen-Operationen (keine Abhängigkeiten)
├── model/        Parameter, Ausdrücke, Feature-Timeline, Kernel-Trait, .fcad-Format
├── view/         Kamera, Projektion, Picking, Schnitt (GUI-unabhängig)
├── kernel-occt/  Kernel-Implementierung mit OpenCascade (opencascade-rs)
└── app/          egui/eframe-Oberfläche (wgpu mit Tiefenpuffer)
vendor/           angepasste Kopien von opencascade-rs (OCCT 7.9, absturzsichere Operationen)
```

## Bekannte Grenzen

1. **Topologisches Benennungsproblem:** Kanten werden über ihren Mittelpunkt (Toleranz 1 mm), Flächen
   über Punkt + Normale wiedergefunden. Große Änderungen können eine Auswahl verlieren → Fehler in der
   Zeitleiste, Kanten/Flächen neu wählen.
2. Geschlossene Splines lassen sich noch nicht trimmen; überlappende *kollineare* Linien werden in
   Regionen nicht zusammengeführt. Revolve-Profile dürfen die Achse nicht schneiden.
3. Hervorgehobene Kanten, Skizzen und Umrisse werden über die Körper gezeichnet (ohne Tiefentest);
   Flächen-Hervorhebungen und Ebenen sind tiefengetestet.
4. Der Solver nutzt eine numerische Jacobimatrix und dichte lineare Algebra – reicht für Hunderte
   Unbekannte; für große Skizzen später analytische Ableitungen und eine Sparse-Zerlegung.
5. Komponenten haben noch keinen eigenen Ursprung und keine eigene Zeitleiste; Kontaktsätze, Gewinde,
   Zeichnungen und Blechteile fehlen noch.

## Mitmachen

Gedanken, Verbesserungen und Feedback sind willkommen – einfach ein Issue oder einen Pull Request
öffnen. Wie das Projekt entsteht, steht im [Disclaimer](DISCLAIMER.md).

## Lizenz

GPL-3.0-or-later. OpenCascade (OCCT) und opencascade-rs stehen unter LGPL-2.1. Skizzentext nutzt die
mit egui gelieferte Schrift Ubuntu (Ubuntu Font Licence 1.0).
