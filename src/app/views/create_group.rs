use eframe::egui::{self, Window};
use crate::app::CpuAffinityApp;

pub fn draw_group_window(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if !app.show_group_window {
        return;
    }

    let mut close = false;
    Window::new("Create Core Group").open(&mut true).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Group name:");
            ui.text_edit_singleline(&mut app.new_group_name);
        });

        ui.label("Select CPU cores:");
        ui.horizontal_wrapped(|ui| {
            for (i, selected) in app.core_selection.iter_mut().enumerate() {
                ui.checkbox(selected, format!("Core {}", i));
            }
        });

        ui.separator();

        ui.horizontal(|ui| {
            if ui.button("✅ Create").clicked() {
                app.create_group();
                close = true;
            }
            if ui.button("❌ Cancel").clicked() {
                close = true;
            }
        });
    });

    if close {
        app.new_group_name.clear();
        app.core_selection.fill(false);
        app.show_group_window = false;
    }
}