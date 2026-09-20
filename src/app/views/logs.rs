#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::features::diagnostics::windows_event_log::WindowsEventLogState;
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    content_frame, glass_frame, inset_frame, palette, toned_button, ToneRole, BUTTON_FONT_SIZE,
};
use eframe::egui::{self, CentralPanel, RichText, ScrollArea};

pub fn draw_logs_window(app: &mut AppState, root_ui: &mut egui::Ui) {
    let mut clear_logs = false;
    let mut open_data_folder = false;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let mut open_crash_reports = false;
    let data_dir = app.active_data_dir();
    let hover = format!(
        "Open {} folder\n{}",
        app.active_storage_mode().as_str(),
        data_dir.display()
    );

    #[cfg(all(target_os = "windows", feature = "windows"))]
    let windows_event_snapshot = app.windows_event_log_snapshot();
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let mut event_log_choice = None;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let windows_event_action_message = app.ui.windows_event_log_action_error.clone();

    let entries = &app.log_manager.entries;
    let local_crash_context = app.log_manager.local_crash_context();
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let diagnostics_enabled = app.windows_event_log_diagnostics_enabled();

    CentralPanel::default()
        .frame(content_frame(root_ui))
        .show(root_ui, |ui| {
            ui.heading(RichText::new("Activity").strong());
            ui.label(
                RichText::new("Recent launches, corrections, and monitoring events")
                    .small()
                    .weak(),
            );
            ui.add_space(5.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                #[cfg(all(target_os = "windows", feature = "windows"))]
                {
                    let mut enabled = diagnostics_enabled;
                    if ui
                        .checkbox(&mut enabled, "Event Log diagnostics")
                        .on_hover_text("Read a bounded, local Application Error record lookup")
                        .changed()
                    {
                        event_log_choice = Some(enabled);
                    }
                }
                if ui
                    .button(RichText::new("Data folder").size(BUTTON_FONT_SIZE))
                    .on_hover_text(hover)
                    .clicked()
                {
                    open_data_folder = true;
                }
                #[cfg(all(target_os = "windows", feature = "windows"))]
                if ui
                    .button(RichText::new("Crash reports").size(BUTTON_FONT_SIZE))
                    .on_hover_text(app.crash_report_state().indicator().label)
                    .clicked()
                {
                    open_crash_reports = true;
                }
                if toned_button(
                    ui,
                    egui::Button::new(RichText::new("Clear").size(BUTTON_FONT_SIZE)),
                    ToneRole::Danger,
                )
                .clicked()
                {
                    clear_logs = true;
                }
            });

            ui.add_space(5.0);

            #[cfg(all(target_os = "windows", feature = "windows"))]
            if let Some(message) = &windows_event_action_message {
                ui.colored_label(ui.visuals().error_fg_color, message);
                ui.add_space(5.0);
            }

            if let Some(local_crash_context) = local_crash_context {
                diagnostic_card(ui, "Saved local crash report", local_crash_context, false);
                ui.add_space(5.0);
            }

            #[cfg(all(target_os = "windows", feature = "windows"))]
            draw_windows_event_log_status(ui, &windows_event_snapshot);

            glass_frame(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if entries.is_empty() {
                            ui.label(RichText::new("No activity yet").small().weak().italics());
                        }
                        for (index, entry) in entries.iter().rev().enumerate() {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(5, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(entry.format())
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
    #[cfg(all(target_os = "windows", feature = "windows"))]
    if open_crash_reports {
        app.set_current_window(crate::app::shell::WindowRoute::CrashReports);
        app.request_crash_report_refresh();
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    if let Some(enabled) = event_log_choice {
        match app.set_windows_event_log_diagnostics_enabled(enabled) {
            Ok(()) => app.ui.windows_event_log_action_error = None,
            Err(error) => app.ui.windows_event_log_action_error = Some(error),
        }
    }
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn draw_windows_event_log_status(ui: &mut egui::Ui, state: &WindowsEventLogState) {
    let (title, detail, stale) = match state {
        WindowsEventLogState::Disabled => return,
        WindowsEventLogState::Idle => return,
        WindowsEventLogState::Loading { last_complete } => (
            "Windows Event Log lookup in progress",
            last_complete.as_ref().map(format_event_record),
            true,
        ),
        WindowsEventLogState::Ready {
            latest: Some(record),
        } => (
            "Unverified Windows Event Log record",
            Some(format_event_record(record)),
            record.stale,
        ),
        WindowsEventLogState::Ready { latest: None } => (
            "Windows Event Log: no matching record",
            Some("No matching recent Application Error record was found.".to_string()),
            false,
        ),
        WindowsEventLogState::Incomplete {
            last_complete,
            reason,
        } => (
            "Windows Event Log unavailable",
            Some(match last_complete {
                Some(record) => format!("{reason}\n{}", format_event_record(record)),
                None => reason.clone(),
            }),
            true,
        ),
    };
    diagnostic_card(ui, title, detail.as_deref().unwrap_or(""), stale);
    ui.add_space(5.0);
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn format_event_record(
    record: &crate::app::features::diagnostics::windows_event_log::WindowsEventLogRecord,
) -> String {
    let mut output = format!(
        "Record ID: {}\nUTC time: {}\nException code: 0x{:08X}\nFaulting module: {}",
        record.event_record_id, record.timestamp_utc, record.exception_code, record.faulting_module,
    );
    if let Some(version) = &record.faulting_module_version {
        output.push_str(&format!("\nFaulting module version: {version}"));
    }
    if let Some(offset) = record.faulting_offset {
        output.push_str(&format!("\nFaulting offset: 0x{offset:X}"));
    }
    if let Some(created) = &record.process_creation_time_utc {
        output.push_str(&format!("\nProcess creation UTC: {created}"));
    }
    output
}

fn diagnostic_card(ui: &mut egui::Ui, title: &str, detail: &str, stale: bool) {
    let title_color = if stale {
        palette(ui).text_secondary
    } else {
        palette(ui).text_primary
    };
    inset_frame(ui)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(title).strong().color(title_color)).wrap());
            ui.add(
                egui::Label::new(
                    RichText::new(detail)
                        .small()
                        .color(palette(ui).text_secondary),
                )
                .wrap(),
            );
        });
}
