#![windows_subsystem = "windows"]

mod app;
mod models;
mod affinity;

use app::CpuAffinityApp;
use eframe::{run_native, NativeOptions};

fn main() {
    let options = NativeOptions::default();

    run_native(
        "CPU Affinity Tool",
        options,
        Box::new(|_cc| Ok(Box::new(CpuAffinityApp::default()))),
    )
    .unwrap();
}
