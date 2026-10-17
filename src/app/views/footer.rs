use crate::app::runtime::{AppState, TrackedProcessSnapshot};
use crate::app::shell::presenters::shared_elements::{
    inter_medium_family, palette, success_color, toned_sized_button, ToneRole, UiPalette,
    BUTTON_FONT_SIZE,
};
use eframe::egui::{self, Layout, Margin, Panel, RichText, Stroke, Vec2};

fn monitor_toggle_icon(monitoring_enabled: bool) -> (&'static str, &'static str) {
    if monitoring_enabled {
        ("⏸", "Pause monitor")
    } else {
        ("▶", "Resume monitor")
    }
}

const TRACKED_ROW_HEIGHT: f32 = 16.0;
const TRACKED_COLUMN_GAP: f32 = 6.0;
const TRACKED_TABLE_GUTTER: f32 = 8.0;

fn tracked_text_cell(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    id: &'static str,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let allow_wheel =
        ui.input(|input| input.modifiers.shift || input.smooth_scroll_delta().x != 0.0);
    let cell = ui.allocate_ui_with_layout(
        egui::vec2(width, TRACKED_ROW_HEIGHT),
        Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, TRACKED_ROW_HEIGHT));
            egui::ScrollArea::horizontal()
                .id_salt(id)
                .max_width(width)
                .max_height(TRACKED_ROW_HEIGHT)
                .auto_shrink([false, true])
                .content_margin(0)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                // A regular vertical wheel belongs to the surrounding list.
                .scroll_source(egui::scroll_area::ScrollSource {
                    mouse_wheel: allow_wheel,
                    drag: egui::scroll_area::DragScroll::Always,
                    scroll_bar: false,
                })
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .size(BUTTON_FONT_SIZE)
                                .family(inter_medium_family()),
                        )
                        .extend(),
                    );
                })
        },
    );
    if cell.inner.content_size.x > cell.inner.inner_rect.width() {
        cell.response
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(format!(
                "{text}\nDrag horizontally or use Shift + mouse wheel to scroll this cell."
            ));
    }
    cell.inner
}

fn draw_tracked_process_list(
    ui: &mut egui::Ui,
    processes: &[TrackedProcessSnapshot],
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let max_height = (ui.ctx().content_rect().height() - 100.0).clamp(40.0, 280.0);
    let scroll_height = (max_height - 22.0).max(TRACKED_ROW_HEIGHT);
    let scroll = egui::ScrollArea::vertical()
        .id_salt("tracked_process_list")
        // Do not inherit a small viewport from the popup's previous content size.
        // Auto-shrink still fits short lists without leaving an empty footer.
        .min_scrolled_height(scroll_height)
        .max_height(scroll_height)
        .max_width(ui.available_width().max(1.0))
        .auto_shrink([false, true])
        .content_margin(0);
    if processes.is_empty() {
        return scroll.show(ui, |ui| {
            ui.label(
                RichText::new("No running process is currently verified.")
                    .small()
                    .weak(),
            );
        });
    }
    ui.spacing_mut().item_spacing = egui::vec2(TRACKED_COLUMN_GAP, 2.0);
    // Keep the final column clear of the floating vertical scrollbar as well.
    let width =
        (ui.available_width() - ui.spacing().scroll.allocated_width() - TRACKED_TABLE_GUTTER)
            .max(120.0);
    let names_width = (width - 56.0 - 2.0 * TRACKED_COLUMN_GAP).max(30.0);
    let widths = [names_width * 0.6, 56.0, names_width * 0.4];
    ui.horizontal(|ui| {
        for (label, width) in ["Process", "PID", "Group"].into_iter().zip(widths) {
            ui.allocate_ui_with_layout(
                egui::vec2(width, TRACKED_ROW_HEIGHT),
                Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(width);
                    ui.label(RichText::new(label).size(9.5).weak());
                },
            );
        }
    });
    ui.separator();
    scroll.show_rows(ui, TRACKED_ROW_HEIGHT, processes.len(), |ui, rows| {
        for row in rows {
            let process = &processes[row];
            if row % 2 == 1 {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(width, TRACKED_ROW_HEIGHT),
                    ),
                    2.0,
                    palette(ui).row,
                );
            }
            ui.push_id(
                (process.pid, &process.group_name, &process.process_name),
                |ui| {
                    ui.horizontal(|ui| {
                        tracked_text_cell(ui, &process.process_name, widths[0], "process");
                        tracked_text_cell(ui, &process.pid.to_string(), widths[1], "pid");
                        tracked_text_cell(ui, &process.group_name, widths[2], "group");
                    });
                },
            );
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
                    if toned_sized_button(
                            ui,
                            [28.0, 26.0],
                            egui::Button::new(RichText::new(toggle_icon).size(16.0)),
                            ToneRole::Primary,
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
                        app.refresh_tracked_processes(std::time::Instant::now());
                        let tracked = &app.ui.tracked_processes;
                        let heading = if tracked.updating { "Tracked processes · updating…" } else { "Tracked processes" };
                        ui.add(egui::Label::new(RichText::new(heading).strong()).truncate())
                            .on_hover_text("Updated once per second while open. If tracking data is busy, the previous snapshot remains visible until the next completed refresh.");
                        ui.add_space(2.0);
                        if tracked.updating && tracked.rows.is_empty() {
                            ui.label(RichText::new("Waiting for process data…").small().weak());
                        } else {
                            draw_tracked_process_list(ui, &tracked.rows);
                        }
                    });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::{footer_frame, monitor_toggle_icon};
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
    fn monitor_control_uses_the_shared_primary_button_color() {
        use super::{draw_bottom_panel, palette, AppState};
        use eframe::egui;
        use std::sync::{Arc, RwLock};
        for dark in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(
                    crate::app::models::AppStateStorage::test_fixture(|_| {}),
                )),
                20,
            );
            let mut fill = egui::Color32::TRANSPARENT;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                fill = palette(ui).primary.fill;
                draw_bottom_panel(&mut app, ui);
            });
            output.textures_delta.clear();
            assert!(
                output.shapes.iter().any(
                    |shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == fill)
                ),
                "monitor button must share New group's primary fill"
            );
        }
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
            let mut output = ctx.run_ui(
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
            output.textures_delta.clear();
        }
        assert!(popup.is_positive());
    }

    #[test]
    fn monitor_popup_height_recovers_after_list_changes_and_stays_stable_on_hover() {
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        for dark in [true, false] {
            for height in [240.0, 600.0, 1000.0] {
                let ctx = egui::Context::default();
                ctx.set_fonts(
                    crate::app::shell::presenters::shared_elements::ui_font_definitions(),
                );
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let processes: Vec<_> = (0..40)
                    .map(|pid| crate::app::runtime::TrackedProcessSnapshot {
                        process_name: if pid < 10 {
                            "chrome.exe"
                        } else {
                            "ChatGPT.exe"
                        }
                        .into(),
                        pid: 1000 + pid,
                        group_name: "Background".into(),
                    })
                    .collect();
                let mut targets = [Pos2::ZERO; 3];
                let mut scroll_id = None;
                let mut full_height = None;
                for frame in 0..39 {
                    let pointer = match frame {
                        0..=2 | 15..=18 => Pos2::new(395.0, 5.0),
                        3..=6 | 19..=34 => targets[0],
                        7..=10 => targets[1],
                        _ => targets[2],
                    };
                    let rows = match frame {
                        0..=6 | 23..=26 => &processes[..3],
                        27..=30 => &processes[..0],
                        _ => &processes[..],
                    };
                    let mut output = ctx.run_ui(RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, height))),
                        time: Some(frame as f64 * 0.1),
                        events: vec![egui::Event::PointerMoved(pointer)],
                        ..Default::default()
                    }, |root| {
                        egui::Panel::bottom("test-footer").show(root, |ui| {
                            ui.horizontal(|ui| {
                                ui.allocate_response(Vec2::new(28.0, 26.0), egui::Sense::hover());
                                let (_, dot) = ui.allocate_exact_size(Vec2::splat(7.0), egui::Sense::hover());
                                let status = ui.vertical(|ui| {
                                    let title = ui.label(egui::RichText::new("Monitoring paused").size(10.5));
                                    let detail = ui.label(egui::RichText::new("Automatic corrections are disabled").size(8.5));
                                    targets = [title.rect.center(), detail.rect.center(), dot.rect.center()];
                                    title.union(detail)
                                }).inner.union(dot);
                                let shown = super::show_tracked_process_tooltip(&status, |ui| {
                                    ui.label(egui::RichText::new("Tracked processes").strong());
                                    ui.add_space(2.0);
                                    super::draw_tracked_process_list(ui, rows)
                                });
                                let settled = matches!(frame, 5..=6 | 9..=14 | 21..=22 | 25..=26 | 29..=30 | 33..=38);
                                if settled {
                                    let shown = shown.expect("hovering any status region must keep one popup open");
                                    let table = shown.inner;
                                    assert_eq!(*scroll_id.get_or_insert(table.id), table.id);
                                    if rows.len() == 40 {
                                        let expected = (height - 100.0).clamp(40.0, 280.0) - 22.0;
                                        assert!((table.inner_rect.height() - expected).abs() < 1.0,
                                            "popup must recover full height after short/empty data, frame={frame}, height={height}, viewport={:?}", table.inner_rect);
                                        assert!((*full_height.get_or_insert(table.inner_rect.height()) - table.inner_rect.height()).abs() < 1.0);
                                        if matches!(frame, 14 | 21..=22) {
                                            assert!(table.state.offset.y > 200.0, "resizing and reopening must preserve scrolling");
                                        }
                                    } else {
                                        // ScrollAreaOutput retains the pre-shrink viewport; the popup
                                        // response is the actual visible size after auto-shrink.
                                        assert!(shown.response.rect.height() < 150.0 && table.content_size.y < 90.0,
                                            "short lists must still fit their content: frame={frame}, height={height}, rows={}, popup={:?}, content={:?}", rows.len(), shown.response.rect, table.content_size);
                                    }
                                    let rect = shown.response.rect;
                                    assert!(rect.top() >= 0.0 && rect.bottom() <= height && rect.left() >= 0.0 && rect.right() <= 400.0,
                                        "popup must fit its viewport: {rect:?}");
                                    if frame == 12 {
                                        let mut state = table.state;
                                        state.offset.y = 220.0;
                                        state.store(&ctx, table.id);
                                    }
                                }
                            });
                        });
                    });
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn cached_process_rows_keep_scroll_and_table_visible_during_busy_refresh() {
        use crate::app::shell::UiSession;
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        use std::time::{Duration, Instant};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let processes: Vec<_> = (0..40)
            .map(|pid| crate::app::runtime::TrackedProcessSnapshot {
                process_name: "sample.exe".into(),
                pid: 1000 + pid,
                group_name: "Background".into(),
            })
            .collect();
        let mut cache = UiSession::new(20).tracked_processes;
        let start = Instant::now();
        let mut refreshes = 0;
        let mut scroll_id = None;
        for frame in 0..156 {
            let now = start + Duration::from_millis(frame * 20);
            if cache.refresh_due(now) {
                let next = match refreshes {
                    0 | 2 => Some(processes.clone()),
                    1 => None,
                    3 => Some(Vec::new()),
                    _ => panic!("refresh must happen only once per second"),
                };
                cache.publish(now, next);
                refreshes += 1;
            }
            let mut offset = 0.0;
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let table = super::draw_tracked_process_list(ui, &cache.rows);
                    assert_eq!(*scroll_id.get_or_insert(table.id), table.id);
                    offset = table.state.offset.y;
                    if frame == 2 {
                        let mut state = table.state;
                        state.offset.y = 220.0;
                        state.store(&ctx, table.id);
                    }
                },
            );
            output.textures_delta.clear();
            if (3..150).contains(&frame) {
                assert!(
                    offset > 200.0,
                    "Busy refresh must never reset scrolling, frame={frame}"
                );
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "sample.exe")));
            }
            if frame >= 150 {
                assert_eq!(offset, 0.0);
            }
        }
        assert_eq!(refreshes, 4);
    }

    #[test]
    fn tracked_process_table_aligns_pid_and_clips_long_text_in_its_own_column() {
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let processes = vec![crate::app::runtime::TrackedProcessSnapshot {
            process_name: "very-long-process-name-".repeat(10),
            pid: 9876,
            group_name: "very-long-group-name-".repeat(10),
        }];
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 400.0))),
                ..Default::default()
            },
            |ui| {
                super::draw_tracked_process_list(ui, &processes);
            },
        );
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.as_str(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                    shape.clip_rect,
                )),
                _ => None,
            })
            .collect();
        let column = |label: &str| {
            texts
                .iter()
                .find(|(text, _, _)| *text == label)
                .expect("table column heading missing")
                .1
        };
        let process_column = column("Process");
        let pid_column = column("PID");
        let group_column = column("Group");
        assert!(
            process_column.left() < pid_column.left() && pid_column.left() < group_column.left()
        );
        let pid = column("9876");
        assert!(
            (pid.left() - pid_column.left()).abs() < 1.0,
            "PID values must align independently of name length"
        );
        let (_, process_rect, process_clip) = texts
            .iter()
            .find(|(text, _, _)| text.starts_with("very-long-process"))
            .unwrap();
        assert!(
            process_rect.intersect(*process_clip).right() < pid_column.left(),
            "process text must not spill into PID"
        );
        let (_, group_rect, group_clip) = texts
            .iter()
            .find(|(text, _, _)| text.starts_with("very-long-group"))
            .unwrap();
        assert!(
            group_rect.intersect(*group_clip).right() <= 320.0,
            "group text must remain inside popup width"
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
                let mut output = ctx.run_ui(
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
                        assert!(
                            output.content_size.x <= output.inner_rect.width() + 1.0,
                            "long cells must not widen the table: content={:?}, viewport={:?}",
                            output.content_size,
                            output.inner_rect
                        );
                        if frame == 0 {
                            let mut state = output.state;
                            state.offset.y = output.content_size.y;
                            state.store(&ctx, output.id);
                        } else {
                            assert!(output.state.offset.y > 0.0);
                        }
                    },
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn long_cells_scroll_sideways_while_the_list_keeps_vertical_wheel_scrolling() {
        use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};
        for horizontal in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            let processes: Vec<_> = (0..100)
                .map(|pid| crate::app::runtime::TrackedProcessSnapshot {
                    process_name: "long-process-name-".repeat(20),
                    pid,
                    group_name: "Background".into(),
                })
                .collect();
            let mut target = Pos2::new(20.0, 10.0);
            let mut offset = 0.0;
            let mut pid_left = 0.0;
            for frame in 0..4 {
                let modifiers = if horizontal {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                };
                let mut events = vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(target),
                ];
                if frame == 2 {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -60.0),
                        phase: egui::TouchPhase::Move,
                        modifiers,
                    });
                }
                let mut output = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(320.0, 400.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if horizontal {
                            ui.horizontal(|ui| {
                                let cell = super::tracked_text_cell(
                                    ui,
                                    &processes[0].process_name,
                                    100.0,
                                    "cell",
                                );
                                target = cell.inner_rect.center();
                                offset = cell.state.offset.x;
                                pid_left = ui.label("9876").rect.left();
                            });
                        } else {
                            let table = super::draw_tracked_process_list(ui, &processes);
                            target = table.inner_rect.left_top() + Vec2::new(20.0, 8.0);
                            offset = table.state.offset.y;
                        }
                    },
                );
                output.textures_delta.clear();
            }
            assert!(
                offset > 0.0,
                "wheel must scroll the requested axis, horizontal={horizontal}"
            );
            if horizontal {
                assert!(
                    pid_left >= 100.0 && pid_left < 115.0,
                    "scrolling a name must not shift PID"
                );
            }
        }
    }
}
