#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::app::features::diagnostics::crash_reports::CrashReportIndicator;
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    paint_focus_ring, palette, row_fill, toned_button, toned_sized_button, ToneRole,
    BUTTON_FONT_SIZE,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Color32, Layout, Margin, Panel, RichText, Stroke};

const NAVIGATION_SWITCH_WIDTH: f32 = 168.0;
const THEME_BUTTON_SIZE: egui::Vec2 = egui::vec2(22.0, 20.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThemeIcon {
    System,
    Light,
    Dark,
}

fn theme_button_spec(theme_index: usize) -> (ThemeIcon, &'static str) {
    match theme_index {
        0 => (ThemeIcon::System, "System theme"),
        1 => (ThemeIcon::Light, "Light theme"),
        _ => (ThemeIcon::Dark, "Dark theme"),
    }
}

fn theme_widget_info(enabled: bool, label: &str) -> egui::WidgetInfo {
    egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
}

fn paint_theme_icon(ui: &egui::Ui, response: &egui::Response, icon: ThemeIcon, stroke: Stroke) {
    let center = response.rect.center();
    let painter = ui.painter();

    match icon {
        ThemeIcon::System => {
            let screen =
                egui::Rect::from_center_size(center - egui::vec2(0.0, 1.0), egui::vec2(10.0, 7.0));
            painter.rect_stroke(screen, 1.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment(
                [
                    egui::pos2(center.x, screen.bottom()),
                    egui::pos2(center.x, screen.bottom() + 2.5),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - 3.0, screen.bottom() + 2.5),
                    egui::pos2(center.x + 3.0, screen.bottom() + 2.5),
                ],
                stroke,
            );
        }
        ThemeIcon::Light => {
            painter.circle_stroke(center, 2.4, stroke);
            for ray in 0..8 {
                let angle = ray as f32 * std::f32::consts::TAU / 8.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 4.1, center + direction * 5.4], stroke);
            }
        }
        ThemeIcon::Dark => {
            let mut points = Vec::with_capacity(20);
            for step in 0..=10 {
                let angle =
                    -std::f32::consts::FRAC_PI_2 - step as f32 * std::f32::consts::PI / 10.0;
                points.push(center + egui::vec2(angle.cos(), angle.sin()) * 5.0);
            }
            for step in 1..=8 {
                let t = step as f32 / 8.0;
                let top = egui::vec2(0.0, -5.0);
                let bottom = egui::vec2(0.0, 5.0);
                let control = egui::vec2(4.0, 0.0);
                points.push(
                    center
                        + bottom * (1.0 - t).powi(2)
                        + control * (2.0 * (1.0 - t) * t)
                        + top * t.powi(2),
                );
            }
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

fn theme_button(ui: &mut egui::Ui, theme_index: usize) -> egui::Response {
    let (icon, label) = theme_button_spec(theme_index);
    let (_, response) = ui.allocate_exact_size(THEME_BUTTON_SIZE, egui::Sense::click());
    let response = response.on_hover_text(label);
    let colors = palette(ui);
    let (fill, icon_color) = if !response.enabled() {
        (
            Color32::TRANSPARENT,
            ui.visuals().widgets.noninteractive.fg_stroke.color,
        )
    } else if response.is_pointer_button_down_on() {
        (colors.inset, colors.text_primary)
    } else if response.hovered() {
        (colors.row, colors.text_primary)
    } else {
        (Color32::TRANSPARENT, colors.text_secondary)
    };
    ui.painter().rect(
        response.rect,
        5.0,
        fill,
        Stroke::NONE,
        egui::StrokeKind::Middle,
    );
    response.widget_info(|| theme_widget_info(response.enabled(), label));
    paint_theme_icon(ui, &response, icon, Stroke::new(1.0, icon_color));
    paint_focus_ring(ui, &response);
    response
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn crash_reports_button(
    ui: &mut egui::Ui,
    indicator: &CrashReportIndicator,
    selected: bool,
) -> egui::Response {
    let width = if indicator.count.unwrap_or_default() > 0 {
        38.0
    } else {
        22.0
    };
    let (_, response) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::click());
    let response = response.on_hover_text(&indicator.label);
    let colors = palette(ui);
    let fill = if selected || response.is_pointer_button_down_on() {
        colors.inset
    } else if response.hovered() {
        colors.row
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(
        response.rect,
        5.0,
        fill,
        Stroke::NONE,
        egui::StrokeKind::Middle,
    );

    let icon_center = egui::pos2(response.rect.left() + 11.0, response.rect.center().y);
    let icon_rect = egui::Rect::from_center_size(icon_center, egui::vec2(8.0, 10.0))
        .translate(egui::vec2(0.0, -0.5));
    let icon_color = if selected {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    ui.painter().rect_stroke(
        icon_rect,
        1.0,
        Stroke::new(1.0, icon_color),
        egui::StrokeKind::Middle,
    );
    ui.painter().line_segment(
        [
            egui::pos2(icon_rect.left() + 2.0, icon_rect.center().y - 1.5),
            egui::pos2(icon_rect.right() - 2.0, icon_rect.center().y - 1.5),
        ],
        Stroke::new(1.0, icon_color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(icon_rect.left() + 2.0, icon_rect.center().y + 1.5),
            egui::pos2(icon_rect.right() - 2.0, icon_rect.center().y + 1.5),
        ],
        Stroke::new(1.0, icon_color),
    );

    if let Some(count) = indicator.count.filter(|count| *count > 0) {
        ui.painter().text(
            egui::pos2(response.rect.right() - 4.0, response.rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(9.0),
            icon_color,
        );
    }
    if indicator.stale {
        ui.painter().circle_filled(
            egui::pos2(response.rect.right() - 3.0, response.rect.top() + 3.0),
            1.5,
            colors.warning.fg,
        );
    }

    response
        .widget_info(|| crash_reports_widget_info(response.enabled(), selected, &indicator.label));
    paint_focus_ring(ui, &response);
    response
}

#[cfg(any(test, all(target_os = "windows", feature = "windows")))]
fn crash_reports_widget_info(enabled: bool, selected: bool, label: &str) -> egui::WidgetInfo {
    egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
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
    let ctx = root_ui.ctx().clone();
    let cpu_model = app.get_cpu_schema().model;
    let total_threads = app.ui.group_form.core_selection.len();
    Panel::top("top_panel").show(root_ui, |ui| {
        egui::Frame::NONE
            .fill(ui.visuals().panel_fill)
            .inner_margin(Margin::symmetric(8, 3))
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
                        if theme_button(ui, app.get_theme_index()).clicked() {
                            app.toggle_theme();
                            ctx.request_repaint();
                        }
                        #[cfg(all(target_os = "windows", feature = "windows"))]
                        {
                            let crash_report_indicator = app.crash_report_state().indicator();
                            let crash_reports_selected =
                                matches!(app.ui.current_window, WindowRoute::CrashReports);
                            if crash_reports_button(
                                ui,
                                &crash_report_indicator,
                                crash_reports_selected,
                            )
                            .clicked()
                            {
                                app.set_current_window(WindowRoute::CrashReports);
                                app.request_crash_report_refresh();
                            }
                        }
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
    use super::{
        centered_leading_space, crash_reports_widget_info, theme_button_spec, theme_widget_info,
        ThemeIcon,
    };
    use eframe::egui::WidgetType;

    #[test]
    fn test_centered_leading_space_uses_full_available_width() {
        assert_eq!(centered_leading_space(400.0, 180.0), 110.0);
        assert_eq!(centered_leading_space(180.0, 180.0), 0.0);
        assert_eq!(centered_leading_space(160.0, 180.0), 0.0);
    }

    #[test]
    fn test_theme_button_spec_maps_persisted_theme_indices() {
        assert_eq!(theme_button_spec(0), (ThemeIcon::System, "System theme"));
        assert_eq!(theme_button_spec(1), (ThemeIcon::Light, "Light theme"));
        assert_eq!(theme_button_spec(2), (ThemeIcon::Dark, "Dark theme"));
        assert_eq!(theme_button_spec(99), (ThemeIcon::Dark, "Dark theme"));
    }

    #[test]
    fn test_theme_painter_widget_metadata_is_one_labeled_button_contract() {
        let enabled = theme_widget_info(true, "System theme");
        assert_eq!(enabled.typ, WidgetType::Button);
        assert!(enabled.enabled);
        assert_eq!(enabled.label.as_deref(), Some("System theme"));

        let disabled = theme_widget_info(false, "Dark theme");
        assert_eq!(disabled.typ, WidgetType::Button);
        assert!(!disabled.enabled);
        assert_eq!(disabled.label.as_deref(), Some("Dark theme"));
    }

    #[test]
    fn crash_report_button_exposes_saved_count_to_accessibility() {
        let info = crash_reports_widget_info(true, true, "Saved crash reports: 2");
        assert_eq!(info.typ, WidgetType::Button);
        assert!(info.enabled);
        assert_eq!(info.selected, Some(true));
        assert_eq!(info.label.as_deref(), Some("Saved crash reports: 2"));
    }
}
