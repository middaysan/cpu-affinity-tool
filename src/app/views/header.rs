

use eframe::egui::{self, RichText, Layout, TopBottomPanel};
use crate::app::CpuAffinityApp;

pub fn draw_top_panel(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    TopBottomPanel::top("top_panel").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let (icon, label) = match app.theme_index {
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
                if ui.button(format!("📄 View Logs({})", app.logs.log_text.len())).clicked() {
                    app.logs.show = true;
                }
                if ui.button("➕ Create Core Group").clicked() {
                    app.show_group_window = true;
                }
            });
        });
        ui.separator();
        ui.label("💡 Tip: Drag & drop executable files (.exe/.lnk) onto a group to add them, then click ▶ to run with the assigned CPU cores");
    });
}
