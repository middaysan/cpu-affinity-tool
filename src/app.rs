use eframe::egui::{self, RichText, ScrollArea, Frame, Layout, TopBottomPanel, CentralPanel, Window, Color32};
use crate::models::{AppState, CoreGroup};

pub struct CpuAffinityApp {
    state: AppState,
    core_selection: Vec<bool>,
    new_group_name: String,
    dropped_file: Option<std::path::PathBuf>,
    show_group_window: bool,
    theme_index: usize,
    log_text: Vec<String>,
    show_log_window: bool,
    edit_group_index: Option<usize>,
    edit_group_selection: Option<Vec<bool>>,
}

impl Default for CpuAffinityApp {
    fn default() -> Self {
        let state = load_state();
        let num_cores = num_cpus::get();
        Self {
            state,
            core_selection: vec![false; num_cores],
            new_group_name: String::new(),
            dropped_file: None,
            show_group_window: false,
            log_text: Vec::new(),
            theme_index: 0,
            show_log_window: false,
            edit_group_index: None,
            edit_group_selection: None,
        }
    }
}

impl eframe::App for CpuAffinityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        handle_dropped_file(ctx, &mut self.dropped_file);

        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (icon, label) = match self.theme_index {
                    0 => ("💻", "System theme"),
                    1 => ("☀", "Light theme"),
                    _ => ("🌙", "Dark theme"),
                };
                
                if ui.button(format!("{icon}")).on_hover_text(label).clicked() {
                    self.theme_index = (self.theme_index + 1) % 3;
                    let visuals = match self.theme_index {
                        0 => egui::Visuals::default(),
                        1 => egui::Visuals::light(),
                        _ => egui::Visuals::dark(),
                    };
                    ctx.set_visuals(visuals);
                }

                ui.separator();

                ui.label(RichText::new("Core Groups").heading());
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(format!("📄 View Logs({})", self.log_text.len())).clicked() {
                    self.show_log_window = true;
                    }

                    if ui.button("➕ Create Core Group").clicked() {
                    self.show_group_window = true;
                    }
                });
            });
            ui.separator();
            ui.label("💡 Tip: Drag & drop executable files (.exe/.lnk) onto a group to add them, then click ▶ to run with the assigned CPU cores");
        });



        CentralPanel::default().show(ctx, |ui| {
            let mut dropped_assigned = false;
            ScrollArea::vertical().show(ui, |ui| {
                dropped_assigned = render_groups(ui, ctx, self);
            });

            if let Some(dropped) = &self.dropped_file {
                if !dropped_assigned {
                    ui.separator();
                    ui.label(RichText::new("Dropped file:").strong());
                    ui.label(dropped.display().to_string());
                    for group in &mut self.state.groups {
                        if ui.button(format!("Add to group '{}'", group.name)).clicked() {
                            group.programs.push(dropped.clone());
                            self.dropped_file = None;
                            save_state(&self.state);
                            break;
                        }
                    }
                }
            }
        });

        group_window(ctx, self);
        edit_group_window(ctx, self);
        log_window(ctx, self);
    }
}

fn render_groups(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut CpuAffinityApp) -> bool {
    let mut dropped_assigned = false;
    let mut modified = false;

    app.state.groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    for (i, group) in app.state.groups.iter_mut().enumerate() {
        Frame::group(ui.style())
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&group.name).heading()).on_hover_text(RichText::new(format!("cores: {:?}", group.cores)).weak());
                        ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui.button("⚙gsettings").on_hover_text("Edit group settings").clicked() {
                                app.edit_group_index = Some(i);
                            }
                            if ui.button("📁add").on_hover_text("Add executables...").clicked() {
                                if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe", "lnk"]).pick_files() {
                                    group.programs.extend(paths);
                                    modified = true;
                                }
                            }
                        });
                    });

                    ui.separator();

                    ScrollArea::vertical()
                    .id_salt(i)
                    .show(ui, |ui| {
                        if group.programs.is_empty() {
                            ui.label("No executables. Drag & drop to add.");
                        } else {
                            for prog in group.programs.clone() {
                                let label = prog.file_name().map_or_else(|| prog.display().to_string(), |n| n.to_string_lossy().to_string());
                                ui.horizontal(|ui| {
                                    let app_name = format!("▶  {}", label);
                                    let button = egui::Button::new(RichText::new(app_name)); // Create a button widget
                                    let response = ui.add_sized(
                                        [ui.available_width() - 30.0, 24.0],
                                        button
                                    );
                                    let delete = ui.button("❌").on_hover_text("Remove from group");
                                    if response.on_hover_text("Run with affinity").clicked() {
                                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                        let ts = format!("{:02}:{:02}:{:02}", (now.as_secs() % 86400) / 3600, (now.as_secs() % 3600) / 60, now.as_secs() % 60);
                                        app.log_text.push(format!("[{}] Starting '{}', app: {}", ts, label, prog.display()));
                                        if let Err(e) = crate::affinity::run_with_affinity(prog.clone(), &group.cores) {
                                            app.log_text.push(format!("[{}] ERROR: {}", ts, e));
                                        } else {
                                            app.log_text.push(format!("[{}] OK: started '{}'", ts, label));
                                        }
                                    }

                                    if delete.clicked() {
                                        group.programs.retain(|p| p != &prog);
                                        modified = true;
                                    }
                                });
                            }
                        }
                    });

                    if let Some(dropped) = &app.dropped_file {
                        let rect = ui.min_rect();
                        if rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                            group.programs.push(dropped.clone());
                            app.dropped_file = None;
                            dropped_assigned = true;
                            modified = true;
                        }
                    }
                });
            });
    }

    if modified {
        save_state(&app.state);
    }

    dropped_assigned
}

fn group_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if !app.show_group_window {
        return;
    }

    let mut close = false;
    Window::new("Create Core Group").open(&mut true).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Group name:");
            ui.text_edit_singleline(&mut app.new_group_name);
        });

        ui.label("Select CPU cores:");
        ui.horizontal_wrapped(|ui| {
            for (i, selected) in app.core_selection.iter_mut().enumerate() {
                ui.checkbox(selected, format!("Core {}", i));
            }
        });

        ui.separator();

        ui.horizontal(|ui| {
            if ui.button("✅ Create").clicked() {
                if let Some(group) = create_core_group(&mut app.new_group_name, &mut app.core_selection) {
                    app.state.groups.push(group);
                    save_state(&app.state);
                    close = true;
                }
            }
            if ui.button("❌ Cancel").clicked() {
                close = true;
            }
        });
    });

    if close {
        app.new_group_name.clear();
        app.core_selection.fill(false);
        app.show_group_window = false;
    }
}

fn edit_group_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if let Some(index) = app.edit_group_index {
        if index >= app.state.groups.len() {
            app.edit_group_index = None;
            app.edit_group_selection = None;
            return;
        }

        if app.edit_group_selection.is_none() {
            let mut selection = vec![false; num_cpus::get()];
            for &core in &app.state.groups[index].cores {
                if core < selection.len() {
                    selection[core] = true;
                }
            }
            app.edit_group_selection = Some(selection);
        }

        let mut open = true;
        Window::new("Edit Group Settings").open(&mut open).show(ctx, |ui| {
            ui.label(format!("Editing group: {}", app.state.groups[index].name));
            ui.label("Select CPU cores:");

            if let Some(selection) = app.edit_group_selection.take() {
                let mut selection = selection;
                ui.horizontal_wrapped(|ui| {
                    for (i, selected) in selection.iter_mut().enumerate() {
                        ui.checkbox(selected, format!("Core {}", i));
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("💾 Save").clicked() {
                        app.state.groups[index].cores = selection.iter().enumerate().filter_map(|(i, &v)| if v { Some(i) } else { None }).collect();
                        save_state(&app.state);
                        app.edit_group_index = None;
                    }
                    if ui.button("❌ Delete Group").clicked() {
                        app.state.groups.remove(index);
                        save_state(&app.state);
                        app.edit_group_index = None;
                    }
                    if ui.button("Cancel").clicked() {
                        app.edit_group_index = None;
                    }
                });

                app.edit_group_selection = Some(selection);
            }
        });

        if !open {
            app.edit_group_index = None;
            app.edit_group_selection = None;
        }
    }
}

fn log_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if !app.show_log_window {
        return;
    }

    let mut open = true;
    Window::new("Execution Log").resizable(true).open(&mut open).show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            for log in &app.log_text {
                let color = if log.contains("ERROR") {
                    Color32::RED
                } else if log.contains("OK") {
                    Color32::GREEN
                } else {
                    Color32::LIGHT_GRAY
                };
                ui.label(RichText::new(log).color(color));
            }
        });
        if ui.button("Clear Logs").clicked() {
            app.log_text.clear();
        }
    });

    app.show_log_window = open;
}

fn handle_dropped_file(ctx: &egui::Context, dropped_file: &mut Option<std::path::PathBuf>) {
    if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
        *dropped_file = Some(path);
    }
}

fn create_core_group(name: &mut String, selection: &mut Vec<bool>) -> Option<CoreGroup> {
    let name_str = name.trim();
    if name_str.is_empty() {
        return None;
    }

    let cores: Vec<_> = selection.iter().enumerate().filter_map(|(i, &v)| if v { Some(i) } else { None }).collect();
    if cores.is_empty() {
        return None;
    }

    let group = CoreGroup {
        name: name_str.to_string(),
        cores,
        programs: vec![],
    };
    name.clear();
    selection.fill(false);
    Some(group)
}

fn state_file_path() -> std::path::PathBuf {
    std::env::current_exe().map(|mut p| { p.set_file_name("state.json"); p }).unwrap_or_else(|_| "state.json".into())
}

fn load_state() -> AppState {
    let path = state_file_path();
    std::fs::read_to_string(&path).ok()
        .and_then(|data| serde_json::from_str::<AppState>(&data).ok())
        .unwrap_or_else(|| AppState { groups: vec![] })
}

fn save_state(state: &AppState) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(state_file_path(), json);
    }
}