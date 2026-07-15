use crate::app::models::{CoreInfo, CoreType, CpuSchema};
use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    ghost_button, glass_frame, inset_frame, inter_semibold_family, paint_focus_ring,
    paint_selected_tone_feedback, palette, toned_button, ToneRole, ToneTokens, UiPalette,
    BUTTON_FONT_SIZE,
};
use crate::app::shell::GroupFormSession;
use eframe::egui::{self, CentralPanel, RichText};

const CORE_TILE_WIDTH: f32 = 56.0;

/// Form for creating/editing a group: divided into rendering the name and the section with cores and clusters.
fn draw_group_form_ui(
    ui: &mut egui::Ui,
    groups: &mut GroupFormSession,
    cpu_schema: &mut CpuSchema,
    is_edit: bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    glass_frame(ui).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Group name").strong());
            ui.add_sized(
                [ui.available_width().min(520.0), 25.0],
                egui::TextEdit::singleline(&mut groups.group_name),
            )
            .request_focus();
        });

        ui.add_space(6.0);

        ui.checkbox(
            &mut groups.run_all_enabled,
            "Show a Run all action for this group",
        );

        ui.add_space(5.0);
        ui.separator();
        ui.add_space(5.0);

        draw_cpu_cores_ui(ui, groups, cpu_schema);

        ui.add_space(9.0);
        ui.separator();
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            let save_label = if is_edit {
                "Save changes"
            } else {
                "Create group"
            };
            if toned_button(
                ui,
                egui::Button::new(RichText::new(save_label).strong())
                    .min_size(egui::vec2(110.0, 28.0)),
                ToneRole::Primary,
            )
            .clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                on_save();
            }

            if ui
                .add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 28.0)))
                .clicked()
            {
                on_cancel();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if is_edit {
                    if let Some(delete_fn) = on_delete {
                        if toned_button(
                            ui,
                            egui::Button::new("Delete group").min_size(egui::vec2(100.0, 28.0)),
                            ToneRole::Danger,
                        )
                        .clicked()
                        {
                            delete_fn();
                        }
                    }
                }
            });
        });
    });
}

/// Rendering the CPU cores section: a list of already created clusters and a panel of free cores.
fn draw_cpu_cores_ui(ui: &mut egui::Ui, groups: &mut GroupFormSession, cpu_schema: &mut CpuSchema) {
    let model_display = if cpu_schema.clusters.is_empty() {
        format!("{} (No preset matched)", cpu_schema.model)
    } else {
        cpu_schema.model.clone()
    };
    ui.heading("CPU topology");
    ui.label(RichText::new(model_display).small().weak());
    ui.label(
        RichText::new(format!(
            "{} of {} threads selected",
            groups
                .core_selection
                .iter()
                .filter(|selected| **selected)
                .count(),
            groups.core_selection.len()
        ))
        .small()
        .strong(),
    );
    ui.add_space(4.0);
    ui.separator();

    let assigned = cpu_schema.get_assigned_cores();
    let total_cores = groups.core_selection.len();
    let free_core_indexes: Vec<usize> =
        (0..total_cores).filter(|i| !assigned.contains(i)).collect();

    for cluster in cpu_schema.clusters.iter_mut() {
        inset_frame(ui).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&cluster.name).strong());
            });
            draw_core_buttons(ui, groups, &mut cluster.cores);
        });
    }

    if !free_core_indexes.is_empty() {
        inset_frame(ui).show(ui, |ui| {
            ui.label(RichText::new("Free Cores").strong());

            // Temporary CoreInfo for drawing buttons of free cores
            let mut free_cores: Vec<CoreInfo> = free_core_indexes
                .iter()
                .map(|&i| CoreInfo {
                    index: i,
                    core_type: CoreType::Other,
                    label: format!("{i}"),
                })
                .collect();

            draw_core_buttons(ui, groups, &mut free_cores);
        });
    }
}

fn core_tile_tokens(
    _core_type: CoreType,
    is_selected: bool,
    colors: &UiPalette,
) -> Option<ToneTokens> {
    is_selected.then_some(colors.primary)
}

#[derive(Debug, PartialEq, Eq)]
struct CoreTileText {
    primary: String,
    secondary: String,
    accessible: String,
}

fn core_tile_text(label: &str, thread_index: usize) -> CoreTileText {
    CoreTileText {
        primary: label.to_string(),
        secondary: format!("thread {thread_index}"),
        accessible: format!("{label}, thread {thread_index}"),
    }
}

fn core_tile_widget_info(
    enabled: bool,
    is_selected: bool,
    text: &CoreTileText,
) -> egui::WidgetInfo {
    egui::WidgetInfo::selected(
        egui::WidgetType::Button,
        enabled,
        is_selected,
        &text.accessible,
    )
}

fn selected_core_tile_fill(tokens: ToneTokens, hovered: bool, pressed: bool) -> egui::Color32 {
    if pressed {
        tokens.active_fill
    } else if hovered {
        tokens.hover_fill
    } else {
        tokens.fill
    }
}

fn core_tile_button(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    core: &CoreInfo,
    is_selected: bool,
    tokens: Option<ToneTokens>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let text = core_tile_text(&core.label, core.index);
    let response = response.on_hover_text(&text.accessible);
    let (fill, border, foreground) = if let Some(tokens) = tokens {
        let fill = selected_core_tile_fill(
            tokens,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );
        (fill, egui::Stroke::new(1.0, tokens.border), tokens.fg)
    } else {
        let visuals = ui.style().interact(&response);
        (
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            visuals.fg_stroke.color,
        )
    };
    ui.painter()
        .rect(rect, 5.0, fill, border, egui::StrokeKind::Middle);
    let painter = ui.painter().with_clip_rect(rect.shrink(2.0));
    painter.text(
        egui::pos2(rect.center().x, rect.center().y - 5.0),
        egui::Align2::CENTER_CENTER,
        &text.primary,
        egui::FontId::new(BUTTON_FONT_SIZE, inter_semibold_family()),
        foreground,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.center().y + 6.0),
        egui::Align2::CENTER_CENTER,
        &text.secondary,
        egui::FontId::proportional(7.5),
        foreground,
    );
    if is_selected {
        let check = egui::pos2(rect.left() + 4.0, rect.top() + 6.0);
        let check_stroke = egui::Stroke::new(1.25, foreground);
        painter.line_segment([check, check + egui::vec2(2.0, 2.0)], check_stroke);
        painter.line_segment(
            [check + egui::vec2(2.0, 2.0), check + egui::vec2(5.0, -2.0)],
            check_stroke,
        );
    }
    response.widget_info(|| core_tile_widget_info(response.enabled(), is_selected, &text));
    paint_focus_ring(ui, &response);
    if let Some(tokens) = tokens {
        paint_selected_tone_feedback(ui, &response, tokens);
    }
    response
}

fn draw_core_buttons(ui: &mut egui::Ui, groups: &mut GroupFormSession, cores: &mut [CoreInfo]) {
    draw_core_buttons_impl(ui, groups, cores, |_| {});
}

fn draw_core_buttons_impl(
    ui: &mut egui::Ui,
    groups: &mut GroupFormSession,
    cores: &mut [CoreInfo],
    mut record_rect: impl FnMut(egui::Rect),
) {
    let colors = palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let all_selected = cores.iter().all(|c| groups.core_selection[c.index]);
        let all_tokens = core_tile_tokens(CoreType::Other, all_selected, colors);
        let mut all_label =
            RichText::new(if all_selected { "✓ All" } else { "All" }).size(BUTTON_FONT_SIZE);
        if let Some(tokens) = all_tokens {
            all_label = all_label.color(tokens.fg);
        }
        let all_button = egui::Button::new(all_label)
            .selected(all_selected)
            .truncate();
        let mut all_response = if let Some(tokens) = all_tokens {
            let response = ui.add_sized(
                egui::vec2(46.0, 30.0),
                all_button
                    .fill(tokens.fill)
                    .stroke(egui::Stroke::new(1.0, tokens.border)),
            );
            paint_focus_ring(ui, &response);
            paint_selected_tone_feedback(ui, &response, tokens);
            response
        } else {
            let response = ui.add_sized(egui::vec2(46.0, 30.0), all_button);
            paint_focus_ring(ui, &response);
            response
        };
        record_rect(all_response.rect);
        if all_response.clicked() {
            let mut changed = false;
            for c in cores.iter() {
                let target = !all_selected;
                if groups.core_selection[c.index] != target {
                    groups.core_selection[c.index] = target;
                    changed = true;
                }
            }
            if changed {
                all_response.mark_changed();
            }
            groups.last_clicked_core = None;
        }

        for core in cores.iter() {
            let is_selected = groups.core_selection[core.index];
            let size = match core.core_type {
                CoreType::Performance => egui::vec2(CORE_TILE_WIDTH, 36.0),
                _ => egui::vec2(CORE_TILE_WIDTH, 30.0),
            };

            let mut response = core_tile_button(
                ui,
                size,
                core,
                is_selected,
                core_tile_tokens(core.core_type, is_selected, colors),
            );
            record_rect(response.rect);

            if response.clicked() {
                let shift = ui.input(|i| i.modifiers.shift);
                if let (true, Some(last_idx)) = (shift, groups.last_clicked_core) {
                    let start = last_idx.min(core.index);
                    let end = last_idx.max(core.index);
                    let target_state = groups.core_selection[last_idx];
                    let mut changed = false;
                    for i in start..=end {
                        if i < groups.core_selection.len()
                            && groups.core_selection[i] != target_state
                        {
                            groups.core_selection[i] = target_state;
                            changed = true;
                        }
                    }
                    if changed {
                        response.mark_changed();
                    }
                } else {
                    groups.core_selection[core.index] = !is_selected;
                    groups.last_clicked_core = Some(core.index);
                    response.mark_changed();
                }
            }
        }
    });
}

#[cfg(test)]
fn draw_core_buttons_for_test(
    ui: &mut egui::Ui,
    groups: &mut GroupFormSession,
    cores: &mut [CoreInfo],
) -> Vec<egui::Rect> {
    let mut control_rects = Vec::with_capacity(cores.len() + 1);
    draw_core_buttons_impl(ui, groups, cores, |rect| control_rects.push(rect));
    control_rects
}

/// Group creation window.
pub fn create_group_window(app: &mut AppState, root_ui: &mut egui::Ui) {
    let mut create_clicked = false;
    let mut cancel_clicked = false;

    CentralPanel::default().show(root_ui, |ui| {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Create affinity group").strong());
                ui.label(
                    RichText::new("Choose a name and the CPU threads this group may use")
                        .small()
                        .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ghost_button(ui, egui::Button::new("Close"))
                    .on_hover_text("Close")
                    .clicked()
                {
                    cancel_clicked = true;
                }
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut schema = app.get_cpu_schema();
                draw_group_form_ui(
                    ui,
                    &mut app.ui.group_form,
                    &mut schema,
                    false,
                    &mut || create_clicked = true,
                    &mut || cancel_clicked = true,
                    None,
                );
            });
    });

    if create_clicked || cancel_clicked {
        if create_clicked {
            app.commit_group_form_session();
        } else {
            app.cancel_group_form_session();
        }
    }
}

/// Group editing window.
pub fn edit_group_window(app: &mut AppState, root_ui: &mut egui::Ui) {
    CentralPanel::default().show(root_ui, |ui| {
        let mut save_clicked = false;
        let mut delete_clicked = false;
        let mut cancel_clicked = false;

        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Edit affinity group").strong());
                ui.label(
                    RichText::new("Update group identity, actions, and CPU topology")
                        .small()
                        .weak(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ghost_button(ui, egui::Button::new("Close"))
                    .on_hover_text("Close")
                    .clicked()
                {
                    cancel_clicked = true;
                }
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut schema = app.get_cpu_schema();
                draw_group_form_ui(
                    ui,
                    &mut app.ui.group_form,
                    &mut schema,
                    true,
                    &mut || save_clicked = true,
                    &mut || cancel_clicked = true,
                    Some(&mut || delete_clicked = true),
                );
            });

        if save_clicked {
            app.commit_group_form_session();
        }

        if delete_clicked {
            app.delete_current_group_form_target();
        }

        if cancel_clicked {
            app.cancel_group_form_session();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        core_tile_text, core_tile_tokens, core_tile_widget_info, draw_core_buttons_for_test,
        selected_core_tile_fill, CORE_TILE_WIDTH,
    };
    use crate::app::models::{CoreInfo, CoreType};
    use crate::app::shell::presenters::shared_elements::{
        inter_semibold_family, palette_for_dark_mode, ui_font_definitions, BUTTON_FONT_SIZE,
    };
    use crate::app::shell::GroupFormSession;
    use eframe::egui::{self, Pos2, RawInput, Rect, WidgetType};

    fn render_narrow_core_tiles(is_selected: bool) -> Vec<Rect> {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_font_definitions());
        let mut groups = GroupFormSession {
            editing_group_id: None,
            editing_selection: None,
            core_selection: vec![is_selected; 20],
            group_name: String::new(),
            run_all_enabled: false,
            last_clicked_core: None,
        };
        let mut cores = (0..20)
            .map(|index| CoreInfo {
                index,
                core_type: match index % 4 {
                    0 => CoreType::Performance,
                    1 => CoreType::Efficient,
                    2 => CoreType::HyperThreading,
                    _ => CoreType::Other,
                },
                label: format!("T{index}"),
            })
            .collect::<Vec<_>>();
        let mut tile_rects = Vec::new();

        let _ = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(190.0, 500.0))),
                ..Default::default()
            },
            |ui| {
                tile_rects = draw_core_buttons_for_test(ui, &mut groups, &mut cores);
            },
        );
        tile_rects
    }

    #[test]
    fn test_core_tile_text_keeps_full_label_and_thread_on_separate_lines() {
        let text = core_tile_text("P0", 0);
        assert_eq!(text.primary, "P0");
        assert_eq!(text.secondary, "thread 0");
        assert_eq!(text.accessible, "P0, thread 0");

        let text = core_tile_text("E3", 15);
        assert_eq!(text.primary, "E3");
        assert_eq!(text.secondary, "thread 15");
        assert_eq!(text.accessible, "E3, thread 15");
    }

    #[test]
    fn test_core_tile_metadata_keeps_button_and_selected_semantics() {
        let text = core_tile_text("E3", 15);
        let info = core_tile_widget_info(true, true, &text);

        assert_eq!(info.typ, WidgetType::Button);
        assert!(info.enabled);
        assert_eq!(info.selected, Some(true));
        assert_eq!(info.label.as_deref(), Some("E3, thread 15"));
    }

    #[test]
    fn test_selected_core_tile_uses_primary_fill_for_each_interaction_state() {
        for colors in [palette_for_dark_mode(true), palette_for_dark_mode(false)] {
            let tokens = colors.primary;
            assert_eq!(selected_core_tile_fill(tokens, false, false), tokens.fill);
            assert_eq!(
                selected_core_tile_fill(tokens, true, false),
                tokens.hover_fill
            );
            assert_eq!(
                selected_core_tile_fill(tokens, true, true),
                tokens.active_fill
            );
        }
    }

    #[test]
    fn test_core_tile_inter_galleys_fit_full_large_thread_labels() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_font_definitions());
        let mut measured = Vec::new();

        let _ = ctx.run_ui(RawInput::default(), |ui| {
            for (label, thread_index) in [("P19", 19), ("P127", 127)] {
                let text = core_tile_text(label, thread_index);
                let primary = ui.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        text.primary,
                        egui::FontId::new(BUTTON_FONT_SIZE, inter_semibold_family()),
                        egui::Color32::WHITE,
                    )
                });
                let secondary = ui.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        text.secondary,
                        egui::FontId::proportional(7.5),
                        egui::Color32::WHITE,
                    )
                });
                measured.push((primary.size(), secondary.size()));
            }
        });

        for (primary, secondary) in measured {
            assert!(primary.x <= CORE_TILE_WIDTH - 4.0);
            assert!(secondary.x <= CORE_TILE_WIDTH - 4.0);
            assert!(primary.y + secondary.y <= 26.0);
        }
    }

    #[test]
    fn test_selected_core_tiles_share_the_muted_accent_tone_in_both_themes() {
        for colors in [palette_for_dark_mode(true), palette_for_dark_mode(false)] {
            for core_type in [
                CoreType::Performance,
                CoreType::Efficient,
                CoreType::HyperThreading,
                CoreType::Other,
            ] {
                assert_eq!(
                    core_tile_tokens(core_type, true, colors),
                    Some(colors.primary)
                );
                assert_eq!(core_tile_tokens(core_type, false, colors), None);
            }

            assert_ne!(colors.primary.fill, colors.core_other);
        }
    }

    #[test]
    fn test_narrow_twenty_thread_layout_wraps_without_selected_width_shift() {
        let unselected = render_narrow_core_tiles(false);
        let selected = render_narrow_core_tiles(true);

        assert_eq!(unselected.len(), 21);
        assert_eq!(selected.len(), unselected.len());
        for (control_index, (unselected_rect, selected_rect)) in
            unselected.iter().zip(&selected).enumerate()
        {
            for rect in [unselected_rect, selected_rect] {
                assert!(rect.left() >= 0.0);
                assert!(rect.right() <= 190.0);
            }
            assert_eq!(selected_rect, unselected_rect);
            let expected_size = if control_index == 0 {
                egui::vec2(46.0, 30.0)
            } else if (control_index - 1) % 4 == 0 {
                egui::vec2(CORE_TILE_WIDTH, 36.0)
            } else {
                egui::vec2(CORE_TILE_WIDTH, 30.0)
            };
            assert_eq!(unselected_rect.size(), expected_size);
        }
        assert!(
            unselected
                .windows(2)
                .any(|pair| pair[1].top() > pair[0].top()),
            "twenty thread controls must wrap to more than one row"
        );
    }
}
