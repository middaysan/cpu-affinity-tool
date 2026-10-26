//! CPU telemetry controls share the header service presentation and existing sampler.
use super::{
    meter::paint_system_summary,
    model::{system_mean, GraphTarget},
};
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    modal, palette, service_control, service_modal_header, service_modal_layout, status_button,
    tooltip_layout, wrapped_tooltip,
};
use eframe::egui::{self, RichText};

pub(crate) fn control_button(app: &mut AppState, ui: &mut egui::Ui, width: f32) {
    let enabled = app.cpu_load_enabled();
    let colors = *palette(ui);
    let color = if enabled {
        colors.success.fg
    } else {
        colors.neutral_status
    };
    let response = status_button(
        ui,
        [width, 21.0],
        if enabled { "" } else { "CPU load" },
        color,
    );
    let value = if enabled {
        system_mean(&app.cpu_load.snapshot().cpus)
    } else {
        None
    };
    if enabled {
        paint_system_summary(
            ui,
            response.rect,
            value,
            color,
            (response.hovered(), response.is_pointer_button_down_on()),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            if enabled {
                format!(
                    "CPU monitoring on, all CPUs {}. Open CPU monitoring controls",
                    super::view::measured_percent(value, false)
                )
            } else {
                "CPU monitoring off. Open CPU monitoring controls".into()
            },
        )
    });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new("cpu-monitor-button"), response.rect));
    crate::debug_mcp_control!("cpu.monitor.open", response);
    if response.on_hover_ui(|ui| {
        tooltip_layout(ui);
        ui.add(egui::Label::new("CPU load recording, history and system graphs. Click for controls. The percentage shows total activity from every program across all CPUs in this computer.").wrap());
        if let Some(error) = &app.cpu_load.preference_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(error) = app.cpu_load.status_error() {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }).clicked() {
        app.ui.cpu_monitor_open = true;
    }
}

pub(crate) fn draw_modal(app: &mut AppState, ctx: &egui::Context) {
    if !app.ui.cpu_monitor_open {
        return;
    }
    let mut enabled = app.cpu_load_enabled();
    let mut history_seconds = app.cpu_load_history_seconds();
    let mut toggle = false;
    let mut close = false;
    let mut graphs = false;
    let response = modal(egui::Id::new("cpu-monitor-modal")).show(ctx, |ui| {
        service_modal_layout(ui, 75.0);
        let button = service_modal_header(ui, "CPU monitoring", "Records CPU load and shows current app RAM.");
        crate::debug_mcp_control!("cpu.monitor.close", button);
        close = button.clicked();
        let colors = *palette(ui);
        let (status, color) = if enabled { ("On", colors.success.fg) } else { ("Off", colors.text_muted) };
        let control = service_control(ui, &mut enabled, true, "Enable CPU monitoring", status, color);
        crate::debug_mcp_control!("settings.cpu_monitor", control);
        toggle = hint(control, "Records each app on each logical CPU, including with graphs closed or in the tray. Adds CPU and memory overhead; turn off if it affects performance.").changed();
        egui::ScrollArea::vertical().id_salt("cpu-monitor-controls-body")
            .max_height(ui.available_height().max(40.0)).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("CPU history");
            let selector = egui::ComboBox::from_id_salt("cpu-load-history")
                .width(100.0).selected_text(history_label(history_seconds)).show_ui(ui, |ui| {
                    for seconds in [30, 300, 1800] {
                        let option = ui.selectable_value(&mut history_seconds, seconds, history_label(seconds));
                        crate::debug_mcp_control!(format!("settings.history.{seconds}"), option);
                    }
                }).response;
            crate::debug_mcp_control!("settings.history", selector);
            hint(selector, "History starts when monitoring is enabled or an app is added. Buffers are reserved in advance and released when an app is removed or monitoring is disabled. Longer history uses more memory; sampling stays once per second.");
        });
        let estimate = app.cpu_load_memory_estimate();
        hint(ui.label(RichText::new(estimate.lines().next().unwrap_or_default()).small().weak()),
            &format!("{estimate}\nCalculated history storage, including the preallocated sparse process-contribution pool, maximum Windows trace buffers and estimated runtime/tracking tables. Actual extra memory varies. Process names are retained only while their historical samples remain."));
        if let Some(error) = &app.cpu_load.preference_error {
            ui.add(egui::Label::new(RichText::new(error).color(colors.danger.fg)).wrap());
        }
        if let Some(error) = app.cpu_load.status_error() {
            ui.add(egui::Label::new(RichText::new(error).color(colors.danger.fg)).wrap());
        }
        let button = ui.add_enabled(enabled && !toggle, egui::Button::new("Open CPU graphs"));
        crate::debug_mcp_control!("cpu.monitor.graphs", button);
        graphs = button.clicked();
        ui.label(RichText::new("All programs · all logical CPUs").size(9.0).color(colors.text_muted));
        });
    });
    if toggle {
        app.cpu_load.preference_error = app.set_cpu_load_enabled(enabled).err();
    }
    if history_seconds != app.cpu_load_history_seconds() {
        app.cpu_load.preference_error = app.set_cpu_load_history_seconds(history_seconds).err();
    }
    if close || response.should_close() || graphs {
        app.ui.cpu_monitor_open = false;
    }
    if graphs {
        app.cpu_load.open(GraphTarget::System);
    }
}

fn hint(response: egui::Response, text: &str) -> egui::Response {
    wrapped_tooltip(response, text)
}

fn history_label(seconds: u32) -> &'static str {
    match seconds {
        30 => "30 seconds",
        1800 => "30 minutes",
        _ => "5 minutes",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppStateStorage;
    use eframe::egui::{Pos2, RawInput, Rect};
    use std::sync::{Arc, RwLock};

    fn frame(
        app: &mut AppState,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                crate::app::shell::presenters::header::draw_top_panel(app, ui);
                crate::app::shell::presenters::service_controls::draw_modals(app, ui.ctx());
            },
        );
        output.textures_delta.clear();
        output
    }

    fn label(output: &egui::FullOutput, name: &str) -> Option<Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == name => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
    }

    fn click(app: &mut AppState, ctx: &egui::Context, point: Pos2) -> egui::FullOutput {
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        frame(app, ctx, vec![])
    }

    #[test]
    fn cpu_controls_toggle_persist_and_open_system_graphs_without_affecting_process_monitor() {
        for dark in [true, false] {
            let ctx = egui::Context::default();
            crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
            ctx.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let storage = Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.cpu_load_monitoring_enabled = false;
                state.process_monitoring_enabled = true;
            })));
            let mut app = AppState::new_for_test(storage.clone(), 20);
            for _ in 0..4 {
                frame(&mut app, &ctx, vec![]);
            }
            let control = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new("cpu-monitor-button")))
                .unwrap();
            let mut output = click(&mut app, &ctx, control.center());
            for _ in 0..3 {
                output = frame(&mut app, &ctx, vec![]);
            }
            assert!(app.ui.cpu_monitor_open);
            assert!(app.ui.monitoring_modal_open(false));
            assert!(
                !app.cpu_load_enabled(),
                "Opening controls must not enable recording"
            );
            output = click(
                &mut app,
                &ctx,
                label(&output, "Open CPU graphs").unwrap().center(),
            );
            assert!(
                app.cpu_load.graph.is_none(),
                "Graphs disabled while recording is Off"
            );
            click(
                &mut app,
                &ctx,
                label(&output, "Enable CPU monitoring").unwrap().center(),
            );
            app.wait_for_test_save();
            assert!(app.cpu_load_enabled());
            assert!(storage.read().unwrap().cpu_load_monitoring_enabled);
            assert!(app.is_process_monitoring_enabled());
            output = frame(&mut app, &ctx, vec![]);
            click(
                &mut app,
                &ctx,
                label(&output, "Open CPU graphs").unwrap().center(),
            );
            assert!(!app.ui.cpu_monitor_open);
            assert_eq!(app.cpu_load.graph, Some(GraphTarget::System));
            assert!(app.cpu_load.show_all_cpus);
            assert!(!app.cpu_load.tracked_only());
            assert!(app.cpu_load_enabled());
            app.cpu_load.graph = None;
            app.ui.cpu_monitor_open = true;
            for _ in 0..4 {
                output = frame(&mut app, &ctx, vec![]);
            }
            click(
                &mut app,
                &ctx,
                label(&output, "Enable CPU monitoring").unwrap().center(),
            );
            app.wait_for_test_save();
            assert!(!app.cpu_load_enabled());
            assert!(!storage.read().unwrap().cpu_load_monitoring_enabled);
            assert!(app.is_process_monitoring_enabled());
            assert!(app.cpu_load.graph.is_none());
            app.ui.cpu_monitor_open = false;
            output = frame(&mut app, &ctx, vec![]);
            assert!(label(&output, "All CPUs").is_none());
        }
    }

    #[test]
    fn header_uses_existing_system_mean_and_does_not_add_a_nested_hit_target() {
        let ctx = egui::Context::default();
        crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
                state.cpu_load_monitoring_enabled = true;
            }))),
            20,
        );
        for (cpus, expected) in [
            (vec![Some(0.0); 20], "0%"),
            (vec![Some(0.0), Some(100.0)], "50%"),
            (vec![Some(100.0); 20], "100%"),
            (vec![None, Some(100.0)], "—"),
        ] {
            *app.cpu_load.published.write().unwrap() = Arc::new(super::super::model::Snapshot {
                cpus,
                ..Default::default()
            });
            let mut output = frame(&mut app, &ctx, vec![]);
            for _ in 0..3 {
                output = frame(&mut app, &ctx, vec![]);
            }
            label(&output, expected).unwrap_or_else(|| panic!("Missing {expected}"));
            let percentage = label(&output, expected).unwrap();
            assert!(
                label(&output, "CPU load").unwrap().right() + 3.0
                    < label(&output, expected).unwrap().left()
            );
            let control = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new("cpu-monitor-button")))
                .unwrap();
            assert!(control.contains_rect(percentage));
            click(&mut app, &ctx, percentage.center());
            assert!(
                app.ui.cpu_monitor_open,
                "The embedded meter has the same control target"
            );
            assert!(app.cpu_load.graph.is_none());
            app.ui.cpu_monitor_open = false;
            for _ in 0..3 {
                frame(&mut app, &ctx, vec![]);
            }
        }
    }

    #[test]
    fn compact_cpu_header_keeps_full_load_visible_across_dpi_and_themes() {
        for dark in [true, false] {
            for scale in [1.0, 1.75] {
                let ctx = egui::Context::default();
                crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
                ctx.all_styles_mut(
                    crate::app::shell::presenters::shared_elements::apply_widget_style,
                );
                ctx.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                ctx.set_pixels_per_point(scale);
                let mut app = AppState::new_for_test(
                    Arc::new(RwLock::new(AppStateStorage::test_fixture(|s| {
                        s.cpu_load_monitoring_enabled = true
                    }))),
                    20,
                );
                *app.cpu_load.published.write().unwrap() =
                    Arc::new(super::super::model::Snapshot {
                        cpus: vec![Some(100.0); 20],
                        ..Default::default()
                    });
                let mut output = frame(&mut app, &ctx, vec![]);
                for _ in 0..3 {
                    output = frame(&mut app, &ctx, vec![]);
                }
                let title = label(&output, "CPU load").unwrap();
                let value = label(&output, "100%").unwrap();
                let button = ctx
                    .data(|d| d.get_temp::<Rect>(egui::Id::new("cpu-monitor-button")))
                    .unwrap();
                assert_eq!(button.width(), 82.0);
                assert!(title.right() + 3.0 < value.left());
                assert!(button.contains_rect(value));
                let shape = output
                    .shapes
                    .iter()
                    .find(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="100%"))
                    .unwrap();
                assert!(
                    shape.clip_rect.contains_rect(value),
                    "Header percentage is clipped at scale {scale}"
                );
            }
        }
    }

    #[test]
    fn cpu_control_errors_are_visible_and_wrapped() {
        let ctx = egui::Context::default();
        crate::app::shell::presenters::shared_elements::install_ui_fonts(&ctx);
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            20,
        );
        app.ui.cpu_monitor_open = true;
        let error = "CPU monitoring changed for this session, but could not be saved. Try again in CPU monitoring.";
        app.cpu_load.preference_error = Some(error.into());
        let mut output = frame(&mut app, &ctx, vec![]);
        for _ in 0..3 {
            output = frame(&mut app, &ctx, vec![]);
        }
        let rect =
            label(&output, error).expect("Save failure must be visible at the new entry point");
        assert!(rect.width() < 360.0);
        assert!(rect.left() >= 0.0 && rect.right() <= 400.0 && rect.bottom() < 600.0);
        let modal = ctx
            .memory(|m| m.area_rect(egui::Id::new("cpu-monitor-modal")))
            .unwrap();
        assert!(
            modal.height() < 245.0,
            "CPU controls should not reserve an empty table: {modal:?}"
        );
        app.cpu_load.preference_error = Some(error.repeat(40));
        *app.cpu_load.published.write().unwrap() = Arc::new(super::super::model::Snapshot {
            error: Some("CPU sampler could not read the requested counter. ".repeat(40)),
            ..Default::default()
        });
        for _ in 0..4 {
            frame(&mut app, &ctx, vec![]);
        }
        let modal = ctx
            .memory(|m| m.area_rect(egui::Id::new("cpu-monitor-modal")))
            .unwrap();
        assert!(
            modal.left() >= 0.0
                && modal.right() <= 400.0
                && modal.top() >= 0.0
                && modal.bottom() <= 600.0,
            "Long errors must scroll within the modal: {modal:?}"
        );
    }
    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn history_selector_keeps_cpu_controls_open_and_applies_all_choices() {
        fn frame(
            app: &mut AppState,
            context: &egui::Context,
            height: f32,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            let mut output = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, height))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    crate::app::shell::presenters::header::draw_top_panel(app, ui);
                    crate::app::shell::presenters::service_controls::draw_modals(app, ui.ctx());
                },
            );
            output.textures_delta.clear();
            output
        }
        fn label(output: &egui::FullOutput, name: &str) -> Option<Pos2> {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
        }
        fn click(
            app: &mut AppState,
            context: &egui::Context,
            height: f32,
            position: Pos2,
        ) -> egui::FullOutput {
            for pressed in [true, false] {
                frame(
                    app,
                    context,
                    height,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            frame(app, context, height, vec![])
        }
        for dark in [true, false] {
            for height in [600.0, 1000.0] {
                let context = egui::Context::default();
                context.set_fonts(
                    crate::app::shell::presenters::shared_elements::ui_font_definitions(),
                );
                context.set_visuals(if dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
                let mut app = AppState::new_for_test(
                    Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
                    20,
                );
                frame(&mut app, &context, height, vec![]);
                frame(&mut app, &context, height, vec![]);
                let settings = context
                    .data(|d| d.get_temp::<Rect>(egui::Id::new("cpu-monitor-button")))
                    .unwrap()
                    .center();
                let mut output = click(&mut app, &context, height, settings);
                assert!(label(&output, "Enable CPU monitoring").is_some());
                let mut current = "5 minutes";
                for (choice, seconds) in
                    [("30 seconds", 30), ("30 minutes", 1800), ("5 minutes", 300)]
                {
                    let selector =
                        label(&output, current).expect("history selector must remain visible");
                    output = click(&mut app, &context, height, selector);
                    assert!(
                        label(&output, "Enable CPU monitoring").is_some(),
                        "opening the history selector closed CPU monitoring"
                    );
                    let option = label(&output, choice)
                        .unwrap_or_else(|| panic!("missing history option {choice}"));
                    output = click(&mut app, &context, height, option);
                    app.wait_for_test_save();
                    assert_eq!(app.cpu_load_history_seconds(), seconds);
                    assert!(
                        label(&output, "Enable CPU monitoring").is_some(),
                        "selecting a history value closed CPU monitoring"
                    );
                    current = choice;
                }
                click(&mut app, &context, height, label(&output, current).unwrap());
                frame(
                    &mut app,
                    &context,
                    height,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
                output = frame(&mut app, &context, height, vec![]);
                assert!(
                    label(&output, "Enable CPU monitoring").is_some(),
                    "First Escape closes only the selector"
                );
                for pressed in [false, true] {
                    frame(
                        &mut app,
                        &context,
                        height,
                        vec![egui::Event::Key {
                            key: egui::Key::Escape,
                            physical_key: None,
                            pressed,
                            repeat: false,
                            modifiers: Default::default(),
                        }],
                    );
                }
                output = frame(&mut app, &context, height, vec![]);
                assert!(label(&output, "Enable CPU monitoring").is_none());
                output = click(&mut app, &context, height, settings);
                click(&mut app, &context, height, label(&output, current).unwrap());
                output = click(&mut app, &context, height, Pos2::new(390.0, height - 10.0));
                assert!(
                    label(&output, "Enable CPU monitoring").is_none(),
                    "outside click must dismiss both menus"
                );
                output = click(&mut app, &context, height, settings);
                assert!(label(&output, "Enable CPU monitoring").is_some());
                let close = label(&output, "Close").unwrap();
                output = click(&mut app, &context, height, close);
                assert!(label(&output, "Enable CPU monitoring").is_none());
            }
        }
    }
}
