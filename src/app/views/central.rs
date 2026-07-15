use crate::app::features::execution::AppRowAction;
use crate::app::models::{AppRuntimeKey, AppStatus};
use crate::app::runtime::{AppState, CentralPanelSnapshot};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::presenters::shared_elements::{
    drag_grip, ghost_button, group_frame, inset_frame, inter_medium_family, inter_semibold_family,
    palette, row_fill, success_color, toned_sized_button, warning_color, ToneRole,
    BUTTON_FONT_SIZE,
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
    let panel_fill = root_ui.visuals().panel_fill;
    let snapshot = app.build_central_panel_snapshot();
    let group_count = snapshot.groups.len();
    CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(panel_fill)
                .inner_margin(egui::Margin::symmetric(6, 4)),
        )
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(RichText::new("Affinity groups").strong());
                    ui.label(
                        RichText::new("CPU placement and application rules")
                            .small()
                            .weak(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::BOTTOM), |ui| {
                    ui.label(
                        RichText::new(format!("{group_count} groups · drag to reorder"))
                            .small()
                            .weak(),
                    );
                });
            });
            ui.add_space(3.0);
            ScrollArea::vertical().show(ui, |ui| {
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
        .take(8)
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if cores.len() > 8 { ", …" } else { "" };
    format!("{} threads · {visible}{suffix}", cores.len())
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
    let file_drop_pos = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
    let groups_len = snapshot.groups.len();

    for (group_index, group) in snapshot.groups.iter().enumerate() {
        let group_id = group.group_id.clone();
        let mut target_rule_index = None;
        let mut drop_indicator = None;
        let mut rejected_rule_drop = None;

        let group_response = group_frame(ui).show(ui, |ui| {
            let colors = palette(ui);
            ui.horizontal(|ui| {
                let group_grip_response = ui
                    .push_id(("central-group-grip", group_id.0.as_str()), |ui| {
                        drag_grip(ui, "Drag to reorder group; when focused, use Up or Down")
                    })
                    .inner;

                if group_grip_response.has_focus() {
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

                egui::Frame::NONE
                    .fill(colors.inset)
                    .stroke(egui::Stroke::new(1.0, colors.border_strong))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(5, 3))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new((group_index + 1).to_string())
                                .color(colors.text_secondary),
                        );
                    });

                let title = ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&group.name)
                            .size(14.0)
                            .family(inter_semibold_family())
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format_core_summary(&group.cores))
                            .size(9.0)
                            .color(colors.text_muted),
                    );
                });
                let title_response = ui
                    .interact(
                        title.response.rect,
                        egui::Id::new(("central-group-drag", group_id.0.as_str())),
                        egui::Sense::click_and_drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::Grab)
                    .on_hover_text("Drag group to change its position");
                let group_drag_payload = GroupDragPayload {
                    group_id: group_id.clone(),
                    preview_label: group.name.clone(),
                    preview_width: title_response.rect.width().max(180.0),
                };
                group_grip_response.dnd_set_drag_payload(group_drag_payload.clone());
                title_response.dnd_set_drag_payload(group_drag_payload);

                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(
                        ui,
                        egui::Button::new(
                            RichText::new(format!("{ICON_EDIT} Edit")).size(BUTTON_FONT_SIZE),
                        ),
                    )
                    .on_hover_text("Edit group settings")
                    .clicked()
                    {
                        actions.push(CentralAction::StartEditGroup(group_id.clone()));
                    }
                });
            });

            ui.add_space(2.0);
            ui.separator();
            ui.add_space(2.0);

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

            ui.add_space(2.0);

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
                    let app_status = app.get_app_status_sync(&program.app_key);

                    let row_response = egui::Frame::NONE
                        .fill(row_fill(ui))
                        .corner_radius(5.0)
                        .inner_margin(egui::Margin::symmetric(4, 1))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let drag_response = ui
                                    .push_id(
                                        (
                                            "central-rule-grip",
                                            group_id.0.as_str(),
                                            program.rule_id.0.as_str(),
                                        ),
                                        |ui| drag_grip(ui, "Drag to move application"),
                                    )
                                    .inner;

                                let (rect, status_response) =
                                    ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                let color = match app_status_tone(app_status) {
                                    AppStatusTone::Neutral => palette(ui).neutral_status,
                                    AppStatusTone::Success => success_color(ui),
                                    AppStatusTone::Warning => warning_color(ui),
                                };
                                let full_status = app_status_label(app_status);
                                let status_tooltip =
                                    app.get_running_app_pids(&program.app_key).map_or_else(
                                        || format!("Status: {full_status}"),
                                        |pids| {
                                            format!(
                                                "Status: {full_status}\nTracking PIDs: {pids:?}"
                                            )
                                        },
                                    );
                                status_response.on_hover_text(status_tooltip.clone());
                                ui.painter().circle_filled(rect.center(), 3.5, color);

                                let action_presentation = app_action_presentation(
                                    app_status,
                                    platform_supports_app_focus(),
                                );
                                let status_color = match app_status_tone(app_status) {
                                    AppStatusTone::Neutral => colors.text_muted,
                                    AppStatusTone::Success => colors.success.fg,
                                    AppStatusTone::Warning => colors.warning.fg,
                                };
                                let action_width = 42.0;
                                let settings_width = 23.0;
                                let status_width = (ui.available_width() * 0.28).clamp(88.0, 150.0);
                                let name_width = (ui.available_width()
                                    - status_width
                                    - action_width
                                    - settings_width
                                    - 12.0)
                                    .max(72.0);
                                let name_response = ui.add_sized(
                                    [name_width, 21.0],
                                    egui::Label::new(
                                        RichText::new(&program.name)
                                            .size(BUTTON_FONT_SIZE)
                                            .family(inter_medium_family())
                                            .strong(),
                                    )
                                    .truncate(),
                                );
                                name_response.on_hover_text(program.launch_target_detail.clone());

                                ui.add_sized(
                                    [status_width, 21.0],
                                    egui::Label::new(
                                        RichText::new(app_status_display_label(
                                            app_status,
                                            status_width,
                                        ))
                                        .size(10.0)
                                        .color(status_color),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(status_tooltip);

                                drag_response.dnd_set_drag_payload(RuleDragPayload {
                                    source_group_id: group_id.clone(),
                                    rule_id: program.rule_id.clone(),
                                    app_key: program.app_key.clone(),
                                    preview_label: program.name.clone(),
                                    preview_width: name_width,
                                });

                                let button = egui::Button::new(
                                    RichText::new(action_presentation.label).size(BUTTON_FONT_SIZE),
                                );
                                let action_response = if action_presentation.action.is_none() {
                                    ui.add_enabled(
                                        false,
                                        button.min_size(egui::vec2(action_width, 21.0)),
                                    )
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
                                    if let Some(row_action) = action_presentation.action {
                                        actions.push(CentralAction::RunGroupProgramAction {
                                            group_id: group_id.clone(),
                                            rule_id: program.rule_id.clone(),
                                            action: row_action,
                                        });
                                    }
                                }

                                if ghost_button(
                                    ui,
                                    egui::Button::new(
                                        RichText::new(ICON_EDIT).size(BUTTON_FONT_SIZE),
                                    )
                                    .min_size(egui::vec2(settings_width, 21.0)),
                                )
                                .on_hover_text("Edit app settings")
                                .clicked()
                                {
                                    actions.push(CentralAction::OpenAppRunSettings {
                                        group_id: group_id.clone(),
                                        rule_id: program.rule_id.clone(),
                                    });
                                }
                            });
                        });

                    if target_rule_index.is_none() {
                        if let Some(pos) = rule_pointer_pos {
                            if row_response.response.rect.contains(pos) {
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
                let target_rect = group_response.response.rect.expand2(Vec2::new(0.0, 6.0));
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
                if group_response.response.rect.contains(pos) {
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
                if group_response.response.rect.contains(pos) {
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

        let dropped_rule_here =
            rule_drop_pos.is_some_and(|pos| group_response.response.rect.contains(pos));
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

    if let Some(group_id) = resolve_file_drop_target(
        &mut app.ui.file_drop_hover_target,
        has_hovered_files,
        has_dropped_files,
        file_drop_pos.is_some(),
        hovered_file_drop_group,
    ) {
        actions.push(CentralAction::ConsumeDroppedFiles(group_id));
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppToRun;
    use crate::app::runtime::{CentralGroupSnapshot, CentralProgramSnapshot};
    use os_api::PriorityClass;

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
            rule_id: RuleId(rule_id.to_string()),
            name: name.to_string(),
            launch_target_detail: name.to_string(),
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
    fn test_format_core_summary_is_compact_and_handles_empty_groups() {
        assert_eq!(format_core_summary(&[]), "No CPU threads assigned");
        assert_eq!(format_core_summary(&[0, 1, 6, 7]), "4 threads · 0, 1, 6, 7");
        assert_eq!(
            format_core_summary(&[0, 1, 2, 3, 4, 5, 6, 7, 8]),
            "9 threads · 0, 1, 2, 3, 4, 5, 6, 7, …"
        );
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
