use eframe::egui::{self, Window};
use crate::app::CpuAffinityApp;

pub fn group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if app.show_group_window {
        create_group_window(app, ctx);
    }
    if app.edit_group_index.is_some() {
        edit_group_window(app, ctx);
    }
}

fn draw_group_form_ui(
    ui: &mut egui::Ui,
    group_name: &mut String,
    core_selection: &mut [bool],
    is_edit: bool,
    on_save: &mut dyn FnMut(),
    on_cancel: &mut dyn FnMut(),
    on_delete: Option<&mut dyn FnMut()>,
) {
    ui.horizontal(|ui| {
        ui.label("Group name:");
        ui.text_edit_singleline(group_name);
    });

    ui.label("Select CPU cores:");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.5;
        ui.spacing_mut().item_spacing.y = 0.5;
        for (i, selected) in core_selection.iter_mut().enumerate() {
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
            
            let button = egui::Button::new(format!("Core {}", i))
                .min_size(egui::vec2(70.0, 20.0))
                .fill(
                    if *selected { selected_color } else { unselected_color }
                );
            let ui_button = ui.add(button);
            if ui_button.clicked() {
                *selected = !*selected;
            }
        }
    });

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
