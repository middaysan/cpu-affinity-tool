use crate::app::models::AppState;
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, CentralPanel, RichText, ScrollArea};

pub fn draw_logs_window(app: &mut AppState, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Logs");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    app.set_current_window(crate::app::controllers::WindowController::Groups(
                        crate::app::controllers::Group::ListGroups,
                    ));
                }
                if ui.button("Clear Logs").clicked() {
                    app.log_manager.entries.clear();
                }
            });
        });

        glass_frame(ui).show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for log_string in app.log_manager.formatted_entries().rev() {
                        ui.label(RichText::new(log_string));
                        ui.separator();
                    }
                });
        });
    });
}
