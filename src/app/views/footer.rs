use crate::app::models::APP_VERSION;
use crate::app::runtime::{AppState, TrackedProcessSnapshot};
use crate::app::shell::presenters::shared_elements::{
    inter_medium_family, palette, success_color, UiPalette, BUTTON_FONT_SIZE,
};
use eframe::egui::{self, Layout, Margin, Panel, RichText, Stroke, Vec2};

fn monitor_toggle_icon(monitoring_enabled: bool) -> (&'static str, &'static str) {
    if monitoring_enabled {
        ("⏸", "Pause monitor")
    } else {
        ("▶", "Resume monitor")
    }
}

fn tracked_process_line(process_name: &str, pid: u32, group_name: &str) -> String {
    format!("{process_name} · PID {pid} · {group_name}")
}

fn draw_tracked_process_list(
    ui: &mut egui::Ui,
    processes: &[TrackedProcessSnapshot],
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let max_height = (ui.ctx().content_rect().height() - 100.0).clamp(40.0, 280.0);
    egui::ScrollArea::both()
        .id_salt("tracked_process_list")
        .max_height(max_height)
        .max_width(ui.available_width().max(1.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if processes.is_empty() {
                ui.label(
                    RichText::new("No running process is currently verified.")
                        .small()
                        .weak(),
                );
            } else {
                for process in processes {
                    ui.add(
                        egui::Label::new(
                            RichText::new(tracked_process_line(
                                &process.process_name,
                                process.pid,
                                &process.group_name,
                            ))
                            .family(inter_medium_family())
                            .strong(),
                        )
                        .extend(),
                    );
                }
            }
        })
}

fn tracked_processes_button(ui: &mut egui::Ui) -> egui::Response {
    let response = ui.add_sized([28.0, 26.0], egui::Button::new(""));
    let stroke = ui.style().visuals.widgets.inactive.fg_stroke;
    let rect = response.rect.shrink2(Vec2::new(8.0, 8.0));
    for offset in [0.0, 5.0, 10.0] {
        let y = rect.top() + offset;
        ui.painter().line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
    }
    response
}

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
                        let (toggle_icon, toggle_label) = monitor_toggle_icon(monitoring_enabled);
                        let process_list_button = tracked_processes_button(ui)
                            .on_hover_text("Show processes currently tracked by the monitor");
                        egui::Popup::menu(&process_list_button)
                            .width(300.0)
                            .show(|ui| {
                                let tracked_processes = app.tracked_processes_snapshot();
                                ui.label(RichText::new("Tracked processes").strong());
                                ui.add_space(2.0);
                                draw_tracked_process_list(ui, &tracked_processes);
                            });
                        if ui
                            .add_sized(
                                [28.0, 26.0],
                                egui::Button::new(RichText::new(toggle_icon).size(16.0)),
                            )
                            .on_hover_text(
                                format!(
                                    "{toggle_label}. Keeps tracked processes on their assigned CPU cores and restores priority."
                                ),
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
    use super::{footer_frame, monitor_toggle_icon, tracked_process_line};
    use crate::app::shell::presenters::shared_elements::palette_for_dark_mode;

    #[test]
    fn test_footer_frame_fills_the_entire_panel_surface() {
        let colors = palette_for_dark_mode(true);
        let frame = footer_frame(colors);

        assert_eq!(frame.fill, colors.group);
        assert_eq!(frame.inner_margin, eframe::egui::Margin::symmetric(8, 4));
    }

    #[test]
    fn monitor_toggle_uses_pause_and_resume_icons() {
        assert_eq!(monitor_toggle_icon(true), ("⏸", "Pause monitor"));
        assert_eq!(monitor_toggle_icon(false), ("▶", "Resume monitor"));
    }

    #[test]
    fn tracked_process_line_keeps_name_pid_and_group_together() {
        assert_eq!(
            tracked_process_line("steam.exe", 9268, "Performance"),
            "steam.exe · PID 9268 · Performance"
        );
    }

    #[test]
    fn tracked_process_list_bounds_many_rows_and_scrolls_to_last() {
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let processes = (0..100)
            .map(|pid| crate::app::runtime::TrackedProcessSnapshot {
                group_name: "A very long group name".repeat(10),
                process_name: "sample.exe".into(),
                pid,
            })
            .collect::<Vec<_>>();
        for height in [240.0, 600.0] {
            for frame in 0..2 {
                let _ = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(320.0, height),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let output = super::draw_tracked_process_list(ui, &processes);
                        assert!(output.inner_rect.height() <= 280.0);
                        assert!(output.inner_rect.height() < height);
                        assert!(output.inner_rect.width() <= 320.0);
                        assert!(output.content_size.y > output.inner_rect.height());
                        assert!(output.content_size.x > output.inner_rect.width());
                        if frame == 0 {
                            let mut state = output.state;
                            state.offset.y = output.content_size.y;
                            state.store(&ctx, output.id);
                        } else {
                            assert!(output.state.offset.y > 0.0);
                        }
                    },
                );
            }
        }
    }
}
