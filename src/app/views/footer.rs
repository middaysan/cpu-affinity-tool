use crate::app::runtime::{AppState, TrackedProcessSnapshot};
use crate::app::shell::presenters::{
    process_table as table,
    shared_elements::{
        close_button, compact_table_spacing, inter_medium_family, palette, table_row_separator,
        COMPACT_TABLE_ROW_HEIGHT,
    },
};
use eframe::egui::{self, Layout, RichText, Stroke};

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
    revision: u64,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<()> {
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
            if processes.is_empty() {
                ui.label(
                    RichText::new("No running process is currently verified.")
                        .small()
                        .weak(),
                );
            }
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

fn information_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            "Tracked processes",
        )
    });
    let colors = palette(ui);
    let color = if response.hovered() || response.has_focus() {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    ui.painter()
        .circle_stroke(rect.center(), 6.0, Stroke::new(1.0, color));
    ui.painter().line_segment(
        [
            rect.center() + egui::vec2(0.0, -0.5),
            rect.center() + egui::vec2(0.0, 3.0),
        ],
        Stroke::new(1.2, color),
    );
    ui.painter()
        .circle_filled(rect.center() + egui::vec2(0.0, -2.8), 0.7, color);
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            3.0,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text("Tracked processes — open the full list. Refreshed once per second.")
}

pub fn draw_bottom_panel(app: &mut AppState, root_ui: &mut egui::Ui) {
    let colors = *palette(root_ui);
    egui::Panel::bottom("bottom_panel").frame(egui::Frame::NONE.fill(colors.surface).inner_margin(egui::Margin{left:7,right:7,top:3,bottom:6})).show(root_ui,|ui| {
        let island=egui::Frame::NONE.fill(colors.group).stroke(Stroke::new(1.0,colors.border_strong)).corner_radius(7.0).inner_margin(egui::Margin::symmetric(6,3)).show(ui,|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x=6.0;
                let enabled=app.is_process_monitoring_enabled();
                let (rect,response)=ui.allocate_exact_size(egui::vec2(20.0,18.0),egui::Sense::click());
                response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,ui.is_enabled(),if enabled{"Pause monitor"}else{"Resume monitor"}));
                let color=colors.accent.fg;
                ui.painter().rect_filled(rect,4.0,if response.hovered(){colors.selected.fill}else{colors.inset});
                ui.painter().rect_stroke(rect,4.0,Stroke::new(1.0,color),egui::StrokeKind::Inside);
                let center=rect.center();
                if enabled {
                    for x in [-2.5,2.5] {ui.painter().line_segment([center+egui::vec2(x,-4.0),center+egui::vec2(x,4.0)],Stroke::new(2.0,color));}
                } else {
                    ui.painter().add(egui::Shape::convex_polygon(vec![center+egui::vec2(-2.5,-4.0),center+egui::vec2(-2.5,4.0),center+egui::vec2(4.0,0.0)],color,Stroke::NONE));
                }
                if response.has_focus(){ui.painter().rect_stroke(rect.expand(1.0),4.0,ui.visuals().selection.stroke,egui::StrokeKind::Inside);}
                crate::debug_mcp_control!("monitor.toggle",response);
                if response.on_hover_text("Toggle automatic affinity and priority corrections. CPU load recording is controlled separately in App settings.").clicked(){app.toggle_process_monitoring();}
                ui.label(RichText::new(if enabled{"Monitoring active"}else{"Monitoring paused"}).size(9.5).family(inter_medium_family()).color(color).strong()).on_hover_text(if enabled{"Affinity and priority are protected."}else{"Automatic corrections are disabled. CPU history can still record."});
                ui.with_layout(Layout::right_to_left(egui::Align::Center),|ui| {
                    let info=information_button(ui);
                    #[cfg(test)] ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("monitor-info"),info.rect));
                    crate::debug_mcp_control!("monitor.info",info);
                    if info.clicked(){app.ui.tracked_processes.open=true;}
                });
            });
        });
        #[cfg(test)] ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("monitor-island"),island.response.rect));
        #[cfg(not(test))] let _=island;
    });
    if app.ui.tracked_processes.open {
        app.refresh_tracked_processes(std::time::Instant::now());
        let tracked = &mut app.ui.tracked_processes;
        let mut close = false;
        let modal=crate::app::shell::presenters::shared_elements::modal(egui::Id::new("tracked-process-modal")).show(root_ui.ctx(),|ui| {
            let height=(ui.ctx().content_rect().height()*0.8-24.0).max(160.0);
            ui.set_width((ui.ctx().content_rect().width()-36.0).clamp(200.0,356.0));
            ui.set_height(height);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tracked processes").strong());
                ui.with_layout(Layout::right_to_left(egui::Align::Center),|ui|{let button=close_button(ui);crate::debug_mcp_control!("monitor.close",button);close=button.clicked();});
            });
            ui.label(RichText::new(format!("{} processes{}",tracked.rows.len(),if tracked.updating{" · updating…"}else{""})).small().weak()).on_hover_text("Verified processes tracked across saved applications and groups. The last completed list and its scroll position remain visible while data updates. This roster includes idle processes; graph tables show measured work on one CPU at one time.");
            draw_tracked_process_list(ui,&tracked.rows,tracked.revision,(height-47.0).max(80.0));
        });
        if close || modal.should_close() {
            tracked.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    fn app() -> AppState {
        AppState::new_for_test(
            Arc::new(RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|_| {}),
            )),
            20,
        )
    }
    #[test]
    fn monitor_is_a_compact_inset_island_and_hover_never_opens_roster() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = app();
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..3 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 500.0),
                    )),
                    events: vec![egui::Event::PointerMoved(egui::pos2(100.0, 480.0))],
                    ..Default::default()
                },
                |ui| {
                    draw_bottom_panel(&mut app, ui);
                    rect = ui
                        .ctx()
                        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("monitor-island")))
                        .unwrap();
                },
            );
            out.textures_delta.clear();
        }
        assert!(rect.height() < 33.0, "footer must be compact: {rect:?}");
        assert!(rect.left() >= 6.0 && rect.right() <= 394.0 && rect.bottom() <= 495.0);
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
                    draw_bottom_panel(&mut app, ui);
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
