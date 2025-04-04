use eframe::egui::{self, Window};
use crate::app::CpuAffinityApp;

pub fn draw_app_run_settings(app: &mut CpuAffinityApp, ctx: &egui::Context) {
    if !app.show_app_run_settings {
        return;
    }

    let was_open = app.show_app_run_settings;
    let mut need_to_close = false;

    Window::new("App run settings")
        .resizable(true)
        .open(&mut app.show_app_run_settings)
        .show(ctx, |ui| {
            let (group_idx, prog_idx) = match app.edit_app_to_run_settings {
                Some((ref mut g, ref mut p)) => (g, p),
                None => return,
            };

            if app.edit_app_clone.is_none() {
                let original = app.state.groups[*group_idx].programs[*prog_idx].clone();
                app.edit_app_clone = Some(original);
            }

            let selected_app = app
                .edit_app_clone
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
                egui::ComboBox::from_label("")
                    .selected_text(format!("{:?}", selected_app.priority))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::Idle, "Idle");
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::BelowNormal, "Below Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::Normal, "Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::AboveNormal, "Above Normal");
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::High, "High");
                        ui.selectable_value(&mut selected_app.priority, crate::app::PriorityClass::Realtime, "RealTime");
                    });
            });

            ui.label("Arguments:");
            let mut arg_to_remove: Option<usize> = None;
            if selected_app.args.is_empty() {
                ui.label("No arguments. Add one below.");
            } else {
                egui::Frame::group(ui.style()).show(ui, |ui| {
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

            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    app.state.groups[*group_idx].programs[*prog_idx] = selected_app.clone();
                    need_to_close = true;
                }
                if ui.button("Cancel").clicked() {
                    need_to_close = true;
                }
            });
        });

    if need_to_close {
        println!("Save or Cancel clicked");
        app.show_app_run_settings = false;
        app.edit_app_clone = None;
    }

    if was_open && !app.show_app_run_settings {
        println!("Window closed");
        app.edit_app_clone = None;
    }
}
