use eframe::egui::{self, RichText, CentralPanel, ScrollArea, Frame, Layout};
use crate::app::{app_state::AppToRun, app_models::CpuAffinityApp};

pub fn draw_central_panel(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        let mut dropped_assigned = false;
        ScrollArea::vertical().show(ui, |ui| {
            dropped_assigned = render_groups(app, ui, ctx);
        });
    });
}

fn render_groups(app: &mut CpuAffinityApp, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
    let mut dropped_assigned = false;
    let mut modified = false;

    let mut run_program: Option<Vec<(usize, AppToRun)>> = None;
    let mut remove_program: Option<(usize, std::path::PathBuf)> = None;
    
    let mut swap_step: Option<(usize, bool)> = None;
    let groups_len = app.state.groups.len();
    
    for i in 0..groups_len {

        Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        if i > 0 {
                            ui.small_button("⬆").on_hover_text("Move group up").clicked().then(|| {
                                swap_step = Some((i, true));
                            });
                        }

                        if i < groups_len - 1 {
                            ui.small_button("⬇").on_hover_text("Move group down").clicked().then(|| {
                                swap_step = Some((i, false));
                            });
                        }
                    });
                    ui.label(RichText::new(&app.state.groups[i].name).heading())
                        .on_hover_text(RichText::new(format!("cores: {:?}", app.state.groups[i].cores)).weak());
                    ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                        if ui.button("⚙").on_hover_text("Edit group settings").clicked() {
                            app.logs.log_text.push(format!("Editing group: {}", app.state.groups[i].name));
                            app.groups.edit_index = Some(i);
                            app.groups.new_name = app.state.groups[i].name.clone();
                            app.set_current_controller(crate::app::controllers::WindowController::Groups(crate::app::controllers::Group::EditGroup));
                        }

                        // TODO: add linux support
                        if ui.button("📁Add").on_hover_text("Add executables...").clicked() {
                            if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe", "lnk"]).pick_files() {
                                app.logs.log_text.push(format!("Adding executables to group: {}, paths: {:?}", app.state.groups[i].name, paths));
                                let res = app.state.groups[i].add_app_to_group(paths);
                                if let Err(err) = res {
                                    app.logs.log_text.push(format!("Error adding executables: {}", err));
                                } else {
                                    app.logs.log_text.push(format!("Added executables to group: {}", app.state.groups[i].name));
                                }
                                modified = true;
                            }
                        }

                        if app.state.groups[i].run_all_button {
                            if ui.button("▶ Run all").on_hover_text("Run all apps in group").clicked() {
                                if app.state.groups[i].programs.is_empty() {
                                    app.logs.log_text.push(format!("No executables to run in group: {}", app.state.groups[i].name));
                                } else {
                                    for prog in &app.state.groups[i].programs {
                                        run_program.get_or_insert_with(Vec::new).push((i, prog.clone()));
                                    }
                                }
                            }
                        }
                    });
                });

                ui.separator();

                ScrollArea::vertical().id_salt(i).show(ui, |ui| {
                    if app.state.groups[i].programs.is_empty() {
                        ui.label("No executables. Drag & drop a file here to add.");
                    } else {
                        for prog in app.state.groups[i].programs.clone() {
                            let label = prog.name.clone();
                            ui.horizontal(|ui| {
                                // Set a fixed width for the entire row
                                let available_width = ui.available_width();
                                
                                // Create the main button with most of the width
                                let app_name = format!("▶  {}", label);
                                let button = egui::Button::new(RichText::new(app_name));
                                let response = ui.add_sized([
                                    available_width - 70.0, // Reserve space for the two buttons
                                    24.0
                                ], button);

                                // Add the two action buttons with fixed widths
                                let edit_settings = ui.add_sized([24.0, 24.0], egui::Button::new("⚙"))
                                    .on_hover_text("Edit app settings");
                                let delete = ui.add_sized([24.0, 24.0], egui::Button::new("❌"))
                                    .on_hover_text("Remove from group");

                                if response.on_hover_text(prog.bin_path.to_str().unwrap_or("")).clicked() {
                                    run_program = Some(vec![(i, prog.clone())]);
                                }
                                if delete.clicked() {
                                    remove_program = Some((i, prog.bin_path.clone()));
                                    modified = true;
                                }
                                if edit_settings.clicked() {
                                    let prog_index = app.state.groups[i].programs.clone().iter().position(|p| p.bin_path == prog.bin_path).unwrap_or_default();
                                    app.apps.edit_run_settings = Some((i, prog_index));
                                    app.set_current_controller(crate::app::controllers::WindowController::AppRunSettings);
                                }
                            });
                        }
                    }
                });

                if let Some(dropped_files) = &app.dropped_files {
                    if !dropped_files.is_empty() {
                        let rect = ui.min_rect();
                        if rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                            if let Err(err) = app.state.groups[i].add_app_to_group(dropped_files.clone()) {
                                app.logs.log_text.push(format!("Error adding executables: {}", err));
                            } else {
                                app.logs.log_text.push(format!("Added {} executables to group: {}", 
                                    dropped_files.len(), app.state.groups[i].name));
                            }
                            dropped_assigned = true;
                            app.dropped_files = None;
                            modified = true;
                        }
                    }
                }
            });
        });
    }

    if let Some((index, is_up)) = swap_step {
        if is_up {
            app.state.groups.swap(index, index - 1);
        } else {
            app.state.groups.swap(index + 1, index);
        }
    }

    if let Some(programs) = run_program {
        for (index, prog) in programs {
            app.run_app_with_affinity(index, prog);
        }
    }

    if let Some((index, prog)) = remove_program {
        app.remove_app_from_group(index, &prog);
    }

    if modified {
        app.state.save_state();
    }

    dropped_assigned
}
