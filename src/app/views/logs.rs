use crate::app::models::AppState;
use eframe::egui::{self, CentralPanel, Frame, RichText, ScrollArea};

pub fn draw_logs_window(app: &mut AppState, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        let frame = Frame::group(ui.style())
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(5.0)
            .inner_margin(15.0);

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Logs");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Back").on_hover_text("Close").clicked() {
                        app.set_current_window(crate::app::controllers::WindowController::Groups(
                            crate::app::controllers::Group::ListGroups,
                        ));
                    }
                    if ui.button("Clear Logs").clicked() {
                        app.log_manager.entries.clear();
                    }
                });
            });
            ui.separator();

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for log in app.log_manager.entries.iter().rev() {
                        ui.label(RichText::new(log));
                        ui.separator();
                    }
                });
        });
    });
}
