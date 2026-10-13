use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    paint_focus_ring, row_fill, toned_sized_button, ToneRole, BUTTON_FONT_SIZE,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Color32, Layout, Margin, Panel, RichText, Stroke};

const NAVIGATION_SWITCH_WIDTH: f32 = 300.0;
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
        toned_sized_button(ui, [98.0, 20.0], button, ToneRole::Selected)
    } else {
        let response = ui.add_sized(
            [98.0, 20.0],
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
                let groups_selected =
                    matches!(app.ui.current_window, WindowRoute::Groups(GroupRoute::List));
                let groups = if groups_selected {
                    RichText::new("Affinity groups")
                        .size(BUTTON_FONT_SIZE)
                        .strong()
                } else {
                    RichText::new("Affinity groups").size(BUTTON_FONT_SIZE)
                };
                if navigation_button(ui, groups, groups_selected).clicked() {
                    app.set_current_window(WindowRoute::Groups(GroupRoute::List));
                }

                if app.modules_enabled() {
                    let modules_selected = matches!(app.ui.current_window, WindowRoute::Tuning);
                    let modules = RichText::new("System Tweaks").size(BUTTON_FONT_SIZE);
                    if navigation_button(ui, modules, modules_selected).clicked() {
                        app.set_current_window(WindowRoute::Tuning);
                    }
                }

                let logs_selected = matches!(app.ui.current_window, WindowRoute::Logs);
                #[cfg(all(target_os = "windows", feature = "windows"))]
                let logs_selected =
                    logs_selected || matches!(app.ui.current_window, WindowRoute::CrashReports);
                let logs = if logs_selected {
                    RichText::new("Logs").size(BUTTON_FONT_SIZE).strong()
                } else {
                    RichText::new("Logs").size(BUTTON_FONT_SIZE)
                };
                if navigation_button(ui, logs, logs_selected).clicked() {
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
                        ui.label(RichText::new("CPU Affinity Tool").heading().strong())
                            .on_hover_text(format!("{cpu_model}\n{total_threads} logical threads"));

                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            settings_menu(app, ui);
                        });
                    });

                    ui.add_space(1.0);
                    ui.separator();

                    let available_width = ui.available_width();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(centered_leading_space(
                            available_width,
                            if app.modules_enabled() {
                                NAVIGATION_SWITCH_WIDTH
                            } else {
                                168.0
                            },
                        ));
                        navigation_switch(app, ui);
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::{centered_leading_space, draw_top_panel};
    use crate::app::models::AppStateStorage;
    use crate::app::runtime::AppState;
    use crate::app::shell::{GroupRoute, WindowRoute};
    use eframe::egui::{self, Pos2, RawInput, Rect};
    use std::sync::{Arc, RwLock};

    #[test]
    fn groups_navigation_and_creation_action_are_in_their_panels() {
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            4,
        );
        let context = egui::Context::default();
        context.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        // Panels measure their height on the first frame; inspect the settled layout.
        context
            .run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    draw_top_panel(&mut app, ui);
                    crate::app::shell::presenters::central::draw_central_panel(&mut app, ui);
                },
            )
            .textures_delta
            .clear();
        let mut output = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
                ..Default::default()
            },
            |ui| {
                draw_top_panel(&mut app, ui);
                crate::app::shell::presenters::central::draw_central_panel(&mut app, ui);
            },
        );
        output.textures_delta.clear();
        let labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos.x)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!labels.iter().any(|(label, _)| label == "CPU"));
        assert!(labels.iter().any(|(label, _)| label == "CPU Affinity Tool"));
        let mut expected = vec!["Affinity groups"];
        if app.modules_enabled() {
            expected.push("System Tweaks");
        }
        expected.push("Logs");
        let positions = expected
            .iter()
            .map(|expected| {
                labels
                    .iter()
                    .find(|(label, _)| label == expected)
                    .unwrap_or_else(|| panic!("Missing navigation label: {expected}; {labels:?}"))
                    .1
            })
            .collect::<Vec<_>>();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            labels
                .iter()
                .filter(|(label, _)| label == "Affinity groups")
                .count(),
            1,
            "The navigation label must not be repeated as a panel heading"
        );
        assert!(!labels
            .iter()
            .any(|(label, _)| matches!(label.as_str(), "Overview" | "Activity" | "CPU Groups")));
        let text_rect = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing label: {label}"))
        };
        let new_group = text_rect("+ New group");
        assert!(new_group.top() > text_rect("Logs").bottom());
        assert!(new_group.left() > 280.0 && new_group.right() <= 400.0);
        let position = new_group.center();
        // Exercise the relocated button through normal pointer input.
        for pressed in [true, false] {
            context
                .run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            egui::vec2(400.0, 600.0),
                        )),
                        events: vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        draw_top_panel(&mut app, ui);
                        crate::app::shell::presenters::central::draw_central_panel(&mut app, ui);
                    },
                )
                .textures_delta
                .clear();
        }
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::Create)
        ));
    }

    #[test]
    fn test_centered_leading_space_uses_full_available_width() {
        assert_eq!(centered_leading_space(400.0, 180.0), 110.0);
        assert_eq!(centered_leading_space(180.0, 180.0), 0.0);
        assert_eq!(centered_leading_space(160.0, 180.0), 0.0);
    }
}
