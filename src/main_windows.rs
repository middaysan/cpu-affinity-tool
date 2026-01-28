#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod tray;

use app::models::App;
use eframe::{run_native, NativeOptions};
use tokio::runtime::Runtime;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    #[cfg(debug_assertions)]
    {
        println!("========================================================");
        println!("DEBUG: Application starting...");
        println!(
            "DEBUG: OS: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        println!("DEBUG: Reactive mode: YES (Wait-based event loop)");
        println!("========================================================");
    }
    // Creating tokio runtime manually
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let _guard = rt.enter();

    #[cfg(debug_assertions)]
    println!("DEBUG: Tokio runtime created and entered.");

    // Set self-priority to Below Normal to avoid interfering with high-load apps
    #[warn(unused_variables)]
    if let Err(_e) = os_api::OS::set_current_process_priority(os_api::PriorityClass::BelowNormal) {
        #[cfg(debug_assertions)]
        eprintln!("DEBUG: Failed to set self priority: {}", _e);
    }

    let options = NativeOptions {
        run_and_return: true,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_min_inner_size([470.0, 600.0])
            .with_max_inner_size([470.0, 1000.0])
            .with_maximize_button(false), // Disable maximize button
        ..Default::default()
    };

    #[cfg(debug_assertions)]
    {
        println!("DEBUG: NativeOptions initialized:");
        println!("  - Renderer: {:?}", options.renderer);
        println!("  - V-Sync: {}", options.vsync);
        println!("  - Run and return: {}", options.run_and_return);
        println!("========================================================");
    }

    // Running eframe on the main thread
    let res = run_native(
        "CPU Affinity Tool",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    );

    if let Err(e) = res {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
