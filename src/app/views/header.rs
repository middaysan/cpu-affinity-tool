use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    paint_focus_ring, palette, row_fill, toned_button, toned_sized_button, ToneRole,
    BUTTON_FONT_SIZE,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Color32, Layout, Margin, Panel, RichText, Stroke};

const NAVIGATION_SWITCH_WIDTH: f32 = 168.0;
fn settings_menu(app: &mut AppState, ui: &mut egui::Ui) {
    let button = ui.button(RichText::new("Settings").size(BUTTON_FONT_SIZE));
    egui::Popup::menu(&button)
        .width(220.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(RichText::new("Application settings").strong());
            ui.separator();
            ui.label("Theme");
            let mut theme_index = app.get_theme_index();
            ui.horizontal(|ui| {
                for (index, name) in [(0, "System"), (1, "Light"), (2, "Dark")] {
                    ui.selectable_value(&mut theme_index, index, name);
                }
            });
            if theme_index != app.get_theme_index() {
                app.set_theme_index(theme_index);
                ui.ctx().request_repaint();
            }

            #[cfg(all(target_os = "windows", feature = "windows"))]
            {
                ui.separator();
                let mut startup_enabled = app.start_minimized();
                if ui
                    .checkbox(&mut startup_enabled, "Start minimized")
                    .changed()
                {
                    app.ui.startup_setting_error = app.set_start_minimized(startup_enabled).err();
                }
                ui.add(
                    egui::Label::new(
                        RichText::new("Open in the system tray the next time you launch this app.")
                            .small()
                            .weak(),
                    )
                    .wrap(),
                );
                if let Some(error) = &app.ui.startup_setting_error {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(ui.visuals().error_fg_color))
                            .wrap(),
                    );
                }
            }
        });
}

fn centered_leading_space(available_width: f32, content_width: f32) -> f32 {
    ((available_width - content_width) * 0.5).max(0.0)
}

fn navigation_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
) -> egui::Response {
    let button = egui::Button::new(label).selected(selected);
    if selected {
        toned_sized_button(ui, [82.0, 20.0], button, ToneRole::Selected)
    } else {
        let response = ui.add_sized(
            [82.0, 20.0],
            button.fill(Color32::TRANSPARENT).stroke(Stroke::NONE),
        );
        paint_focus_ring(ui, &response);
        response
    }
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
                #[cfg(all(target_os = "windows", feature = "windows"))]
                let activity_selected =
                    activity_selected || matches!(app.ui.current_window, WindowRoute::CrashReports);
                let activity_label = "Activity";
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
    let cpu_model = app.get_cpu_schema().model;
    let total_threads = app.ui.group_form.core_selection.len();
    Panel::top("top_panel")
        .frame(
            egui::Frame::NONE
                .fill(root_ui.visuals().panel_fill)
                .inner_margin(Margin::symmetric(8, 6)),
        )
        .show(root_ui, |ui| {
            egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let colors = palette(ui);
                        let cpu_badge = egui::Frame::NONE
                            .fill(colors.accent.fill)
                            .stroke(Stroke::new(1.0, colors.accent.border))
                            .corner_radius(5.0)
                            .inner_margin(Margin::symmetric(5, 2))
                            .show(ui, |ui| {
                                ui.label(RichText::new("CPU").color(colors.accent.fg).strong());
                            });
                        cpu_badge
                            .response
                            .on_hover_text(format!("{cpu_model}\n{total_threads} logical threads"));

                        ui.label(RichText::new("CPU Affinity Tool").heading().strong());

                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            settings_menu(app, ui);
                            if toned_button(
                                ui,
                                egui::Button::new(
                                    RichText::new("+ New group").size(BUTTON_FONT_SIZE).strong(),
                                ),
                                ToneRole::Primary,
                            )
                            .clicked()
                            {
                                app.start_creating_group();
                            }
                        });
                    });

                    ui.add_space(1.0);
                    ui.separator();

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
