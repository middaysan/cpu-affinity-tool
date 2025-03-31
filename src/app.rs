use eframe::egui::{self};
use crate::models::{AppState, CoreGroup};

pub struct CpuAffinityApp {
    state: AppState,
    core_selection: Vec<bool>,
    new_group_name: String,
    dropped_file: Option<std::path::PathBuf>,
    show_group_window: bool,
    log_text: Vec<String>,
    show_log_window: bool,
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
        }
    }
}

impl eframe::App for CpuAffinityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        handle_dropped_file(ctx, &mut self.dropped_file);
        main_panel(ctx, self);
        group_window(ctx, self);
        bottom_panel(ctx, self);
        log_window(ctx, self);
    }
}

// --- UI Views ---
fn main_panel(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if ui.button("Create core group").clicked() {
            app.show_group_window = true;
        }

        let mut dropped_assigned = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            dropped_assigned = render_groups(ui, ctx, app);
        });

        if let Some(dropped) = &app.dropped_file {
            if !dropped_assigned {
                let dropped_path = dropped.clone();
                render_dropped_file(ui, &dropped_path, app);
            }
        }
    });
}

fn group_window(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    if !app.show_group_window {
        return;
    }

    let mut close = false;

    egui::Window::new("Create core group").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Group name:");
            ui.text_edit_singleline(&mut app.new_group_name);
        });

        ui.label("Select cores of CPU:");
        ui.horizontal_wrapped(|ui| {
            for (i, selected) in app.core_selection.iter_mut().enumerate() {
                ui.checkbox(selected, format!("Core {}", i));
            }
        });

        ui.horizontal(|ui| {
            if ui.button("Create").clicked() {
                if let Some(group) = create_core_group(&mut app.new_group_name, &mut app.core_selection) {
                    app.state.groups.push(group);
                    save_state(&app.state);
                    close = true;
                }
            }
            if ui.button("Cancel").clicked() {
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

fn bottom_panel(ctx: &egui::Context, app: &mut CpuAffinityApp) {
    egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            let log_count = app.log_text.len();
            let button_text = if log_count > 0 {
                format!("Logs ({})", log_count)
            } else {
                "Logs".to_string()
            };
            
            if ui.button(button_text).clicked() {
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
    egui::Window::new("Log").resizable(true).open(&mut open).show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for log in &app.log_text {
                ui.label(log);
            }
        });
    });
    app.show_log_window = open;
}

// --- Components ---
fn render_groups(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut CpuAffinityApp) -> bool {
    let mut dropped_assigned = false;
    let mut to_remove = vec![];
    let mut modified = false;

    for (i, group) in app.state.groups.iter_mut().enumerate() {
        ui.separator();
        let mut remove = false;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&group.name).underline());
            ui.label(format!("cores: {:?}", group.cores));
            if ui.button("X").clicked() {
                remove = true;
            }
        });

        ui.group(|ui| {
            if group.programs.is_empty() {
                if ui.button("Open exe file…").clicked() {
                    if let Some(paths) = rfd::FileDialog::new().add_filter("Executables", &["exe", "lnk"]).pick_files() {
                        group.programs.extend(paths);
                        modified = true;
                    }
                }
                ui.label("Open or drag files here to add them to the group.");
            } else {
                for prog in group.programs.clone() {
                    ui.horizontal(|ui| {
                        if ui.button("▶").clicked() {
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
                        ui.label(prog.file_name().map_or_else(|| prog.display().to_string(), |n| n.to_string_lossy().to_string()));
                        if ui.button("Delete").clicked() {
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

        if remove {
            to_remove.push(i);
        }
    }

    for i in to_remove.into_iter().rev() {
        app.state.groups.remove(i);
        modified = true;
    }

    if modified {
        save_state(&app.state);
    }

    dropped_assigned
}

fn render_dropped_file(ui: &mut egui::Ui, dropped: &std::path::PathBuf, app: &mut CpuAffinityApp) {
    ui.separator();
    ui.label(format!("Waiting to be added: {}", dropped.display()));
    for group in &mut app.state.groups {
        if ui.button(format!("Add to group {}", group.name)).clicked() {
            group.programs.push(dropped.clone());
            app.dropped_file = None;
            save_state(&app.state);
            break;
        }
    }
}

// --- Utils ---
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
