use eframe::egui::{self, RichText, Window, ScrollArea, Color32};
use crate::app::CpuAffinityApp;

pub fn draw_log_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if !app.show_log_window {
        return;
    }

    let mut open = true;
    Window::new("Execution Log").resizable(true).open(&mut open).show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            for log in &app.log_text {
                let color = if log.contains("ERROR") {
                    Color32::RED
                } else if log.contains("OK") {
                    Color32::GREEN
                } else {
                    Color32::LIGHT_GRAY
                };
                ui.label(RichText::new(log).color(color));
            }
        });
        if ui.button("Clear Logs").clicked() {
            app.log_text.clear();
        }
    });

    app.show_log_window = open;
}