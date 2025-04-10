use eframe::egui::{self, CentralPanel, ComboBox, Context, Frame, Layout, Align};
use crate::app::app_models::AffinityAppState;

pub fn draw_app_run_settings(app: &mut AffinityAppState, ctx: &Context) {

    CentralPanel::default().show(ctx, |ui| {
        let mut is_close = false;

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
            let (group_idx, prog_idx) = match app.app_edit_state.run_settings {
                Some((ref mut g, ref mut p)) => (g, p),
                None => return,
            };

            if app.app_edit_state.current_edit.is_none() {
                let original = app.persistent_state.groups[*group_idx].programs[*prog_idx].clone();
                app.app_edit_state.current_edit = Some(original);
            }

            let selected_app = app
                .app_edit_state.current_edit
                .as_mut()
                .expect("edit_app_clone must be initialized");

            ui.horizontal(|ui| {
                ui.label("App name:");
                ui.text_edit_singleline(&mut selected_app.name).changed()
            });

            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label("Binary path:");
                let mut bin_path_str = selected_app.bin_path.to_string_lossy().to_string();
                if ui.text_edit_singleline(&mut bin_path_str).changed() {
                    selected_app.bin_path = std::path::PathBuf::from(bin_path_str);
                }

                if ui.button("📁add").on_hover_text("Add executables...").clicked() {
                    // TODO: add linux support
                    if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe"]).pick_file() {
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
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::Idle, "Idle");
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::BelowNormal, "Below Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::Normal, "Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::AboveNormal, "Above Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::High, "High");
                        ui.selectable_value(&mut selected_app.priority, crate::app::os_cmd::PriorityClass::Realtime, "RealTime");
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
                            if ui.button("❌").clicked() {
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
                if ui.add_sized(egui::vec2(100.0, 30.0), egui::Button::new("Save")).clicked() {
                    app.persistent_state.groups[*group_idx].programs[*prog_idx] = selected_app.clone();
                    app.persistent_state.save_state();
                    is_close = true;
                }
                if ui.add_sized(egui::vec2(100.0, 30.0), egui::Button::new("Cancel")).clicked() {
                    is_close = true;
                }
            });


        });

        if is_close {
            app.app_edit_state.current_edit = None;
            app.app_edit_state.run_settings = None;
            app.set_current_controller(crate::app::controllers::WindowController::Groups(crate::app::controllers::Group::ListGroups));
        }
    });
}
