mod app;
mod models;
mod affinity;

use app::CpuAffinityApp;
use eframe::{run_native, NativeOptions};

fn main() {
    let app = Box::new(CpuAffinityApp::default());
    let native_options = NativeOptions {
        run_and_return: true,
        viewport: eframe::egui::ViewportBuilder::default().with_min_inner_size([450.0, 200.0]),
        ..Default::default()
    };

    let res = run_native(
        "CPU Affinity Tool",
        native_options,
        Box::new(|_cc| Ok(app)),
    );

    if let Err(_) = res {
        std::process::exit(1)
    }
}
