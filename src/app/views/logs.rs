use eframe::egui::{self, RichText, CentralPanel, ScrollArea, Color32};
use crate::app::app_models::CpuAffinityApp;

pub fn draw_logs_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Logs");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("❌").on_hover_text("Close").clicked() {
                    app.set_current_controller(crate::app::controllers::WindowController::Groups(crate::app::controllers::Group::ListGroups));
                }
                if ui.button("Clear Logs").clicked() {
                    app.logs.log_text.clear();
                }
    
            });
        });
        ui.separator();
        ScrollArea::vertical()
            .auto_shrink([false, false]) // Отключаем автоматическое сжатие
            .show(ui, |ui| {
                for log in app.logs.log_text.iter().rev() {
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
    });
}
