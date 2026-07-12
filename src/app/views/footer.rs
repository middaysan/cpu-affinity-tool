use crate::app::models::APP_VERSION;
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{success_color, BUTTON_FONT_SIZE};
use eframe::egui::{self, Layout, Margin, Panel, RichText, Vec2};

/// Draws the bottom panel (footer) of the application.
///
/// This panel contains:
/// - A toggle button for enabling/disabling automatic CPU settings re-apply
/// - A label showing the current status of the automatic correction feature
///
/// # Parameters
///
/// * `app` - The application state
/// * `root_ui` - The root egui UI
pub fn draw_bottom_panel(app: &mut AppState, root_ui: &mut egui::Ui) {
    Panel::bottom("bottom_panel").show(root_ui, |ui| {
        egui::Frame::NONE
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let monitoring_enabled = app.is_process_monitoring_enabled();
                    let (label, detail, color) = if monitoring_enabled {
                        (
                            "Monitoring active",
                            "Affinity and priority are protected",
                            success_color(ui),
                        )
                    } else {
                        (
                            "Monitoring paused",
                            "Automatic corrections are disabled",
                            ui.visuals().widgets.noninteractive.fg_stroke.color,
                        )
                    };

                    let (dot_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 4.0, color);

                    ui.vertical(|ui| {
                        ui.label(RichText::new(label).color(color).strong());
                        ui.label(RichText::new(detail).small().weak());
                    });

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!("v{APP_VERSION}")).small().weak());
                        let action_label = if monitoring_enabled {
                            "Pause monitor"
                        } else {
                            "Resume monitor"
                        };
                        if ui
                            .button(RichText::new(action_label).size(BUTTON_FONT_SIZE))
                            .on_hover_text(
                                "Keeps tracked app processes on their assigned CPU cores and restores priority",
                            )
                            .clicked()
                        {
                            app.toggle_process_monitoring();
                        }
                    });
                });
            });
    });
}
