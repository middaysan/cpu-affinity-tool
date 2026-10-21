use crate::app::runtime::{AppState, TrackedProcessSnapshot};
use crate::app::shell::presenters::{
    process_table as table,
    shared_elements::{
        compact_table_spacing, inter_medium_family, palette, service_control, service_empty_state,
        service_modal_header, service_modal_layout, service_tabs, status_button,
        table_row_separator, wrapped_tooltip, COMPACT_TABLE_ROW_HEIGHT,
    },
};
use eframe::egui::{self, RichText};

fn tracked_text_cell(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    id: &'static str,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let allow_wheel =
        ui.input(|input| input.modifiers.shift || input.smooth_scroll_delta().x != 0.0);
    let cell = ui.allocate_ui_with_layout(
        egui::vec2(width, COMPACT_TABLE_ROW_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, COMPACT_TABLE_ROW_HEIGHT));
            egui::ScrollArea::horizontal()
                .id_salt(id)
                .max_width(width)
                .max_height(COMPACT_TABLE_ROW_HEIGHT)
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
                            RichText::new(text).size(9.0).family(inter_medium_family()),
                        )
                        .extend(),
                    );
                })
        },
    );
    if cell.inner.content_size.x > cell.inner.inner_rect.width() {
        wrapped_tooltip(
            cell.response
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal),
            &format!("{text}\nDrag horizontally or use Shift + mouse wheel to scroll this cell."),
        );
    }
    cell.inner
}

fn draw_tracked_process_list(
    ui: &mut egui::Ui,
    processes: &[TrackedProcessSnapshot],
    revision: u64,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    if processes.is_empty() {
        let scroll = egui::ScrollArea::vertical()
            .id_salt("tracked_process_list")
            .max_height(height)
            .show(ui, |ui| {
                service_empty_state(
                    ui,
                    "No running tracked processes",
                    "Running processes from your saved apps will appear here.",
                );
            });
        #[cfg(all(debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(
            ui.ctx(),
            "table.tracked",
            scroll.inner_rect,
            true,
            serde_json::json!({"rows":0,"offset":scroll.state.offset.y}),
        );
        return scroll;
    }
    compact_table_spacing(ui);
    ui.spacing_mut().item_spacing.x = 6.0;
    let width = (ui.available_width() - ui.spacing().scroll.allocated_width() - 4.0).max(120.0);
    let names = (width - 52.0 - 12.0).max(40.0);
    let widths = [names * 0.62, 52.0, names * 0.38];
    let id = ui.id().with("tracked-sort");
    let mut order = ui
        .ctx()
        .data(|d| d.get_temp::<table::Order>(id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        for (i, (title, column)) in [
            ("Process", table::Column::Name),
            ("PID", table::Column::Pid),
            ("Group", table::Column::Group),
        ]
        .into_iter()
        .enumerate()
        {
            let response = table::heading(ui, widths[i], title, column, &mut order);
            crate::debug_mcp_control!(format!("sort.tracked.{i}"), response);
        }
    });
    let sort = order.sort.unwrap_or(table::Sort {
        column: table::Column::Pid,
        descending: false,
    });
    order.prepare(
        revision,
        processes.len(),
        |_| true,
        |a, b| {
            let (a, b) = (&processes[a], &processes[b]);
            let primary = sort.compare(match sort.column {
                table::Column::Name => table::name_cmp(&a.process_name, &b.process_name),
                table::Column::Group => table::name_cmp(&a.group_name, &b.group_name),
                _ => a.pid.cmp(&b.pid),
            });
            primary
                .then(a.pid.cmp(&b.pid))
                .then_with(|| a.group_name.cmp(&b.group_name))
                .then_with(|| a.process_name.cmp(&b.process_name))
        },
    );
    let indices = order.rows.clone();
    ui.ctx().data_mut(|d| d.insert_temp(id, order));
    ui.separator();
    let scroll = egui::ScrollArea::vertical()
        .id_salt("tracked_process_list")
        .min_scrolled_height(height)
        .max_height(height)
        .auto_shrink([false, false])
        .content_margin(0)
        .show_rows(ui, COMPACT_TABLE_ROW_HEIGHT, indices.len(), |ui, range| {
            for index in range {
                let row = &processes[indices[index]];
                let response = ui.push_id((row.pid, &row.group_name, &row.process_name), |ui| {
                    ui.horizontal(|ui| {
                        tracked_text_cell(ui, &row.process_name, widths[0], "process");
                        table::cell(
                            ui,
                            widths[1],
                            true,
                            RichText::new(row.pid.to_string()).small(),
                        );
                        tracked_text_cell(ui, &row.group_name, widths[2], "group");
                    })
                });
                table_row_separator(ui, response.inner.response.rect);
            }
        });
    #[cfg(all(debug_assertions, feature = "debug-mcp"))]
    crate::app::features::diagnostics::debug_mcp::rect_control(
        ui.ctx(),
        "table.tracked",
        scroll.inner_rect,
        true,
        serde_json::json!({"rows":processes.len(),"offset":scroll.state.offset.y}),
    );
    scroll
}

/// Compact service controls embedded in the measured header row.
pub(crate) fn preferred_width() -> f32 {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    {
        50.0 + 86.0 + 82.0 + 12.0
    }
    #[cfg(not(all(target_os = "windows", feature = "windows")))]
    {
        86.0
    }
}

pub(crate) fn draw_buttons(app: &mut AppState, ui: &mut egui::Ui, width: f32) {
    let colors = *palette(ui);
    let group = ui.allocate_ui_with_layout(
        egui::vec2(width, 21.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            #[cfg(all(target_os="windows",feature="windows"))]
            let scale = ((width - 12.0) / (preferred_width() - 12.0)).clamp(0.0, 1.0);
            #[cfg(all(target_os="windows",feature="windows"))]
            let button_width = 86.0 * scale;
            #[cfg(not(all(target_os="windows",feature="windows")))]
            let button_width = width;
            #[cfg(all(target_os="windows",feature="windows"))]
            app.mcp.status_button(ui, 50.0 * scale);
            let enabled = app.is_process_monitoring_enabled();
            let remaining = enabled.then(|| app.ui.tracked_processes.last_activity
                .and_then(|last| std::time::Duration::from_millis(180).checked_sub(last.elapsed()))
                .filter(|d| !d.is_zero())).flatten();
            let color = if remaining.is_some() { colors.danger.fg } else if enabled { colors.success.fg } else { colors.neutral_status };
            let response = status_button(ui, [button_width,21.0], "Process monitor", color);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Process monitor. Open affinity and priority controls, tracked processes and logs."));
            #[cfg(test)] ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("monitor-info"), response.rect));
            crate::debug_mcp_control!("monitor.open", response);
            if wrapped_tooltip(response, "Process monitor: automatic affinity and priority for your saved apps. Click for controls, tracked processes and logs.").clicked() {
                app.ui.tracked_processes.open = true;
            }
            if let Some(remaining) = remaining { ui.ctx().request_repaint_after(remaining); }
            #[cfg(all(target_os="windows",feature="windows"))]
            crate::app::features::cpu_load::controls::control_button(app, ui, 82.0 * scale);
        });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("service-controls"), group.response.rect));
    #[cfg(not(test))]
    let _ = group;
}

/// Service dialogs do not reserve a main-window panel.
pub(crate) fn draw_modals(app: &mut AppState, ctx: &egui::Context) {
    #[cfg(all(target_os = "windows", feature = "windows"))]
    crate::app::features::cpu_load::controls::draw_modal(app, ctx);
    if app.ui.tracked_processes.open {
        if !app.ui.tracked_processes.show_logs {
            app.refresh_tracked_processes(std::time::Instant::now());
        }
        let mut enabled = app.is_process_monitoring_enabled();
        let log_manager = &app.log_manager;
        let mut toggle = false;
        let tracked = &mut app.ui.tracked_processes;
        let mut close = false;
        let modal=crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tracked-process-modal")).show(ctx,|ui| {
            let populated=if tracked.show_logs {log_manager.monitor_entries().next().is_some()} else {!tracked.rows.is_empty()};
            service_modal_layout(ui, if populated {260.0} else {110.0});
            let button=service_modal_header(ui, "Process monitor", "Automatic affinity and priority for your saved apps.");
            crate::debug_mcp_control!("monitor.close",button);
            close=button.clicked();
            let colors=*palette(ui);
            let (status,color)=if enabled {("Active",colors.success.fg)} else {("Paused",colors.text_muted)};
            let control=wrapped_tooltip(service_control(ui,&mut enabled,true,"Enable process monitor",status,color), "Finds running apps from your groups and, when active, keeps them on the CPUs you selected with the priority you chose.");
            crate::debug_mcp_control!("monitor.toggle",control);
            toggle=control.changed();
            let [processes,logs]=service_tabs(ui,&mut tracked.show_logs,["Processes","Logs"]);
            crate::debug_mcp_control!("monitor.tab.processes",processes);
            crate::debug_mcp_control!("monitor.tab.logs",logs);
            if tracked.show_logs {
                draw_monitor_logs(ui,log_manager);
            } else {
                wrapped_tooltip(ui.label(RichText::new(format!("{} tracked processes{}",tracked.rows.len(),if tracked.updating {" · updating…"}else{""})).size(9.0).color(colors.text_muted)),
                    "Running processes verified for your saved apps, including idle ones. The list updates while corrections are paused. A busy refresh keeps the last completed list and scroll position.");
                let list_height=(ui.available_height()-COMPACT_TABLE_ROW_HEIGHT-6.0).max(30.0);
                draw_tracked_process_list(ui,&tracked.rows,tracked.revision,list_height);
            }
        });
        if close || modal.should_close() {
            tracked.open = false;
        }
        if toggle {
            app.toggle_process_monitoring();
        }
    }
}

fn draw_monitor_logs(ui: &mut egui::Ui, logs: &crate::app::models::LogManager) {
    let rows: Vec<_> = logs.monitor_entries().collect();
    if rows.is_empty() {
        let response = service_empty_state(
            ui,
            "No monitor events yet",
            "Process discoveries, corrections and warnings will appear here.",
        );
        #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
        crate::app::features::diagnostics::debug_mcp::rect_control(
            ui.ctx(),
            "monitor.logs",
            response.rect,
            true,
            serde_json::json!({"rows":0}),
        );
        #[cfg(not(all(target_os = "windows", debug_assertions, feature = "debug-mcp")))]
        let _ = response;
        return;
    }
    compact_table_spacing(ui);
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.label(
        RichText::new("Last 100 retained monitor events · newest first")
            .small()
            .weak(),
    );
    let width = (ui.available_width() - ui.spacing().scroll.allocated_width() - 4.0).max(110.0);
    let widths = [46.0, (width - 52.0).max(50.0)];
    ui.horizontal(|ui| {
        table::cell(ui, widths[0], true, RichText::new("Time").small().weak());
        table::cell(ui, widths[1], false, RichText::new("Event").small().weak());
    });
    ui.separator();
    let height = ui.available_height().max(30.0);
    let scroll = egui::ScrollArea::vertical()
        .id_salt("process-monitor-log")
        .min_scrolled_height(height)
        .max_height(height)
        .auto_shrink([false, false])
        .content_margin(0)
        .show_rows(ui, COMPACT_TABLE_ROW_HEIGHT, rows.len(), |ui, range| {
            for index in range {
                let row = rows[index];
                #[cfg(target_os = "windows")]
                let clock = os_api::cpu_load::sample_clock_label(row.timestamp);
                #[cfg(not(target_os = "windows"))]
                let clock = row
                    .format()
                    .split(" :: ")
                    .next()
                    .unwrap_or("—")
                    .trim_matches(['[', ']'])
                    .to_string();
                let response = ui.push_id(row.sequence, |ui| {
                    ui.horizontal(|ui| {
                        table::cell(ui, widths[0], true, RichText::new(clock).size(9.0));
                        wrapped_tooltip(
                            table::cell(
                                ui,
                                widths[1],
                                false,
                                RichText::new(row.message.trim_start_matches("MONITOR: "))
                                    .size(9.0),
                            ),
                            &row.message,
                        );
                    })
                });
                table_row_separator(ui, response.inner.response.rect);
            }
        });
    #[cfg(all(target_os = "windows", debug_assertions, feature = "debug-mcp"))]
    crate::app::features::diagnostics::debug_mcp::rect_control(
        ui.ctx(),
        "monitor.logs",
        scroll.inner_rect,
        true,
        serde_json::json!({"rows":rows.len()}),
    );
    #[cfg(not(all(target_os = "windows", debug_assertions, feature = "debug-mcp")))]
    let _ = scroll;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    fn draw_service_ui(app: &mut AppState, ui: &mut egui::Ui) {
        crate::app::shell::presenters::header::draw_top_panel(app, ui);
        draw_modals(app, ui.ctx());
    }
    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn cpu_header_shows_load_only_when_enabled_in_both_themes() {
        for dark in [true, false] {
            for enabled in [false, true] {
                let ctx = egui::Context::default();
                crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut app = AppState::new_for_test(
                    Arc::new(RwLock::new(
                        crate::app::models::AppStateStorage::test_fixture(|state| {
                            state.cpu_load_monitoring_enabled = enabled;
                        }),
                    )),
                    20,
                );
                let mut output = egui::FullOutput::default();
                for _ in 0..4 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(400.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ui| draw_service_ui(&mut app, ui),
                    );
                    output.textures_delta.clear();
                }
                let text = |label: &str| {
                    output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let rect = text.galley.rect.translate(text.pos.to_vec2());
                            assert!(
                                shape.clip_rect.contains_rect(rect),
                                "Header text is clipped: {label}"
                            );
                            Some((rect, text))
                        }
                        _ => None,
                    })
                };
                let (cpu, label) = text("CPU load").expect("CPU header control");
                let (monitor, _) = text("Process monitor").unwrap();
                assert!(cpu.left() > monitor.right());
                let button = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("cpu-monitor-button")))
                    .unwrap();
                let monitor_button = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("monitor-info")))
                    .unwrap();
                assert!(
                    monitor_button.width() > button.width(),
                    "Longer service names need a wider button"
                );
                let group = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("service-controls")))
                    .unwrap();
                let mcp = text("MCP").unwrap().0;
                assert!((mcp.center().x - (group.left() + 25.0)).abs() < 0.5);
                assert!((group.right() - button.right()).abs() < 0.5);
                assert!(
                    (monitor_button.center().y - button.center().y).abs() < 0.5,
                    "Header buttons must share a vertical center"
                );
                assert!(button.contains_rect(cpu) && button.right() < 394.0);
                assert_eq!(
                    button.height(),
                    21.0,
                    "CPU control must match adjacent button height"
                );
                assert!(text("All CPUs").is_none());
                assert_eq!(
                    output
                        .shapes
                        .iter()
                        .filter(
                            |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "CPU load")
                        )
                        .count(),
                    1
                );
                if enabled {
                    let (value, _) = text("—").expect("Unavailable load must not become zero");
                    assert!(
                        value.left() > cpu.right() + 3.0,
                        "CPU title and value must not overlap"
                    );
                } else {
                    assert!(text("—").is_none());
                }
                let colors =
                    crate::app::shell::presenters::shared_elements::palette_for_dark_mode(dark);
                let expected = if enabled {
                    colors.success.fg
                } else {
                    colors.neutral_status
                };
                assert_eq!(label.galley.job.sections[0].format.color, expected);
                let frames: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Rect(r) if r.rect == button && !r.stroke.is_empty() => Some(r),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    frames.len(),
                    1,
                    "CPU load must not repaint a second button frame"
                );
                assert_eq!(
                    frames[0].stroke.color,
                    colors.border_strong.lerp_to_gamma(expected, 0.35)
                );
            }
        }
    }
    fn app() -> AppState {
        AppState::new_for_test(
            Arc::new(RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|_| {}),
            )),
            20,
        )
    }
    #[test]
    fn service_monitor_empty_states_are_compact_and_tabs_are_balanced() {
        for dark in [true, false] {
            let ctx = egui::Context::default();
            crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut app = app();
            app.ui.tracked_processes.open = true;
            app.ui
                .tracked_processes
                .publish(std::time::Instant::now(), Some(Vec::new()));
            for logs in [false, true] {
                app.ui.tracked_processes.show_logs = logs;
                let mut output = egui::FullOutput::default();
                for _ in 0..4 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(400.0, 1000.0),
                            )),
                            ..Default::default()
                        },
                        |ui| draw_service_ui(&mut app, ui),
                    );
                    output.textures_delta.clear();
                }
                let modal_bounds = ctx
                    .memory(|m| m.area_rect(egui::Id::new("tracked-process-modal")))
                    .unwrap();
                let visible = |label: &str| {
                    output
                        .shapes
                        .iter()
                        .find_map(|s| {
                            if let egui::Shape::Text(t) = &s.shape {
                                let r = t.galley.rect.translate(t.pos.to_vec2());
                                (t.galley.job.text == label
                                    && s.clip_rect.contains_rect(r)
                                    && modal_bounds.contains_rect(r))
                                .then_some(r)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| panic!("Missing visible label: {label}"))
                };
                visible(if logs {
                    "No monitor events yet"
                } else {
                    "No running tracked processes"
                });
                let a = visible("Processes");
                let b = visible("Logs");
                assert!(
                    (a.center().y - b.center().y).abs() < 1.0
                        && b.center().x - a.center().x > 120.0
                );
                let rect = ctx
                    .memory(|m| m.area_rect(egui::Id::new("tracked-process-modal")))
                    .unwrap();
                assert!(
                    rect.height() <= 300.0,
                    "Empty monitor should be compact: {rect:?}"
                );
            }
        }
    }
    #[test]
    fn header_opens_process_monitor_controls_without_toggling_it() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = app();
        let initial = app.is_process_monitoring_enabled();
        let mut point = egui::Pos2::ZERO;
        for step in 0..6 {
            let events = if step == 4 || step == 5 {
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: step == 4,
                        modifiers: Default::default(),
                    },
                ]
            } else {
                vec![]
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| draw_service_ui(&mut app, ui),
            );
            output.textures_delta.clear();
            if step < 4 {
                point = output
                    .shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape {
                            if t.galley.job.text == "Process monitor" {
                                Some(t.pos + t.galley.size() * 0.5)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .expect("Process monitor header button");
            }
        }
        assert!(app.ui.tracked_processes.open);
        assert_eq!(
            app.is_process_monitoring_enabled(),
            initial,
            "Opening controls must not switch monitoring"
        );
    }
    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn mcp_status_precedes_process_monitor_in_the_header() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = app();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| draw_service_ui(&mut app, ui),
        );
        output.textures_delta.clear();
        for _ in 0..3 {
            output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| draw_service_ui(&mut app, ui),
            );
            output.textures_delta.clear();
        }
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text)
                } else {
                    None
                }
            })
            .collect();
        let mcp = texts
            .iter()
            .find(|text| text.galley.job.text == "MCP")
            .expect("Header MCP status");
        let monitor = texts
            .iter()
            .find(|text| text.galley.job.text.eq("Process monitor"))
            .unwrap();
        assert!(mcp.pos.x + mcp.galley.size().x + 20.0 < monitor.pos.x);
        assert!(mcp.galley.size().y <= 18.0);
    }
    #[test]
    fn services_leave_the_bottom_available_and_hover_does_not_open_controls() {
        let ctx = egui::Context::default();
        crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
        let mut app = app();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 500.0),
                    )),
                    events: vec![egui::Event::PointerMoved(egui::pos2(100.0, 480.0))],
                    ..Default::default()
                },
                |ui| {
                    draw_service_ui(&mut app, ui);
                    assert_eq!(ui.available_rect_before_wrap().bottom(), 500.0);
                },
            );
            output.textures_delta.clear();
        }
        assert!(!app.ui.tracked_processes.open);
    }
    #[test]
    fn information_button_opens_modal_by_click_not_hover() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = app();
        let mut button = egui::Rect::NOTHING;
        for step in 0..5 {
            let point = if button.is_positive() {
                button.center()
            } else {
                egui::pos2(390.0, 480.0)
            };
            let mut events = vec![egui::Event::PointerMoved(point)];
            if step >= 3 {
                events.push(egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: step == 3,
                    modifiers: Default::default(),
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    draw_service_ui(&mut app, ui);
                    button = ui
                        .ctx()
                        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("monitor-info")))
                        .unwrap();
                },
            );
            output.textures_delta.clear();
            if step < 4 {
                assert!(!app.ui.tracked_processes.open);
            }
        }
        assert!(app.ui.tracked_processes.open);
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
                        let output = super::draw_tracked_process_list(ui, &processes, 1, 200.0);
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
                            let table = super::draw_tracked_process_list(ui, &processes, 1, 200.0);
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
