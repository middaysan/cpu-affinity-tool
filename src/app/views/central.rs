use crate::app::features::execution::AppRowAction;
use crate::app::models::{AppRuntimeKey, AppStatus};
use crate::app::runtime::{AppState, CentralPanelSnapshot};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::presenters::shared_elements::{
    app_run_button, content_frame, draggable_name, ghost_button, group_frame, inset_frame,
    inter_medium_family, inter_semibold_family, palette, row_fill, success_color,
    toned_sized_button, warning_color, ToneRole, BUTTON_FONT_SIZE,
};
use eframe::egui::{self, Align, CentralPanel, Color32, Layout, RichText, ScrollArea, Vec2};
use std::path::PathBuf;

const ICON_EDIT: &str = "\u{2699}";

enum CentralAction {
    MoveGroupToIndex {
        group_id: GroupId,
        target_index: usize,
    },
    StartEditGroup(GroupId),
    ToggleGroupHidden {
        group_id: GroupId,
        is_hidden: bool,
    },
    AddSelectedFiles {
        group_id: GroupId,
        paths: Vec<PathBuf>,
    },
    OpenInstalledAppPicker(GroupId),
    OpenAppRunSettings {
        group_id: GroupId,
        rule_id: RuleId,
    },
    RunGroup(GroupId),
    RunGroupProgramAction {
        group_id: GroupId,
        rule_id: RuleId,
        action: AppRowAction,
    },
    MoveRuleToGroup {
        source_group_id: GroupId,
        rule_id: RuleId,
        target_group_id: GroupId,
        target_rule_index: usize,
    },
    LogMessage(String),
    ConsumeDroppedFiles(GroupId),
    DiscardDroppedFiles,
}

#[derive(Clone)]
struct RuleDragPayload {
    source_group_id: GroupId,
    rule_id: RuleId,
    app_key: AppRuntimeKey,
    preview_label: String,
    preview_width: f32,
}

#[derive(Clone)]
struct GroupDragPayload {
    group_id: GroupId,
    preview_label: String,
    preview_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleDropClassification {
    Valid,
    SamePosition,
    DuplicateInTarget,
    StalePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppStatusTone {
    Neutral,
    Success,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppActionPresentation {
    label: &'static str,
    hover_text: &'static str,
    action: Option<AppRowAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupMoveDirection {
    Up,
    Down,
}

pub fn draw_central_panel(app: &mut AppState, root_ui: &mut egui::Ui) {
    let ctx = root_ui.ctx().clone();
    let snapshot = app.build_central_panel_snapshot();
    let group_count = snapshot.groups.len();
    CentralPanel::default()
        .frame(content_frame(root_ui))
        .show(root_ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let description_width = (ui.available_width() - 105.0 - 8.0).max(40.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(description_width, 25.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new("CPU placement and application rules")
                                    .small()
                                    .weak(),
                            )
                            .truncate(),
                        )
                        .on_hover_text("CPU placement and application rules");
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{group_count} groups · drag to reorder"))
                                    .small()
                                    .weak(),
                            )
                            .truncate(),
                        );
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let new_group = toned_sized_button(
                        ui,
                        [105.0, 25.0],
                        egui::Button::new(
                            RichText::new("+ New group").size(BUTTON_FONT_SIZE).strong(),
                        ),
                        ToneRole::Primary,
                    );
                    crate::debug_mcp_control!("group.new", new_group);
                    if new_group.clicked() {
                        app.start_creating_group();
                    }
                });
            });
            ui.add_space(5.0);
            if let Some(error)=app.ui.file_import_error {
                inset_frame(ui).show(ui,|ui|{
                    let reason=match error {
                        crate::app::features::system_tweaks::SaveError::Storage=>"Could not write the settings file. Your saved apps are unchanged.",
                        crate::app::features::system_tweaks::SaveError::Invalid=>"Settings changed or this import was refused. Your saved apps are unchanged.",
                        crate::app::features::system_tweaks::SaveError::Pending=>"Another save is still running. Wait, then retry.",
                        crate::app::features::system_tweaks::SaveError::Recovering=>"Recovery needs attention. Check System Tweaks before retrying.",
                    };
                    ui.add(egui::Label::new(RichText::new(reason).small().color(warning_color(ui))).wrap());
                    ui.horizontal(|ui|{
                        if ui.button("Retry").clicked() {if let Some(group)=app.ui.file_drop_hover_target.clone(){app.consume_dropped_files_into_group(group);}}
                        if ui.button("Dismiss").clicked() {app.ui.dropped_files=None;app.ui.file_drop_position=None;app.ui.file_drop_hover_target=None;app.ui.file_import_error=None;}
                    });
                });
                ui.add_space(5.0);
            }
            ScrollArea::vertical()
                .id_salt("affinity-groups-scroll")
                .max_height(ui.available_height())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        let actions = render_groups(app, ui, &ctx, &snapshot);
                        execute_actions(app, actions);
                    });
                });
        });
    render_rule_drag_preview(&ctx);
    render_group_drag_preview(&ctx);
}

#[cfg(target_os = "windows")]
fn pick_open_app_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new()
        .add_filter("Apps", &["exe", "lnk", "url"])
        .pick_files()
}

#[cfg(not(target_os = "windows"))]
fn pick_open_app_files() -> Option<Vec<PathBuf>> {
    rfd::FileDialog::new().pick_files()
}

#[cfg(target_os = "windows")]
fn open_app_hover_text() -> &'static str {
    "Add executables, shortcuts, or URLs"
}

#[cfg(not(target_os = "windows"))]
fn open_app_hover_text() -> &'static str {
    "Add binaries, scripts, or .desktop launchers"
}

#[cfg(target_os = "windows")]
fn installed_app_hover_text() -> &'static str {
    "Find installed Start-backed apps"
}

#[cfg(target_os = "linux")]
fn installed_app_hover_text() -> &'static str {
    "Find apps from desktop entries and matching PATH executables"
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn installed_app_hover_text() -> &'static str {
    "Find supported installed apps"
}

fn format_core_summary(cores: &[usize]) -> String {
    if cores.is_empty() {
        return "No CPU threads assigned".to_string();
    }

    let visible = cores
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{} threads · {visible}", cores.len())
}

fn app_status_label(status: AppStatus) -> &'static str {
    match status {
        AppStatus::Running => "Running · protected",
        AppStatus::SettingsMismatch => "Running · correction needed",
        AppStatus::NotRunning => "Stopped",
    }
}

fn app_status_tone(status: AppStatus) -> AppStatusTone {
    match status {
        AppStatus::NotRunning => AppStatusTone::Neutral,
        AppStatus::Running => AppStatusTone::Success,
        AppStatus::SettingsMismatch => AppStatusTone::Warning,
    }
}

fn app_status_display_label(status: AppStatus, available_width: f32) -> &'static str {
    match status {
        AppStatus::Running if available_width < 112.0 => "Protected",
        AppStatus::SettingsMismatch if available_width < 132.0 => "Needs correction",
        _ => app_status_label(status),
    }
}

fn app_action_presentation(status: AppStatus, focus_supported: bool) -> AppActionPresentation {
    match status {
        AppStatus::NotRunning => AppActionPresentation {
            label: "Run",
            hover_text: "Start this application with the assigned settings",
            action: Some(AppRowAction::Run),
        },
        AppStatus::SettingsMismatch => AppActionPresentation {
            label: "Fix",
            hover_text: "Reapply the assigned affinity and priority",
            action: Some(AppRowAction::Fix),
        },
        AppStatus::Running if focus_supported => AppActionPresentation {
            label: "Focus",
            hover_text: "Focus the existing application window",
            action: Some(AppRowAction::Focus),
        },
        AppStatus::Running => AppActionPresentation {
            label: "Active",
            hover_text: "Window focus is unavailable on this platform",
            action: None,
        },
    }
}

fn platform_supports_app_focus() -> bool {
    cfg!(feature = "windows")
}

fn adjacent_group_target(
    current_index: usize,
    groups_len: usize,
    direction: GroupMoveDirection,
) -> Option<usize> {
    if current_index >= groups_len {
        return None;
    }

    match direction {
        GroupMoveDirection::Up => current_index.checked_sub(1),
        GroupMoveDirection::Down => current_index
            .checked_add(1)
            .filter(|target| *target < groups_len),
    }
}

fn resolve_group_drop_index(
    source_index: usize,
    insertion_slot: usize,
    groups_len: usize,
) -> Option<usize> {
    if source_index >= groups_len || insertion_slot > groups_len {
        return None;
    }

    let target_index = if insertion_slot > source_index {
        insertion_slot - 1
    } else {
        insertion_slot
    };
    (target_index < groups_len && target_index != source_index).then_some(target_index)
}

fn render_groups(
    app: &mut AppState,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    snapshot: &CentralPanelSnapshot,
) -> Vec<CentralAction> {
    let control = app.runtime.control_handle().view();
    let mut actions = Vec::new();
    let mut hovered_file_drop_group = None;
    let active_group_payload = egui::DragAndDrop::payload::<GroupDragPayload>(ctx);
    let group_drag_pos = if active_group_payload.is_some() {
        ctx.pointer_interact_pos()
    } else {
        None
    };
    let group_drop_pos =
        if active_group_payload.is_some() && ctx.input(|input| input.pointer.any_released()) {
            ctx.pointer_interact_pos()
        } else {
            None
        };
    let group_pointer_pos = group_drop_pos.or(group_drag_pos);
    let mut group_drop_target_claimed = false;
    let active_rule_payload = egui::DragAndDrop::payload::<RuleDragPayload>(ctx);
    let rule_drag_pos = if active_rule_payload.is_some() {
        ctx.pointer_interact_pos()
    } else {
        None
    };
    let rule_drop_pos = if ctx.input(|i| i.pointer.any_released()) {
        ctx.pointer_interact_pos()
    } else {
        None
    };
    let rule_pointer_pos = rule_drop_pos.or(rule_drag_pos);
    let has_hovered_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
    let has_dropped_files = app
        .ui
        .dropped_files
        .as_ref()
        .is_some_and(|files| !files.is_empty());
    let file_drop_pos = if has_dropped_files {
        app.ui
            .file_drop_position
            .or_else(|| ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos())))
    } else {
        ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()))
    };
    let groups_len = snapshot.groups.len();

    for (group_index, group) in snapshot.groups.iter().enumerate() {
        let group_id = group.group_id.clone();
        let mut target_rule_index = None;
        let mut drop_indicator = None;
        let mut rejected_rule_drop = None;

        let group_response = group_frame(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let colors = palette(ui);

            ui.horizontal(|ui| {
                #[cfg(all(target_os = "windows", feature = "windows"))]
                let meter_width = if app.cpu_load_enabled() {
                    78.0 + ui.spacing().item_spacing.x
                } else {
                    0.0
                };
                #[cfg(not(all(target_os = "windows", feature = "windows")))]
                let meter_width = 0.0;
                let edit_text = format!("{ICON_EDIT} Edit");
                let edit_width = (ui
                    .painter()
                    .layout_no_wrap(
                        edit_text.clone(),
                        egui::FontId::proportional(BUTTON_FONT_SIZE),
                        colors.text_secondary,
                    )
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x)
                    .max(ui.spacing().interact_size.x);
                let title_width =
                    (ui.available_width() - edit_width - ui.spacing().item_spacing.x - meter_width)
                        .max(40.0);
                let core_summary = format_core_summary(&group.cores);
                let title = ui.allocate_ui_with_layout(
                    egui::vec2(title_width, 32.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        let name = draggable_name(
                            ui,
                            egui::Id::new(("central-group-drag", group_id.0.as_str())),
                            RichText::new(&group.name)
                                .size(14.0)
                                .family(inter_semibold_family())
                                .strong(),
                            egui::vec2(title_width, 18.0),
                        )
                        .on_hover_text(format!(
                            "{}\nDrag to reorder group; when focused, use Up or Down",
                            group.name
                        ));
                        ui.add(
                            egui::Label::new(
                                RichText::new(&core_summary)
                                    .size(9.0)
                                    .color(colors.text_muted),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&core_summary);
                        name
                    },
                );
                let title_response = title.inner;
                let group_drag_payload = GroupDragPayload {
                    group_id: group_id.clone(),
                    preview_label: group.name.clone(),
                    preview_width: title_response.rect.width().max(180.0),
                };
                title_response.dnd_set_drag_payload(group_drag_payload);

                if title_response.has_focus() {
                    let move_direction = ui.input_mut(|input| {
                        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                            Some(GroupMoveDirection::Up)
                        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                            Some(GroupMoveDirection::Down)
                        } else {
                            None
                        }
                    });
                    if let Some(target_index) = move_direction.and_then(|direction| {
                        adjacent_group_target(group_index, groups_len, direction)
                    }) {
                        actions.push(CentralAction::MoveGroupToIndex {
                            group_id: group_id.clone(),
                            target_index,
                        });
                    }
                }

                #[cfg(all(target_os = "windows", feature = "windows"))]
                crate::app::features::cpu_load::view::group_indicator(
                    app,
                    ui,
                    &group_id,
                    &group.name,
                    &group.cores,
                );

                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    let edit = ghost_button(
                        ui,
                        egui::Button::new(RichText::new(edit_text).size(BUTTON_FONT_SIZE)),
                    )
                    .on_hover_text("Edit group settings");
                    crate::debug_mcp_control!(format!("group.edit.{}", group_id.0), edit);
                    if edit.clicked() {
                        actions.push(CentralAction::StartEditGroup(group_id.clone()));
                    }
                });
            });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if group.run_all_button
                    && ui
                        .button(
                            RichText::new("\u{25B6} Run all")
                                .size(BUTTON_FONT_SIZE)
                                .strong(),
                        )
                        .on_hover_text("Run all apps in group")
                        .clicked()
                {
                    actions.push(CentralAction::RunGroup(group_id.clone()));
                }

                if crate::app::adapters::discovery::supports_installed_app_picker()
                    && ui
                        .button(RichText::new("⊞  Add installed…").size(BUTTON_FONT_SIZE))
                        .on_hover_text(installed_app_hover_text())
                        .clicked()
                {
                    actions.push(CentralAction::OpenInstalledAppPicker(group_id.clone()));
                }

                if ui
                    .button(RichText::new("+  Add file…").size(BUTTON_FONT_SIZE))
                    .on_hover_text(open_app_hover_text())
                    .clicked()
                {
                    if let Some(paths) = pick_open_app_files() {
                        actions.push(CentralAction::AddSelectedFiles {
                            group_id: group_id.clone(),
                            paths,
                        });
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let visibility_label = if group.is_hidden {
                        "▶ Show list"
                    } else {
                        "▼ Hide list"
                    };
                    if ghost_button(
                        ui,
                        egui::Button::new(RichText::new(visibility_label).size(BUTTON_FONT_SIZE)),
                    )
                    .clicked()
                    {
                        actions.push(CentralAction::ToggleGroupHidden {
                            group_id: group_id.clone(),
                            is_hidden: !group.is_hidden,
                        });
                    }
                });
            });

            ui.add_space(4.0);

            if !group.is_hidden && group.programs.is_empty() {
                inset_frame(ui).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Drop an executable inside this group").strong());
                        ui.label(
                            RichText::new("or use Add file / Add installed")
                                .small()
                                .weak(),
                        );
                    });
                });
            } else if !group.is_hidden {
                for (program_index, program) in group.programs.iter().enumerate() {
                    let app_status = app.get_app_status_sync(&program.runtime_key);
                    let choose_owner=control.needs_choice(&program.rule_id);
                    let shared_control=control.shared_elsewhere(&program.rule_id);

                    let row_response = egui::Frame::NONE
                        .fill(row_fill(ui))
                        .corner_radius(5.0)
                        .inner_margin(egui::Margin::symmetric(4, 1))
                        .show(ui, |ui| {
                            let color = match app_status_tone(app_status) {
                                AppStatusTone::Neutral => palette(ui).neutral_status,
                                AppStatusTone::Success => success_color(ui),
                                AppStatusTone::Warning => warning_color(ui),
                            };
                            let starting = app_status == AppStatus::NotRunning && crate::app::features::execution::launch::steam_launch_pending(&program.rule_id,program.steam_app_id);
                            let needs_selection=app.runtime.steam_needs_selection(&program.rule_id);
                            let full_status = if choose_owner {"Choose process owner"} else if shared_control {"Shared process managed by another app"} else if needs_selection {"Choose game process"} else if starting { "Starting game..." } else { app_status_label(app_status) };
                            let status_tooltip =
                                app.get_running_app_pids(&program.runtime_key).map_or_else(
                                    || format!("Status: {full_status}"),
                                    |pids| {
                                        format!("Status: {full_status}\nTracking PIDs: {pids:?}")
                                    },
                                );
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                #[cfg(not(all(target_os = "windows", feature = "windows")))]
                                {
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::splat(8.0),
                                        egui::Sense::hover(),
                                    );
                                    response.on_hover_text(status_tooltip.clone());
                                    ui.painter().circle_filled(rect.center(), 3.5, color);
                                }

                                let mut action_presentation = app_action_presentation(
                                    app_status,
                                    platform_supports_app_focus(),
                                );
                                if choose_owner {action_presentation.label="Resolve";action_presentation.hover_text="Choose which app controls the shared process. CPU and RAM tracking stay unchanged.";action_presentation.action=Some(AppRowAction::Focus);}
                                else if shared_control&&app_status!=AppStatus::NotRunning {action_presentation=app_action_presentation(AppStatus::Running,platform_supports_app_focus());}
                                else if needs_selection {action_presentation.label="Select";action_presentation.hover_text="Automatic detection is uncertain. Open app settings and choose the game process.";action_presentation.action=Some(AppRowAction::Focus);}
                                else if starting { action_presentation.label="Wait"; action_presentation.hover_text="Waiting for the Steam game. If detection is unavailable, open its settings and choose the game process."; action_presentation.action=None; }
                                let status_color = match app_status_tone(app_status) {
                                    AppStatusTone::Neutral => colors.text_muted,
                                    AppStatusTone::Success => colors.success.fg,
                                    AppStatusTone::Warning => colors.warning.fg,
                                };
                                #[cfg(all(target_os = "windows", feature = "windows"))]
                                let meter_width = if app.cpu_load_enabled() {
                                    crate::app::features::cpu_load::view::APP_METRICS_WIDTH
                                        + ui.spacing().item_spacing.x
                                } else {
                                    0.0
                                };
                                #[cfg(not(all(target_os = "windows", feature = "windows")))]
                                let meter_width = 0.0;
                                let action_width = 42.0;
                                let settings_width = 23.0;
                                let status_galley = ui.painter().layout_no_wrap(
                                    if choose_owner {"Choose owner".into()} else if shared_control {"Shared control".into()} else if needs_selection {"Select game".into()} else if starting { "Starting...".into() } else { app_status_display_label(app_status, 0.0).to_owned() },
                                    egui::FontId::proportional(10.0),
                                    status_color,
                                );
                                let status_width = status_galley.size().x.ceil() + 2.0;
                                let name_width = (ui.available_width()
                                    - status_width
                                    - action_width
                                    - settings_width
                                    - meter_width
                                    - 16.0)
                                    .max(48.0);
                                let drag_response = draggable_name(
                                    ui,
                                    egui::Id::new((
                                        "central-rule-drag",
                                        group_id.0.as_str(),
                                        program.rule_id.0.as_str(),
                                    )),
                                    RichText::new(&program.name)
                                        .size(BUTTON_FONT_SIZE)
                                        .family(inter_medium_family())
                                        .strong(),
                                    egui::vec2(name_width, 21.0),
                                )
                                .on_hover_text(format!(
                                    "{}\nDrag to move application",
                                    program.launch_target_detail
                                ));

                                ui.add_space(4.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(status_width, 21.0),
                                    Layout::right_to_left(Align::Center),
                                    |ui| {
                                        ui.add(egui::Label::new(status_galley));
                                    },
                                )
                                .response
                                .on_hover_text(&status_tooltip);

                                drag_response.dnd_set_drag_payload(RuleDragPayload {
                                    source_group_id: group_id.clone(),
                                    rule_id: program.rule_id.clone(),
                                    app_key: program.app_key.clone(),
                                    preview_label: program.name.clone(),
                                    preview_width: name_width,
                                });

                                #[cfg(all(target_os = "windows", feature = "windows"))]
                                crate::app::features::cpu_load::view::process_indicator(
                                    app,
                                    ui,
                                    crate::app::features::cpu_load::RuleTarget {
                                        group: group_id.clone(),
                                        rule: program.rule_id.clone(),
                                    },
                                    &program.name,
                                    app_status != AppStatus::NotRunning,
                                    color,
                                    &status_tooltip,
                                );

                                let button = egui::Button::new(
                                    RichText::new(action_presentation.label).size(BUTTON_FONT_SIZE),
                                );
                                let action_response = if action_presentation.action.is_none() {
                                    ui.add_enabled(
                                        false,
                                        button.min_size(egui::vec2(action_width, 21.0)),
                                    )
                                } else if action_presentation.action == Some(AppRowAction::Run) {
                                    app_run_button(ui, [action_width, 21.0], button)
                                } else if app_status == AppStatus::SettingsMismatch {
                                    toned_sized_button(
                                        ui,
                                        [action_width, 21.0],
                                        button,
                                        ToneRole::Warning,
                                    )
                                } else {
                                    ui.add(button.min_size(egui::vec2(action_width, 21.0)))
                                }
                                .on_hover_text(action_presentation.hover_text);
                                if action_response.clicked() {
                                    if choose_owner||needs_selection { actions.push(CentralAction::OpenAppRunSettings{group_id:group_id.clone(),rule_id:program.rule_id.clone()}); }
                                    else if let Some(row_action) = action_presentation.action {
                                        actions.push(CentralAction::RunGroupProgramAction {
                                            group_id: group_id.clone(),
                                            rule_id: program.rule_id.clone(),
                                            action: row_action,
                                        });
                                    }
                                }

                                let edit_response = ghost_button(
                                    ui,
                                    egui::Button::new(
                                        RichText::new(ICON_EDIT).size(BUTTON_FONT_SIZE),
                                    )
                                    .min_size(egui::vec2(settings_width, 21.0)),
                                )
                                .on_hover_text("Edit app settings");
                                crate::debug_mcp_control!(
                                    format!("rule.edit.{}.{}", group_id.0, program.rule_id.0),
                                    edit_response
                                );
                                if edit_response.clicked() {
                                    actions.push(CentralAction::OpenAppRunSettings {
                                        group_id: group_id.clone(),
                                        rule_id: program.rule_id.clone(),
                                    });
                                }
                            });
                        });

                    if target_rule_index.is_none() {
                        if let Some(pos) = rule_pointer_pos {
                            if row_response
                                .response
                                .rect
                                .intersect(ui.clip_rect())
                                .contains(pos)
                            {
                                let insert_index = if pos.y < row_response.response.rect.center().y
                                {
                                    program_index
                                } else {
                                    program_index + 1
                                };
                                target_rule_index = Some(insert_index);
                                if let Some(payload) = active_rule_payload.as_deref() {
                                    match classify_rule_drop(
                                        snapshot,
                                        payload,
                                        &group.group_id,
                                        insert_index,
                                    ) {
                                        RuleDropClassification::Valid => {
                                            let y = if insert_index == program_index {
                                                row_response.response.rect.top()
                                            } else {
                                                row_response.response.rect.bottom()
                                            };
                                            drop_indicator = Some((row_response.response.rect, y));
                                        }
                                        RuleDropClassification::DuplicateInTarget => {
                                            rejected_rule_drop = Some("Already in this group");
                                        }
                                        RuleDropClassification::SamePosition
                                        | RuleDropClassification::StalePayload => {}
                                    }
                                }
                            }
                        }
                    }

                    if group
                        .programs
                        .last()
                        .is_some_and(|last| last.rule_id != program.rule_id)
                    {
                        ui.add_space(0.0);
                    }
                }
            }
        });

        if !group_drop_target_claimed {
            if let (Some(payload), Some(pos)) = (active_group_payload.as_deref(), group_pointer_pos)
            {
                let target_rect = group_response
                    .response
                    .rect
                    .expand2(Vec2::new(0.0, 6.0))
                    .intersect(ui.clip_rect());
                if target_rect.contains(pos) {
                    let insertion_slot = if pos.y < group_response.response.rect.center().y {
                        group_index
                    } else {
                        group_index + 1
                    };
                    if let Some(source_index) = snapshot
                        .groups
                        .iter()
                        .position(|group| group.group_id == payload.group_id)
                    {
                        if let Some(target_index) =
                            resolve_group_drop_index(source_index, insertion_slot, groups_len)
                        {
                            let y = if insertion_slot <= group_index {
                                group_response.response.rect.top()
                            } else {
                                group_response.response.rect.bottom()
                            };
                            paint_rule_drop_indicator(ui, group_response.response.rect, y);
                            group_drop_target_claimed = true;

                            if group_drop_pos.is_some()
                                && egui::DragAndDrop::take_payload::<GroupDragPayload>(ctx)
                                    .is_some()
                            {
                                actions.push(CentralAction::MoveGroupToIndex {
                                    group_id: payload.group_id.clone(),
                                    target_index,
                                });
                            }
                        }
                    }
                }
            }
        }

        if (has_hovered_files || has_dropped_files) && hovered_file_drop_group.is_none() {
            if let Some(pos) = file_drop_pos {
                if group_response
                    .response
                    .rect
                    .intersect(ui.clip_rect())
                    .contains(pos)
                {
                    hovered_file_drop_group = Some(group_id.clone());
                    if has_hovered_files {
                        paint_group_file_drop_target(ui, group_response.response.rect);
                    }
                }
            }
        }

        if drop_indicator.is_none() && target_rule_index.is_none() {
            if let Some(pos) = rule_drag_pos {
                let append_index = group.programs.len();
                if group_response
                    .response
                    .rect
                    .intersect(ui.clip_rect())
                    .contains(pos)
                {
                    if let Some(payload) = active_rule_payload.as_deref() {
                        match classify_rule_drop(snapshot, payload, &group.group_id, append_index) {
                            RuleDropClassification::Valid => {
                                let rect =
                                    group_response.response.rect.shrink2(Vec2::new(14.0, 8.0));
                                drop_indicator = Some((rect, rect.bottom()));
                                target_rule_index.get_or_insert(append_index);
                            }
                            RuleDropClassification::DuplicateInTarget => {
                                rejected_rule_drop = Some("Already in this group");
                            }
                            RuleDropClassification::SamePosition
                            | RuleDropClassification::StalePayload => {}
                        }
                    }
                }
            }
        }
        if let Some((rect, y)) = drop_indicator {
            paint_rule_drop_indicator(ui, rect, y);
        }
        if let Some(message) = rejected_rule_drop {
            paint_rule_drop_rejection(ctx, message);
        }

        let dropped_rule_here = rule_drop_pos.is_some_and(|pos| {
            group_response
                .response
                .rect
                .intersect(ui.clip_rect())
                .contains(pos)
        });
        let dropped_rule = if dropped_rule_here {
            egui::DragAndDrop::take_payload::<RuleDragPayload>(ctx)
        } else {
            None
        };
        if let Some(payload) = dropped_rule {
            let target_rule_index = target_rule_index.unwrap_or(group.programs.len());
            match classify_rule_drop(snapshot, &payload, &group_id, target_rule_index) {
                RuleDropClassification::Valid => {
                    actions.push(CentralAction::MoveRuleToGroup {
                        source_group_id: payload.source_group_id.clone(),
                        rule_id: payload.rule_id.clone(),
                        target_group_id: group_id,
                        target_rule_index,
                    });
                }
                RuleDropClassification::DuplicateInTarget => {
                    actions.push(CentralAction::LogMessage(format!(
                        "Cannot move app '{}': target group '{}' already contains the same launch rule",
                        payload.preview_label, group.name
                    )));
                }
                RuleDropClassification::SamePosition | RuleDropClassification::StalePayload => {}
            }
        }
    }

    if app.ui.file_import_error.is_some() {
        return actions;
    }
    if let Some(group_id) = resolve_file_drop_target(
        &mut app.ui.file_drop_hover_target,
        has_hovered_files,
        has_dropped_files,
        file_drop_pos.is_some(),
        hovered_file_drop_group,
    ) {
        actions.push(CentralAction::ConsumeDroppedFiles(group_id));
    } else if has_dropped_files && app.ui.file_drop_position.is_some() {
        actions.push(CentralAction::DiscardDroppedFiles);
    }

    actions
}

fn resolve_file_drop_target(
    cached_target: &mut Option<GroupId>,
    has_hovered_files: bool,
    has_dropped_files: bool,
    pointer_pos_known: bool,
    hovered_group: Option<GroupId>,
) -> Option<GroupId> {
    if has_dropped_files {
        let target = hovered_group.or_else(|| {
            if pointer_pos_known {
                None
            } else {
                cached_target.clone()
            }
        });
        *cached_target = target.clone();
        return target;
    }

    if has_hovered_files {
        if pointer_pos_known {
            *cached_target = hovered_group;
        }
        return None;
    }

    *cached_target = None;
    None
}

fn classify_rule_drop(
    snapshot: &CentralPanelSnapshot,
    payload: &RuleDragPayload,
    target_group_id: &GroupId,
    target_rule_index: usize,
) -> RuleDropClassification {
    let Some(source_group) = snapshot
        .groups
        .iter()
        .find(|group| group.group_id == payload.source_group_id)
    else {
        return RuleDropClassification::StalePayload;
    };
    let Some((source_rule_index, source_program)) = source_group
        .programs
        .iter()
        .enumerate()
        .find(|(_, program)| program.rule_id == payload.rule_id)
    else {
        return RuleDropClassification::StalePayload;
    };
    if source_program.app_key != payload.app_key {
        return RuleDropClassification::StalePayload;
    }

    let Some(target_group) = snapshot
        .groups
        .iter()
        .find(|group| &group.group_id == target_group_id)
    else {
        return RuleDropClassification::StalePayload;
    };
    if target_rule_index > target_group.programs.len() {
        return RuleDropClassification::StalePayload;
    }

    if &payload.source_group_id == target_group_id
        && (target_rule_index == source_rule_index || target_rule_index == source_rule_index + 1)
    {
        return RuleDropClassification::SamePosition;
    }

    if &payload.source_group_id != target_group_id
        && target_group
            .programs
            .iter()
            .any(|program| program.app_key == payload.app_key)
    {
        return RuleDropClassification::DuplicateInTarget;
    }

    RuleDropClassification::Valid
}

fn paint_rule_drop_indicator(ui: &egui::Ui, rect: egui::Rect, y: f32) {
    let color = palette(ui).drop_target;
    let glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70);
    let left = rect.left() + 4.0;
    let right = rect.right() - 4.0;

    ui.painter().line_segment(
        [egui::pos2(left, y), egui::pos2(right, y)],
        egui::Stroke::new(6.0, glow),
    );
    ui.painter().line_segment(
        [egui::pos2(left, y), egui::pos2(right, y)],
        egui::Stroke::new(2.0, color),
    );
    ui.painter().circle_filled(egui::pos2(left, y), 3.0, color);
    ui.painter().circle_filled(egui::pos2(right, y), 3.0, color);
}

fn paint_group_file_drop_target(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        8.0,
        egui::Stroke::new(2.0, palette(ui).drop_target),
        egui::StrokeKind::Inside,
    );
}

fn paint_rule_drop_rejection(ctx: &egui::Context, message: &str) {
    ctx.output_mut(|output| output.cursor_icon = egui::CursorIcon::NotAllowed);

    let Some(pointer_pos) = ctx.pointer_interact_pos() else {
        return;
    };

    egui::Area::new(egui::Id::new("central-rule-drop-rejection"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer_pos + Vec2::new(14.0, 14.0))
        .constrain(false)
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(RichText::new(message).small().strong());
        });
}

fn render_rule_drag_preview(ctx: &egui::Context) {
    let Some(payload) = egui::DragAndDrop::payload::<RuleDragPayload>(ctx) else {
        return;
    };
    let Some(pointer_pos) = ctx.pointer_interact_pos() else {
        return;
    };

    let width = payload.preview_width.clamp(140.0, 360.0);
    let size = Vec2::new(width, 20.0);

    egui::Area::new(egui::Id::new("central-rule-drag-preview"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer_pos - size / 2.0)
        .constrain(false)
        .interactable(false)
        .show(ctx, |ui| {
            ui.add_sized(
                [size.x, size.y],
                egui::Button::new(RichText::new(&payload.preview_label).strong()),
            );
        });
}

fn render_group_drag_preview(ctx: &egui::Context) {
    let Some(payload) = egui::DragAndDrop::payload::<GroupDragPayload>(ctx) else {
        return;
    };
    let Some(pointer_pos) = ctx.pointer_interact_pos() else {
        return;
    };

    let width = payload.preview_width.clamp(180.0, 360.0);
    let size = Vec2::new(width, 38.0);

    egui::Area::new(egui::Id::new("central-group-drag-preview"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer_pos - size / 2.0)
        .constrain(false)
        .interactable(false)
        .show(ctx, |ui| {
            ui.add_sized(
                [size.x, size.y],
                egui::Button::new(
                    RichText::new(format!("Move group: {}", payload.preview_label)).strong(),
                ),
            );
        });
}

fn execute_actions(app: &mut AppState, actions: Vec<CentralAction>) {
    for action in actions {
        match action {
            CentralAction::MoveGroupToIndex {
                group_id,
                target_index,
            } => {
                let _ = app.move_group_to_index(group_id, target_index);
            }
            CentralAction::StartEditGroup(group_id) => {
                app.start_editing_group(group_id);
            }
            CentralAction::ToggleGroupHidden {
                group_id,
                is_hidden,
            } => {
                app.set_group_is_hidden(group_id, is_hidden);
            }
            CentralAction::AddSelectedFiles { group_id, paths } => {
                app.add_selected_files_to_group(group_id, paths);
            }
            CentralAction::OpenInstalledAppPicker(group_id) => {
                app.open_installed_app_picker(group_id);
            }
            CentralAction::OpenAppRunSettings { group_id, rule_id } => {
                app.open_app_run_settings(group_id, rule_id);
            }
            CentralAction::RunGroup(group_id) => {
                app.run_group(group_id);
            }
            CentralAction::RunGroupProgramAction {
                group_id,
                rule_id,
                action,
            } => {
                app.run_group_program_action(group_id, rule_id, action);
            }
            CentralAction::MoveRuleToGroup {
                source_group_id,
                rule_id,
                target_group_id,
                target_rule_index,
            } => {
                let _ = app.move_rule_to_group_at(
                    source_group_id,
                    rule_id,
                    target_group_id,
                    target_rule_index,
                );
            }
            CentralAction::LogMessage(message) => {
                app.log_manager.add_entry(message);
            }
            CentralAction::ConsumeDroppedFiles(group_id) => {
                let _ = app.consume_dropped_files_into_group(group_id);
            }
            CentralAction::DiscardDroppedFiles => {
                app.ui.file_import_error = None;
                app.ui.dropped_files = None;
                app.ui.file_drop_position = None;
                app.ui.file_drop_hover_target = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppToRun;
    use crate::app::runtime::{CentralGroupSnapshot, CentralProgramSnapshot};
    use os_api::PriorityClass;

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn toolbar_has_no_duplicate_cpu_summary_and_new_group_stays_fixed() {
        use std::sync::{Arc, RwLock};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        ctx.all_styles_mut(crate::app::shell::presenters::shared_elements::apply_widget_style);
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|state| {
                    state.cpu_load_monitoring_enabled = true;
                    state.groups = (0..12)
                        .map(|index| crate::app::models::CoreGroup {
                            name: format!("Group {index}"),
                            cores: vec![0],
                            programs: vec![],
                            is_hidden: false,
                            run_all_button: false,
                        })
                        .collect();
                }),
            )),
            20,
        );
        let group_id = app.rules.group_id_for_index(0).unwrap();
        let name_id = egui::Id::new(("central-group-drag", group_id.0.as_str()));
        let frame = |app: &mut AppState, time, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 450.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| draw_central_panel(app, ui),
            );
            output.textures_delta.clear();
            output
        };
        let mut output = frame(&mut app, 0.0, vec![]);
        for index in 1..3 {
            output = frame(&mut app, index as f64 * 0.1, vec![]);
        }
        let position = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .unwrap()
        };
        let new_group = position(&output, "+ New group");
        let description = position(&output, "CPU placement and application rules");
        assert!(description.x < new_group.x);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "All CPUs")));
        let before = ctx.read_response(name_id).unwrap().rect.top();
        output = frame(
            &mut app,
            0.4,
            vec![
                egui::Event::PointerMoved(egui::pos2(200.0, 250.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -300.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                },
            ],
        );
        for index in 5..10 {
            output = frame(&mut app, index as f64 * 0.1, vec![]);
        }
        assert_eq!(
            position(&output, "CPU placement and application rules"),
            description
        );
        assert_eq!(position(&output, "+ New group"), new_group);
        assert!(ctx.read_response(name_id).unwrap().rect.top() < before - 1.0);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn s104_failed_file_import_is_not_resubmitted_by_subsequent_frames() {
        use std::sync::{Arc, RwLock};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|state| {
                    state.groups = vec![crate::app::models::CoreGroup {
                        name: "Import test".into(),
                        cores: vec![0],
                        programs: vec![],
                        is_hidden: false,
                        run_all_button: false,
                    }];
                }),
            )),
            20,
        );
        let snapshot = app.build_central_panel_snapshot();
        let group = snapshot.groups[0].group_id.clone();
        // Simulate a stale GUI projection: the worker must refuse this candidate.
        app.persistent_state.write().unwrap().tuning =
            crate::app::features::system_tweaks::config::TuningEnvelope::from_config(
                &Default::default(),
            )
            .unwrap();
        app.ui.dropped_files = Some(vec![r"C:\Import-test.exe".into()]);
        assert!(app.consume_dropped_files_into_group(group.clone()));
        app.wait_for_test_save();
        assert!(app.ui.dropped_files.is_some());
        for _ in 0..3 {
            let mut actions = vec![];
            let _ = ctx
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(600.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        actions = render_groups(&mut app, ui, &ctx, &snapshot);
                    },
                )
                .textures_delta
                .clear();
            assert!(
                !actions.iter().any(|a| matches!(
                    a,
                    CentralAction::ConsumeDroppedFiles(_) | CentralAction::DiscardDroppedFiles
                )),
                "A failed save needs explicit retry; no per-frame import loop"
            );
            assert_eq!(app.ui.file_drop_hover_target.as_ref(), Some(&group));
        }
    }

    #[test]
    fn file_import_uses_the_recorded_release_position_and_discards_outside_drops() {
        use std::sync::{Arc, RwLock};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
        let mut app = AppState::new_for_test(
            Arc::new(RwLock::new(
                crate::app::models::AppStateStorage::test_fixture(|_| {}),
            )),
            20,
        );
        let snapshot = CentralPanelSnapshot {
            groups: vec![group("g", "G", vec![])],
        };
        for (position, accepted) in [
            (egui::pos2(30.0, 30.0), true),
            (egui::pos2(-10.0, -10.0), false),
        ] {
            app.ui.dropped_files = Some(vec![PathBuf::from(r"C:\one.exe")]);
            app.ui.file_drop_position = Some(position);
            let mut actions = vec![];
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 600.0),
                    )),
                    events: vec![egui::Event::PointerMoved(if accepted {
                        egui::pos2(350.0, 550.0)
                    } else {
                        egui::pos2(30.0, 30.0)
                    })],
                    ..Default::default()
                },
                |ui| {
                    actions = render_groups(&mut app, ui, &ctx, &snapshot);
                },
            )
            .textures_delta
            .clear();
            assert_eq!(
                actions
                    .iter()
                    .any(|a| matches!(a, CentralAction::ConsumeDroppedFiles(id) if id.0 == "g")),
                accepted
            );
            assert_eq!(
                actions
                    .iter()
                    .any(|a| matches!(a, CentralAction::DiscardDroppedFiles)),
                !accepted
            );
        }
    }

    #[test]
    fn short_group_and_app_names_drag_from_their_padded_name_area_without_grips() {
        use std::sync::{Arc, RwLock};
        for group_drag in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            ctx.all_styles_mut(crate::app::shell::presenters::shared_elements::apply_widget_style);
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(
                    crate::app::models::AppStateStorage::test_fixture(|_| {}),
                )),
                20,
            );
            let mut g = group("g", "G", vec![program("r", "X", app_key("X"))]);
            g.run_all_button = false;
            let mut h = group("h", "H", vec![]);
            h.run_all_button = false;
            let snapshot = CentralPanelSnapshot { groups: vec![g, h] };
            let frame = |app: &mut AppState, events| {
                let mut actions = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        actions = render_groups(app, ui, &ctx, &snapshot);
                    },
                );
                output.textures_delta.clear();
                (output, actions)
            };
            for _ in 0..2 {
                frame(&mut app, vec![]);
            }
            let id = if group_drag {
                egui::Id::new(("central-group-drag", "h"))
            } else {
                egui::Id::new(("central-rule-drag", "g", "r"))
            };
            let response = ctx
                .read_response(id)
                .expect("name must own a stable drag hit target");
            assert!(
                response.rect.width() > 70.0,
                "short names must retain the full allotted drag width"
            );
            if group_drag {
                ctx.memory_mut(|memory| memory.request_focus(id));
                let (_, actions) = frame(
                    &mut app,
                    vec![egui::Event::Key {
                        key: egui::Key::ArrowUp,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                );
                assert!(actions.iter().any(|action| matches!(action, CentralAction::MoveGroupToIndex {group_id, target_index: 0} if group_id.0 == "h")),
                    "keyboard reorder must remain available on the group name");
            }
            let start = response.rect.right_center() - egui::vec2(5.0, 0.0);
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            let (output, _) = frame(
                &mut app,
                vec![egui::Event::PointerMoved(start - egui::vec2(20.0, 0.0))],
            );
            if group_drag {
                assert_eq!(
                    egui::DragAndDrop::payload::<GroupDragPayload>(&ctx)
                        .unwrap()
                        .group_id,
                    GroupId("h".into())
                );
            } else {
                assert_eq!(
                    egui::DragAndDrop::payload::<RuleDragPayload>(&ctx)
                        .unwrap()
                        .rule_id,
                    RuleId("r".into())
                );
            }
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.radius == 1.0)),
                "the old six-dot drag grips must not remain");
            let target_id =
                egui::Id::new(("central-group-drag", if group_drag { "g" } else { "h" }));
            let target = ctx.read_response(target_id).unwrap().rect.center();
            frame(&mut app, vec![egui::Event::PointerMoved(target)]);
            let (_, actions) = frame(
                &mut app,
                vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            if group_drag {
                assert!(actions.iter().any(|action| matches!(action, CentralAction::MoveGroupToIndex {group_id, target_index: 0} if group_id.0 == "h")));
            } else {
                assert!(actions.iter().any(
                    |action| matches!(action, CentralAction::MoveRuleToGroup {
                    source_group_id, rule_id, target_group_id, target_rule_index: 0,
                } if source_group_id.0 == "g" && rule_id.0 == "r" && target_group_id.0 == "h")
                ));
            }
        }
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn cpu_meters_keep_long_names_and_actions_inside_400_pixels() {
        use std::sync::{Arc, RwLock};
        for monitoring in [true, false] {
            let context = egui::Context::default();
            context
                .set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            context
                .all_styles_mut(crate::app::shell::presenters::shared_elements::apply_widget_style);
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(
                    crate::app::models::AppStateStorage::test_fixture(|state| {
                        state.cpu_load_monitoring_enabled = monitoring
                    }),
                )),
                20,
            );
            let mut group = group(
                "g",
                &"A long affinity group name ".repeat(5),
                vec![program(
                    "r",
                    &"Long game executable name ".repeat(4),
                    app_key("Game"),
                )],
            );
            group.cores = (0..20).collect();
            group.run_all_button = false;
            let snapshot = CentralPanelSnapshot {
                groups: vec![group],
            };
            for _ in 0..3 {
                let mut content = egui::Rect::NOTHING;
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        render_groups(&mut app, ui, &context, &snapshot);
                        content = ui.min_rect();
                    },
                );
                output.textures_delta.clear();
                assert!(
                    !output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == "1")),
                    "group ordinal badge must be absent"
                );
                assert!(
                    content.right() <= 400.0,
                    "content overflow with monitoring={monitoring}: {content:?}"
                );
                let edit = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text.ends_with(" Edit") => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .expect("group Edit action must remain visible");
                assert!(
                    edit.right() <= 400.0,
                    "group Edit action is pushed out: {edit:?}"
                );
            }
        }
    }

    fn app_key(name: &str) -> AppRuntimeKey {
        AppToRun::new_path(
            PathBuf::from(format!(r"C:\{name}.lnk")),
            Vec::new(),
            PathBuf::from(format!(r"C:\{name}.exe")),
            PriorityClass::Normal,
            false,
        )
        .get_key()
    }

    fn program(rule_id: &str, name: &str, app_key: AppRuntimeKey) -> CentralProgramSnapshot {
        CentralProgramSnapshot {
            steam_app_id: None,
            rule_id: RuleId(rule_id.to_string()),
            name: name.to_string(),
            launch_target_detail: name.to_string(),
            runtime_key: app_key.clone(),
            app_key,
        }
    }

    fn group(
        group_id: &str,
        name: &str,
        programs: Vec<CentralProgramSnapshot>,
    ) -> CentralGroupSnapshot {
        CentralGroupSnapshot {
            group_id: GroupId(group_id.to_string()),
            name: name.to_string(),
            cores: vec![0],
            is_hidden: false,
            run_all_button: true,
            programs,
        }
    }

    fn payload(group_id: &str, rule_id: &str, app_key: AppRuntimeKey) -> RuleDragPayload {
        RuleDragPayload {
            source_group_id: GroupId(group_id.to_string()),
            rule_id: RuleId(rule_id.to_string()),
            app_key,
            preview_label: "Sample".to_string(),
            preview_width: 240.0,
        }
    }

    #[test]
    fn test_file_drop_target_uses_current_hovered_group() {
        let mut cached = Some(GroupId("old-group".to_string()));
        let target = resolve_file_drop_target(
            &mut cached,
            false,
            true,
            true,
            Some(GroupId("new-group".to_string())),
        );

        assert_eq!(target, Some(GroupId("new-group".to_string())));
        assert_eq!(cached, Some(GroupId("new-group".to_string())));
    }

    #[test]
    fn test_file_drop_target_falls_back_to_cached_hover_when_pointer_missing() {
        let mut cached = Some(GroupId("cached-group".to_string()));
        let target = resolve_file_drop_target(&mut cached, false, true, false, None);

        assert_eq!(target, Some(GroupId("cached-group".to_string())));
        assert_eq!(cached, Some(GroupId("cached-group".to_string())));
    }

    #[test]
    fn test_file_drop_hover_updates_and_clears_cached_target() {
        let mut cached = None;
        let target = resolve_file_drop_target(
            &mut cached,
            true,
            false,
            true,
            Some(GroupId("hovered-group".to_string())),
        );

        assert_eq!(target, None);
        assert_eq!(cached, Some(GroupId("hovered-group".to_string())));

        let target = resolve_file_drop_target(&mut cached, true, false, true, None);

        assert_eq!(target, None);
        assert_eq!(cached, None);
    }

    #[test]
    fn test_file_drop_hover_without_pointer_preserves_cached_target() {
        let mut cached = Some(GroupId("cached-group".to_string()));
        let target = resolve_file_drop_target(&mut cached, true, false, false, None);

        assert_eq!(target, None);
        assert_eq!(cached, Some(GroupId("cached-group".to_string())));
    }

    #[test]
    fn test_file_drop_release_with_known_pointer_outside_group_clears_cached_target() {
        let mut cached = Some(GroupId("cached-group".to_string()));
        let target = resolve_file_drop_target(&mut cached, false, true, true, None);

        assert_eq!(target, None);
        assert_eq!(cached, None);
    }

    #[test]
    fn test_file_drop_target_clears_when_no_external_file_drag_is_active() {
        let mut cached = Some(GroupId("cached-group".to_string()));
        let target = resolve_file_drop_target(&mut cached, false, false, false, None);

        assert_eq!(target, None);
        assert_eq!(cached, None);
    }

    #[test]
    fn test_rule_drop_classifier_accepts_cross_group_non_duplicate() {
        let sample_key = app_key("Sample");
        let other_key = app_key("Other");
        let snapshot = CentralPanelSnapshot {
            groups: vec![
                group(
                    "group-a",
                    "Games",
                    vec![program("rule-a", "Sample", sample_key.clone())],
                ),
                group(
                    "group-b",
                    "Background",
                    vec![program("rule-b", "Other", other_key)],
                ),
            ],
        };

        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", sample_key),
                &GroupId("group-b".to_string()),
                1,
            ),
            RuleDropClassification::Valid
        );
    }

    #[test]
    fn test_rule_drop_classifier_rejects_cross_group_duplicate() {
        let sample_key = app_key("Sample");
        let snapshot = CentralPanelSnapshot {
            groups: vec![
                group(
                    "group-a",
                    "Games",
                    vec![program("rule-a", "Sample", sample_key.clone())],
                ),
                group(
                    "group-b",
                    "Background",
                    vec![program("rule-b", "Sample Copy", sample_key.clone())],
                ),
            ],
        };

        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", sample_key),
                &GroupId("group-b".to_string()),
                1,
            ),
            RuleDropClassification::DuplicateInTarget
        );
    }

    #[test]
    fn test_rule_drop_classifier_accepts_same_group_reorder() {
        let sample_key = app_key("Sample");
        let other_key = app_key("Other");
        let snapshot = CentralPanelSnapshot {
            groups: vec![group(
                "group-a",
                "Games",
                vec![
                    program("rule-a", "Sample", sample_key.clone()),
                    program("rule-b", "Other", other_key),
                ],
            )],
        };

        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", sample_key),
                &GroupId("group-a".to_string()),
                2,
            ),
            RuleDropClassification::Valid
        );
    }

    #[test]
    fn test_rule_drop_classifier_marks_same_position_noop() {
        let sample_key = app_key("Sample");
        let snapshot = CentralPanelSnapshot {
            groups: vec![group(
                "group-a",
                "Games",
                vec![program("rule-a", "Sample", sample_key.clone())],
            )],
        };

        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", sample_key.clone()),
                &GroupId("group-a".to_string()),
                0,
            ),
            RuleDropClassification::SamePosition
        );
        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", sample_key),
                &GroupId("group-a".to_string()),
                1,
            ),
            RuleDropClassification::SamePosition
        );
    }

    #[test]
    fn test_rule_drop_classifier_rejects_stale_payload() {
        let sample_key = app_key("Sample");
        let stale_key = app_key("Stale");
        let snapshot = CentralPanelSnapshot {
            groups: vec![group(
                "group-a",
                "Games",
                vec![program("rule-a", "Sample", sample_key)],
            )],
        };

        assert_eq!(
            classify_rule_drop(
                &snapshot,
                &payload("group-a", "rule-a", stale_key),
                &GroupId("group-a".to_string()),
                0,
            ),
            RuleDropClassification::StalePayload
        );
    }

    #[test]
    fn test_format_core_summary_retains_every_cpu_for_layout_clipping_and_hover() {
        assert_eq!(format_core_summary(&[]), "No CPU threads assigned");
        assert_eq!(format_core_summary(&[0, 1, 6, 7]), "4 threads · 0, 1, 6, 7");
        assert_eq!(
            format_core_summary(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
            "9 threads · 0, 1, 2, 3, 4, 5, 6, 7, 8"
        );
        assert_eq!(
            format_core_summary(&(0..12).collect::<Vec<_>>()),
            "12 threads · 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11"
        );
    }

    #[test]
    fn core_summary_only_elides_when_it_reaches_header_controls() {
        use std::sync::{Arc, RwLock};
        for monitoring in [true, false] {
            for (count, elided) in [(12, false), (64, true)] {
                let ctx = egui::Context::default();
                ctx.set_fonts(
                    crate::app::shell::presenters::shared_elements::ui_font_definitions(),
                );
                ctx.all_styles_mut(
                    crate::app::shell::presenters::shared_elements::apply_widget_style,
                );
                let mut app = AppState::new_for_test(
                    Arc::new(RwLock::new(
                        crate::app::models::AppStateStorage::test_fixture(|state| {
                            state.cpu_load_monitoring_enabled = monitoring;
                        }),
                    )),
                    64,
                );
                let mut group = group("g", "Performance", vec![]);
                group.cores = (0..count).collect();
                let snapshot = CentralPanelSnapshot {
                    groups: vec![group],
                };
                for _ in 0..3 {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(400.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            render_groups(&mut app, ui, &ctx, &snapshot);
                        },
                    );
                    output.textures_delta.clear();
                    let summary = output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t)
                                if t.galley.job.text.starts_with(&format!("{count} threads")) =>
                            {
                                Some(t)
                            }
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(
                        summary.galley.job.text,
                        format_core_summary(&(0..count).collect::<Vec<_>>())
                    );
                    assert_eq!(
                        summary.galley.elided, elided,
                        "monitoring={monitoring}, count={count}"
                    );
                    let title = ctx
                        .read_response(egui::Id::new(("central-group-drag", "g")))
                        .unwrap()
                        .rect;
                    assert!(summary.pos.x + summary.galley.rect.right() <= title.right() + 0.5);
                }
            }
        }
    }

    #[test]
    fn test_app_status_label_is_explicit() {
        assert_eq!(app_status_label(AppStatus::Running), "Running · protected");
        assert_eq!(
            app_status_label(AppStatus::SettingsMismatch),
            "Running · correction needed"
        );
        assert_eq!(app_status_label(AppStatus::NotRunning), "Stopped");
    }

    #[test]
    fn test_app_status_tone_keeps_stopped_neutral() {
        assert_eq!(
            app_status_tone(AppStatus::NotRunning),
            AppStatusTone::Neutral
        );
        assert_eq!(app_status_tone(AppStatus::Running), AppStatusTone::Success);
        assert_eq!(
            app_status_tone(AppStatus::SettingsMismatch),
            AppStatusTone::Warning
        );
    }

    #[test]
    fn test_app_action_presentation_is_platform_capability_aware() {
        assert_eq!(
            app_action_presentation(AppStatus::NotRunning, true),
            AppActionPresentation {
                label: "Run",
                hover_text: "Start this application with the assigned settings",
                action: Some(AppRowAction::Run),
            }
        );
        assert_eq!(
            app_action_presentation(AppStatus::SettingsMismatch, true),
            AppActionPresentation {
                label: "Fix",
                hover_text: "Reapply the assigned affinity and priority",
                action: Some(AppRowAction::Fix),
            }
        );
        assert_eq!(
            app_action_presentation(AppStatus::Running, true),
            AppActionPresentation {
                label: "Focus",
                hover_text: "Focus the existing application window",
                action: Some(AppRowAction::Focus),
            }
        );
        assert_eq!(
            app_action_presentation(AppStatus::Running, false),
            AppActionPresentation {
                label: "Active",
                hover_text: "Window focus is unavailable on this platform",
                action: None,
            }
        );
    }

    #[cfg(feature = "windows")]
    #[test]
    fn test_windows_row_presentation_exposes_focus_action() {
        assert_eq!(
            app_action_presentation(AppStatus::Running, platform_supports_app_focus()).action,
            Some(AppRowAction::Focus)
        );
    }

    #[cfg(feature = "linux")]
    #[test]
    fn test_linux_row_presentation_disables_unavailable_focus_action() {
        assert_eq!(
            app_action_presentation(AppStatus::Running, platform_supports_app_focus()),
            AppActionPresentation {
                label: "Active",
                hover_text: "Window focus is unavailable on this platform",
                action: None,
            }
        );
    }

    #[test]
    fn test_keyboard_group_reorder_targets_only_valid_adjacent_positions() {
        assert_eq!(adjacent_group_target(0, 3, GroupMoveDirection::Up), None);
        assert_eq!(
            adjacent_group_target(0, 3, GroupMoveDirection::Down),
            Some(1)
        );
        assert_eq!(adjacent_group_target(1, 3, GroupMoveDirection::Up), Some(0));
        assert_eq!(
            adjacent_group_target(1, 3, GroupMoveDirection::Down),
            Some(2)
        );
        assert_eq!(adjacent_group_target(2, 3, GroupMoveDirection::Down), None);
        assert_eq!(adjacent_group_target(0, 0, GroupMoveDirection::Down), None);
    }

    #[cfg(all(target_os = "windows", feature = "windows"))]
    #[test]
    fn compact_status_returns_unused_space_to_names_with_readable_gaps() {
        use std::sync::{Arc, RwLock};
        let mut name_widths = Vec::new();
        for enabled in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::app::shell::presenters::shared_elements::ui_font_definitions());
            ctx.all_styles_mut(crate::app::shell::presenters::shared_elements::apply_widget_style);
            let mut app = AppState::new_for_test(
                Arc::new(RwLock::new(
                    crate::app::models::AppStateStorage::test_fixture(|state| {
                        state.cpu_load_monitoring_enabled = enabled;
                    }),
                )),
                20,
            );
            let snapshot = CentralPanelSnapshot {
                groups: vec![group(
                    "g",
                    "G",
                    vec![program(
                        "r",
                        "A long application name that needs the row width",
                        app_key("Stopped test"),
                    )],
                )],
            };
            let mut output = None;
            for _ in 0..3 {
                let mut frame = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(400.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        render_groups(&mut app, ui, &ctx, &snapshot);
                    },
                );
                frame.textures_delta.clear();
                output = Some(frame);
            }
            let output = output.unwrap();
            let name = ctx
                .read_response(egui::Id::new(("central-rule-drag", "g", "r")))
                .unwrap()
                .rect;
            name_widths.push(name.width());
            let status = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Stopped" => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap();
            let gap = status.left() - name.right();
            assert!(
                (7.0..=12.0).contains(&gap),
                "reserve a small gap, not a proportional empty column: {gap}"
            );
            if enabled {
                let meter = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(shape) if shape.rect.size() == egui::vec2(52.0, 21.0) => {
                            Some(shape.rect)
                        }
                        _ => None,
                    })
                    .unwrap();
                assert!((3.0..=7.0).contains(&(meter.left() - status.right())));
            }
        }
        assert!((name_widths[0] - name_widths[1] - 56.0).abs() < 1.0);
    }

    #[test]
    fn test_status_label_adapts_without_losing_full_status_text() {
        assert_eq!(
            app_status_display_label(AppStatus::SettingsMismatch, 100.0),
            "Needs correction"
        );
        assert_eq!(
            app_status_display_label(AppStatus::SettingsMismatch, 150.0),
            "Running · correction needed"
        );
        assert_eq!(
            app_status_display_label(AppStatus::Running, 90.0),
            "Protected"
        );
        assert_eq!(
            app_status_display_label(AppStatus::Running, 130.0),
            "Running · protected"
        );
        assert_eq!(
            app_status_label(AppStatus::SettingsMismatch),
            "Running · correction needed"
        );
    }

    #[test]
    fn test_resolve_group_drop_index_uses_insertion_slots() {
        assert_eq!(resolve_group_drop_index(0, 0, 3), None);
        assert_eq!(resolve_group_drop_index(0, 1, 3), None);
        assert_eq!(resolve_group_drop_index(0, 2, 3), Some(1));
        assert_eq!(resolve_group_drop_index(0, 3, 3), Some(2));
        assert_eq!(resolve_group_drop_index(2, 0, 3), Some(0));
        assert_eq!(resolve_group_drop_index(1, 3, 3), Some(2));
        assert_eq!(resolve_group_drop_index(1, 4, 3), None);
    }
}
