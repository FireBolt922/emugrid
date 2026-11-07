#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod devices;
mod grid;
mod input;
mod ios;
mod sdk;
mod touch;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("emugrid")
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([360.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        "emugrid",
        options,
        Box::new(|cc| Ok(Box::new(app::EmuGrid::new(&cc.egui_ctx)))),
    )
}
