use crate::app::models::APP_VERSION;
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    inter_medium_family, palette, success_color, UiPalette, BUTTON_FONT_SIZE,
};
use eframe::egui::{self, Layout, Margin, Panel, RichText, Stroke, Vec2};

fn footer_frame(colors: &UiPalette) -> egui::Frame {
    egui::Frame::NONE
        .fill(colors.group)
        .stroke(Stroke::new(1.0, colors.border_subtle))
        .inner_margin(Margin::symmetric(8, 4))
}

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
    let colors = *palette(root_ui);
    Panel::bottom("bottom_panel")
        .frame(footer_frame(&colors))
        .show(root_ui, |ui| {
        let monitoring_enabled = app.is_process_monitoring_enabled();
        ui.horizontal(|ui| {
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
                            colors.neutral_status,
                        )
                    };

                    let (dot_rect, _) =
                        ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 3.5, color);

                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(label)
                                .size(BUTTON_FONT_SIZE)
                                .family(inter_medium_family())
                                .color(color)
                                .strong(),
                        );
                        ui.label(RichText::new(detail).size(8.5).color(colors.text_muted));
                    });

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{APP_VERSION}"))
                                .size(8.5)
                                .color(colors.text_muted),
                        );
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
}

#[cfg(test)]
mod tests {
    use super::footer_frame;
    use crate::app::shell::presenters::shared_elements::palette_for_dark_mode;

    #[test]
    fn test_footer_frame_fills_the_entire_panel_surface() {
        let colors = palette_for_dark_mode(true);
        let frame = footer_frame(colors);

        assert_eq!(frame.fill, colors.group);
        assert_eq!(frame.inner_margin, eframe::egui::Margin::symmetric(8, 4));
    }
}
