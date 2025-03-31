#![windows_subsystem = "windows"]

mod app;
mod models;
mod affinity;

use app::CpuAffinityApp;
use eframe::{run_native, NativeOptions};

fn main() {
    let native_options = NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_min_inner_size([450.0, 200.0]), // Устанавливаем минимальный размер окна
        ..Default::default()
    };

    run_native(
        "CPU Affinity Tool",
        native_options,
        Box::new(|_cc| Ok(Box::new(CpuAffinityApp::default()))),
    )
    .unwrap();
}
