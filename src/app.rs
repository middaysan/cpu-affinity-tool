use eframe::egui::{self, RichText, ScrollArea, Frame, Layout, TopBottomPanel, CentralPanel, Window, Vec2};
use crate::models::{AppState, CoreGroup};

pub struct CpuAffinityApp {
    state: AppState,
    core_selection: Vec<bool>,
    new_group_name: String,
    dropped_file: Option<std::path::PathBuf>,
    show_group_window: bool,
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
            show_log_window: false,
            edit_group_index: None,
            edit_group_selection: None,
        }
    }
}

impl eframe::App for CpuAffinityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        handle_dropped_file(ctx, &mut self.dropped_file);
        CentralPanel::default().show(ctx, |ui| {
            main_panel(ui, ctx, self);
        });
        group_window(ctx, self);
        edit_group_window(ctx, self);
        bottom_panel(ctx, self);
        log_window(ctx, self);
    }
}

fn main_panel(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if ui.button("➕ Create Core Group").clicked() {
        app.show_group_window = true;
    }

    let mut dropped_assigned = false;

    ScrollArea::vertical().show(ui, |ui| {
        dropped_assigned = render_groups(ui, ctx, app);
    });

    if let Some(dropped) = &app.dropped_file {
        if !dropped_assigned {
            ui.separator();
            ui.label(RichText::new("Dropped file:").strong());
            ui.label(dropped.display().to_string());
            for group in &mut app.state.groups {
                if ui.button(format!("Add to group '{}'", group.name)).clicked() {
                    group.programs.push(dropped.clone());
                    app.dropped_file = None;
                    save_state(&app.state);
                    break;
                }
            }
        }
    }
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


// В edit_group_window теперь сохраняем состояние выбора между кадрами
fn edit_group_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if let Some(index) = app.edit_group_index {
        if index >= app.state.groups.len() {
            app.edit_group_index = None;
            app.edit_group_selection = None;
            return;
        }

        // lazy init
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
        Window::new("Edit Group Settings")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("Editing group: {}", app.state.groups[index].name));
                ui.label("Select CPU cores:");

                if let Some(selection) = &mut app.edit_group_selection {
                    ui.horizontal_wrapped(|ui| {
                        for (i, selected) in selection.iter_mut().enumerate() {
                            ui.checkbox(selected, format!("Core {}", i));
                        }
                    });
        
                    let mut save = false;
                    let mut delete = false;
                    let mut cancel = false;
                    
                    ui.horizontal(|ui| {
                        save = ui.button("💾 Save").clicked();
                        delete = ui.button("❌ Delete Group").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                    
                    if save {
                        app.state.groups[index].cores = selection.iter().enumerate()
                            .filter_map(|(i, &v)| if v { Some(i) } else { None })
                            .collect();
                        save_state(&app.state);
                        app.edit_group_index = None;
                        app.edit_group_selection = None;
                    }
                    if delete {
                        app.state.groups.remove(index);
                        save_state(&app.state);
                        app.edit_group_index = None;
                        app.edit_group_selection = None;
                    }
                    if cancel {
                        app.edit_group_index = None;
                        app.edit_group_selection = None;
                    }
                }
            });

        if !open {
            app.edit_group_index = None;
            app.edit_group_selection = None;
        }
    }
}

fn render_groups(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut CpuAffinityApp) -> bool {
    let mut dropped_assigned = false;
    let mut modified = false;

    for (i, group) in app.state.groups.iter_mut().enumerate() {
        Frame::group(ui.style())
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_min_height(60.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&group.name).heading());
                    ui.label(RichText::new(format!("Cores: {:?}", group.cores)).weak());
                    ui.with_layout(Layout::right_to_left(egui::Align::RIGHT), |ui| {
                        if ui.button("⚙").on_hover_text("Edit group settings").clicked() {
                            app.edit_group_index = Some(i);
                        }
                        if ui.button("📁").on_hover_text("Add executables...").clicked() {
                            if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe", "lnk"]).pick_files() {
                                group.programs.extend(paths);
                                modified = true;
                            }
                        }
                    });
                });

                ScrollArea::vertical()
                .max_height(160.0)
                .id_salt(format!("group_{}", i))
                .show(ui, |ui| {
                    if group.programs.is_empty() {
                        ui.label("No executables. Drag & drop to add.");
                    } else {
                        for prog in group.programs.clone() {
                            let label = prog.file_name()
                                .map_or_else(|| prog.display().to_string(), |n| n.to_string_lossy().to_string());

                            if ui.button(format!("▶ {}", label)).on_hover_text("Run with affinity").clicked() {
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default();
                                let ts = format!("{:02}:{:02}:{:02}",
                                    (now.as_secs() % 86400) / 3600,
                                    (now.as_secs() % 3600) / 60,
                                    now.as_secs() % 60);
                                if let Err(e) = crate::affinity::run_with_affinity(prog.clone(), &group.cores) {
                                    app.log_text.push(format!("[{}] {}", ts, e));
                                }
                            }
                        }
                    }

                    ui.separator();
                    ui.label("💡 Tip: You can drag & drop files to add them to this group.");
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
    }

    if modified {
        save_state(&app.state);
    }

    dropped_assigned
}

fn bottom_panel(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            let log_count = app.log_text.len();
            let text = if log_count > 0 {
                format!("📄 Logs ({})", log_count)
            } else {
                "📄 Logs".to_string()
            };
            if ui.button(text).clicked() {
                app.show_log_window = true;
            }
        });
    });
}

fn log_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if !app.show_log_window {
        return;
    }

    let mut open = true;
    Window::new("Execution Log").resizable(true).open(&mut open).show(ctx, |ui| {
        ScrollArea::vertical().show(ui, |ui| {
            for log in &app.log_text {
                ui.label(log);
            }
        });
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
