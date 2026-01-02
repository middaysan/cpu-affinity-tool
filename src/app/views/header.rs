use crate::app::models::AppState;
use eframe::egui::{self, Layout, RichText, TopBottomPanel};

/// Static array of tips to display in the application header
/// These tips rotate every 3 minutes
pub static TIPS: [&str; 5] = [
    "💡 Tip: Drag & drop executable files (.exe/.lnk) onto a group to add them, then click ▶ to run with the assigned CPU cores",
    "💡 Tip: Create different core groups for different types of applications to optimize performance",
    "💡 Tip: You can enable autorun for applications to start them automatically when the tool launches",
    "💡 Tip: Check the logs to see the history of application launches and their CPU affinity settings",
    "💡 Tip: Use the theme toggle button in the top-left corner to switch between light, dark, and system themes",
];

pub fn draw_top_panel(app: &mut AppState, ctx: &egui::Context) {
    TopBottomPanel::top("top_panel").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let (icon, label) = match app.get_theme_index() {
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
                if ui
                    .button(format!("📄 View Logs({})", app.log_manager.entries.len()))
                    .clicked()
                {
                    app.set_current_window(crate::app::controllers::WindowController::Logs);
                }
                if ui.button("➕ Create Core Group").clicked() {
                    app.set_current_window(crate::app::controllers::WindowController::Groups(
                        crate::app::controllers::Group::Create,
                    ));
                }
                if ui.button("🧙 Hide").on_hover_text("Hide to system tray").clicked() {
                    app.hide_requested = true;
                }
            });
        });
        ui.separator();

        // Tip rotation logic
        let current_time = ctx.input(|i| i.time);
        let time_since_last_change = current_time - app.last_tip_change_time;

        // Check if it's time to change the tip (every 3 minutes = 180 seconds)
        const TIP_CHANGE_INTERVAL: f64 = 120.0; // 3 minutes in seconds

        if time_since_last_change >= TIP_CHANGE_INTERVAL {
            // Update to the next tip
            app.current_tip_index = (app.current_tip_index + 1) % TIPS.len();
            app.last_tip_change_time = current_time;
        }

        // Display the current tip without any transition
        let current_tip = TIPS[app.current_tip_index];
        ui.label(current_tip);

        ui.separator();
        ui.add_space(3.0);
    });
}
