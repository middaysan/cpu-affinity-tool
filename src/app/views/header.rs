use eframe::egui::{self, Layout, RichText, TopBottomPanel};
use crate::app::models::AppState;

pub fn draw_top_panel(app: &mut AppState, ctx: &egui::Context) {
    TopBottomPanel::top("top_panel").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let (icon, label) = match app.persistent_state.theme_index {
                0 => ("💻", "System theme"),
                1 => ("☀", "Light theme"),
                _ => ("🌙", "Dark theme"),
            };
            if ui.button(icon).on_hover_text(label).clicked() {
                app.toggle_theme(ctx);
            }
            ui.separator();
            ui.label(RichText::new("Core Groups").heading());
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(format!("📄 View Logs({})", app.log_manager.entries.len())).clicked() {
                    app.set_current_window(crate::app::controllers::WindowController::Logs);
                }
                if ui.button("➕ Create Core Group").clicked() {
                    app.set_current_window(crate::app::controllers::WindowController::Groups(crate::app::controllers::Group::CreateGroup));
                }
            });
        });
        ui.separator();
        ui.label("💡 Tip: Drag & drop executable files (.exe/.lnk) onto a group to add them, then click ▶ to run with the assigned CPU cores");
        ui.separator();
        ui.add_space(3.0);
    });
}
