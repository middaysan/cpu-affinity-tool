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

fn show_tracked_process_tooltip<R>(
    response: &egui::Response,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    let ctx = &response.ctx;
    let id = response.id.with("tracked_process_hover");
    let (mut rect, mut last_hover) = ctx.data(|data| {
        data.get_temp::<(egui::Rect, f64)>(id)
            .unwrap_or((egui::Rect::NOTHING, f64::NEG_INFINITY))
    });
    let now = ctx.input(|input| input.time);
    let over_list = ctx.input(|input| {
        input
            .pointer
            .hover_pos()
            .is_some_and(|pos| rect.contains(pos))
    });
    if response.contains_pointer() || over_list {
        last_hover = now;
    }
    // Keep the list reachable across the small gap, including while scrolling it.
    let open = now - last_hover < 0.2;
    let shown = egui::Popup::from_response(response)
        .id(id.with("popup"))
        .kind(egui::PopupKind::Tooltip)
        .layout(Layout::top_down(egui::Align::Min))
        .width(300.0)
        .open(open)
        .show(content);
    if let Some(shown) = &shown {
        rect = shown.response.rect;
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    } else {
        rect = egui::Rect::NOTHING;
    }
    ctx.data_mut(|data| data.insert_temp(id, (rect, last_hover)));
    shown
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
                    let (toggle_icon, toggle_label) = monitor_toggle_icon(monitoring_enabled);
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

                    let (dot_rect, dot_response) =
                        ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 3.5, color);

                    let status_response = ui.vertical(|ui| {
                        let title = ui.label(
                            RichText::new(label)
                                .size(BUTTON_FONT_SIZE)
                                .family(inter_medium_family())
                                .color(color)
                                .strong(),
                        );
                        let detail = ui.label(RichText::new(detail).size(8.5).color(colors.text_muted));
                        title.union(detail)
                    }).inner.union(dot_response);
                    show_tracked_process_tooltip(&status_response, |ui| {
                        let tracked_processes = app.tracked_processes_snapshot();
                        ui.label(RichText::new("Tracked processes").strong());
                        ui.add_space(2.0);
                        draw_tracked_process_list(ui, &tracked_processes);
                    });

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{APP_VERSION}"))
                                .size(8.5)
                                .color(colors.text_muted),
                        );

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
    fn monitor_hover_opens_without_click_and_keeps_list_reachable() {
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        let ctx = egui::Context::default();
        let mut status = Rect::NOTHING;
        let mut popup = Rect::NOTHING;
        for (time, pointer, expected) in [
            (0.0, None, false),
            (1.0, Some(Pos2::new(20.0, 20.0)), true),
            (1.05, Some(Pos2::new(300.0, 200.0)), true),
            (1.5, None, true),
            (2.0, Some(Pos2::new(300.0, 200.0)), false),
        ] {
            let pointer = if time == 1.5 {
                Some(popup.center())
            } else {
                pointer
            };
            let _ = ctx.run_ui(
                RawInput {
                    time: Some(time),
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, 400.0))),
                    events: pointer.into_iter().map(egui::Event::PointerMoved).collect(),
                    ..Default::default()
                },
                |ui| {
                    let response =
                        ui.allocate_response(Vec2::new(180.0, 30.0), egui::Sense::hover());
                    status = response.rect;
                    let shown = super::show_tracked_process_tooltip(&response, |ui| {
                        ui.label("sample.exe · PID 701 · Games");
                    });
                    assert_eq!(shown.is_some(), expected, "time={time}, status={status:?}");
                    if let Some(shown) = shown {
                        popup = shown.response.rect;
                    }
                },
            );
        }
        assert!(popup.is_positive());
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
