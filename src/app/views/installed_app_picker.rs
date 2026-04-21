use crate::app::runtime::AppState;
use eframe::egui::{
    self, Align, CentralPanel, Context, Key, Layout, RichText, ScrollArea, Stroke, TextEdit,
};

enum InstalledAppPickerAction {
    Close,
    SetQuery(String),
    Refresh,
    SelectEntry(usize),
    SelectNext,
    SelectPrevious,
    ConfirmSelection,
}

#[cfg(target_os = "windows")]
fn installed_app_picker_heading() -> (&'static str, &'static str) {
    (
        "Find Installed App",
        "Browse supported Start-backed apps from the current Windows installation.",
    )
}

#[cfg(target_os = "linux")]
fn installed_app_picker_heading() -> (&'static str, &'static str) {
    (
        "Find Installed App",
        "Browse apps discovered from desktop entries. Typing a search also surfaces matching executables from PATH.",
    )
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn installed_app_picker_heading() -> (&'static str, &'static str) {
    ("Find Installed App", "Browse supported installed apps.")
}

pub fn draw_installed_app_picker(app: &mut AppState, ctx: &Context) {
    if app.ui.installed_app_picker.target_group_index.is_none() {
        app.close_installed_app_picker();
        return;
    }

    let snapshot = app.build_installed_app_picker_snapshot();
    let mut actions = Vec::new();
    let needs_focus = app.take_installed_app_picker_focus_request();

    CentralPanel::default().show(ctx, |ui| {
        let (heading, description) = installed_app_picker_heading();
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new(heading).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    actions.push(InstalledAppPickerAction::Close);
                }
            });
        });
        ui.label(RichText::new(description).small().weak());
        ui.add_space(10.0);

        let search_id = egui::Id::new("installed_app_picker_search");
        let mut query = snapshot.query.clone();

        ui.horizontal(|ui| {
            let response = ui.add_sized(
                [(ui.available_width() - 108.0).max(220.0), 28.0],
                TextEdit::singleline(&mut query)
                    .id(search_id)
                    .hint_text("Search installed apps..."),
            );

            if needs_focus {
                response.request_focus();
            }

            if response.changed() {
                actions.push(InstalledAppPickerAction::SetQuery(query.clone()));
            }

            if ui
                .add_enabled(
                    !snapshot.is_refreshing,
                    egui::Button::new("Refresh").min_size(egui::vec2(96.0, 28.0)),
                )
                .clicked()
            {
                actions.push(InstalledAppPickerAction::Refresh);
            }
        });

        ui.add_space(8.0);

        if snapshot.is_refreshing {
            ui.label(RichText::new("Loading installed apps...").weak().italics());
            ui.add_space(4.0);
        }

        if let Some(error) = &snapshot.last_error {
            ui.colored_label(
                egui::Color32::from_rgb(255, 140, 140),
                format!("Refresh failed: {error}"),
            );
            ui.add_space(6.0);
        }

        if snapshot.rows.is_empty() && !snapshot.is_refreshing && snapshot.last_error.is_none() {
            ui.label(RichText::new("No matching apps").weak().italics());
        }

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in &snapshot.rows {
                    let visuals = ui.visuals();
                    let fill = if row.selected {
                        visuals.selection.bg_fill
                    } else {
                        visuals.widgets.inactive.bg_fill
                    };
                    let stroke = if row.selected {
                        visuals.selection.stroke
                    } else {
                        Stroke::new(1.0, visuals.widgets.inactive.bg_stroke.color)
                    };

                    let frame = egui::Frame::new()
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(8, 3));

                    let inner = frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(&row.name).size(12.0));
                            ui.add_space(0.3);
                            ui.add(
                                egui::Label::new(RichText::new(&row.detail).small().weak()).wrap(),
                            );
                        });
                    });

                    let response = ui.interact(
                        inner.response.rect,
                        egui::Id::new(("installed_app_picker_row", row.entry_index)),
                        egui::Sense::click(),
                    );

                    if response.clicked() {
                        actions.push(InstalledAppPickerAction::SelectEntry(row.entry_index));
                    }
                    if response.double_clicked() {
                        actions.push(InstalledAppPickerAction::SelectEntry(row.entry_index));
                        actions.push(InstalledAppPickerAction::ConfirmSelection);
                    }

                    ui.add_space(1.0);
                }
            });

        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "If the app is not listed, use Open App with the direct path, launcher file, or portable binary.",
            )
            .small()
            .weak()
            .italics(),
        );
    });

    actions.extend(collect_keyboard_actions(ctx));
    execute_actions(app, actions);
}

fn collect_keyboard_actions(ctx: &Context) -> Vec<InstalledAppPickerAction> {
    let mut actions = Vec::new();

    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        actions.push(InstalledAppPickerAction::Close);
    }
    if ctx.input(|input| input.key_pressed(Key::ArrowDown)) {
        actions.push(InstalledAppPickerAction::SelectNext);
    }
    if ctx.input(|input| input.key_pressed(Key::ArrowUp)) {
        actions.push(InstalledAppPickerAction::SelectPrevious);
    }
    if ctx.input(|input| input.key_pressed(Key::Enter)) {
        actions.push(InstalledAppPickerAction::ConfirmSelection);
    }

    actions
}

fn execute_actions(app: &mut AppState, actions: Vec<InstalledAppPickerAction>) {
    for action in actions {
        match action {
            InstalledAppPickerAction::Close => {
                app.close_installed_app_picker();
            }
            InstalledAppPickerAction::SetQuery(query) => {
                app.set_installed_app_picker_query(query);
            }
            InstalledAppPickerAction::Refresh => {
                app.request_installed_app_picker_refresh();
            }
            InstalledAppPickerAction::SelectEntry(entry_index) => {
                app.select_installed_app_picker_entry(entry_index);
            }
            InstalledAppPickerAction::SelectNext => {
                app.select_next_installed_app_picker_entry();
            }
            InstalledAppPickerAction::SelectPrevious => {
                app.select_previous_installed_app_picker_entry();
            }
            InstalledAppPickerAction::ConfirmSelection => {
                let _ = app.confirm_selected_installed_app();
            }
        }
    }
}
