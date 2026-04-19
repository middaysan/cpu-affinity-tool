use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::AppState;
use crate::app::views::shared_elements::glass_frame;
use eframe::egui::{self, Align, CentralPanel, ComboBox, Context, Layout, RichText, Vec2};
use os_api::PriorityClass;
use std::path::PathBuf;

pub fn draw_app_run_settings(app: &mut AppState, ctx: &Context) {
    let (group_idx, prog_idx) = match app.ui.app_edit_state.run_settings {
        Some((g, p)) => (g, p),
        None => {
            app.set_current_window(WindowRoute::Groups(GroupRoute::List));
            return;
        }
    };

    if !app.ensure_current_edit_loaded(group_idx, prog_idx) {
        return;
    }

    let mut is_close = false;
    let mut save_clicked = false;
    let mut delete_clicked = false;

    CentralPanel::default().show(ctx, |ui| {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Edit App Settings").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close").clicked() {
                    is_close = true;
                }
            });
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            glass_frame(ui).show(ui, |ui| {
                let selected_app = app
                    .ui
                    .app_edit_state
                    .current_edit
                    .as_mut()
                    .expect("edit_app_clone must be initialized");

                egui::Grid::new("app_settings_grid")
                    .spacing(Vec2::new(10.0, 10.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new("App Name:").strong());
                        ui.text_edit_singleline(&mut selected_app.name);
                        ui.end_row();

                        if selected_app.is_path_target() {
                            ui.label(RichText::new("Binary Path:").strong());
                            ui.horizontal(|ui| {
                                let mut bin_path_str = selected_app
                                    .bin_path()
                                    .map(|path| path.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                if ui.text_edit_singleline(&mut bin_path_str).changed() {
                                    if let Some(bin_path) = selected_app.bin_path_mut() {
                                        *bin_path = PathBuf::from(bin_path_str);
                                    }
                                }

                                if ui
                                    .button("Browse")
                                    .on_hover_text("Select executable...")
                                    .clicked()
                                {
                                    if let Some(path) = rfd::FileDialog::new()
                                        .add_filter("Executables", &["exe"])
                                        .pick_file()
                                    {
                                        if let Some(bin_path) = selected_app.bin_path_mut() {
                                            *bin_path = path;
                                        }
                                    }
                                }
                            });
                            ui.end_row();
                        } else {
                            ui.label(RichText::new("Installed App:").strong());
                            ui.label(&selected_app.name);
                            ui.end_row();

                            ui.label(RichText::new("AUMID:").strong());
                            ui.label(
                                RichText::new(
                                    selected_app
                                        .installed_aumid()
                                        .unwrap_or("Unknown installed app"),
                                )
                                .small()
                                .monospace(),
                            );
                            ui.end_row();
                        }

                        ui.label(RichText::new("Priority:").strong());
                        ComboBox::from_id_salt("priority_combo")
                            .selected_text(format!("{:?}", selected_app.priority))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Realtime,
                                    "RealTime",
                                );
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::High,
                                    "High",
                                );
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::AboveNormal,
                                    "Above Normal",
                                );
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Normal,
                                    "Normal",
                                );
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::BelowNormal,
                                    "Below Normal",
                                );
                                ui.selectable_value(
                                    &mut selected_app.priority,
                                    PriorityClass::Idle,
                                    "Low",
                                );
                            });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.checkbox(
                    &mut selected_app.autorun,
                    RichText::new("Start this app on startup").strong(),
                );
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                if selected_app.is_args_editable() {
                    ui.label(RichText::new("Command Line Arguments:").strong());
                    ui.add_space(5.0);

                    let mut arg_to_remove = None;
                    if selected_app.args.is_empty() {
                        ui.label(RichText::new("No arguments defined.").weak().italics());
                    } else {
                        for (i, arg) in selected_app.args.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}:", i + 1));
                                ui.text_edit_singleline(arg);
                                if ui.button("Remove").clicked() {
                                    arg_to_remove = Some(i);
                                }
                            });
                        }
                    }

                    if let Some(idx) = arg_to_remove {
                        selected_app.args.remove(idx);
                    }

                    ui.add_space(5.0);
                    if ui.button("Add Argument").clicked() {
                        selected_app.args.push(String::new());
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                ui.label(RichText::new("Additional Process Names (Auto-track):").strong());
                ui.add_space(5.0);

                let mut proc_to_remove = None;
                if selected_app.additional_processes.is_empty() {
                    ui.label(
                        RichText::new("No additional processes defined.")
                            .weak()
                            .italics(),
                    );
                } else {
                    for (i, proc_name) in selected_app.additional_processes.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}:", i + 1));
                            ui.text_edit_singleline(proc_name);
                            if ui.button("Remove").clicked() {
                                proc_to_remove = Some(i);
                            }
                        });
                    }
                }

                if let Some(idx) = proc_to_remove {
                    selected_app.additional_processes.remove(idx);
                }

                ui.add_space(5.0);
                if ui.button("Add Process Name").clicked() {
                    selected_app.additional_processes.push(String::new());
                }

                ui.add_space(15.0);
                ui.separator();
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Save Changes").strong())
                                .min_size(egui::vec2(120.0, 32.0)),
                        )
                        .clicked()
                    {
                        save_clicked = true;
                        is_close = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(100.0, 32.0)))
                        .clicked()
                    {
                        is_close = true;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Remove from group").color(egui::Color32::RED),
                                )
                                .min_size(egui::vec2(150.0, 32.0)),
                            )
                            .on_hover_text("Remove this application from the group")
                            .clicked()
                        {
                            delete_clicked = true;
                            is_close = true;
                        }
                    });
                });
            });
        });
    });

    if save_clicked {
        app.commit_current_app_edit_session();
    } else if delete_clicked {
        app.delete_current_app_edit_target();
    } else if is_close {
        app.close_app_run_settings();
    }
}
