mod app;
mod tray;

use app::shell::App;
use app::startup::parse_startup_args;
use eframe::{run_native, NativeOptions};
use tokio::runtime::Runtime;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let startup_args = std::env::args().skip(1).collect::<Vec<_>>();
    let startup_intent = match parse_startup_args(&startup_args) {
        Ok(intent) => intent,
        Err(err) => {
            eprintln!("Invalid startup arguments: {err:?}");
            std::process::exit(2);
        }
    };

    let rt = Runtime::new().expect("failed to create tokio runtime");
    let _guard = rt.enter();

    let _ = app::adapters::os::set_current_process_priority(os_api::PriorityClass::BelowNormal);

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
        Box::new(move |cc| Ok(Box::new(App::new_with_startup_intent(cc, startup_intent)))),
    );

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
