mod app;
mod tray;

use app::runtime::App;
use eframe::{run_native, NativeOptions};
use tokio::runtime::Runtime;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let _guard = rt.enter();

    let _ = os_api::OS::set_current_process_priority(os_api::PriorityClass::BelowNormal);

    let res = run_native(
        "CPU Affinity Tool",
        NativeOptions {
            run_and_return: true,
            viewport: eframe::egui::ViewportBuilder::default()
                .with_min_inner_size([470.0, 600.0])
                .with_max_inner_size([470.0, 1000.0])
                .with_maximize_button(false),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    );

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
