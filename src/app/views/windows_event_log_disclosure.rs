use crate::app::runtime::AppState;
use eframe::egui::{self, RichText};

/// The disclosure is deliberately a shell modal rather than an Activity route:
/// it must be visible before the first Event Log lookup can be armed.
pub fn draw_windows_event_log_disclosure(app: &mut AppState, root_ui: &mut egui::Ui) {
    if !app.windows_event_log_disclosure_required() {
        return;
    }

    let mut choice = None;
    egui::Window::new("Windows crash diagnostics")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(root_ui.ctx(), |ui| {
            ui.label(RichText::new("Optional local Windows Event Log lookup").strong());
            ui.add_space(6.0);
            ui.label(
                "After you continue, the app may read a small, recent subset of the local Application log to find an unverified crash record for this executable.",
            );
            ui.label(
                "It does not upload data, change Windows settings, read raw event XML, or copy anything to the clipboard.",
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Activity can show only the record time, exception code, and faulting module name.",
                )
                .small()
                .weak(),
            );
            if let Some(error) = &app.ui.windows_event_log_disclosure_error {
                ui.add_space(6.0);
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Disable").clicked() {
                    choice = Some(false);
                }
                if ui.button("Continue").clicked() {
                    choice = Some(true);
                }
            });
        });

    if let Some(enabled) = choice {
        match app.choose_windows_event_log_diagnostics(enabled) {
            Ok(()) => app.ui.windows_event_log_disclosure_error = None,
            Err(error) => app.ui.windows_event_log_disclosure_error = Some(error),
        }
    }
}
