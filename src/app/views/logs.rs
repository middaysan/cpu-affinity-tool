use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{glass_frame, inset_frame};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, CentralPanel, RichText, ScrollArea};

pub fn draw_logs_window(app: &mut AppState, root_ui: &mut egui::Ui) {
    let mut clear_logs = false;
    let mut open_data_folder = false;
    let data_dir = app.active_data_dir();
    let hover = format!(
        "Open {} folder\n{}",
        app.active_storage_mode().as_str(),
        data_dir.display()
    );

    CentralPanel::default().show(root_ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("Activity");
                ui.label(
                    RichText::new("Monitor corrections, launches, and configuration changes")
                        .small()
                        .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    app.set_current_window(WindowRoute::Groups(GroupRoute::List));
                }
                if ui.button("Clear").clicked() {
                    clear_logs = true;
                }
                if ui.button("Data folder").on_hover_text(hover).clicked() {
                    open_data_folder = true;
                }
            });
        });

        ui.add_space(8.0);

        glass_frame(ui).show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for log_string in app.log_manager.formatted_entries().rev() {
                        inset_frame(ui).show(ui, |ui| {
                            ui.label(RichText::new(log_string));
                        });
                        ui.add_space(4.0);
                    }
                });
        });
    });

    if clear_logs {
        app.clear_logs();
    }

    if open_data_folder {
        app.open_active_data_dir();
    }
}
