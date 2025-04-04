use eframe::egui::{self, Window};
use crate::app::CpuAffinityApp;


pub fn group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if app.show_group_window {
        create_group_window(app, ctx);
    } else if app.edit_group_index.is_some() {
        edit_group_window(app, ctx);
    }
}

fn draw_group_form_ui(
    ui: &mut egui::Ui,
    group_name: &mut String,
    core_selection: &mut [bool],
    perf_cores_indexes: &mut Vec<usize>,
    is_edit: bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    ui.spacing_mut().item_spacing.y = 10.0;
    ui.horizontal(|ui| {
        ui.label("Group name:");
        ui.text_edit_singleline(group_name);
    });

    ui.spacing_mut().item_spacing.y = 10.0;
    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
        ui.heading("Select CPU cores");
    });
    ui.separator();

    ui.spacing_mut().item_spacing.y = 0.5;
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

    let mut efficiency_cores_indexes = Vec::with_capacity(core_selection.len() - perf_cores_indexes.len());
    for i in 0..core_selection.len() {
        if !perf_cores_indexes.contains(&i) {
            efficiency_cores_indexes.push(i);
        }
    }

    if perf_cores_indexes.len() > 0 {
        draw_core_buttons(
            ui, 
            "1 cluster", 
            perf_cores_indexes,
            &efficiency_cores_indexes,
            core_selection, 
            selected_color, 
            unselected_color
        );

        ui.spacing_mut().item_spacing.y = 3.0;
        ui.separator();
    }


    let label_str = if perf_cores_indexes.len() > 0 {
        "2 cluster".to_string()
    } else {
        "Cores".to_string()
    };

    let skip_indexes = perf_cores_indexes.clone();
    draw_core_buttons(
        ui,
        &label_str, 
        perf_cores_indexes,
        &skip_indexes,
        core_selection, 
        selected_color, 
        unselected_color
    );
    
    ui.separator();

    ui.horizontal(|ui| {
        if ui.button("💾 Save").clicked() {
            on_save();
        }

        if ui.button("❌ Cancel").clicked() {
            on_cancel();
        }

        ui.separator();

        if is_edit {
            if let Some(delete_fn) = on_delete {
                if ui.button("❌ Delete Group").clicked() {
                    delete_fn();
                }
            }
        }
    });
}

pub fn create_group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    let mut open = true;
    let mut create_clicked = false;
    let mut cancel_clicked = false;

    Window::new("Create Core Group")
        .open(&mut open)
        .show(ctx, |ui| {
            draw_group_form_ui(ui,
                &mut app.new_group_name,
                &mut app.core_selection,
                &mut app.state.cluster_cores_indexes,
                false,
                &mut || create_clicked = true,
                &mut || cancel_clicked = true,
                None,
            );
        });

    if create_clicked {
        app.create_group();
        app.reset_group_form();
    }

    if cancel_clicked {
        app.reset_group_form();
    }

    if !open {
        app.reset_group_form();
    }
}

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

            draw_group_form_ui(
                ui,
                &mut app.state.groups[index].name,
                &mut app.edit_group_selection.as_mut().unwrap(),
                &mut app.state.cluster_cores_indexes,
                true,
                &mut || save_clicked = true,
                &mut || cancel_clicked = true,
                Some(&mut || delete_clicked = true),
            );

            if save_clicked {
                app.state.groups[index].cores = app.edit_group_selection
                    .as_ref()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                    .collect();
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

fn draw_core_buttons(
    ui: &mut egui::Ui,
    label: &str,
    perf_cores_indexes: &mut Vec<usize>,
    skip_indexes: &[usize],
    core_selection: &mut [bool],
    selected_color: egui::Color32,
    unselected_color: egui::Color32,
) {
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(label);
    ui.spacing_mut().item_spacing.y = 0.5;
    let mut is_all_selected = true;
    let mut is_no_ht_selected = true;
    for i in 0..core_selection.len() {
        if skip_indexes.contains(&i) {
            continue;
        }

        if i % 2 == 0 {
            is_no_ht_selected = is_no_ht_selected && core_selection[i];
        } else {
            is_no_ht_selected = is_no_ht_selected && !core_selection[i];
        }

        is_all_selected = is_all_selected && core_selection[i];
    }

    ui.spacing_mut().item_spacing.y = 2.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;

        let button = egui::Button::new("All").fill(
            if is_all_selected { selected_color } else { unselected_color }
        );
        let ui_button = ui.add(button);
        if ui_button.clicked() {
            for i in 0..core_selection.len() {
                if skip_indexes.contains(&i) { continue; }
                core_selection[i] = !is_all_selected;
            }
        }

        // Add "No HT" button to select only even cores
        let button = egui::Button::new("No HT").fill(
            if is_no_ht_selected { selected_color } else { unselected_color }
        );
        let ui_button = ui.add(button);
        if ui_button.clicked() {
            for i in 0..core_selection.len() {
                if skip_indexes.contains(&i) { continue; }
                if i % 2 == 0 { 
                    core_selection[i] = !is_no_ht_selected; 
                } else { 
                    core_selection[i] = false; 
                }
            }
        }

        ui.separator();

        // Add "No HT" button to select only even cores
        let button = egui::Button::new("Make cluster");
        let ui_button = ui.add(button);
        if ui_button.clicked() {
            perf_cores_indexes.clear();
            for i in 0..core_selection.len() {
                if core_selection[i] {
                    perf_cores_indexes.push(i);
                }
            }
        }

        let button = egui::Button::new("Clear cluster");
        let ui_button = ui.add(button);
        if ui_button.clicked() {
            perf_cores_indexes.clear();
        }
    });

    ui.spacing_mut().item_spacing.y = 0.5;
    ui.horizontal_wrapped(|ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            for i in 0..core_selection.len() {
                if skip_indexes.contains(&i) {
                    continue;
                }

                let button = egui::Button::new(format!("Core {}", i))
                    .min_size(egui::vec2(70.0, 20.0))
                    .fill(if core_selection[i] { selected_color } else { unselected_color });

                let ui_button = ui.add(button);

                if ui_button.clicked() {
                    core_selection[i] = !core_selection[i];
                }
            }
        });
    });
}