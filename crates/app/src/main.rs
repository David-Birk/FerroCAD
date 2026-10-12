#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod browser;
mod dialogs;
mod features_ui;
mod icons;
mod inspect;
mod manipulators;
mod picking;
mod render3d;
mod ribbon;
mod sketch_ui;
mod text;
mod theme;
mod timeline;
mod viewport;

#[cfg(test)]
mod ui_tests;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ferrocad")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([800.0, 500.0]),
        // The 3D view draws into egui's pass with a real depth buffer.
        depth_buffer: 24,
        multisampling: render3d::MSAA as u16,
        ..Default::default()
    };
    eframe::run_native(
        "ferrocad",
        options,
        Box::new(|cc| {
            theme::setup(&cc.egui_ctx);
            let gpu = render3d::init(cc);
            let mut app = app::FerroApp::new();
            app.gpu = gpu;
            // `ferrocad part.fcad` opens a document.
            if let Some(path) = std::env::args().nth(1) {
                app.doc_path = path;
                app.open_document_from_path();
            }
            Ok(Box::new(app))
        }),
    )
}
