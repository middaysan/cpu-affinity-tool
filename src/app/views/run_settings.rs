use eframe::egui::{Context, CentralPanel, ComboBox, Frame};
use crate::app::app_models::CpuAffinityApp;

pub fn draw_app_run_settings(app: &mut CpuAffinityApp, ctx: &Context) {

    CentralPanel::default().show(ctx, |ui| {
        Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
            let (group_idx, prog_idx) = match app.apps.edit_run_settings {
                Some((ref mut g, ref mut p)) => (g, p),
                None => return,
            };

            if app.apps.edit.is_none() {
                let original = app.state.groups[*group_idx].programs[*prog_idx].clone();
                app.apps.edit = Some(original);
            }

            let selected_app = app
                .apps.edit
                .as_mut()
                .expect("edit_app_clone must be initialized");

            ui.horizontal(|ui| {
                ui.label("App name:");
                ui.text_edit_singleline(&mut selected_app.name).changed()
            });

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

            if ui.button("Add Argument").clicked() {
                selected_app.args.push(String::new());
            }

            ui.separator();

            let mut is_close = false;
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    app.state.groups[*group_idx].programs[*prog_idx] = selected_app.clone();
                    app.state.save_state();
                    is_close = true;
                }
                if ui.button("Cancel").clicked() {
                    is_close = true;
                }
            });

            if is_close {
                app.set_current_controller(crate::app::controllers::WindowController::Groups(crate::app::controllers::Group::ListGroups));
            }
        });
    });
}
