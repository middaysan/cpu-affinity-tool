#![windows_subsystem = "windows"]

extern crate single_instance;


mod app;

use app::models::App;
use eframe::{run_native, NativeOptions};
use tokio::runtime::Runtime;

use os_api::OS;


fn main() {

    //Validate if there is an instance of the application running.
    let already_running = OS::is_already_running();

    if (already_running) {
        return;
    }

    // Creating tokio runtime manually
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Running eframe inside the runtime
    rt.block_on(async {
        let res = run_native(
            "CPU Affinity Tool",
            NativeOptions {
                run_and_return: true,
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_min_inner_size([470.0, 600.0])
                    .with_max_inner_size([470.0, 1000.0])
                    .with_maximize_button(false), // Disable maximize button
                ..Default::default()
            },
            Box::new(|cc| Ok(Box::new(App::new(cc)))),
        );

        if res.is_err() {
            std::process::exit(1);
        }
    });
}
