use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    neutral_emphasis_fill, row_fill, BUTTON_FONT_SIZE,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Color32, Layout, Margin, Panel, RichText, Stroke};

const NAVIGATION_SWITCH_WIDTH: f32 = 180.0;

fn centered_leading_space(available_width: f32, content_width: f32) -> f32 {
    ((available_width - content_width) * 0.5).max(0.0)
}

fn navigation_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        neutral_emphasis_fill(ui)
    } else {
        Color32::TRANSPARENT
    };
    ui.add_sized(
        [88.0, 22.0],
        egui::Button::new(label).fill(fill).stroke(if selected {
            ui.visuals().widgets.hovered.bg_stroke
        } else {
            Stroke::NONE
        }),
    )
}

fn navigation_switch(app: &mut AppState, ui: &mut egui::Ui) {
    egui::Frame::NONE
        .fill(row_fill(ui))
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(6.0)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                let overview_selected =
                    matches!(app.ui.current_window, WindowRoute::Groups(GroupRoute::List));
                let overview = if overview_selected {
                    RichText::new("Overview").size(BUTTON_FONT_SIZE).strong()
                } else {
                    RichText::new("Overview").size(BUTTON_FONT_SIZE)
                };
                if navigation_button(ui, overview, overview_selected).clicked() {
                    app.set_current_window(WindowRoute::Groups(GroupRoute::List));
                }

                let activity_selected = matches!(app.ui.current_window, WindowRoute::Logs);
                let activity_label = format!("Activity  {}", app.log_manager.len());
                let activity = if activity_selected {
                    RichText::new(activity_label)
                        .size(BUTTON_FONT_SIZE)
                        .strong()
                } else {
                    RichText::new(activity_label).size(BUTTON_FONT_SIZE)
                };
                if navigation_button(ui, activity, activity_selected).clicked() {
                    app.set_current_window(WindowRoute::Logs);
                }
            });
        });
}

pub fn draw_top_panel(app: &mut AppState, root_ui: &mut egui::Ui) {
    let ctx = root_ui.ctx().clone();
    let cpu_model = app.get_cpu_schema().model;
    let total_threads = app.ui.group_form.core_selection.len();
    Panel::top("top_panel").show(root_ui, |ui| {
        egui::Frame::NONE
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let cpu_badge = egui::Frame::NONE
                        .fill(neutral_emphasis_fill(ui))
                        .corner_radius(5.0)
                        .inner_margin(Margin::symmetric(6, 3))
                        .show(ui, |ui| {
                            ui.label(RichText::new("CPU").strong());
                        });
                    cpu_badge
                        .response
                        .on_hover_text(format!("{cpu_model}\n{total_threads} logical threads"));

                    ui.label(RichText::new("CPU Affinity Tool").heading().strong());

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        let (icon, label) = match app.get_theme_index() {
                            0 => ("\u{1F4BB}", "System theme"),
                            1 => ("\u{2600}", "Light theme"),
                            _ => ("\u{1F319}", "Dark theme"),
                        };
                        if ui
                            .button(RichText::new(icon).size(12.7))
                            .on_hover_text(label)
                            .clicked()
                        {
                            app.toggle_theme();
                            ctx.request_repaint();
                        }
                        if ui
                            .button(RichText::new("+ New group").size(BUTTON_FONT_SIZE).strong())
                            .clicked()
                        {
                            app.start_creating_group();
                        }
                    });
                });

                ui.add_space(2.0);
                ui.separator();
                ui.add_space(1.0);

                let available_width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(centered_leading_space(
                        available_width,
                        NAVIGATION_SWITCH_WIDTH,
                    ));
                    navigation_switch(app, ui);
                });
            });
    });
}

#[cfg(test)]
mod tests {
    use super::centered_leading_space;

    #[test]
    fn test_centered_leading_space_uses_full_available_width() {
        assert_eq!(centered_leading_space(400.0, 180.0), 110.0);
        assert_eq!(centered_leading_space(180.0, 180.0), 0.0);
        assert_eq!(centered_leading_space(160.0, 180.0), 0.0);
    }
}
