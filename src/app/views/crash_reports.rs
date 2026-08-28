use crate::app::features::diagnostics::crash_reports::{
    CrashReportEntry, CrashReportIndexState, RETAIN_REPORTS,
};
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    glass_frame, palette, toned_button, ToneRole, BUTTON_FONT_SIZE,
};
use crate::app::shell::WindowRoute;
use eframe::egui::{self, CentralPanel, RichText, ScrollArea};

pub fn draw_crash_reports_window(app: &mut AppState, root_ui: &mut egui::Ui) {
    let index_state = app.crash_report_state().clone();
    let snapshot = index_state.snapshot().cloned();
    let report_directory = app.crash_report_directory();
    let mut open_folder = false;
    let mut request_refresh = false;
    let mut show_report: Option<CrashReportEntry> = None;
    let mut copy_path: Option<String> = None;

    CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(root_ui.visuals().panel_fill)
                .inner_margin(egui::Margin::symmetric(6, 4)),
        )
        .show(root_ui, |ui| {
            if ui.small_button("← Back to Activity").clicked() {
                app.set_current_window(WindowRoute::Logs);
            }
            ui.heading(RichText::new("Crash reports").strong());
            ui.label(
                RichText::new("Local reports created after a main application failure")
                    .small()
                    .weak(),
            );
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(RichText::new("Open folder").size(BUTTON_FONT_SIZE))
                    .on_hover_text(report_directory.display().to_string())
                    .clicked()
                {
                    open_folder = true;
                }
                if ui
                    .button(RichText::new("Copy path").size(BUTTON_FONT_SIZE))
                    .on_hover_text("Copy the crash reports folder path")
                    .clicked()
                {
                    copy_path = Some(report_directory.display().to_string());
                }
                let can_delete_all = index_state.is_ready()
                    && snapshot
                        .as_ref()
                        .is_some_and(|snapshot| !snapshot.reports.is_empty());
                if ui
                    .add_enabled(
                        can_delete_all,
                        egui::Button::new(
                            RichText::new("Delete saved reports").size(BUTTON_FONT_SIZE),
                        ),
                    )
                    .clicked()
                {
                    app.ui.crash_report_delete_saved_confirmation = snapshot.clone();
                    app.ui.crash_report_delete_saved_confirmation_focus_pending = true;
                    app.ui.crash_report_delete_confirmation = None;
                    app.ui.crash_report_delete_confirmation_focus_pending = false;
                }
            });

            ui.add_space(5.0);
            glass_frame(ui).show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "The app does not upload these files. Reports may contain local paths \
                         or other system details. Review a report before sharing it. If redaction \
                         is needed, make a copy and redact the copy.",
                    )
                    .small()
                    .color(palette(ui).text_secondary),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new(format!(
                        "After a successful background refresh, the app keeps the newest \
                         {RETAIN_REPORTS} complete reports; older reports are deleted \
                         automatically."
                    ))
                    .small()
                    .weak(),
                );
            });

            if let Some(reason) = index_state.warning() {
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("List may be incomplete.").strong());
                    ui.label(RichText::new(reason).small());
                    if ui.small_button("Refresh").clicked() {
                        request_refresh = true;
                    }
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(report_directory.display().to_string())
                            .small()
                            .monospace(),
                    )
                    .selectable(true),
                );
            }

            if let Some(message) = &app.ui.crash_report_action_message {
                ui.add_space(4.0);
                ui.label(RichText::new(message).small());
            }

            ui.add_space(5.0);
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match (&index_state, &snapshot) {
                    (
                        CrashReportIndexState::Loading {
                            last_complete: None,
                        },
                        _,
                    ) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading crash reports…");
                        });
                    }
                    (
                        CrashReportIndexState::Incomplete {
                            last_complete: None,
                            ..
                        },
                        _,
                    ) => {
                        ui.label(
                            RichText::new("Crash report count is unavailable.")
                                .small()
                                .italics(),
                        );
                    }
                    (_, Some(snapshot)) if snapshot.reports.is_empty() => {
                        ui.label(
                            RichText::new("No saved crash reports")
                                .small()
                                .weak()
                                .italics(),
                        );
                    }
                    (_, Some(snapshot)) => {
                        for (index, report) in snapshot.reports.iter().enumerate() {
                            draw_report_row(
                                ui,
                                &snapshot.report_directory,
                                report,
                                &mut show_report,
                                &mut copy_path,
                                &mut app.ui.crash_report_delete_confirmation,
                                &mut app.ui.crash_report_delete_confirmation_focus_pending,
                            );
                            if index + 1 < snapshot.reports.len() {
                                ui.separator();
                            }
                        }
                    }
                    _ => {}
                });
        });

    if open_folder {
        app.ui.crash_report_action_message = Some(
            app.open_crash_report_directory()
                .map(|_| "Opened the crash reports folder.".to_string())
                .unwrap_or_else(|error| {
                    format!(
                        "Could not open Explorer: {error}. You can copy the folder path instead."
                    )
                }),
        );
    }
    if let Some(report) = show_report {
        app.ui.crash_report_action_message = Some(
            app.show_crash_report_in_explorer(&report)
                .map(|_| "Shown in Explorer.".to_string())
                .unwrap_or_else(|error| {
                    format!(
                        "Could not show the report in Explorer: {error}. Use Copy path instead."
                    )
                }),
        );
    }
    if let Some(path) = copy_path {
        root_ui.ctx().copy_text(path);
        app.ui.crash_report_action_message = Some("Path copied.".to_string());
    }
    if request_refresh {
        app.request_crash_report_refresh();
    }

    draw_delete_confirmation(app, root_ui.ctx());
    draw_delete_saved_confirmation(app, root_ui.ctx());
}

fn draw_report_row(
    ui: &mut egui::Ui,
    report_directory: &std::path::Path,
    report: &CrashReportEntry,
    show_report: &mut Option<CrashReportEntry>,
    copy_path: &mut Option<String>,
    delete_confirmation: &mut Option<CrashReportEntry>,
    delete_confirmation_focus_pending: &mut bool,
) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(5, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(report.kind.user_title()).strong());
            ui.label(
                RichText::new(format!(
                    "{} · version {} · {}",
                    friendly_timestamp(&report.timestamp_utc),
                    report.app_version,
                    format_size(report.size_bytes)
                ))
                .small()
                .weak(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&report.reason)
                        .small()
                        .color(palette(ui).text_secondary),
                )
                .wrap(),
            );
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(RichText::new("Show in Explorer").size(BUTTON_FONT_SIZE))
                    .clicked()
                {
                    *show_report = Some(report.clone());
                }
                if ui
                    .button(RichText::new("Copy path").size(BUTTON_FONT_SIZE))
                    .clicked()
                {
                    *copy_path = Some(report.path_in(report_directory).display().to_string());
                }
                if toned_button(
                    ui,
                    egui::Button::new(RichText::new("Delete").size(BUTTON_FONT_SIZE)),
                    ToneRole::Danger,
                )
                .clicked()
                {
                    *delete_confirmation = Some(report.clone());
                    *delete_confirmation_focus_pending = true;
                }
            });
        });
}

fn draw_delete_confirmation(app: &mut AppState, context: &egui::Context) {
    let Some(report) = app.ui.crash_report_delete_confirmation.clone() else {
        return;
    };
    let focus_cancel = std::mem::take(&mut app.ui.crash_report_delete_confirmation_focus_pending);
    let mut cancel = false;
    let mut confirm = false;
    let modal = egui::Modal::new(egui::Id::new("delete-crash-report-modal")).show(context, |ui| {
        ui.heading("Delete crash report?");
        ui.label(format!(
            "{} from {} will be permanently deleted.",
            report.kind.user_title(),
            friendly_timestamp(&report.timestamp_utc)
        ));
        ui.horizontal(|ui| {
            let cancel_button = ui.button("Cancel");
            if focus_cancel {
                cancel_button.request_focus();
            }
            if cancel_button.clicked() {
                cancel = true;
            }
            if toned_button(ui, egui::Button::new("Delete"), ToneRole::Danger).clicked() {
                confirm = true;
            }
        });
    });
    cancel |= modal.should_close();

    if confirm {
        app.ui.crash_report_action_message = Some(
            app.delete_crash_report(&report)
                .map(|_| "Crash report deleted.".to_string())
                .unwrap_or_else(|error| format!("Could not delete the crash report: {error}")),
        );
        app.ui.crash_report_delete_confirmation = None;
        app.ui.crash_report_delete_confirmation_focus_pending = false;
    } else if cancel {
        app.ui.crash_report_delete_confirmation = None;
        app.ui.crash_report_delete_confirmation_focus_pending = false;
    }
}

fn draw_delete_saved_confirmation(app: &mut AppState, context: &egui::Context) {
    let Some(snapshot) = app.ui.crash_report_delete_saved_confirmation.clone() else {
        return;
    };
    let count = snapshot.reports.len();
    let focus_cancel =
        std::mem::take(&mut app.ui.crash_report_delete_saved_confirmation_focus_pending);
    let mut cancel = false;
    let mut confirm = false;
    let modal =
        egui::Modal::new(egui::Id::new("delete-saved-crash-reports-modal")).show(context, |ui| {
            ui.heading("Delete saved crash reports?");
            ui.label(format!(
                "Delete these {count} saved crash reports permanently?"
            ));
            ui.horizontal(|ui| {
                let cancel_button = ui.button("Cancel");
                if focus_cancel {
                    cancel_button.request_focus();
                }
                if cancel_button.clicked() {
                    cancel = true;
                }
                if toned_button(
                    ui,
                    egui::Button::new("Delete saved reports"),
                    ToneRole::Danger,
                )
                .clicked()
                {
                    confirm = true;
                }
            });
        });
    cancel |= modal.should_close();

    if confirm {
        app.ui.crash_report_action_message = Some(
            app.delete_saved_crash_reports(&snapshot)
                .map(|deleted| format!("Deleted {deleted} saved crash reports."))
                .unwrap_or_else(|error| format!("Could not delete saved crash reports: {error}")),
        );
        app.ui.crash_report_delete_saved_confirmation = None;
        app.ui.crash_report_delete_saved_confirmation_focus_pending = false;
    } else if cancel {
        app.ui.crash_report_delete_saved_confirmation = None;
        app.ui.crash_report_delete_saved_confirmation_focus_pending = false;
    }
}

fn friendly_timestamp(timestamp: &str) -> String {
    if timestamp.len() != 20 {
        return timestamp.to_string();
    }
    format!(
        "{}-{}-{} {}:{}:{}.{} UTC",
        &timestamp[0..4],
        &timestamp[4..6],
        &timestamp[6..8],
        &timestamp[9..11],
        &timestamp[11..13],
        &timestamp[13..15],
        &timestamp[16..19],
    )
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_size, friendly_timestamp};

    #[test]
    fn crash_report_metadata_formatting_is_compact_and_utc() {
        assert_eq!(
            friendly_timestamp("20250726T121045.678Z"),
            "2025-07-26 12:10:45.678 UTC"
        );
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KiB");
    }
}
