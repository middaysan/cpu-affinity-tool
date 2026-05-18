use crate::app::models::AppStatus;
use crate::app::runtime::AppState;
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
    ConsumeDroppedFiles(GroupId),
}

#[derive(Clone)]
struct RuleDragPayload {
    source_group_id: GroupId,
    rule_id: RuleId,
    preview_label: String,
    preview_width: f32,
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
    let mut drop_target = None;
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
    let snapshot = app.build_central_panel_snapshot();
    let groups_len = snapshot.groups.len();

    for (group_index, group) in snapshot.groups.iter().enumerate() {
        let group_id = group.group_id.clone();
        let mut target_rule_index = None;
        let mut drop_indicator = None;

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
                                    if active_rule_payload.as_deref().is_some_and(|payload| {
                                        let source_index = group
                                            .programs
                                            .iter()
                                            .position(|program| program.rule_id == payload.rule_id);
                                        !is_same_position_rule_drop(
                                            payload,
                                            &group.group_id,
                                            source_index,
                                            insert_index,
                                        )
                                    }) {
                                        let y = if insert_index == program_index {
                                            row_response.response.rect.top()
                                        } else {
                                            row_response.response.rect.bottom()
                                        };
                                        drop_indicator = Some((row_response.response.rect, y));
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

            let has_dropped_files = app
                .ui
                .dropped_files
                .as_ref()
                .is_some_and(|files| !files.is_empty());

            if drop_target.is_none() && has_dropped_files {
                let rect = ui.min_rect();
                let hover_pos = ctx.input(|i| i.pointer.hover_pos().unwrap_or_default());
                if rect.contains(hover_pos) {
                    drop_target = Some(group_id.clone());
                }
            }
        });

        if drop_indicator.is_none() {
            if let Some(pos) = rule_drag_pos {
                let append_index = group.programs.len();
                if group_response.response.rect.contains(pos)
                    && active_rule_payload.as_deref().is_some_and(|payload| {
                        let source_index = group
                            .programs
                            .iter()
                            .position(|program| program.rule_id == payload.rule_id);
                        !is_same_position_rule_drop(
                            payload,
                            &group.group_id,
                            source_index,
                            append_index,
                        )
                    })
                {
                    let rect = group_response.response.rect.shrink2(Vec2::new(14.0, 8.0));
                    drop_indicator = Some((rect, rect.bottom()));
                }
            }
        }
        if let Some((rect, y)) = drop_indicator {
            paint_rule_drop_indicator(ui, rect, y);
        }

        let dropped_rule_here =
            rule_drop_pos.is_some_and(|pos| group_response.response.rect.contains(pos));
        let dropped_rule = if dropped_rule_here {
            egui::DragAndDrop::take_payload::<RuleDragPayload>(ctx)
        } else {
            None
        };
        if let Some(payload) = dropped_rule {
            actions.push(CentralAction::MoveRuleToGroup {
                source_group_id: payload.source_group_id.clone(),
                rule_id: payload.rule_id.clone(),
                target_group_id: group_id,
                target_rule_index: target_rule_index.unwrap_or(group.programs.len()),
            });
        }
    }

    if let Some(group_id) = drop_target {
        actions.push(CentralAction::ConsumeDroppedFiles(group_id));
    }

    actions
}

fn is_same_position_rule_drop(
    payload: &RuleDragPayload,
    group_id: &GroupId,
    source_rule_index: Option<usize>,
    target_rule_index: usize,
) -> bool {
    if &payload.source_group_id != group_id {
        return false;
    }

    source_rule_index.is_some_and(|source_index| {
        target_rule_index == source_index || target_rule_index == source_index + 1
    })
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
            CentralAction::ConsumeDroppedFiles(group_id) => {
                let _ = app.consume_dropped_files_into_group(group_id);
            }
        }
    }
}
