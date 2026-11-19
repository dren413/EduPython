#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod console;
mod editor;
mod highlight;
mod icons;
mod project;
mod python;
mod runtime;
mod settings;
mod uml;

fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Pythonizer")
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([800.0, 500.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    // Optional: open a project folder given on the command line.
    let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
    eframe::run_native("Pythonizer", options, Box::new(move |cc| Ok(Box::new(app::PythonizerApp::new(cc, arg)))))
}
