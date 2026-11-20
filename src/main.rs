#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod avd;
mod capture;
mod devices;
mod grid;
mod icons;
mod input;
mod ios;
mod sdk;
mod settings;
mod shortcuts;
mod theme;
mod widgets;

fn main() -> eframe::Result {
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("emugrid")
        .with_inner_size([960.0, 720.0])
        .with_min_inner_size([360.0, 300.0]);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "emugrid",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(app::EmuGrid::new(&cc.egui_ctx)))
        }),
    )
}
