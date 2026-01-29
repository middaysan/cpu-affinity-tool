use crate::app::models::GroupFormState;
use crate::app::models::{AppState, CoreInfo, CoreType, CpuSchema};
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, CentralPanel, RichText};

/// Form for creating/editing a group: divided into rendering the name and the section with cores and clusters.
fn draw_group_form_ui(
    ui: &mut egui::Ui,
    groups: &mut GroupFormState,
    cpu_schema: &mut CpuSchema,
    is_edit: bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    glass_frame(ui).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Group name:").strong());
            ui.text_edit_singleline(&mut groups.group_name)
                .request_focus();
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut groups.run_all_enabled, "");
            ui.label(RichText::new("Enable \"Run All\" button").strong());
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        draw_cpu_cores_ui(ui, groups, cpu_schema);

        ui.add_space(15.0);
        ui.separator();
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("💾 Save Changes").strong())
                        .min_size(egui::vec2(120.0, 32.0)),
                )
                .clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                on_save();
            }

            if ui
                .add(egui::Button::new("❌ Cancel").min_size(egui::vec2(100.0, 32.0)))
                .clicked()
            {
                on_cancel();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if is_edit {
                    if let Some(delete_fn) = on_delete {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🗑 Delete Group").color(egui::Color32::RED),
                                )
                                .min_size(egui::vec2(120.0, 32.0)),
                            )
                            .clicked()
                        {
                            delete_fn();
                        }
                    }
                }
            });
        });
    });
}

/// Rendering the CPU cores section: a list of already created clusters and a panel of free cores.
fn draw_cpu_cores_ui(ui: &mut egui::Ui, groups: &mut GroupFormState, cpu_schema: &mut CpuSchema) {
    ui.with_layout(
        egui::Layout::top_down_justified(egui::Align::Center),
        |ui| {
            let model_display = if cpu_schema.clusters.is_empty() {
                format!("{} (No preset matched)", cpu_schema.model)
            } else {
                cpu_schema.model.clone()
            };
            ui.heading(format!("Select CPU cores ({})", model_display));
        },
    );
    ui.separator();

    let assigned = cpu_schema.get_assigned_cores();
    let total_cores = groups.core_selection.len();
    let free_core_indexes: Vec<usize> =
        (0..total_cores).filter(|i| !assigned.contains(i)).collect();

    for cluster in cpu_schema.clusters.iter_mut() {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&cluster.name).strong());
            });
            draw_core_buttons(ui, groups, &mut cluster.cores);
        });
    }

    if !free_core_indexes.is_empty() {
        ui.group(|ui| {
            ui.label(RichText::new("Free Cores").strong());

            // Temporary CoreInfo for drawing buttons of free cores
            let mut free_cores: Vec<CoreInfo> = free_core_indexes
                .iter()
                .map(|&i| CoreInfo {
                    index: i,
                    core_type: CoreType::Other,
                    label: format!("{i}"),
                })
                .collect();

            draw_core_buttons(ui, groups, &mut free_cores);
        });
    }
}

fn get_core_color(core_type: CoreType, dark_mode: bool) -> egui::Color32 {
    match core_type {
        CoreType::Performance => {
            if dark_mode {
                egui::Color32::from_rgb(100, 150, 250)
            } else {
                egui::Color32::from_rgb(50, 100, 200)
            }
        }
        CoreType::Efficient => {
            if dark_mode {
                egui::Color32::from_rgb(100, 200, 100)
            } else {
                egui::Color32::from_rgb(50, 150, 50)
            }
        }
        CoreType::HyperThreading => {
            if dark_mode {
                egui::Color32::from_rgb(200, 150, 100)
            } else {
                egui::Color32::from_rgb(150, 100, 50)
            }
        }
        CoreType::Other => {
            if dark_mode {
                egui::Color32::DARK_GRAY
            } else {
                egui::Color32::GRAY
            }
        }
    }
}

fn draw_core_buttons(ui: &mut egui::Ui, groups: &mut GroupFormState, cores: &mut [CoreInfo]) {
    let dark_mode = ui.visuals().dark_mode;
    let all_selected_color = if dark_mode {
        egui::Color32::from_rgb(61, 79, 3)
    } else {
        egui::Color32::from_rgb(175, 191, 124)
    };

    ui.horizontal(|ui| {
        let all_selected = cores.iter().all(|c| groups.core_selection[c.index]);
        if ui
            .add(egui::Button::new("All").fill(if all_selected {
                all_selected_color
            } else {
                get_core_color(CoreType::Other, dark_mode)
            }))
            .clicked()
        {
            for c in cores.iter() {
                groups.core_selection[c.index] = !all_selected;
            }
            groups.last_clicked_core = None;
        }

        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            for core in cores.iter() {
                let is_selected = groups.core_selection[core.index];
                let fill_color = if is_selected {
                    get_core_color(core.core_type, dark_mode)
                } else {
                    get_core_color(CoreType::Other, dark_mode)
                };

                let size = match core.core_type {
                    CoreType::Performance => egui::vec2(55.0, 45.0),
                    _ => egui::vec2(55.0, 35.0),
                };

                let response = ui.add_sized(size, egui::Button::new("").fill(fill_color));

                let rect = response.rect;
                let visuals = ui.style().interact(&response);
                let text_color = visuals.fg_stroke.color;

                // Draw main label (P0, E1, etc)
                ui.painter().text(
                    rect.center_top() + egui::vec2(0.0, 15.0),
                    egui::Align2::CENTER_CENTER,
                    &core.label,
                    egui::FontId::proportional(14.0),
                    text_color,
                );

                // Draw thread index (thread 0, etc)
                ui.painter().text(
                    rect.center_bottom() - egui::vec2(0.0, 6.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("thread{}", core.index),
                    egui::FontId::proportional(8.0),
                    text_color,
                );

                if response.clicked() {
                    let shift = ui.input(|i| i.modifiers.shift);
                    if let (true, Some(last_idx)) = (shift, groups.last_clicked_core) {
                        let start = last_idx.min(core.index);
                        let end = last_idx.max(core.index);
                        let target_state = groups.core_selection[last_idx];
                        for i in start..=end {
                            if i < groups.core_selection.len() {
                                groups.core_selection[i] = target_state;
                            }
                        }
                    } else {
                        groups.core_selection[core.index] = !is_selected;
                        groups.last_clicked_core = Some(core.index);
                    }
                }
            }
        });
    });
}

/// Group creation window.
pub fn create_group_window(app: &mut AppState, ctx: &egui::Context) {
    let mut create_clicked = false;
    let mut cancel_clicked = false;

    CentralPanel::default().show(ctx, |ui| {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("➕ Create New Group").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    cancel_clicked = true;
                }
            });
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Get cpu schema using helper method
                let mut schema = app.get_cpu_schema().unwrap_or_else(|| {
                    let cpu_model = crate::app::models::AppStateStorage::get_effective_cpu_model();
                    CpuSchema {
                        model: cpu_model,
                        clusters: vec![],
                    }
                });
                draw_group_form_ui(
                    ui,
                    &mut app.group_form,
                    &mut schema,
                    false,
                    &mut || create_clicked = true,
                    &mut || cancel_clicked = true,
                    None,
                );
                // Update schema if needed
                app.set_cpu_schema(schema);
            });
    });

    if create_clicked || cancel_clicked {
        if create_clicked {
            app.create_group();
        }
        app.reset_group_form();
        app.set_current_window(crate::app::controllers::WindowController::Groups(
            crate::app::controllers::Group::ListGroups,
        ));
    }
}

/// Group editing window.
pub fn edit_group_window(app: &mut AppState, ctx: &egui::Context) {
    let index = app.group_form.editing_index.unwrap();

    CentralPanel::default().show(ctx, |ui| {
        let mut save_clicked = false;
        let mut delete_clicked = false;
        let mut cancel_clicked = false;

        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("⚙ Edit Group").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    cancel_clicked = true;
                }
            });
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Get cpu schema using helper method
                let mut schema = app.get_cpu_schema().unwrap_or_else(|| {
                    let cpu_model = crate::app::models::AppStateStorage::get_effective_cpu_model();
                    CpuSchema {
                        model: cpu_model,
                        clusters: vec![],
                    }
                });
                draw_group_form_ui(
                    ui,
                    &mut app.group_form,
                    &mut schema,
                    true,
                    &mut || save_clicked = true,
                    &mut || cancel_clicked = true,
                    Some(&mut || delete_clicked = true),
                );
                // Update schema if needed
                app.set_cpu_schema(schema);
            });

        if save_clicked {
            // Gather indices of selected cores only.
            let selected_cores: Vec<usize> = app
                .group_form
                .core_selection
                .iter()
                .enumerate()
                .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                .collect();

            // Update group properties
            app.update_group_properties(
                index,
                app.group_form.group_name.clone(),
                selected_cores,
                app.group_form.run_all_enabled,
            );

            app.reset_group_form();
        }

        if delete_clicked {
            app.remove_group(index);
            app.reset_group_form();
        }

        if cancel_clicked {
            app.reset_group_form();
        }

        if save_clicked || delete_clicked || cancel_clicked {
            app.set_current_window(crate::app::controllers::WindowController::Groups(
                crate::app::controllers::Group::ListGroups,
            ));
        }
    });
}
