#![windows_subsystem = "windows"]

mod app;
use app::app_models::AffinityApp;

use eframe::{run_native, NativeOptions};

fn main() {
    let res = run_native(
        "CPU Affinity Tool",
        NativeOptions {
            run_and_return: true,
            viewport: eframe::egui::ViewportBuilder::default()
                .with_min_inner_size([450.0, 600.0])
                .with_max_inner_size([450.0, 1000.0])
                .with_maximize_button(false), // Disable maximize button
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(AffinityApp::default()))),
    );

    if let Err(_) = res {
        std::process::exit(1);
    }
}
