use eframe::egui::{self, Window};
use crate::app::CpuAffinityApp;

pub fn draw_edit_group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if let Some(index) = app.edit_group_index {
        if index >= app.state.groups.len() {
            app.edit_group_index = None;
            app.edit_group_selection = None;
            return;
        }

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
        Window::new("Edit Group Settings").open(&mut open).show(ctx, |ui| {
            ui.label(format!("Editing group: {}", app.state.groups[index].name));
            ui.label("Select CPU cores:");

            if let Some(selection) = app.edit_group_selection.as_mut() {
                ui.horizontal_wrapped(|ui| {
                    for (i, selected) in selection.iter_mut().enumerate() {
                        ui.checkbox(selected, format!("Core {}", i));
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save").clicked() {
                        app.state.groups[index].cores = selection.iter().enumerate().filter_map(|(i, &v)| if v { Some(i) } else { None }).collect();
                        app.state.save_state();
                        app.edit_group_index = None;
                    }
                    if ui.button("❌ Delete Group").clicked() {
                        app.state.groups.remove(index);
                        app.state.save_state();
                        app.edit_group_index = None;
                    }
                    if ui.button("Cancel").clicked() {
                        app.edit_group_index = None;
                    }
                });
            }
        });

        if !open {
            app.edit_group_index = None;
            app.edit_group_selection = None;
        }
    }
}