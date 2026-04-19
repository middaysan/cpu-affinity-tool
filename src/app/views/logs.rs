use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::AppState;
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, CentralPanel, RichText, ScrollArea};

pub fn draw_logs_window(app: &mut AppState, ctx: &egui::Context) {
    let mut clear_logs = false;

    CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Logs");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    app.set_current_window(WindowRoute::Groups(GroupRoute::List));
                }
                if ui.button("Clear Logs").clicked() {
                    clear_logs = true;
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

    if clear_logs {
        app.clear_logs();
    }
}
