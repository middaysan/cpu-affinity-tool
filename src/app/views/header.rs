use crate::app::models::APP_VERSION;
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    paint_focus_ring, palette, row_fill, BUTTON_FONT_SIZE,
};
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Color32, Layout, Margin, Panel, RichText, Stroke};

fn settings_menu(app: &mut AppState, ui: &mut egui::Ui) {
    let button = ui.button(RichText::new("⚙ App settings").size(BUTTON_FONT_SIZE));
    crate::debug_mcp_control!("settings.open", button);
    let was_open = app.ui.app_settings_open;
    let mut open = was_open;
    if button.clicked() {
        open = !open;
    }
    let selector_layer = std::cell::Cell::new(None);
    // egui's shared popup slot belongs to the nested ComboBox. Keep the parent
    // menu's lifetime separate so opening that selector cannot replace it.
    let menu = egui::Popup::menu(&button)
        .open_bool(&mut open)
        .width(220.0)
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .show(|ui| {
            ui.label(RichText::new("Application settings").strong());
            ui.separator();
            ui.label("Theme");
            let mut theme_index = app.get_theme_index();
            ui.horizontal(|ui| {
                for (index, name) in [(0, "System"), (1, "Light"), (2, "Dark")] {
                    let response=ui.selectable_value(&mut theme_index, index, name);
                    crate::debug_mcp_control!(format!("settings.theme.{index}"),response);
                }
            });
            if theme_index != app.get_theme_index() {
                app.set_theme_index(theme_index);
                ui.ctx().request_repaint();
            }

            #[cfg(all(target_os = "windows", feature = "windows"))]
            {
                ui.separator();
                let mut cpu_load_enabled = app.cpu_load_enabled();
                let monitoring = ui.checkbox(&mut cpu_load_enabled, "CPU load monitoring");
                crate::debug_mcp_control!("settings.cpu_monitor", monitoring);
                if settings_hint(
                    monitoring,
                    "Records each app on each logical CPU, including with graphs closed or in the tray. Adds CPU and memory overhead; turn off if it affects performance.",
                ).changed() {
                    app.cpu_load.preference_error = app.set_cpu_load_enabled(cpu_load_enabled).err();
                }
                let mut history_seconds = app.cpu_load_history_seconds();
                ui.horizontal(|ui| {
                    ui.label("CPU history");
                    let selector = egui::ComboBox::from_id_salt("cpu-load-history")
                        .width(100.0)
                        .selected_text(history_label(history_seconds))
                        .show_ui(ui, |ui| {
                            selector_layer.set(Some(ui.layer_id()));
                            for seconds in [30, 300, 1800] {
                                let option = ui.selectable_value(&mut history_seconds, seconds, history_label(seconds));
                                crate::debug_mcp_control!(format!("settings.history.{seconds}"), option);
                            }
                        })
                        .response;
                    crate::debug_mcp_control!("settings.history", selector);
                    settings_hint(selector, "History starts when monitoring is enabled or an app is added. Buffers are reserved in advance and released when an app is removed or monitoring is disabled. Longer history uses more memory; sampling stays once per second.");
                });
                if history_seconds != app.cpu_load_history_seconds() {
                    app.cpu_load.preference_error = app.set_cpu_load_history_seconds(history_seconds).err();
                }
                let estimate = app.cpu_load_memory_estimate();
                ui.label(RichText::new(estimate.lines().next().unwrap_or_default()).small().weak())
                    .on_hover_ui(|ui| {
                        ui.set_max_width(260.0);
                        ui.add(egui::Label::new(&estimate).wrap());
                        ui.separator();
                        ui.add(egui::Label::new("Calculated history storage, including the preallocated sparse process-contribution pool, plus the configured maximum Windows trace buffers and estimated runtime/tracking tables. Actual extra memory varies. Process names are retained only while their historical samples remain. Changing history or saved apps updates the estimate; process/thread counts update while monitoring runs.").wrap());
                    });
                if let Some(error) = &app.cpu_load.preference_error {
                    ui.add(egui::Label::new(RichText::new(error).color(ui.visuals().error_fg_color)).wrap());
                }
                if let Some(error) = app.cpu_load.status_error() {
                    ui.add(egui::Label::new(RichText::new(error).color(ui.visuals().error_fg_color)).wrap());
                }
                ui.separator();
                let mut startup_enabled = app.start_minimized();
                if settings_hint(
                    ui.checkbox(&mut startup_enabled, "Start minimized"),
                    "Open in the system tray the next time you launch this app.",
                ).changed()
                {
                    app.ui.startup_setting_error = app.set_start_minimized(startup_enabled).err();
                }
                if let Some(error) = &app.ui.startup_setting_error {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(ui.visuals().error_fg_color))
                            .wrap(),
                    );
                }
            }
        });
    // A click on the selector's foreground layer is still inside App settings,
    // including the frame in which choosing an option closes the selector.
    if was_open && menu.is_some_and(|menu| menu.response.clicked_elsewhere()) {
        let clicked_selector = ui
            .ctx()
            .pointer_interact_pos()
            .and_then(|position| ui.ctx().layer_id_at(position))
            .is_some_and(|layer| Some(layer) == selector_layer.get());
        if !clicked_selector {
            open = false;
        }
    }
    if was_open && !open {
        egui::Popup::close_all(ui.ctx());
    }
    app.ui.app_settings_open = open;
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn settings_hint(response: egui::Response, text: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(260.0);
        ui.add(egui::Label::new(text).wrap());
    })
}

#[cfg(all(target_os = "windows", feature = "windows"))]
fn history_label(seconds: u32) -> &'static str {
    match seconds {
        30 => "30 seconds",
        1800 => "30 minutes",
        _ => "5 minutes",
    }
}

fn navigation_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    selected: bool,
    width: f32,
) -> egui::Response {
    let colors = *palette(ui);
    let label = label.into().color(if selected {
        colors.text_primary
    } else {
        colors.text_secondary
    });
    // Reserve the background before the ordinary button paints its text and focus state.
    let background = ui.painter().add(egui::Shape::Noop);
    let border = ui.painter().add(egui::Shape::Noop);
    let response = ui.add_sized(
        [width, 28.0],
        egui::Button::new(label)
            .selected(selected)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    );
    if ui.is_rect_visible(response.rect) {
        // Reuse egui's bounded transitions: no worker, timer or repaint while settled.
        let duration = |seconds: f32| ui.style().animation_time.min(seconds).max(0.0);
        let selected_amount = ui.ctx().animate_bool_with_time(
            response.id.with("selection"),
            selected,
            duration(0.12),
        );
        let hover_amount = ui.ctx().animate_bool_with_time(
            response.id.with("hover"),
            response.hovered(),
            duration(0.08),
        );
        if selected_amount > 0.0 || hover_amount > 0.0 {
            let top = colors.selected.fill.lerp_to_gamma(
                if ui.visuals().dark_mode {
                    colors.primary.fill
                } else {
                    colors.group
                },
                if ui.visuals().dark_mode { 0.6 } else { 0.35 },
            );
            let mut top = row_fill(ui).lerp_to_gamma(top, selected_amount);
            let mut bottom = row_fill(ui).lerp_to_gamma(colors.selected.fill, selected_amount);
            top = top.lerp_to_gamma(colors.text_primary, hover_amount * 0.04);
            bottom = bottom.lerp_to_gamma(colors.text_primary, hover_amount * 0.04);
            if response.is_pointer_button_down_on() {
                top = top.lerp_to_gamma(Color32::BLACK, 0.1);
                bottom = bottom.lerp_to_gamma(Color32::BLACK, 0.1);
            }
            let rect = response.rect.shrink(0.5);
            ui.painter().set(
                background,
                egui::Shape::mesh(navigation_gradient_mesh(rect, top, bottom)),
            );
            ui.painter().set(
                border,
                egui::Shape::rect_stroke(
                    rect,
                    5.0,
                    Stroke::new(1.0, colors.selected.border.gamma_multiply(selected_amount)),
                    egui::StrokeKind::Inside,
                ),
            );
        }
    }
    paint_focus_ring(ui, &response);
    response
}

fn navigation_gradient_mesh(rect: egui::Rect, top: Color32, bottom: Color32) -> egui::epaint::Mesh {
    let mut points = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(
        &mut points,
        rect,
        egui::epaint::CornerRadiusF32::same(5.0),
    );
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(rect.center(), top.lerp_to_gamma(bottom, 0.5));
    for point in &points {
        let amount = ((point.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        mesh.colored_vertex(*point, top.lerp_to_gamma(bottom, amount));
    }
    for index in 0..points.len() as u32 {
        mesh.add_triangle(0, index + 1, (index + 1) % points.len() as u32 + 1);
    }
    mesh
}

fn paint_navigation_connection(
    ui: &egui::Ui,
    bounds: egui::Rect,
    count: usize,
    selected: usize,
) -> egui::Rect {
    // A single scalar in egui's existing animation manager; only paint moves.
    // Keep the rail against the content-facing edge without adding layout height.
    let position = ui.ctx().animate_value_with_time(
        ui.id().with("navigation-connection"),
        selected as f32,
        ui.style().animation_time.clamp(0.0, 0.12),
    );
    let width = bounds.width() / count as f32;
    let rect = egui::Rect::from_min_max(
        egui::pos2(
            bounds.left() + position * width + 6.0,
            bounds.bottom() - 2.0,
        ),
        egui::pos2(
            bounds.left() + (position + 1.0) * width - 6.0,
            bounds.bottom(),
        ),
    );
    ui.painter().rect_filled(rect, 1.0, palette(ui).accent.fg);
    rect
}

fn navigation_switch(app: &mut AppState, ui: &mut egui::Ui) {
    egui::Frame::NONE
        .fill(row_fill(ui))
        .stroke(ui.visuals().widgets.inactive.bg_stroke)
        .corner_radius(6.0)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let strip = ui.horizontal(|ui| {
                let width = ui.available_width() / if app.modules_enabled() { 3.0 } else { 2.0 };
                let groups_selected =
                    matches!(app.ui.current_window, WindowRoute::Groups(GroupRoute::List));
                let groups = if groups_selected {
                    RichText::new("Affinity groups")
                        .size(BUTTON_FONT_SIZE)
                        .strong()
                } else {
                    RichText::new("Affinity groups").size(BUTTON_FONT_SIZE)
                };
                let groups_response = navigation_button(ui, groups, groups_selected, width);
                crate::debug_mcp_control!("nav.groups", groups_response);
                if groups_response.clicked() {
                    app.set_current_window(WindowRoute::Groups(GroupRoute::List));
                }

                if app.modules_enabled() {
                    let modules_selected = matches!(app.ui.current_window, WindowRoute::Tuning);
                    let modules = RichText::new("System Tweaks").size(BUTTON_FONT_SIZE);
                    let modules_response = navigation_button(ui, modules, modules_selected, width);
                    crate::debug_mcp_control!("nav.tweaks", modules_response);
                    if modules_response.clicked() {
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
                let logs_response = navigation_button(ui, logs, logs_selected, width);
                crate::debug_mcp_control!("nav.logs", logs_response);
                if logs_response.clicked() {
                    app.set_current_window(WindowRoute::Logs);
                }
            });
            let count = if app.modules_enabled() { 3 } else { 2 };
            let selected = match app.ui.current_window {
                WindowRoute::Groups(_)
                | WindowRoute::AppRunSettings
                | WindowRoute::InstalledAppPicker => 0,
                WindowRoute::Tuning => 1,
                _ => count - 1,
            };
            paint_navigation_connection(ui, strip.response.rect, count, selected);
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
                        settings_menu(app, ui);
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("v{APP_VERSION}")).size(8.5).weak())
                                .on_hover_text(format!(
                                    "{cpu_model}\n{total_threads} logical threads"
                                ));
                        });
                    });

                    ui.add_space(1.0);
                    ui.separator();

                    navigation_switch(app, ui);
                });
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn navigation_connection_slides_snaps_and_settles_without_layout_motion() {
        let context = egui::Context::default();
        let bounds = Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(300.0, 28.0));
        let frame = |time, index| {
            let mut rect = Rect::NOTHING;
            let mut output = context.run_ui(
                RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    rect = super::paint_navigation_connection(ui, bounds, 3, index);
                },
            );
            output.textures_delta.clear();
            (rect, output)
        };
        let (start, _) = frame(0.0, 0);
        frame(0.1, 2);
        let (moving, _) = frame(0.16, 2);
        let (end, _) = frame(0.4, 2);
        assert!(moving.left() > start.left() && moving.left() < end.left());
        assert_eq!(start.size(), end.size());
        assert_eq!(end.bottom(), bounds.bottom());
        assert!(bounds.contains_rect(end));
        frame(0.5, 2);
        let (_, settled) = frame(0.6, 2);
        assert_eq!(
            settled.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::MAX
        );
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let (snapped, _) = frame(0.7, 0);
        assert_eq!(snapped, start);
    }

    use super::{draw_top_panel, navigation_button};
    use crate::app::models::AppStateStorage;
    use crate::app::runtime::AppState;
    use crate::app::shell::{GroupRoute, WindowRoute};
    use eframe::egui::{self, Pos2, RawInput, Rect};
    use std::sync::{Arc, RwLock};

    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn compact_settings_keep_explanations_in_bounded_hover_tooltips() {
        fn frame(
            app: &mut AppState,
            context: &egui::Context,
            time: f64,
            pointer: Option<Pos2>,
        ) -> egui::FullOutput {
            let mut output = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
                    time: Some(time),
                    events: vec![
                        pointer.map_or(egui::Event::PointerGone, egui::Event::PointerMoved)
                    ],
                    ..Default::default()
                },
                |ui| draw_top_panel(app, ui),
            );
            output.textures_delta.clear();
            output
        }
        fn text_rect(output: &egui::FullOutput, label: &str) -> Rect {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing settings label {label}"))
        }
        for dark in [true, false] {
            let context = egui::Context::default();
            context
                .set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            context.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            context.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
                20,
            );
            frame(&mut app, &context, 0.0, None);
            app.ui.app_settings_open = true;
            frame(&mut app, &context, 0.1, None);
            let output = frame(&mut app, &context, 0.2, None);
            let heading = text_rect(&output, "Application settings");
            let startup = text_rect(&output, "Start minimized");
            assert!(
                startup.bottom() - heading.top() <= 200.0,
                "settings must remain compact without explanatory paragraphs"
            );
            assert!(
                output.shapes.iter().all(|shape| match &shape.shape {
                    egui::Shape::Text(text) => ![
                        "Records each app",
                        "History:",
                        "tracked processes",
                        "Open in the system tray"
                    ]
                    .iter()
                    .any(|detail| text.galley.text().contains(detail)),
                    _ => true,
                }),
                "long explanations belong in tooltips"
            );
            let estimate = app.cpu_load_memory_estimate();
            let summary = estimate.lines().next().unwrap();
            text_rect(&output, summary);
            let mut time = 1.0;
            for (label, expected) in [
                ("CPU load monitoring", "Records each app"),
                ("5 minutes", "History starts"),
                (summary, "Calculated history storage"),
                ("Start minimized", "Open in the system tray"),
            ] {
                let pointer = text_rect(&output, label).center();
                frame(&mut app, &context, time, Some(pointer));
                frame(&mut app, &context, time + 0.1, Some(pointer));
                let hovered = frame(&mut app, &context, time + 0.2, Some(pointer));
                let hint = hovered
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text().contains(expected) => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing hover explanation for {label}"));
                assert!(
                    hint.width() <= 265.0
                        && hint.left() >= 0.0
                        && hint.right() <= 400.0
                        && hint.top() >= 0.0
                        && hint.bottom() <= 600.0,
                    "tooltip must fit the minimum viewport: {hint:?}"
                );
                assert!(app.ui.app_settings_open);
                frame(&mut app, &context, time + 0.3, None);
                time += 1.0;
            }
            app.cpu_load.preference_error = Some("CPU preference save failed".into());
            app.ui.startup_setting_error = Some("Startup preference save failed".into());
            let output = frame(&mut app, &context, time, None);
            text_rect(&output, "CPU preference save failed");
            text_rect(&output, "Startup preference save failed");
        }
    }

    #[test]
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn history_selector_keeps_settings_open_and_applies_all_choices() {
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
                |ui| draw_top_panel(app, ui),
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
                let output = frame(&mut app, &context, height, vec![]);
                let settings = label(&output, "⚙ App settings").unwrap();
                let mut output = click(&mut app, &context, height, settings);
                assert!(label(&output, "Application settings").is_some());
                let mut current = "5 minutes";
                for (choice, seconds) in
                    [("30 seconds", 30), ("30 minutes", 1800), ("5 minutes", 300)]
                {
                    let selector =
                        label(&output, current).expect("history selector must remain visible");
                    output = click(&mut app, &context, height, selector);
                    assert!(
                        label(&output, "Application settings").is_some(),
                        "opening the history selector closed App settings"
                    );
                    let option = label(&output, choice)
                        .unwrap_or_else(|| panic!("missing history option {choice}"));
                    output = click(&mut app, &context, height, option);
                    app.wait_for_test_save();
                    assert_eq!(app.cpu_load_history_seconds(), seconds);
                    assert!(
                        label(&output, "Application settings").is_some(),
                        "selecting a history value closed App settings"
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
                assert!(label(&output, "Application settings").is_none());
                output = click(&mut app, &context, height, settings);
                click(&mut app, &context, height, label(&output, current).unwrap());
                output = click(&mut app, &context, height, Pos2::new(390.0, height - 10.0));
                assert!(
                    label(&output, "Application settings").is_none(),
                    "outside click must dismiss both menus"
                );
                output = click(&mut app, &context, height, settings);
                assert!(label(&output, "Application settings").is_some());
                output = click(&mut app, &context, height, settings);
                assert!(
                    label(&output, "Application settings").is_none(),
                    "settings button must toggle its menu closed"
                );
            }
        }
    }

    fn navigation_frame(
        context: &egui::Context,
        time: f64,
        selected: bool,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, egui::Response) {
        let mut response = None;
        let mut output = context.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| response = Some(navigation_button(ui, "Test tab", selected, 120.0)),
        );
        output.textures_delta.clear();
        (output, response.unwrap())
    }

    fn navigation_gradient(output: &egui::FullOutput) -> Option<&egui::epaint::Mesh> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) => Some(mesh.as_ref()),
            _ => None,
        })
    }

    #[test]
    fn navigation_gradient_supports_both_themes_and_disabled_motion() {
        for dark in [true, false] {
            let context = egui::Context::default();
            context.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            context.all_styles_mut(|style| style.animation_time = 0.0);
            navigation_frame(&context, 0.0, true, vec![]);
            let (output, selected) = navigation_frame(&context, 0.02, true, vec![]);
            let mesh = navigation_gradient(&output).expect("selected tab needs a gradient");
            assert!(mesh.is_valid());
            assert!(
                mesh.vertices.len() <= 128,
                "small tab effect needs only bounded geometry"
            );
            let first_color = mesh.vertices[0].color;
            assert!(mesh
                .vertices
                .iter()
                .any(|vertex| vertex.color != first_color));
            assert!(mesh
                .vertices
                .iter()
                .all(|vertex| selected.rect.contains(vertex.pos)));
            let (output, unselected) = navigation_frame(&context, 0.04, false, vec![]);
            assert!(
                navigation_gradient(&output).is_none(),
                "disabled motion snaps to the new state"
            );
            assert_eq!(selected.rect, unselected.rect);
        }
    }

    #[test]
    fn navigation_effects_retarget_and_stop_requesting_idle_frames() {
        let context = egui::Context::default();
        for (index, selected) in [true, false, true, false, true].into_iter().enumerate() {
            navigation_frame(&context, index as f64 / 60.0, selected, vec![]);
        }
        let (_, response) = navigation_frame(&context, 0.1, true, vec![]);
        let rect = response.rect;
        context.memory_mut(|memory| memory.request_focus(response.id));
        let (_, response) = navigation_frame(
            &context,
            0.12,
            true,
            vec![egui::Event::PointerMoved(rect.center())],
        );
        assert!(response.has_focus());
        assert_eq!(response.rect, rect);
        let (_, pressed) = navigation_frame(
            &context,
            0.14,
            true,
            vec![egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
        );
        assert!(pressed.is_pointer_button_down_on());
        let (_, released) = navigation_frame(
            &context,
            0.16,
            true,
            vec![egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            released.clicked(),
            "decoration preserves the normal button interaction"
        );
        assert_eq!(released.rect, rect);
        navigation_frame(&context, 0.18, true, vec![egui::Event::PointerGone]);
        for index in 12..40 {
            navigation_frame(&context, index as f64 / 60.0, true, vec![]);
        }
        let (output, _) = navigation_frame(&context, 1.0, true, vec![]);
        let settled =
            navigation_gradient(&output).expect("selected gradient remains after settling");
        let (next, _) = navigation_frame(&context, 1.1, true, vec![]);
        let unchanged = navigation_gradient(&next).unwrap();
        assert_eq!(settled.vertices, unchanged.vertices);
        assert_eq!(
            next.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
            std::time::Duration::MAX,
            "the decorative effect must not request frames while idle"
        );
    }

    #[test]
    fn compact_shell_controls_have_one_clear_location() {
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
                    crate::app::shell::presenters::footer::draw_bottom_panel(&mut app, ui);
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
                crate::app::shell::presenters::footer::draw_bottom_panel(&mut app, ui);
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
        assert!(!labels.iter().any(|(label, _)| label == "CPU Affinity Tool"));
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
                        // Right-aligned text uses an anchor, not its visible top-left corner.
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing label: {label}"))
        };
        let new_group = text_rect("+ New group");
        assert!(new_group.top() > text_rect("Logs").bottom());
        assert!(new_group.left() > 280.0 && new_group.right() <= 400.0);
        let settings = text_rect("⚙ App settings");
        let version_text = format!("v{}", crate::app::models::APP_VERSION);
        let version = text_rect(&version_text);
        assert!(settings.left() <= 20.0 && settings.bottom() < text_rect("Affinity groups").top());
        assert!(version.left() > 340.0 && version.right() <= 400.0);
        assert!((version.center().y - settings.center().y).abs() < 8.0);
        assert_eq!(
            labels
                .iter()
                .filter(|(label, _)| label == &version_text)
                .count(),
            1
        );
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
                        crate::app::shell::presenters::footer::draw_bottom_panel(&mut app, ui);
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
    #[cfg(all(target_os = "windows", feature = "windows"))]
    fn main_navigation_is_larger_than_the_tweak_section_switch() {
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(AppStateStorage::test_fixture(|_| {}))),
            4,
        );
        app.set_current_window(WindowRoute::Tuning);
        let context = egui::Context::default();
        context.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let frame = |app: &mut AppState| {
            let mut output = context.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    draw_top_panel(app, ui);
                    crate::app::shell::presenters::system_tweaks::draw_system_tweaks(app, ui);
                },
            );
            output.textures_delta.clear();
            output
        };
        frame(&mut app);
        let output = frame(&mut app);
        let track = |label: &str| {
            let center = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .unwrap();
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.rect.width() > 250.0
                            && rect.rect.height() < 60.0
                            && rect.rect.contains(center) =>
                    {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .min_by(|a, b| a.height().total_cmp(&b.height()))
                .unwrap()
        };
        let primary = track("Affinity groups");
        let secondary = track("System");
        assert!(
            primary.height() >= 32.0 && secondary.height() <= 26.0,
            "main switch must be taller: {primary:?} vs {secondary:?}"
        );
        assert!(
            primary.width() >= 370.0 && secondary.width() <= 310.0,
            "main switch fills the window, secondary stays compact: {primary:?} vs {secondary:?}"
        );
        assert!((primary.center().x - secondary.center().x).abs() < 3.0);
        assert!(primary.bottom() < secondary.top());
    }
}
