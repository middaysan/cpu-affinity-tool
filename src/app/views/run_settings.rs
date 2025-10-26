use crate::app::models::AppState;
use eframe::egui::{self, Align, CentralPanel, ComboBox, Context, Frame, Layout};
use os_api::PriorityClass;

pub fn draw_app_run_settings(app: &mut AppState, ctx: &Context) {
    // Extract group_idx and prog_idx early to avoid borrow checker issues
    let (group_idx, prog_idx) = match app.app_edit_state.run_settings {
        Some((g, p)) => (g, p), // Copy the values
        None => {
            // If no run settings, return to groups view
            app.set_current_window(crate::app::controllers::WindowController::Groups(
                crate::app::controllers::Group::ListGroups,
            ));
            return;
        }
    };

    // Initialize the current_edit if needed
    if app.app_edit_state.current_edit.is_none() {
        // Get program using helper method
        if let Some(original) = app.get_group_program(group_idx, prog_idx) {
            app.app_edit_state.current_edit = Some(original);
        } else {
            // If program not found, return to groups view
            app.app_edit_state.current_edit = None;
            app.app_edit_state.run_settings = None;
            app.set_current_window(crate::app::controllers::WindowController::Groups(
                crate::app::controllers::Group::ListGroups,
            ));
            return;
        }
    }

    // Variables to track UI state
    let mut is_close = false;
    let mut save_clicked = false;
    let mut run_clicked = false;
    let mut updated_app = None;

    CentralPanel::default().show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Edit App Run Settings");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("❌").on_hover_text("Close").clicked() {
                    is_close = true;
                }
            });
        });

        ui.separator();

        Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
            let selected_app = app
                .app_edit_state
                .current_edit
                .as_mut()
                .expect("edit_app_clone must be initialized");

            ui.horizontal(|ui| {
                ui.label("App name:");
                ui.text_edit_singleline(&mut selected_app.name).changed();
                if ui
                    .button("▶")
                    .on_hover_text("Test run the command")
                    .clicked()
                {
                    updated_app = Some(selected_app.clone());
                    run_clicked = true;
                };
            });

            ui.add_space(5.0);

            ui.checkbox(&mut selected_app.autorun, "Start this app on startup")
                .on_hover_text("This app will be started when the group is started.");

            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label("Binary path:");
                let mut bin_path_str = selected_app.bin_path.to_string_lossy().to_string();
                if ui.text_edit_singleline(&mut bin_path_str).changed() {
                    selected_app.bin_path = std::path::PathBuf::from(bin_path_str);
                }

                if ui
                    .button("📁add")
                    .on_hover_text("Add executables...")
                    .clicked()
                {
                    // TODO: add linux support
                    if let Some(paths) = rfd::FileDialog::new()
                        .add_filter("Executables", &["exe"])
                        .pick_file()
                    {
                        selected_app.bin_path = paths.clone();
                    }
                }
            });

            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label("Priority:");
                ComboBox::from_label("")
                    .selected_text(format!("{:?}", selected_app.priority))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::Idle,
                            "Idle",
                        );
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::BelowNormal,
                            "Below Normal",
                        );
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::Normal,
                            "Normal",
                        );
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::AboveNormal,
                            "Above Normal",
                        );
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::High,
                            "High",
                        );
                        ui.selectable_value(
                            &mut selected_app.priority,
                            PriorityClass::Realtime,
                            "RealTime",
                        );
                    });
            });

            ui.add_space(5.0);

            ui.label("Arguments:");
            let mut arg_to_remove: Option<usize> = None;
            if selected_app.args.is_empty() {
                ui.label("No arguments. Add one below.");
            } else {
                Frame::group(ui.style()).show(ui, |ui| {
                    for (i, arg) in selected_app.args.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Arg {}:", i + 1));
                            ui.text_edit_singleline(arg);
                            if ui.button("remove 🗑").clicked() {
                                arg_to_remove = Some(i);
                            }
                        });
                    }
                });
            }

            if let Some(idx) = arg_to_remove {
                selected_app.args.remove(idx);
            }

            ui.separator();

            if ui.button("Add Argument").clicked() {
                selected_app.args.push(String::new());
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui
                    .add_sized(egui::vec2(100.0, 30.0), egui::Button::new("Save"))
                    .clicked()
                {
                    // Store the updated app for later use
                    updated_app = Some(selected_app.clone());
                    save_clicked = true;
                    is_close = true;
                }
                if ui
                    .add_sized(egui::vec2(100.0, 30.0), egui::Button::new("Cancel"))
                    .clicked()
                {
                    is_close = true;
                }
            });
        });
    });



    // Handle save outside of any closures
    if run_clicked {
        run_clicked = false;
        if updated_app.is_some() {
            // app.update_program(group_idx, prog_idx, updated);
            app.run_app_with_affinity_sync(group_idx, prog_idx, updated_app.clone().unwrap());

        }
    }


    // Handle save outside of any closures
    if save_clicked {
        if let Some(updated) = updated_app {
            app.update_program(group_idx, prog_idx, updated);
        }
    }

    // Handle close outside of any closures
    if is_close {
        app.app_edit_state.current_edit = None;
        app.app_edit_state.run_settings = None;
        app.set_current_window(crate::app::controllers::WindowController::Groups(
            crate::app::controllers::Group::ListGroups,
        ));
    }
}
