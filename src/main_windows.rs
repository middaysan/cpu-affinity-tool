//#![windows_subsystem = "windows"]

mod app;
use std::panic;

use app::models::AffinityApp;

use eframe::{run_native, NativeOptions};
use tokio::runtime::Runtime;

fn main() {
    panic::set_hook(Box::new(|info| {
        println!("!!! PANIC !!!");

        if let Some(s) = info.payload().downcast_ref::<&str>() {
            println!("Payload: {}", s);
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            println!("Payload: {}", s);
        }

        if let Some(loc) = info.location() {
            println!("At {}:{}", loc.file(), loc.line());
        }
    }));

    // Создаём tokio runtime вручную
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Запускаем eframe внутри runtime
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
            Box::new(|cc| Ok(Box::new(AffinityApp::new(cc)))),
        );
    
        if let Err(_) = res {
            std::process::exit(1);
        }
    });
}
