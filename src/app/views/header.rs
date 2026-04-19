use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::AppState;
use eframe::egui::{self, Layout, Margin, RichText, TopBottomPanel};

pub fn draw_top_panel(app: &mut AppState, ctx: &egui::Context) {
    TopBottomPanel::top("top_panel").show(ctx, |ui| {
        egui::Frame::NONE
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon, label) = match app.get_theme_index() {
                        0 => ("\u{1F4BB}", "System theme"),
                        1 => ("\u{2600}", "Light theme"),
                        _ => ("\u{1F319}", "Dark theme"),
                    };
                    if ui
                        .button(RichText::new(icon).size(16.0))
                        .on_hover_text(label)
                        .clicked()
                    {
                        app.toggle_theme();
                        ctx.request_repaint();
                    }
                    ui.separator();
                    ui.label(RichText::new("CPU Affinity Tool").heading().strong());

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(
                                RichText::new(format!(
                                    "\u{1F4C4} Logs ({})",
                                    app.log_manager.entries.len()
                                ))
                                .strong(),
                            )
                            .clicked()
                        {
                            app.set_current_window(WindowRoute::Logs);
                        }
                        if ui
                            .button(RichText::new("\u{2795} Create Group").strong())
                            .clicked()
                        {
                            app.set_current_window(WindowRoute::Groups(GroupRoute::Create));
                        }
                    });
                });
                ui.add_space(4.0);
                ui.separator();

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
