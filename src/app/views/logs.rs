use eframe::egui::{self, CentralPanel, RichText, ScrollArea};
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
            .auto_shrink([false, false]) 
            .show(ui, |ui| {
                for log in app.logs.log_text.iter().rev() {
                    ui.label(RichText::new(log));
                    ui.separator();
                }
            });
    });
}
