mod app;
use app::CpuAffinityApp;

use eframe::{run_native, NativeOptions};

fn main() {
    let res = run_native(
        "CPU Affinity Tool",
        NativeOptions {
            run_and_return: true,
            viewport: eframe::egui::ViewportBuilder::default().with_min_inner_size([450.0, 200.0]),
            ..Default::default()
        },
        Box::new(|_cc| Ok(Box::new(CpuAffinityApp::default()))),
    );
}
