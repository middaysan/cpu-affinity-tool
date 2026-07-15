use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    glass_frame, palette, toned_button, ToneRole, BUTTON_FONT_SIZE,
};
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

    let entries = app
        .log_manager
        .formatted_entries()
        .rev()
        .collect::<Vec<_>>();

    CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(root_ui.visuals().panel_fill)
                .inner_margin(egui::Margin::symmetric(6, 4)),
        )
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(RichText::new("Activity").strong());
                    ui.label(
                        RichText::new("Recent launches, corrections, and monitoring events")
                            .small()
                            .weak(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if toned_button(
                        ui,
                        egui::Button::new(RichText::new("Clear").size(BUTTON_FONT_SIZE)),
                        ToneRole::Danger,
                    )
                    .clicked()
                    {
                        clear_logs = true;
                    }
                    if ui
                        .button(RichText::new("Data folder").size(BUTTON_FONT_SIZE))
                        .on_hover_text(hover)
                        .clicked()
                    {
                        open_data_folder = true;
                    }
                });
            });

            ui.add_space(5.0);

            glass_frame(ui).show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if entries.is_empty() {
                            ui.label(RichText::new("No activity yet").small().weak().italics());
                        }
                        for (index, log_string) in entries.iter().enumerate() {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(5, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(log_string)
                                            .size(10.0)
                                            .color(palette(ui).text_secondary),
                                    );
                                });
                            if index + 1 < entries.len() {
                                ui.separator();
                            }
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
