use crate::app::models::APP_VERSION;
use crate::app::runtime::AppState;
use eframe::egui::{self, Color32, Layout, Margin, RichText, TopBottomPanel};

/// Draws the bottom panel (footer) of the application.
///
/// This panel contains:
/// - A toggle button for enabling/disabling automatic CPU settings re-apply
/// - A label showing the current status of the automatic correction feature
///
/// # Parameters
///
/// * `app` - The application state
/// * `ctx` - The egui context
pub fn draw_bottom_panel(app: &mut AppState, ctx: &egui::Context) {
    TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
        egui::Frame::NONE
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Get the current auto re-apply status
                    let monitoring_enabled = app.is_process_monitoring_enabled();

                    // Create the toggle button with appropriate icon and label
                    let (icon, label, color) = if monitoring_enabled {
                        ("🔄", "ACTIVE", Color32::from_rgb(0, 200, 0))
                    } else {
                        (
                            "⏹",
                            "DISABLED",
                            ui.visuals().widgets.noninteractive.fg_stroke.color,
                        )
                    };

                    // Add the toggle button with hover text
                    if ui
                        .button(icon)
                        .on_hover_text(
                            "Keeps tracked app processes on their assigned CPU cores\n\
                                 and restores priority if they change settings",
                        )
                        .clicked()
                    {
                        app.toggle_process_monitoring();
                    }

                    ui.add_space(4.0);
                    // Add a label explaining the feature
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Auto Re-apply App Process Affinity: ")
                                .color(ui.visuals().widgets.noninteractive.fg_stroke.color)
                                .small()
                                .strong(),
                        );
                        ui.label(RichText::new(label).color(color).small().strong());
                    });

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("v{APP_VERSION}"))
                                .small()
                                .weak(),
                        );
                    });
                });
            });
    });
}
