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

fn stage_process_owner(
    choices: &mut Vec<crate::app::features::execution::control::policy::Choice>,
    image: &str,
    owner: crate::app::shared::ids::RuleId,
) {
    let image = crate::app::features::execution::control::policy::image_key(image);
    choices.retain(|choice| {
        crate::app::features::execution::control::policy::image_key(&choice.image) != image
            || choice.owner != owner
    });
    choices.push(crate::app::features::execution::control::policy::Choice { image, owner });
}

fn process_control_editor(
    ui: &mut egui::Ui,
    view: &crate::app::features::execution::control::View,
    rule: &crate::app::shared::ids::RuleId,
    draft: &crate::app::models::AppToRun,
    choices: &mut Vec<crate::app::features::execution::control::policy::Choice>,
) -> bool {
    let mut changed = false;
    if let Some(error) = &view.error {
        ui.add(
            Label::new(
                RichText::new(error)
                    .color(crate::app::shell::presenters::shared_elements::warning_color(ui)),
            )
            .wrap(),
        );
    }
    let duplicates = view.duplicate_labels(rule, draft);
    if !duplicates.is_empty() {
        ui.add(
            Label::new(
                RichText::new(format!(
                    "Process names also listed in: {}",
                    duplicates.join(", ")
                ))
                .small()
                .weak(),
            )
            .wrap(),
        );
        ui.add(Label::new(RichText::new("A shared name alone does not mean the same process. CPU and RAM can appear in both apps; only one rule controls a shared process.").small().weak()).wrap());
    }
    for notice in view
        .notices
        .iter()
        .filter(|notice| notice.different && notice.claimants.contains(rule))
    {
        ui.add_space(5.0);
        crate::app::shell::presenters::shared_elements::inset_frame(ui).show(ui,|ui| {
            ui.label(RichText::new(format!("Shared process: {}",notice.name)).strong()).on_hover_text(&notice.image);
            let mut owner=choices.iter().rev().find(|choice|crate::app::features::execution::control::policy::image_key(&choice.image)==notice.image&&notice.claimants.contains(&choice.owner)).map(|choice|choice.owner.clone()).or_else(||notice.owner.clone());
            let before=owner.clone();
            if let Some(owner_id)=&owner {
                ui.add(Label::new(RichText::new(format!("Managed by {}",view.label(owner_id))).small()).wrap());
                if owner_id!=rule&&ui.button("Take control here").on_hover_text("Use this app's affinity and priority for this shared executable. Process lists and CPU/RAM tracking stay unchanged. Saved with Save Changes.").clicked(){owner=Some(rule.clone());}
            }else{
                ui.add(Label::new(RichText::new("These existing rules request different settings. Choose one app to control this process.").small().color(crate::app::shell::presenters::shared_elements::warning_color(ui))).wrap());
                ComboBox::from_id_salt(("process-owner",&notice.image,&notice.claimants)).selected_text("Choose controlling app…").width(ui.available_width().min(360.0)).show_ui(ui,|ui|{for id in &notice.claimants{ui.selectable_value(&mut owner,Some(id.clone()),view.label(id));}});
            }
            if owner!=before {if let Some(owner)=owner {
                stage_process_owner(choices,&notice.image,owner);changed=true;
            }}
        });
    }
    if !choices.is_empty() {
        ui.label(
            RichText::new("Control changes will be applied when you save.")
                .small()
                .weak(),
        );
    }
    changed
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

    let control_view = app.runtime.control_handle().view();
    let control_rule = app
        .ui
        .app_edit_state
        .target
        .as_ref()
        .unwrap()
        .rule_id
        .clone();
    let steam_issue = app.runtime.steam_detection_issue(&control_rule);
    let mut control_choices = app.ui.app_edit_state.control_choices.clone();
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
                let close = close_button(ui).on_hover_text("Close");
                crate::debug_mcp_control!("rule.form.close", close);
                if close.clicked() {
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

                if selected_app.steam_shortcut_id().is_some() {
                    ui.label(RichText::new("This shortcut starts a Steam game. Switch tracking from Steam to the game itself.").weak());
                    if ui.button("Use Steam game tracking").clicked() { draft_changed |= selected_app.convert_steam_shortcut(); }
                }
                if let crate::app::models::LaunchTarget::Steam { app_id, executable } = &mut selected_app.launch_target {
                    ui.label(RichText::new(format!("Steam game · App ID {app_id}")).strong());
                    ui.label(RichText::new("Launches through Steam. CPU, RAM, affinity and Focus follow the game, not the Steam client.").small().weak());
                    if let Some(issue) = &steam_issue {
                        let warning = ui.add(egui::Label::new(RichText::new(issue.reason()).color(crate::app::shell::presenters::shared_elements::warning_color(ui))).wrap());
                        crate::debug_mcp_control!("steam.detection_reason", warning);
                    }
                    draft_changed |= app.ui.app_edit_state.steam_picker.show(ui, executable);
                    ui.add_space(6.0);
                }

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
                        draft_changed |= crate::app::features::system_tweaks::background_ui::draw_rule(ui, draft, supported, false);
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
                    ui.label(RichText::new(if selected_app.steam_game().is_some() {"Optional helper process names"} else {"Tracked process names"}).strong());
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
                    } else if selected_app.steam_game().is_some() {
                        "The main game process is tracked separately. Optional names add helpers inside this game's folder."
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

                draft_changed |= process_control_editor(ui,&control_view,&control_rule,selected_app,&mut control_choices);

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
                    let save = toned_button(
                        ui,
                        egui::Button::new(RichText::new("Save Changes").strong())
                            .min_size(egui::vec2(105.0, 28.0)),
                        ToneRole::Primary,
                    );
                    crate::debug_mcp_control!("rule.form.save", save);
                    if save.clicked() {
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
    }
    app.ui.app_edit_state.control_choices = control_choices;
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
    #[test]
    fn s110_control_warning_and_take_control_fit_at_100_percent() {
        use crate::app::features::execution::control::{
            policy::{Choice, Policy},
            tests::{fixture, observe},
            Authority,
        };
        let mut state = fixture();
        let control = Authority::default();
        control.configure(&state);
        observe(&control, &state, &[0, 1], 1, r"C:\Shared\helper.exe");
        let owner = control.view().rules[0].rule.clone();
        Policy::choose(
            &mut state,
            Choice {
                image: r"C:\Shared\helper.exe".into(),
                owner,
            },
        )
        .unwrap();
        control.configure(&state);
        observe(&control, &state, &[0, 1], 1, r"C:\Shared\helper.exe");
        let view = control.view();
        let rule = view.rules[1].rule.clone();
        let ctx = eframe::egui::Context::default();
        let mut choices = vec![];
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.set_width(380.0);
            super::process_control_editor(
                ui,
                &view,
                &rule,
                &state.groups[1].programs[0],
                &mut choices,
            );
        });
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                eframe::egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(labels.iter().any(|text| text.contains("Managed by")));
        assert!(labels.contains(&"Take control here"));
        assert!(
            choices.is_empty(),
            "rendering alone cannot change ownership"
        );
        for shape in &output.shapes {
            if let eframe::egui::Shape::Text(text) = &shape.shape {
                assert!(
                    text.galley.rect.translate(text.pos.to_vec2()).right() <= 400.0,
                    "{}",
                    text.galley.text()
                );
            }
        }
    }
    #[test]
    fn s112_editor_preserves_preferences_for_overlapping_claimant_sets() {
        use crate::app::shared::ids::RuleId;
        let (b, c) = (RuleId("b".into()), RuleId("c".into()));
        let mut staged = vec![];
        super::stage_process_owner(&mut staged, "helper.exe", b.clone());
        super::stage_process_owner(&mut staged, "helper.exe", c.clone());
        assert_eq!(
            staged
                .iter()
                .map(|choice| choice.owner.clone())
                .collect::<Vec<_>>(),
            [b, c]
        );
    }
}
