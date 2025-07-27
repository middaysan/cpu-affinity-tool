use crate::app::models::AppState;
use eframe::egui::{self, RichText, TopBottomPanel};

/// Draws the bottom panel (footer) of the application.
///
/// This panel contains:
/// - A toggle button for enabling/disabling process monitoring
/// - A label showing the current status of the monitoring feature
///
/// # Parameters
///
/// * `app` - The application state
/// * `ctx` - The egui context
pub fn draw_bottom_panel(app: &mut AppState, ctx: &egui::Context) {
    TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            // Get the current monitoring status
            let monitoring_enabled = app.is_process_monitoring_enabled();

            // Create the toggle button with appropriate icon and label
            let (icon, label) = if monitoring_enabled {
                ("🔄", "Process Monitoring: ON")
            } else {
                ("⏹", "Process Monitoring: OFF")
            };

            // Add the toggle button with hover text
            if ui.button(icon).on_hover_text("💡 When enabled, automatically restores CPU affinity and priority settings if processes change them").clicked() {
                app.toggle_process_monitoring();
            }

            ui.separator();

            // Add a label explaining the feature
            ui.label(RichText::new(label));
        });
    });
}
