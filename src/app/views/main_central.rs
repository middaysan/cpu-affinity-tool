use eframe::egui::{self, RichText, CentralPanel, ScrollArea, Frame, Layout};
use crate::app::CpuAffinityApp;

pub fn draw_central_panel(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        let mut dropped_assigned = false;
        ScrollArea::vertical().show(ui, |ui| {
            dropped_assigned = render_groups(app, ui, ctx);
        });

        if let Some(dropped) = &app.dropped_file {
            if !dropped_assigned {
                ui.separator();
                ui.label(RichText::new("Dropped file:").strong());
                ui.label(dropped.display().to_string());
                for group in &mut app.state.groups {
                    if ui.button(format!("Add to group '{}'", group.name)).clicked() {
                        group.programs.push(dropped.clone());
                        app.dropped_file = None;
                        app.state.save_state();
                        break;
                    }
                }
            }
        }
    });
}

fn render_groups(app: &mut CpuAffinityApp, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
    let mut dropped_assigned = false;
    let mut modified = false;

    app.state.groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    
    // Take ownership of dropped_file to avoid borrowing conflicts
    let dropped_file = app.dropped_file.take();
    let mut edit_index = None;
    let mut run_program: Option<(usize, std::path::PathBuf)> = None;
    let mut remove_program: Option<(usize, std::path::PathBuf)> = None;

    for (i, group) in app.state.groups.iter_mut().enumerate() {
        Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&group.name).heading())
                        .on_hover_text(RichText::new(format!("cores: {:?}", group.cores)).weak());
                    ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                        if ui.button("⚙").on_hover_text("Edit group settings").clicked() {
                            edit_index = Some(i);
                        }
                        if ui.button("📁add").on_hover_text("Add executables...").clicked() {
                            if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe", "lnk"]).pick_files() {
                                group.programs.extend(paths);
                                modified = true;
                            }
                        }
                    });
                });

                ui.separator();

                ScrollArea::vertical().id_salt(i).show(ui, |ui| {
                    if group.programs.is_empty() {
                        ui.label("No executables. Drag & drop to add.");
                    } else {
                        for prog in group.programs.clone() {
                            let label = prog.file_name().map_or_else(|| prog.display().to_string(), |n| n.to_string_lossy().to_string());
                            ui.horizontal(|ui| {
                                let app_name = format!("▶  {}", label);
                                let button = egui::Button::new(RichText::new(app_name));
                                let response = ui.add_sized([
                                    ui.available_width() - 30.0,
                                    24.0
                                ], button);

                                let delete = ui.button("❌").on_hover_text("Remove from group");

                                if response.on_hover_text(prog.to_str().unwrap_or("")).clicked() {
                                    run_program = Some((i, prog.clone()));
                                }
                                if delete.clicked() {
                                    remove_program = Some((i, prog.clone()));
                                    modified = true;
                                }
                            });
                        }
                    }
                });

                if let Some(dropped) = &dropped_file {
                    let rect = ui.min_rect();
                    if rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                        group.programs.push(dropped.clone());
                        dropped_assigned = true;
                        modified = true;
                    }
                }
            });
        });
    }

    // Handle actions outside of the iterator
    if let Some(index) = edit_index {
        app.edit_group_index = Some(index);
    }
    if let Some((index, prog)) = run_program {
        app.run_program_with_affinity(index, prog);
    }
    if let Some((index, prog)) = remove_program {
        app.remove_program_from_group(index, &prog);
    }
    
    // Only put dropped_file back if it wasn't assigned to a group
    if !dropped_assigned {
        app.dropped_file = dropped_file;
    }

    if modified {
        app.state.save_state();
    }

    dropped_assigned
}
