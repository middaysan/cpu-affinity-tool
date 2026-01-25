use crate::app::models::AppState;
use crate::app::models::GroupFormState;
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, CentralPanel, RichText};
use std::collections::HashSet;

/// Form for creating/editing a group: divided into rendering the name and the section with cores and clusters.
fn draw_group_form_ui(
    ui: &mut egui::Ui,
    groups: &mut GroupFormState,
    clusters: &mut Vec<Vec<usize>>,
    is_edit: bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    clusters.retain(|cluster| !cluster.is_empty());

    glass_frame(ui).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Group name:").strong());
            ui.text_edit_singleline(&mut groups.group_name).request_focus();
        });

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut groups.run_all_enabled, "");
            ui.label(RichText::new("Enable \"Run All\" button").strong());
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        draw_cpu_cores_ui(ui, &mut groups.core_selection, clusters);

        ui.add_space(15.0);
        ui.separator();
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("💾 Save Changes").strong()).min_size(egui::vec2(120.0, 32.0)))
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
                            .add(egui::Button::new(RichText::new("🗑 Delete Group").color(egui::Color32::RED)).min_size(egui::vec2(120.0, 32.0)))
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
/// Using HashSet for optimal calculation of free cores.
fn draw_cpu_cores_ui(
    ui: &mut egui::Ui,
    core_selection: &mut [bool],
    clusters: &mut Vec<Vec<usize>>,
) {
    let selected_color = if ui.visuals().dark_mode {
        egui::Color32::from_rgb(61, 79, 3)
    } else {
        egui::Color32::from_rgb(175, 191, 124)
    };
    let unselected_color = if ui.visuals().dark_mode {
        egui::Color32::DARK_GRAY
    } else {
        egui::Color32::GRAY
    };

    ui.with_layout(
        egui::Layout::top_down_justified(egui::Align::Center),
        |ui| {
            ui.heading("Select CPU cores");
        },
    );
    ui.separator();

    let assigned: HashSet<usize> = clusters.iter().flatten().copied().collect();
    let total_cores = core_selection.len();
    let free_core_indexes: Vec<usize> =
        (0..total_cores).filter(|i| !assigned.contains(i)).collect();

    for (i, cluster) in clusters.iter_mut().enumerate() {
        ui.group(|ui| {
            let cluster_num = i + 1;
            ui.label(format!("Cluster {cluster_num}"));
            draw_core_buttons(
                ui,
                core_selection,
                cluster,
                selected_color,
                unselected_color,
                true,
            );
        });
    }

    if !free_core_indexes.is_empty() {
        ui.group(|ui| {
            ui.label("Free Cores");
            draw_core_buttons(
                ui,
                core_selection,
                &mut free_core_indexes.clone(),
                selected_color,
                unselected_color,
                false,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui
                    .button("➕ Add New Cluster")
                    .on_hover_text("Add selected cores to a new cluster")
                    .clicked()
                {
                    let new_cluster: Vec<usize> = free_core_indexes
                        .into_iter()
                        .filter(|&i| core_selection[i])
                        .collect();
                    if !new_cluster.is_empty() {
                        clusters.push(new_cluster);
                        if let Some(last) = clusters.last() {
                            for &i in last {
                                core_selection[i] = false;
                            }
                        }
                    }
                }
            });
        });
    }
}

/// Rendering a set of buttons to toggle the state of cores in a given set (cluster or free cores).
/// The function includes "All", "No HT", and individual toggles for each core.
fn draw_core_buttons(
    ui: &mut egui::Ui,
    core_selection: &mut [bool],
    indexes: &mut Vec<usize>,
    selected_color: egui::Color32,
    unselected_color: egui::Color32,
    is_clear_button: bool,
) {
    ui.horizontal(|ui| {
        let all_selected = indexes.iter().all(|&i| core_selection[i]);
        if ui
            .add(egui::Button::new("All").fill(if all_selected {
                selected_color
            } else {
                unselected_color
            }))
            .clicked()
        {
            for &i in indexes.iter() {
                core_selection[i] = !all_selected;
            }
        }

        let no_ht_selected = indexes
            .iter()
            .filter(|&&i| i % 2 == 0)
            .all(|&i| core_selection[i])
            && indexes
                .iter()
                .filter(|&&i| i % 2 != 0)
                .all(|&i| !core_selection[i]);
        if ui
            .add(egui::Button::new("No HT").fill(if no_ht_selected {
                selected_color
            } else {
                unselected_color
            }))
            .clicked()
        {
            for &i in indexes.iter() {
                if i % 2 == 0 {
                    core_selection[i] = !no_ht_selected;
                } else {
                    core_selection[i] = false;
                }
            }
        }

        if is_clear_button && ui.button("Clear").clicked() {
            indexes.clear();
        }
    });

    ui.horizontal_wrapped(|ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.spacing_mut().item_spacing.y = 1.0;
            for &i in indexes.iter() {
                if ui
                    .add(
                        egui::Button::new(format!("Core {i}"))
                            .min_size(egui::vec2(70.0, 20.0))
                            .fill(if core_selection[i] {
                                selected_color
                            } else {
                                unselected_color
                            }),
                    )
                    .clicked()
                {
                    core_selection[i] = !core_selection[i];
                }
            }
        });
    });
}

/// Group creation window.
/// Uses the refactored draw_group_form_ui and updated state (clusters instead of cluster_cores_indexes).
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
                // Get clusters using helper method
                let mut clusters = app.get_clusters().unwrap_or_default();
                draw_group_form_ui(
                    ui,
                    &mut app.group_form,
                    &mut clusters,
                    false,
                    &mut || create_clicked = true,
                    &mut || cancel_clicked = true,
                    None,
                );
                // Update clusters if needed
                app.set_clusters(clusters);
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
/// The logic is similar to creation but with loading group data, and the final state of cores is formed as a union of clusters and free cores.
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
                if ui.button("❌").on_hover_text("Close").clicked() {
                    cancel_clicked = true;
                }
            });
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Get clusters using helper method
                let mut clusters = app.get_clusters().unwrap_or_default();
                draw_group_form_ui(
                    ui,
                    &mut app.group_form,
                    &mut clusters,
                    true,
                    &mut || save_clicked = true,
                    &mut || cancel_clicked = true,
                    Some(&mut || delete_clicked = true),
                );
                // Update clusters if needed
                app.set_clusters(clusters);
            });

        if save_clicked {
            // Get updated clusters
            let clusters = app.get_clusters().unwrap_or_default();
            let mut assigned: HashSet<usize> = clusters.iter().flatten().copied().collect();

            for (i, &selected) in app.group_form.core_selection.iter().enumerate() {
                if selected {
                    assigned.insert(i);
                }
            }

            // Update group properties
            app.update_group_properties(
                index,
                app.group_form.group_name.clone(),
                assigned.into_iter().collect(),
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
