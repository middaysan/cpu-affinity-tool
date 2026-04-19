use crate::app::models::AppState;
use crate::app::navigation::{GroupRoute, WindowRoute};
use eframe::egui::{self, Layout, Margin, RichText, TopBottomPanel};

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
        egui::Frame::NONE
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon, label) = match app.get_theme_index() {
                        0 => ("💻", "System theme"),
                        1 => ("☀", "Light theme"),
                        _ => ("🌙", "Dark theme"),
                    };
                    if ui
                        .button(RichText::new(icon).size(16.0))
                        .on_hover_text(label)
                        .clicked()
                    {
                        app.toggle_theme(ctx);
                    }
                    ui.separator();
                    ui.label(RichText::new("CPU Affinity Tool").heading().strong());

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(
                                RichText::new(format!(
                                    "📄 Logs ({})",
                                    app.log_manager.entries.len()
                                ))
                                .strong(),
                            )
                            .clicked()
                        {
                            app.set_current_window(WindowRoute::Logs);
                        }
                        if ui
                            .button(RichText::new("➕ Create Group").strong())
                            .clicked()
                        {
                            app.set_current_window(WindowRoute::Groups(GroupRoute::Create));
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();

                // Display the current tip
                ui.vertical(|ui| {
                    ui.add_sized(
                        [450.0, 25.0],
                        egui::Label::new(
                            RichText::new(app.get_tip(ctx.input(|i| i.time)))
                                .small()
                                .weak()
                                .italics(),
                        ),
                    )
                });
            });
    });
}
