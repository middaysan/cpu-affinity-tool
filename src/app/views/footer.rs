use crate::app::models::AppState;
use eframe::egui::{self, Color32, Layout, Margin, RichText, TopBottomPanel};
use crate::app::models::APP_VERSION;

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
    TopBottomPanel::bottom("bottom_panel")
        .show(ctx, |ui| {
            egui::Frame::NONE
                .inner_margin(Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Get the current monitoring status
                        let monitoring_enabled = app.is_process_monitoring_enabled();

                        // Create the toggle button with appropriate icon and label
                        let (icon, label, color) = if monitoring_enabled {
                            ("🔄", "ACTIVE", Color32::from_rgb(0, 200, 0))
                        } else {
                            ("⏹", "DISABLED", ui.visuals().widgets.noninteractive.fg_stroke.color)
                        };

                        // Add the toggle button with hover text
                        if ui.button(icon).on_hover_text("Automatically restores CPU affinity and priority\n settings if processes change them").clicked() {
                            app.toggle_process_monitoring();
                        }

                        ui.add_space(4.0);
                        // Add a label explaining the feature
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Process Monitoring: ").color(ui.visuals().widgets.noninteractive.fg_stroke.color).small().strong());
                            ui.label(RichText::new(label).color(color).small().strong());
                        });

                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(format!("v{APP_VERSION}")).small().weak());
                        });
                    });
                });
        });
}
