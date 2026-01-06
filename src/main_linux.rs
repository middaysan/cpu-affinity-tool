mod app;
mod tray;

use app::models::App;
use eframe::{run_native, NativeOptions};

fn main() {
    let res = run_native(
        "CPU Affinity Tool",
        NativeOptions {
            run_and_return: true,
            viewport: eframe::egui::ViewportBuilder::default().with_min_inner_size([450.0, 200.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    );

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
