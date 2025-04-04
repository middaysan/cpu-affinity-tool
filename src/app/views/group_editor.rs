use eframe::egui::{self, Window};
use std::collections::HashSet;
use crate::app::CpuAffinityApp;

/// Rendering the main group window
pub fn group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if app.show_group_window {
        create_group_window(app, ctx);
    } else if app.edit_group_index.is_some() {
        edit_group_window(app, ctx);
    }
}

/// Form for creating/editing a group: divided into rendering the name and the section with cores and clusters.
fn draw_group_form_ui(
    ui: &mut egui::Ui,
    group_name: &mut String,
    core_selection: &mut Vec<bool>,
    clusters: &mut Vec<Vec<usize>>,
    is_edit: bool,
    enable_run_all_button: &mut bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    clusters.retain(|cluster| !cluster.is_empty());

    ui.spacing_mut().item_spacing.y = 10.0;
    draw_group_name_ui(ui, group_name);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Enable run all button:");
        ui.checkbox(enable_run_all_button, "Run all apps in group");
    });

    ui.separator();
    draw_cpu_cores_ui(ui, core_selection, clusters);
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("💾 Save").clicked() {
            on_save();
        }
        if ui.button("❌ Cancel").clicked() {
            on_cancel();
        }
        if is_edit {
            if let Some(delete_fn) = on_delete {
                if ui.button("❌ Delete Group").clicked() {
                    delete_fn();
                }
            }
        }
    });
}

/// Rendering the group name input field
fn draw_group_name_ui(ui: &mut egui::Ui, group_name: &mut String) {
    ui.horizontal(|ui| {
        ui.label("Group name:");
        ui.text_edit_singleline(group_name);
    });
}

/// Rendering the CPU cores section: a list of already created clusters and a panel of free cores.
/// Using HashSet for optimal calculation of free cores.
fn draw_cpu_cores_ui(ui: &mut egui::Ui, core_selection: &mut Vec<bool>, clusters: &mut Vec<Vec<usize>>) {
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

    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
        ui.heading("Select CPU cores");
    });
    ui.separator();

    let assigned: HashSet<usize> = clusters.iter().flatten().copied().collect();
    let total_cores = core_selection.len();
    let free_core_indexes: Vec<usize> = (0..total_cores).filter(|i| !assigned.contains(i)).collect();

    for (i, cluster) in clusters.iter_mut().enumerate() {
        ui.group(|ui| {
            ui.label(format!("Cluster {}", i + 1));
            draw_core_buttons(ui, core_selection, cluster, selected_color, unselected_color);
        });
        ui.separator();
    }

    ui.group(|ui| {
        ui.label("Free Cores");
        draw_core_buttons(ui, core_selection, &mut free_core_indexes.clone(), selected_color, unselected_color);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.button("➕ Add New Cluster").on_hover_text("Add selected cores to a new cluster").clicked() {
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

/// Rendering a set of buttons to toggle the state of cores in a given set (cluster or free cores).
/// The function includes "All", "No HT", and individual toggles for each core.
fn draw_core_buttons(
    ui: &mut egui::Ui,
    core_selection: &mut [bool],
    indexes: &mut Vec<usize>,
    selected_color: egui::Color32,
    unselected_color: egui::Color32,
) {
    ui.horizontal(|ui| {
        let all_selected = indexes.iter().all(|&i| core_selection[i]);
        if ui.add(egui::Button::new("All").fill(if all_selected { selected_color } else { unselected_color })).clicked() {
            for &i in indexes.iter() {
                core_selection[i] = !all_selected;
            }
        }

        let no_ht_selected = indexes.iter().filter(|&&i| i % 2 == 0).all(|&i| core_selection[i])
            && indexes.iter().filter(|&&i| i % 2 != 0).all(|&i| !core_selection[i]);
        if ui.add(egui::Button::new("No HT").fill(if no_ht_selected { selected_color } else { unselected_color })).clicked() {
            for &i in indexes.iter() {
                if i % 2 == 0 {
                    core_selection[i] = !no_ht_selected;
                } else {
                    core_selection[i] = false;
                }
            }
        }

        if ui.button("Clear").clicked() {
            indexes.clear();
        }
    });

    ui.horizontal_wrapped(|ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.spacing_mut().item_spacing.y = 1.0;
            for &i in indexes.iter() {
                if ui.add(egui::Button::new(format!("Core {}", i))
                     .min_size(egui::vec2(70.0, 20.0))
                     .fill(if core_selection[i] { selected_color } else { unselected_color }))
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
pub fn create_group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    let mut open = true;
    let mut create_clicked = false;
    let mut cancel_clicked = false;

    Window::new("Create Core Group")
        .open(&mut open)
        .resizable(true)  // Allow window to be resized
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                draw_group_form_ui(
                    ui,
                    &mut app.new_group_name,
                    &mut app.core_selection,
                    &mut app.state.clusters,
                    false,
                    &mut app.enable_run_all_button,
                    &mut || create_clicked = true,
                    &mut || cancel_clicked = true,
                    None,
                );
            });
        });

    if create_clicked {
        app.create_group();
        app.reset_group_form();
    }
    if cancel_clicked || !open {
        app.reset_group_form();
    }
}

/// Group editing window.
/// The logic is similar to creation but with loading group data, and the final state of cores is formed as a union of clusters and free cores.
pub fn edit_group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    let index = match app.edit_group_index {
        Some(i) if i < app.state.groups.len() => i,
        _ => {
            app.edit_group_index = None;
            app.edit_group_selection = None;
            return;
        }
    };

    if app.edit_group_selection.is_none() {
        let mut selection = vec![false; num_cpus::get()];
        for &core in &app.state.groups[index].cores {
            if core < selection.len() {
                selection[core] = true;
            }
        }
        app.edit_group_selection = Some(selection);
    }

    let mut open = true;

    Window::new("Edit Group Settings")
        .open(&mut open)
        .show(ctx, |ui| {
            let mut save_clicked = false;
            let mut delete_clicked = false;
            let mut cancel_clicked = false;
            app.enable_run_all_button = app.state.groups[index].run_all_button;

            draw_group_form_ui(
                ui,
                &mut app.state.groups[index].name,
                &mut app.edit_group_selection.as_mut().unwrap(),
                &mut app.state.clusters,
                true,
                &mut app.enable_run_all_button,
                &mut || save_clicked = true,
                &mut || cancel_clicked = true,
                Some(&mut || delete_clicked = true),
            );

            app.state.groups[index].run_all_button = app.enable_run_all_button;

            if save_clicked {
                let mut assigned: HashSet<usize> = app.state.clusters.iter().flatten().copied().collect();
                for (i, &selected) in app.edit_group_selection.as_ref().unwrap().iter().enumerate() {
                    if selected {
                        assigned.insert(i);
                    }
                }
                app.state.groups[index].cores = assigned.into_iter().collect();
                app.state.groups[index].run_all_button = app.enable_run_all_button;
                app.state.save_state();
                app.reset_group_form();
            }
            if delete_clicked {
                app.state.groups.remove(index);
                app.state.save_state();
                app.reset_group_form();
            }
            if cancel_clicked {
                app.reset_group_form();
            }
        });

    if !open {
        app.edit_group_index = None;
        app.edit_group_selection = None;
    }
}
