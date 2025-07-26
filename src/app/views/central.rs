use crate::app::models::AppState;
use crate::app::models::AppToRun;
use eframe::egui::{self, CentralPanel, Frame, Layout, RichText, ScrollArea};
use eframe::egui::{Color32, Painter, Vec2};

pub fn draw_central_panel(app: &mut AppState, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        let mut dropped_assigned = false;
        ScrollArea::vertical().show(ui, |ui| {
            ui.vertical(|ui| {
                dropped_assigned = render_groups(app, ui, ctx);
            });
        });
    });
}

fn render_groups(app: &mut AppState, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
    let mut dropped_assigned = false;
    let mut modified = false;

    let mut run_program: Option<Vec<(usize, usize, AppToRun)>> = None;
    let mut remove_program: Option<(usize, usize)> = None;

    let mut swap_step: Option<(usize, bool)> = None;
    let groups_len = app.persistent_state.groups.len();

    for g_i in 0..groups_len {
        Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if g_i > 0 {
                        ui.small_button("⬆")
                            .on_hover_text("Move group up")
                            .clicked()
                            .then(|| {
                                swap_step = Some((g_i, true));
                            });
                    }

                    if g_i < groups_len - 1 {
                        ui.small_button("⬇")
                            .on_hover_text("Move group down")
                            .clicked()
                            .then(|| {
                                swap_step = Some((g_i, false));
                            });
                    }
                });
                ui.label(RichText::new(&app.persistent_state.groups[g_i].name).heading())
                    .on_hover_text(
                        RichText::new(format!(
                            "cores: {:?}",
                            app.persistent_state.groups[g_i].cores
                        ))
                        .weak(),
                    );
                ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                    if ui
                        .button("⚙")
                        .on_hover_text("Edit group settings")
                        .clicked()
                    {
                        app.start_editing_group(g_i);
                    }

                    let button_text = if app.persistent_state.groups[g_i].is_hidden {
                        RichText::new("\u{1F441}").strikethrough()
                    } else {
                        RichText::new("\u{1F441}").strong()
                    };

                    let hover_text = if app.persistent_state.groups[g_i].is_hidden {
                        "Show apps list"
                    } else {
                        "Hide apps list"
                    };

                    if ui.button(button_text).on_hover_text(hover_text).clicked() {
                        app.persistent_state.groups[g_i].is_hidden =
                            !app.persistent_state.groups[g_i].is_hidden;
                        modified = true;
                    }

                    // TODO: add linux support
                    if ui
                        .button("📁Add")
                        .on_hover_text("Add executables...")
                        .clicked()
                    {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("Executables", &["exe", "lnk", "url"])
                            .pick_files()
                        {
                            app.log_manager.add_entry(format!(
                                "Adding executables to group: {}, paths: {:?}",
                                app.persistent_state.groups[g_i].name, paths
                            ));
                            let res = app.persistent_state.groups[g_i].add_app_to_group(paths);
                            if let Err(err) = res {
                                app.log_manager
                                    .add_entry(format!("Error adding executables: {err}"));
                            } else {
                                app.log_manager.add_entry(format!(
                                    "Added executables to group: {}",
                                    app.persistent_state.groups[g_i].name
                                ));
                            }
                            modified = true;
                        }
                    }

                    if app.persistent_state.groups[g_i].run_all_button
                        && ui
                            .button("▶ Run all")
                            .on_hover_text("Run all apps in group")
                            .clicked()
                    {
                        if app.persistent_state.groups[g_i].programs.is_empty() {
                            app.log_manager.add_entry(format!(
                                "No executables to run in group: {}",
                                app.persistent_state.groups[g_i].name
                            ));
                        } else {
                            for (prog_index, prog) in
                                app.persistent_state.groups[g_i].programs.iter().enumerate()
                            {
                                run_program.get_or_insert_with(Vec::new).push((
                                    g_i,
                                    prog_index,
                                    prog.clone(),
                                ));
                            }
                        }
                    }
                });
            });

            ui.separator();

            if !app.persistent_state.groups[g_i].is_hidden {
                if app.persistent_state.groups[g_i].programs.is_empty() {
                    ui.label("No executables. Drag & drop a file here to add.");
                    ui.add_space(15.0);
                } else {
                    let len = app.persistent_state.groups[g_i].programs.len();
                    for prog_index in 0..len {
                        ui.horizontal(|ui| {
                            let is_app_run = app.is_app_running(
                                &app.persistent_state.groups[g_i].programs[prog_index].get_key(),
                            );
                            let prog = &app.persistent_state.groups[g_i].programs[prog_index];
                            let label = prog.name.clone();
                            // Set a fixed width for the entire row
                            let available_width = ui.available_width();

                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(15.0), egui::Sense::hover());
                            let color = if is_app_run {
                                Color32::GREEN
                            } else {
                                Color32::RED
                            };
                            let painter = Painter::new(ui.ctx().clone(), ui.layer_id(), rect);
                            painter.circle_filled(rect.center(), 4.0, color);

                            let app_title = RichText::new(format!("▶  {label}"));
                            let button = egui::Button::new(app_title);
                            let response = ui.add_sized(
                                [
                                    available_width - 90.0, // Reserve space for the two buttons
                                    24.0,
                                ],
                                button,
                            );

                            // Add the two action buttons with fixed widths
                            let edit_settings = ui
                                .add_sized([24.0, 24.0], egui::Button::new("⚙"))
                                .on_hover_text("Edit app settings");
                            let delete = ui
                                .add_sized([24.0, 24.0], egui::Button::new("❌"))
                                .on_hover_text("Remove from group");

                            if response
                                .on_hover_text(prog.bin_path.to_str().unwrap_or(""))
                                .clicked()
                            {
                                run_program = Some(vec![(g_i, prog_index, prog.clone())]);
                            }
                            if delete.clicked() {
                                remove_program = Some((g_i, prog_index));
                                modified = true;
                            }
                            if edit_settings.clicked() {
                                app.app_edit_state.run_settings = Some((g_i, prog_index));
                                app.set_current_window(
                                    crate::app::controllers::WindowController::AppRunSettings,
                                );
                            }
                        });
                    }
                }
            }

            if let Some(dropped_files) = &app.dropped_files {
                if !dropped_files.is_empty() {
                    let rect = ui.min_rect();
                    if rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                        if let Err(err) =
                            app.persistent_state.groups[g_i].add_app_to_group(dropped_files.clone())
                        {
                            app.log_manager
                                .add_entry(format!("Error adding executables: {err}"));
                        } else {
                            app.log_manager.add_entry(format!(
                                "Added {} executables to group: {}",
                                dropped_files.len(),
                                app.persistent_state.groups[g_i].name
                            ));
                        }
                        dropped_assigned = true;
                        app.dropped_files = None;
                        modified = true;
                    }
                }
            }
        });
    }

    if let Some((index, is_up)) = swap_step {
        if is_up {
            app.persistent_state.groups.swap(index, index - 1);
        } else {
            app.persistent_state.groups.swap(index + 1, index);
        }
    }

    if let Some(programs) = run_program {
        for (g_index, p_index, prog) in programs {
            app.run_app_with_affinity(g_index, p_index, prog);
        }
    }

    if let Some((g_i, p_i)) = remove_program {
        app.remove_app_from_group(g_i, p_i);
    }

    if modified {
        app.persistent_state.save_state();
    }

    dropped_assigned
}
