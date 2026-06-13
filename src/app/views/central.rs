use crate::app::models::{AppRuntimeKey, AppStatus};
use crate::app::runtime::{AppState, CentralPanelSnapshot};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::presenters::shared_elements::glass_frame;
use eframe::egui::{self, Align, CentralPanel, Color32, Layout, RichText, ScrollArea, Vec2};
use std::path::PathBuf;

const ICON_MOVE_UP: &str = "\u{23F6}";
const ICON_MOVE_DOWN: &str = "\u{23F7}";
const ICON_EDIT: &str = "\u{2699}";
const ICON_SHOW: &str = "\u{1F441}";

enum CentralAction {
    MoveGroupUp(GroupId),
    MoveGroupDown(GroupId),
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
    RunGroupProgram {
        group_id: GroupId,
        rule_id: RuleId,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleDropClassification {
    Valid,
    SamePosition,
    DuplicateInTarget,
    StalePayload,
}

pub fn draw_central_panel(app: &mut AppState, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            ui.vertical(|ui| {
                let actions = render_groups(app, ui, ctx);
                execute_actions(app, actions);
            });
        });
    });
    render_rule_drag_preview(ctx);
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

fn render_groups(app: &mut AppState, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<CentralAction> {
    let mut actions = Vec::new();
    let mut hovered_file_drop_group = None;
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
    let snapshot = app.build_central_panel_snapshot();
    let groups_len = snapshot.groups.len();

    for (group_index, group) in snapshot.groups.iter().enumerate() {
        let group_id = group.group_id.clone();
        let mut target_rule_index = None;
        let mut drop_indicator = None;
        let mut rejected_rule_drop = None;

        let group_response = glass_frame(ui).outer_margin(5.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if group_index > 0 {
                        if ui
                            .button(ICON_MOVE_UP)
                            .on_hover_text("Move group up")
                            .clicked()
                        {
                            actions.push(CentralAction::MoveGroupUp(group_id.clone()));
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new(ICON_MOVE_UP));
                    }

                    if group_index < groups_len - 1 {
                        if ui
                            .button(ICON_MOVE_DOWN)
                            .on_hover_text("Move group down")
                            .clicked()
                        {
                            actions.push(CentralAction::MoveGroupDown(group_id.clone()));
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new(ICON_MOVE_DOWN));
                    }
                });

                ui.add_space(8.0);

                ui.vertical(|ui| {
                    ui.label(RichText::new(&group.name).heading().strong());
                    ui.add_sized(
                        [350.0, 0.0],
                        egui::Label::new(RichText::new(format!("Cores: {:?}", group.cores)).small().weak()),
                    );
                });

                ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui
                        .button(ICON_EDIT)
                        .on_hover_text("Edit group settings")
                        .clicked()
                    {
                        actions.push(CentralAction::StartEditGroup(group_id.clone()));
                    }

                    ui.separator();

                    let hide_text = if group.is_hidden {
                        RichText::new(ICON_SHOW).strikethrough()
                    } else {
                        RichText::new(ICON_SHOW)
                    };
                    let hover_text = if group.is_hidden {
                        "Show apps list"
                    } else {
                        "Hide apps list"
                    };

                    if ui.button(hide_text).on_hover_text(hover_text).clicked() {
                        actions.push(CentralAction::ToggleGroupHidden {
                            group_id: group_id.clone(),
                            is_hidden: !group.is_hidden,
                        });
                    }

                    if ui
                        .button("Open App")
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

                    if crate::app::adapters::discovery::supports_installed_app_picker()
                        && ui
                            .button("Find Installed")
                            .on_hover_text(installed_app_hover_text())
                            .clicked()
                    {
                        actions.push(CentralAction::OpenInstalledAppPicker(group_id.clone()));
                    }

                    if group.run_all_button
                        && ui
                            .button(
                                RichText::new("\u{25B6} Run All")
                                    .color(Color32::from_rgb(0, 200, 0)),
                            )
                            .on_hover_text("Run all apps in group")
                            .clicked()
                    {
                        actions.push(CentralAction::RunGroup(group_id.clone()));
                    }
                });
            });

            if !group.is_hidden {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if group.programs.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("No apps yet. Use Open App, Find Installed, or drag files here.")
                                .weak()
                                .italics(),
                        );
                    });
                } else {
                    for (program_index, program) in group.programs.iter().enumerate() {
                        let app_status = app.get_app_status_sync(&program.app_key);

                        let row_response = ui.horizontal(|ui| {
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                            let color = match app_status {
                                AppStatus::Running => {
                                    if let Some(pids) = app.get_running_app_pids(&program.app_key) {
                                        response.on_hover_text(format!(
                                            "Tracking PIDs: {:?}\nStatus: All settings applied",
                                            pids
                                        ));
                                    }
                                    Color32::from_rgb(0, 255, 0)
                                }
                                AppStatus::SettingsMismatch => {
                                    if let Some(pids) = app.get_running_app_pids(&program.app_key) {
                                        response.on_hover_text(format!(
                                            "Tracking PIDs: {:?}\nStatus: Settings mismatch (CPU affinity or priority)",
                                            pids
                                        ));
                                    }
                                    Color32::from_rgb(255, 255, 0)
                                }
                                AppStatus::NotRunning => Color32::from_rgb(200, 0, 0),
                            };
                            ui.painter().circle_filled(rect.center(), 5.0, color);

                            ui.add_space(4.0);

                            let app_button_id = (
                                "central-rule-drag",
                                group_id.0.as_str(),
                                program.rule_id.0.as_str(),
                            );
                            let app_response = ui
                                .push_id(app_button_id, |ui| {
                                    ui.add_sized(
                                        [ui.available_width() - 40.0, 20.0],
                                        egui::Button::new(RichText::new(&program.name).strong())
                                            .sense(egui::Sense::click_and_drag()),
                                    )
                                })
                                .inner
                                .on_hover_text(program.launch_target_detail.clone());
                            let drag_payload = RuleDragPayload {
                                source_group_id: group_id.clone(),
                                rule_id: program.rule_id.clone(),
                                app_key: program.app_key.clone(),
                                preview_label: program.name.clone(),
                                preview_width: app_response.rect.width(),
                            };
                            app_response.dnd_set_drag_payload(drag_payload);
                            let app_was_dragged = app_response.drag_started()
                                || app_response.dragged()
                                || app_response.drag_stopped();

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .button(ICON_EDIT)
                                    .on_hover_text("Edit app settings")
                                    .clicked()
                                {
                                    actions.push(CentralAction::OpenAppRunSettings {
                                        group_id: group_id.clone(),
                                        rule_id: program.rule_id.clone(),
                                    });
                                }
                            });

                            if app_response.clicked() && !app_was_dragged {
                                actions.push(CentralAction::RunGroupProgram {
                                    group_id: group_id.clone(),
                                    rule_id: program.rule_id.clone(),
                                });
                            }
                        });

                        if target_rule_index.is_none() {
                            if let Some(pos) = rule_pointer_pos {
                                if row_response.response.rect.contains(pos) {
                                    let insert_index =
                                        if pos.y < row_response.response.rect.center().y {
                                            program_index
                                        } else {
                                            program_index + 1
                                        };
                                    target_rule_index = Some(insert_index);
                                    if let Some(payload) = active_rule_payload.as_deref() {
                                        match classify_rule_drop(
                                            &snapshot,
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
                                                drop_indicator =
                                                    Some((row_response.response.rect, y));
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
                            ui.add_space(2.0);
                        }
                    }
                }
            }

        });

        if (has_hovered_files || has_dropped_files) && hovered_file_drop_group.is_none() {
            if let Some(pos) = file_drop_pos {
                if group_response.response.rect.contains(pos) {
                    hovered_file_drop_group = Some(group_id.clone());
                }
            }
        }

        if drop_indicator.is_none() && target_rule_index.is_none() {
            if let Some(pos) = rule_drag_pos {
                let append_index = group.programs.len();
                if group_response.response.rect.contains(pos) {
                    if let Some(payload) = active_rule_payload.as_deref() {
                        match classify_rule_drop(&snapshot, payload, &group.group_id, append_index)
                        {
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
            match classify_rule_drop(&snapshot, &payload, &group_id, target_rule_index) {
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
    let color = ui.visuals().selection.stroke.color;
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

fn execute_actions(app: &mut AppState, actions: Vec<CentralAction>) {
    for action in actions {
        match action {
            CentralAction::MoveGroupUp(group_id) => {
                let _ = app.move_group_up(group_id);
            }
            CentralAction::MoveGroupDown(group_id) => {
                let _ = app.move_group_down(group_id);
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
            CentralAction::RunGroupProgram { group_id, rule_id } => {
                app.run_group_program(group_id, rule_id);
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
}
