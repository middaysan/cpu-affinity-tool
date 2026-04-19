use crate::app::models::AppStatus;
use crate::app::runtime::AppState;
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, Align, CentralPanel, Color32, Layout, RichText, ScrollArea, Vec2};
use std::path::PathBuf;

const ICON_MOVE_UP: &str = "\u{23F6}";
const ICON_MOVE_DOWN: &str = "\u{23F7}";
const ICON_EDIT: &str = "\u{2699}";
const ICON_SHOW: &str = "\u{1F441}";

enum CentralAction {
    MoveGroup {
        from: usize,
        to: usize,
    },
    StartEditGroup(usize),
    ToggleGroupHidden {
        index: usize,
        is_hidden: bool,
    },
    AddSelectedFiles {
        group_index: usize,
        paths: Vec<PathBuf>,
    },
    OpenInstalledAppPicker(usize),
    OpenAppRunSettings {
        group_index: usize,
        program_index: usize,
    },
    RunGroup(usize),
    RunGroupProgram {
        group_index: usize,
        program_index: usize,
    },
    ConsumeDroppedFiles(usize),
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
}

fn render_groups(app: &mut AppState, ui: &mut egui::Ui, ctx: &egui::Context) -> Vec<CentralAction> {
    let mut actions = Vec::new();
    let mut drop_target = None;
    let snapshot = app.build_central_panel_snapshot();
    let groups_len = snapshot.groups.len();

    for group in snapshot.groups {
        let group_index = group.group_index;

        glass_frame(ui).outer_margin(5.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if group_index > 0 {
                        if ui
                            .button(ICON_MOVE_UP)
                            .on_hover_text("Move group up")
                            .clicked()
                        {
                            actions.push(CentralAction::MoveGroup {
                                from: group_index,
                                to: group_index - 1,
                            });
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
                            actions.push(CentralAction::MoveGroup {
                                from: group_index + 1,
                                to: group_index,
                            });
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
                        actions.push(CentralAction::StartEditGroup(group_index));
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
                            index: group_index,
                            is_hidden: !group.is_hidden,
                        });
                    }

                    if ui
                        .button("Open App")
                        .on_hover_text("Add executables, shortcuts, or URLs")
                        .clicked()
                    {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("Executables", &["exe", "lnk", "url"])
                            .pick_files()
                        {
                            actions.push(CentralAction::AddSelectedFiles { group_index, paths });
                        }
                    }

                    #[cfg(target_os = "windows")]
                    if ui
                        .button("Find Installed")
                        .on_hover_text("Find installed Start-backed apps")
                        .clicked()
                    {
                        actions.push(CentralAction::OpenInstalledAppPicker(group_index));
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
                        actions.push(CentralAction::RunGroup(group_index));
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
                    let len = group.programs.len();
                    for program in &group.programs {
                        let app_status = app.get_app_status_sync(&program.app_key);

                        ui.horizontal(|ui| {
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

                            let app_button = egui::Button::new(RichText::new(&program.name).strong());
                            let response =
                                ui.add_sized([ui.available_width() - 40.0, 20.0], app_button);

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .button(ICON_EDIT)
                                    .on_hover_text("Edit app settings")
                                    .clicked()
                                {
                                    actions.push(CentralAction::OpenAppRunSettings {
                                        group_index: program.group_index,
                                        program_index: program.program_index,
                                    });
                                }
                            });

                            if response
                                .on_hover_text(program.launch_target_detail.clone())
                                .clicked()
                            {
                                actions.push(CentralAction::RunGroupProgram {
                                    group_index: program.group_index,
                                    program_index: program.program_index,
                                });
                            }
                        });

                        if program.program_index < len - 1 {
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
                    drop_target = Some(group_index);
                }
            }
        });
    }

    if let Some(group_index) = drop_target {
        actions.push(CentralAction::ConsumeDroppedFiles(group_index));
    }

    actions
}

fn execute_actions(app: &mut AppState, actions: Vec<CentralAction>) {
    for action in actions {
        match action {
            CentralAction::MoveGroup { from, to } => {
                app.swap_groups(from, to);
            }
            CentralAction::StartEditGroup(group_index) => {
                app.start_editing_group(group_index);
            }
            CentralAction::ToggleGroupHidden { index, is_hidden } => {
                app.set_group_is_hidden(index, is_hidden);
            }
            CentralAction::AddSelectedFiles { group_index, paths } => {
                app.add_selected_files_to_group(group_index, paths);
            }
            CentralAction::OpenInstalledAppPicker(group_index) => {
                app.open_installed_app_picker(group_index);
            }
            CentralAction::OpenAppRunSettings {
                group_index,
                program_index,
            } => {
                app.open_app_run_settings(group_index, program_index);
            }
            CentralAction::RunGroup(group_index) => {
                app.run_group(group_index);
            }
            CentralAction::RunGroupProgram {
                group_index,
                program_index,
            } => {
                app.run_group_program(group_index, program_index);
            }
            CentralAction::ConsumeDroppedFiles(group_index) => {
                let _ = app.consume_dropped_files_into_group(group_index);
            }
        }
    }
}
