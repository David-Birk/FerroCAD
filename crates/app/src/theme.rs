//! Colours and egui style, loosely following Fusion 360's dark theme.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle};

pub const ACCENT: Color32 = Color32::from_rgb(14, 143, 216);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(20, 78, 112);
pub const ORANGE: Color32 = Color32::from_rgb(242, 140, 40);
pub const GREEN: Color32 = Color32::from_rgb(88, 178, 72);
pub const ERROR: Color32 = Color32::from_rgb(255, 110, 100);

pub const TEXT: Color32 = Color32::from_rgb(222, 224, 228);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(150, 154, 162);
pub const ICON: Color32 = Color32::from_rgb(208, 212, 218);

/// Application bar (very top) and status bar.
pub const BAR: Color32 = Color32::from_rgb(28, 30, 33);
/// Ribbon, browser, timeline.
pub const PANEL: Color32 = Color32::from_rgb(44, 47, 51);
/// Floating dialogs and palettes.
pub const WINDOW: Color32 = Color32::from_rgb(50, 53, 58);
pub const BORDER: Color32 = Color32::from_rgb(70, 74, 80);
pub const HOVER: Color32 = Color32::from_rgb(64, 68, 75);

pub const VIEW_TOP: Color32 = Color32::from_rgb(70, 75, 83);
pub const VIEW_BOTTOM: Color32 = Color32::from_rgb(32, 34, 38);

/// Theme plus fonts: the proportional font lacks most geometric and
/// box-drawing symbols, the bundled monospace font has them.
pub fn setup(ctx: &egui::Context) {
    apply(ctx);
    let mut fonts = egui::FontDefinitions::default();
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.push("Hack".to_owned());
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(6.0, 5.0);
        s.button_padding = egui::vec2(8.0, 3.0);
        s.interact_size.y = 22.0;
        s.window_margin = Margin::same(10);
        s.menu_margin = Margin::same(6);

        let v = &mut style.visuals;
        v.override_text_color = Some(TEXT);
        v.panel_fill = PANEL;
        v.window_fill = WINDOW;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(5);
        v.window_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(110) };
        v.popup_shadow = egui::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(100) };
        v.extreme_bg_color = Color32::from_rgb(30, 32, 35);
        v.faint_bg_color = Color32::from_rgb(48, 51, 56);
        v.selection.bg_fill = ACCENT_DIM;
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.error_fg_color = ERROR;
        v.warn_fg_color = ORANGE;
        v.striped = true;

        let w = &mut v.widgets;
        let radius = CornerRadius::same(4);
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.weak_bg_fill = Color32::from_rgb(56, 60, 66);
        w.inactive.bg_fill = Color32::from_rgb(56, 60, 66);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.hovered.weak_bg_fill = HOVER;
        w.hovered.bg_fill = HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(92, 98, 106));
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.weak_bg_fill = ACCENT_DIM;
        w.active.bg_fill = ACCENT_DIM;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.open.weak_bg_fill = HOVER;
        for wv in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            wv.corner_radius = radius;
            wv.expansion = 0.0;
        }
    });
}

/// Frame for the floating command dialogs and palettes.
pub fn dialog_frame() -> egui::Frame {
    egui::Frame::window(&egui::Style::default())
        .fill(WINDOW)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(0))
}

/// Small uppercase section header (Fusion's "OPTIONS", "DIMENSIONS", …).
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(text.to_uppercase()).small().strong().color(TEXT_WEAK));
}

/// Fusion's blue primary button.
pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let button = egui::Button::new(egui::RichText::new(text).color(Color32::WHITE))
        .fill(if enabled { ACCENT } else { ACCENT_DIM })
        .min_size(egui::vec2(72.0, 24.0));
    ui.add_enabled(enabled, button)
}

pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).min_size(egui::vec2(72.0, 24.0)))
}

/// Body appearances: name, display colour and density (g/cm³). The first is
/// the default.
pub const MATERIALS: &[(&str, [u8; 3], f64)] = &[
    ("Steel", [168, 178, 192], 7.85),
    ("Aluminium", [205, 210, 216], 2.7),
    ("Stainless steel", [190, 192, 196], 8.0),
    ("Brass", [208, 172, 92], 8.5),
    ("Copper", [204, 124, 84], 8.96),
    ("Gold", [232, 192, 84], 19.3),
    ("Plastic – black", [58, 61, 66], 1.05),
    ("Plastic – white", [236, 236, 232], 1.05),
    ("Plastic – red", [200, 62, 56], 1.05),
    ("Plastic – blue", [62, 112, 200], 1.05),
    ("Plastic – green", [82, 160, 92], 1.05),
    ("Plastic – yellow", [232, 196, 60], 1.05),
    ("Wood", [172, 124, 74], 0.6),
    ("Rubber", [40, 42, 44], 1.2),
];

pub fn material_color(name: &str) -> [u8; 3] {
    MATERIALS.iter().find(|(n, ..)| *n == name).map(|(_, c, _)| *c).unwrap_or(MATERIALS[0].1)
}

pub fn material_density(name: &str) -> f64 {
    MATERIALS.iter().find(|(n, ..)| *n == name).map(|(.., d)| *d).unwrap_or(MATERIALS[0].2)
}
