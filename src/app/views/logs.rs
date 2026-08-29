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
    let local_crash_context = app.log_manager.local_crash_context().map(str::to_owned);
    let windows_event_context = app.log_manager.windows_event_context().cloned();

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

            if let Some(local_crash_context) = local_crash_context {
                diagnostic_card(ui, "Saved local crash report", &local_crash_context, false);
                ui.add_space(5.0);
            }

            if let Some(context) = windows_event_context {
                let stale = if context.stale { " (stale)" } else { "" };
                let detail = format!(
                    "Record {} at {}\nException code: 0x{:08X}\nFaulting module: {}",
                    context.event_record_id,
                    context.timestamp_utc,
                    context.exception_code,
                    context.faulting_module,
                );
                diagnostic_card(
                    ui,
                    &format!("Unverified Windows Event Log record{stale}"),
                    &detail,
                    context.stale,
                );
                ui.add_space(5.0);
            }

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

fn diagnostic_card(ui: &mut egui::Ui, title: &str, detail: &str, stale: bool) {
    let title_color = if stale {
        palette(ui).text_secondary
    } else {
        palette(ui).text_primary
    };
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong().color(title_color));
            ui.label(
                RichText::new(detail)
                    .small()
                    .color(palette(ui).text_secondary),
            );
        });
}
