use crate::app::runtime::AppState;
use crate::app::shell::presenters::shared_elements::{
    danger_color, glass_frame, neutral_emphasis_fill, success_color,
};
use crate::app::shell::sessions::RuleShortcutResult;
use crate::app::shell::{GroupRoute, WindowRoute};
use eframe::egui::{self, Align, CentralPanel, ComboBox, Layout, RichText, Vec2};
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

pub fn draw_app_run_settings(app: &mut AppState, root_ui: &mut egui::Ui) {
    if app.ui.app_edit_state.target.is_none() {
        app.set_current_window(WindowRoute::Groups(GroupRoute::List));
        return;
    }

    if !app.ensure_current_edit_loaded() {
        return;
    }

    let mut is_close = false;
    let mut save_clicked = false;
    let mut delete_clicked = false;
    let mut create_shortcut_clicked = false;
    let mut draft_changed = false;
    let shortcut_status = app.current_app_edit_shortcut_status();
    let shortcut_result = app.ui.app_edit_state.shortcut_result.clone();

    CentralPanel::default().show(root_ui, |ui| {
        ui.add_space(5.0);
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
                if ui.button("Close").on_hover_text("Close").clicked() {
                    is_close = true;
                }
            });
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            glass_frame(ui).show(ui, |ui| {
                let selected_app = app
                    .ui
                    .app_edit_state
                    .current_edit
                    .as_mut()
                    .expect("edit_app_clone must be initialized");

                egui::Grid::new("app_settings_grid")
                    .spacing(Vec2::new(10.0, 10.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new("App Name:").strong());
                        if ui.text_edit_singleline(&mut selected_app.name).changed() {
                            draft_changed = true;
                        }
                        ui.end_row();

                        if selected_app.is_path_target() {
                            ui.label(RichText::new("Binary Path:").strong());
                            ui.horizontal(|ui| {
                                let mut bin_path_str = selected_app
                                    .bin_path()
                                    .map(|path| path.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                if ui.text_edit_singleline(&mut bin_path_str).changed() {
                                    if let Some(bin_path) = selected_app.bin_path_mut() {
                                        *bin_path = PathBuf::from(bin_path_str);
                                        draft_changed = true;
                                    }
                                }

                                if ui
                                    .button("Browse")
                                    .on_hover_text(browse_binary_hover_text())
                                    .clicked()
                                {
                                    if let Some(path) = pick_binary_path() {
                                        if let Some(bin_path) = selected_app.bin_path_mut() {
                                            *bin_path = path;
                                            draft_changed = true;
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                        } else {
                            ui.label(RichText::new("Installed App:").strong());
                            ui.label(&selected_app.name);
                            ui.end_row();

                            ui.label(RichText::new("AUMID:").strong());
                            ui.label(
                                RichText::new(
                                    selected_app
                                        .installed_aumid()
                                        .unwrap_or("Unknown installed app"),
                                )
                                .small()
                                .monospace(),
                            );
                            ui.end_row();
                        }

                        ui.label(RichText::new("Priority:").strong());
                        ComboBox::from_id_salt("priority_combo")
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
                        ui.end_row();
                    });

                ui.add_space(10.0);
                if ui
                    .checkbox(
                        &mut selected_app.autorun,
                        RichText::new("Start this app on startup").strong(),
                    )
                    .changed()
                {
                    draft_changed = true;
                }
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                if selected_app.is_args_editable() {
                    ui.label(RichText::new("Command Line Arguments:").strong());
                    ui.add_space(5.0);

                    let mut arg_to_remove = None;
                    if selected_app.args.is_empty() {
                        ui.label(RichText::new("No arguments defined.").weak().italics());
                    } else {
                        for (i, arg) in selected_app.args.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}:", i + 1));
                                if ui.text_edit_singleline(arg).changed() {
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

                    ui.add_space(5.0);
                    if ui.button("Add Argument").clicked() {
                        selected_app.args.push(String::new());
                        draft_changed = true;
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                ui.label(RichText::new("Tracked Process Names:").strong());
                ui.add_space(5.0);

                let mut proc_to_remove = None;
                if selected_app.additional_processes.is_empty() {
                    let empty_text = if selected_app.is_path_target() {
                        "Process-name rediscovery is off for this path app. Add a name to let Auto Re-apply find it later."
                    } else {
                        "No tracked process names defined."
                    };
                    ui.label(RichText::new(empty_text).weak().italics());
                } else {
                    for (i, proc_name) in selected_app.additional_processes.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}:", i + 1));
                            if ui.text_edit_singleline(proc_name).changed() {
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

                ui.add_space(5.0);
                if ui.button("Add Process Name").clicked() {
                    selected_app.additional_processes.push(String::new());
                    draft_changed = true;
                }

                ui.add_space(15.0);
                ui.separator();
                ui.add_space(10.0);

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
                                    .min_size(egui::vec2(170.0, 32.0)),
                            )
                            .clicked()
                        {
                            create_shortcut_clicked = true;
                        }
                        if let Some(message) = shortcut_message {
                            ui.label(RichText::new(message).weak());
                        }
                    });
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
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Save Changes").strong())
                                .fill(neutral_emphasis_fill(ui))
                                .min_size(egui::vec2(120.0, 34.0)),
                        )
                        .clicked()
                    {
                        save_clicked = true;
                        is_close = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(100.0, 32.0)))
                        .clicked()
                    {
                        is_close = true;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Remove from group").color(danger_color(ui)),
                                )
                                .min_size(egui::vec2(150.0, 32.0)),
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
    use super::{shortcut_button_enabled_for_current_frame, shortcut_message_for_current_frame};

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
}
