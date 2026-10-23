use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    close_button, content_frame, danger_color, form_text_edit, glass_frame, success_color,
    toned_button, ToneRole,
};
use crate::app::shell::sessions::RuleShortcutResult;
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, CentralPanel, ComboBox, FontId, Label, Layout, RichText, TextFormat,
};
use os_api::PriorityClass;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
fn pick_binary_path() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Executables", &["exe"])
        .pick_file()
}

#[cfg(not(target_os = "windows"))]
fn pick_binary_path() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_file()
}

#[cfg(target_os = "windows")]
fn browse_binary_hover_text() -> &'static str {
    "Select executable..."
}

#[cfg(not(target_os = "windows"))]
fn browse_binary_hover_text() -> &'static str {
    "Select binary path..."
}

fn shortcut_button_enabled_for_current_frame(status_enabled: bool, draft_changed: bool) -> bool {
    status_enabled && !draft_changed
}

fn shortcut_message_for_current_frame(
    status_message: Option<&str>,
    status_enabled: bool,
    draft_changed: bool,
) -> Option<&str> {
    if status_enabled && draft_changed {
        Some("Save changes first.")
    } else {
        status_message
    }
}

fn wrapped_identifier_layout_job(value: &str, max_width: f32, color: egui::Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = max_width;
    job.wrap.break_anywhere = true;
    job.append(
        value,
        0.0,
        TextFormat {
            font_id: FontId::monospace(10.0),
            color,
            ..Default::default()
        },
    );
    job
}

fn binary_path_field(ui: &mut egui::Ui, path: &mut String) -> egui::Response {
    let edit = form_text_edit(path).desired_width(f32::INFINITY).show(ui);
    if edit.galley.size().x > edit.text_clip_rect.width() {
        ui.add(
            Label::new(wrapped_identifier_layout_job(
                path,
                ui.available_width(),
                ui.visuals().text_color(),
            ))
            .selectable(true),
        );
    }
    edit.response.response
}

pub fn draw_app_run_settings(app: &mut AppState, root_ui: &mut egui::Ui) {
    if app.ui.app_edit_state.target.is_none() {
        app.set_current_window(WindowRoute::Groups(GroupRoute::List));
        return;
    }

    if !app.ensure_current_edit_loaded() {
        return;
    }
    app.ui
        .app_edit_state
        .process_names
        .prepare(root_ui.ctx(), !cfg!(test));
    let process_names = app.ui.app_edit_state.process_names.names.clone();

    let mut is_close = false;
    let mut save_clicked = false;
    let mut delete_clicked = false;
    let mut create_shortcut_clicked = false;
    let mut draft_changed = false;
    let shortcut_status = app.current_app_edit_shortcut_status();
    let shortcut_result = app.ui.app_edit_state.shortcut_result.clone();
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let mut background = app.ui.app_edit_state.background.clone();
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let tuning_snapshot = app.tuning_snapshot();
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let mut inspect_background = false;
    #[cfg(all(target_os = "windows", feature = "windows"))]
    let background_error = app.ui.app_edit_state.background_error.clone();

    CentralPanel::default().frame(content_frame(root_ui)).show(root_ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Application rule").strong());
                ui.label(
                    RichText::new("Configure launch, scheduling, and process tracking")
                        .small()
                        .weak(),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if close_button(ui)
                    .on_hover_text("Close")
                    .clicked()
                {
                    is_close = true;
                }
            });
        });
        ui.add_space(6.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            glass_frame(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let selected_app = app
                    .ui
                    .app_edit_state
                    .current_edit
                    .as_mut()
                    .expect("edit_app_clone must be initialized");
                let installed_aumid = selected_app.installed_aumid().map(str::to_owned);

                ui.label(RichText::new("App name").strong());
                if ui.add(form_text_edit(&mut selected_app.name).desired_width(f32::INFINITY)).changed() {
                    draft_changed = true;
                }
                ui.add_space(6.0);

                if selected_app.is_path_target() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Binary path").strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Browse").on_hover_text(browse_binary_hover_text()).clicked() {
                                if let Some(path) = pick_binary_path() {
                                    if let Some(bin_path) = selected_app.bin_path_mut() {
                                        *bin_path = path;
                                        draft_changed = true;
                                    }
                                }
                            }
                        });
                    });
                    let mut path = selected_app.bin_path()
                        .map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
                    if binary_path_field(ui, &mut path).changed() {
                        if let Some(bin_path) = selected_app.bin_path_mut() {
                            *bin_path = PathBuf::from(path);
                            draft_changed = true;
                        }
                    }
                }

                if let Some(aumid) = installed_aumid {
                    ui.add_space(5.0);
                    ui.label(RichText::new("Installed app ID (AUMID)").strong());
                    ui.add(Label::new(wrapped_identifier_layout_job(
                        &aumid,
                        ui.available_width(),
                        ui.visuals().text_color(),
                    )));
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Priority").strong());
                    ComboBox::from_id_salt("priority_combo")
                        .width(140.0)
                        .selected_text(format!("{:?}", selected_app.priority))
                        .show_ui(ui, |ui| {
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Realtime,
                                    "RealTime",
                                )
                                .changed();
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::High,
                                    "High",
                                )
                                .changed();
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::AboveNormal,
                                    "Above Normal",
                                )
                                .changed();
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Normal,
                                    "Normal",
                                )
                                .changed();
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::BelowNormal,
                                    "Below Normal",
                                )
                                .changed();
                            draft_changed |= ui
                                .selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Idle,
                                    "Low",
                                )
                                .changed();
                        });
                });

                ui.add_space(6.0);
                if ui
                    .checkbox(
                        &mut selected_app.autorun,
                        RichText::new("Start this app on startup").strong(),
                    )
                    .changed()
                {
                    draft_changed = true;
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                #[cfg(all(target_os = "windows", feature = "windows"))]
                {
                    if let Some(draft) = &mut background {
                        let supported = selected_app.bin_path().is_some_and(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")));
                        let (changed, inspect) = crate::app::features::system_tweaks::background_ui::draw_rule(ui, &tuning_snapshot.background, draft, supported, false);
                        draft_changed |= changed;
                        inspect_background |= inspect;
                    }
                    if let Some(error) = &background_error {
                        ui.add(Label::new(RichText::new(error).weak()).wrap());
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }

                if selected_app.is_args_editable() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Command line arguments").strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Add Argument").clicked() {
                                selected_app.args.push(String::new());
                                draft_changed = true;
                            }
                        });
                    });
                    ui.add_space(3.0);

                    let mut arg_to_remove = None;
                    if selected_app.args.is_empty() {
                        ui.label(RichText::new("No arguments defined.").weak().italics());
                    } else {
                        for (i, arg) in selected_app.args.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}:", i + 1));
                                if ui.add(form_text_edit(arg).desired_width((ui.available_width() - 65.0).clamp(60.0, 360.0))).changed() {
                                    draft_changed = true;
                                }
                                if ui.button("Remove").clicked() {
                                    arg_to_remove = Some(i);
                                }
                            });
                        }
                    }

                    if let Some(idx) = arg_to_remove {
                        selected_app.args.remove(idx);
                        draft_changed = true;
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }

                ui.label(RichText::new("Process tracking").strong());
                if ui
                    .checkbox(
                        &mut selected_app.manage_descendants,
                        "Manage descendant processes",
                    )
                    .on_hover_text(
                        "Apply this rule to children of verified primary processes. "
                            .to_string()
                            + "Tracked process names stay explicit matches and never become descendant roots.",
                    )
                    .changed()
                {
                    draft_changed = true;
                }

                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Tracked process names").strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Add Process Name").clicked() {
                            selected_app.additional_processes.push(String::new());
                            draft_changed = true;
                        }
                    });
                });
                ui.add_space(3.0);

                let mut proc_to_remove = None;
                if selected_app.additional_processes.is_empty() {
                    let empty_text = if selected_app.is_path_target() {
                        "Process-name rediscovery is off for this path app. Add a name so active monitoring can find it later."
                    } else {
                        "No tracked process names defined."
                    };
                    ui.label(RichText::new(empty_text).weak().italics());
                } else {
                    for (i, proc_name) in selected_app.additional_processes.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}:", i + 1));
                            if crate::app::features::rules::process_names::field(ui,proc_name,(ui.available_width() - 65.0).clamp(60.0,360.0),&process_names,i) {
                                draft_changed = true;
                            }
                            if ui.button("Remove").clicked() {
                                proc_to_remove = Some(i);
                            }
                        });
                    }
                }

                if let Some(idx) = proc_to_remove {
                    selected_app.additional_processes.remove(idx);
                    draft_changed = true;
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                if shortcut_status.visible {
                    let shortcut_enabled = shortcut_button_enabled_for_current_frame(
                        shortcut_status.enabled,
                        draft_changed,
                    );
                    let shortcut_message = shortcut_message_for_current_frame(
                        shortcut_status.message.as_deref(),
                        shortcut_status.enabled,
                        draft_changed,
                    );

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                shortcut_enabled,
                                egui::Button::new("Create desktop shortcut")
                                    .min_size(egui::vec2(150.0, 27.0)),
                            )
                            .clicked()
                        {
                            create_shortcut_clicked = true;
                        }
                    });
                    if let Some(message) = shortcut_message {
                        ui.add(Label::new(RichText::new(message).weak()).wrap());
                    }
                    if !draft_changed {
                        if let Some(result) = &shortcut_result {
                            match result {
                                RuleShortcutResult::Created { filename } => {
                                    ui.label(RichText::new(filename).color(success_color(ui)));
                                }
                                RuleShortcutResult::Failed { message } => {
                                    ui.label(RichText::new(message).color(danger_color(ui)));
                                }
                            }
                        }
                    }
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }

                ui.horizontal(|ui| {
                    if toned_button(
                        ui,
                        egui::Button::new(RichText::new("Save Changes").strong())
                            .min_size(egui::vec2(105.0, 28.0)),
                        ToneRole::Primary,
                    )
                        .clicked()
                    {
                        save_clicked = true;
                        is_close = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(80.0, 28.0)))
                        .clicked()
                    {
                        is_close = true;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if toned_button(
                            ui,
                            egui::Button::new("Remove from group")
                                .min_size(egui::vec2(125.0, 28.0)),
                            ToneRole::Danger,
                        )
                            .on_hover_text("Remove this application from the group")
                            .clicked()
                        {
                            delete_clicked = true;
                            is_close = true;
                        }
                    });
                });
            });
        });
    });

    if root_ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("process-names-refresh")))
        .unwrap_or(false)
    {
        app.ui.app_edit_state.process_names.request_refresh();
        app.ui
            .app_edit_state
            .process_names
            .prepare(root_ui.ctx(), !cfg!(test));
    }
    #[cfg(all(target_os = "windows", feature = "windows"))]
    {
        app.ui.app_edit_state.background = background;
        if inspect_background
            && app
                .request_tuning(
                    crate::app::features::system_tweaks::TuningRequest::InspectBackground,
                )
                .is_err()
        {
            app.ui.app_edit_state.background_error = Some(
                "CPU list is busy or unavailable. Try again after the current operation finishes."
                    .into(),
            );
        }
    }
    if draft_changed {
        app.clear_current_app_shortcut_result();
    }
    if create_shortcut_clicked && !draft_changed {
        let _ = app.create_shortcut_for_current_rule();
    }

    if save_clicked {
        app.commit_current_app_edit_session();
    } else if delete_clicked {
        app.delete_current_app_edit_target();
    } else if is_close {
        app.close_app_run_settings();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        binary_path_field, shortcut_button_enabled_for_current_frame,
        shortcut_message_for_current_frame, wrapped_identifier_layout_job,
    };
    use eframe::egui::Color32;

    #[test]
    fn test_shortcut_status_for_current_frame_disables_same_frame_dirty_edit() {
        assert!(!shortcut_button_enabled_for_current_frame(true, true));
        assert_eq!(
            shortcut_message_for_current_frame(None, true, true),
            Some("Save changes first.")
        );
    }

    #[test]
    fn test_shortcut_status_for_current_frame_keeps_hidden_status_hidden() {
        assert!(!shortcut_button_enabled_for_current_frame(false, true));
        assert_eq!(shortcut_message_for_current_frame(None, false, true), None);
    }

    #[test]
    fn long_aumid_layout_wraps_inside_its_value_column() {
        let job = wrapped_identifier_layout_job("Publisher.App_123!Main", 240.0, Color32::WHITE);
        assert_eq!(job.wrap.max_width, 240.0);
        assert!(job.wrap.break_anywhere);
    }

    #[test]
    fn long_binary_path_is_fully_readable_without_changing_the_editable_value() {
        for width in [220.0, 356.0] {
            let ctx = eframe::egui::Context::default();
            let original = format!(
                "C:\\Program Files\\{}\\rustdesk.exe",
                "LongDirectoryWithoutSpaces".repeat(5)
            );
            let mut path = original.clone();
            let mut field = eframe::egui::Rect::NOTHING;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                ui.set_width(width);
                field = binary_path_field(ui, &mut path).rect;
            });
            output.textures_delta.clear();
            assert!((field.width() - width).abs() < 1.0);
            assert_eq!(path, original);
            let preview = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    eframe::egui::Shape::Text(text)
                        if text.galley.text() == original && text.galley.rows.len() > 1 =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .expect("a long executable path must be visible in full, with soft wrapping");
            assert!(preview.galley.job.wrap.break_anywhere);
            let bounds = preview.galley.rect.translate(preview.pos.to_vec2());
            assert!(bounds.right() <= field.right() + 1.0);
        }
    }
}
