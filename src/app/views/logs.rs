use eframe::egui::{self, RichText, Window, ScrollArea, Color32};
use crate::app::app_models::CpuAffinityApp;

pub fn draw_logs_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if !app.logs.show {
        return;
    }

    let mut open = true;
    Window::new("Execution Log").resizable(true).open(&mut open).show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            for log in &app.logs.log_text {
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
            app.logs.log_text.clear();
        }
    });

    app.logs.show = open;
}