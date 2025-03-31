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
        let state = Self::load_state();
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
        self.handle_dropped_file(ctx);
        self.draw_top_panel(ctx);
        self.draw_main_panel(ctx);
        self.draw_group_window(ctx);
        self.draw_edit_group_window(ctx);
        self.draw_log_window(ctx);
    }
}

impl CpuAffinityApp {
    pub fn draw_top_panel(&mut self, ctx: &egui::Context) {
        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (icon, label) = match self.theme_index {
                    0 => ("💻", "System theme"),
                    1 => ("☀", "Light theme"),
                    _ => ("🌙", "Dark theme"),
                };
                if ui.button(icon).on_hover_text(label).clicked() {
                    self.toggle_theme(ctx);
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
    }

    pub fn draw_main_panel(&mut self, ctx: &egui::Context) {
        CentralPanel::default().show(ctx, |ui| {
            let mut dropped_assigned = false;
            ScrollArea::vertical().show(ui, |ui| {
                dropped_assigned = self.render_groups(ui, ctx);
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
                            Self::save_state(&self.state);
                            break;
                        }
                    }
                }
            }
        });
    }

    fn render_groups(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut dropped_assigned = false;
        let mut modified = false;

        self.state.groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        
        // Take ownership of dropped_file to avoid borrowing conflicts
        let dropped_file = self.dropped_file.take();
        let mut edit_index = None;
        let mut run_program: Option<(usize, std::path::PathBuf)> = None;
        let mut remove_program: Option<(usize, std::path::PathBuf)> = None;

        for (i, group) in self.state.groups.iter_mut().enumerate() {
            Frame::group(ui.style()).outer_margin(5.0).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&group.name).heading())
                            .on_hover_text(RichText::new(format!("cores: {:?}", group.cores)).weak());
                        ui.with_layout(Layout::right_to_left(egui::Align::TOP), |ui| {
                            if ui.button("⚙").on_hover_text("Edit group settings").clicked() {
                                edit_index = Some(i);
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

                    ScrollArea::vertical().id_salt(i).show(ui, |ui| {
                        if group.programs.is_empty() {
                            ui.label("No executables. Drag & drop to add.");
                        } else {
                            for prog in group.programs.clone() {
                                let label = prog.file_name().map_or_else(|| prog.display().to_string(), |n| n.to_string_lossy().to_string());
                                ui.horizontal(|ui| {
                                    let app_name = format!("▶  {}", label);
                                    let button = egui::Button::new(RichText::new(app_name));
                                    let response = ui.add_sized([
                                        ui.available_width() - 30.0,
                                        24.0
                                    ], button);

                                    let delete = ui.button("❌").on_hover_text("Remove from group");

                                    if response.on_hover_text(prog.to_str().unwrap_or("")).clicked() {
                                        run_program = Some((i, prog.clone()));
                                    }
                                    if delete.clicked() {
                                        remove_program = Some((i, prog.clone()));
                                        modified = true;
                                    }
                                });
                            }
                        }
                    });

                    if let Some(dropped) = &dropped_file {
                        let rect = ui.min_rect();
                        if rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                            group.programs.push(dropped.clone());
                            dropped_assigned = true;
                            modified = true;
                        }
                    }
                });
            });
        }

        // Handle actions outside of the iterator
        if let Some(index) = edit_index {
            self.edit_group_index = Some(index);
        }
        if let Some((index, prog)) = run_program {
            self.run_program_with_affinity(index, prog);
        }
        if let Some((index, prog)) = remove_program {
            self.remove_program_from_group(index, &prog);
        }
        
        // Only put dropped_file back if it wasn't assigned to a group
        if !dropped_assigned {
            self.dropped_file = dropped_file;
        }

        if modified {
            Self::save_state(&self.state);
        }

        dropped_assigned
    }

    pub fn draw_group_window(&mut self, ctx: &egui::Context) {
        if !self.show_group_window {
            return;
        }

        let mut close = false;
        Window::new("Create Core Group").open(&mut true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Group name:");
                ui.text_edit_singleline(&mut self.new_group_name);
            });

            ui.label("Select CPU cores:");
            ui.horizontal_wrapped(|ui| {
                for (i, selected) in self.core_selection.iter_mut().enumerate() {
                    ui.checkbox(selected, format!("Core {}", i));
                }
            });

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("✅ Create").clicked() {
                    self.create_group();
                    close = true;
                }
                if ui.button("❌ Cancel").clicked() {
                    close = true;
                }
            });
        });

        if close {
            self.new_group_name.clear();
            self.core_selection.fill(false);
            self.show_group_window = false;
        }
    }

    pub fn draw_edit_group_window(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.edit_group_index {
            if index >= self.state.groups.len() {
                self.edit_group_index = None;
                self.edit_group_selection = None;
                return;
            }

            if self.edit_group_selection.is_none() {
                let mut selection = vec![false; num_cpus::get()];
                for &core in &self.state.groups[index].cores {
                    if core < selection.len() {
                        selection[core] = true;
                    }
                }
                self.edit_group_selection = Some(selection);
            }

            let mut open = true;
            Window::new("Edit Group Settings").open(&mut open).show(ctx, |ui| {
                ui.label(format!("Editing group: {}", self.state.groups[index].name));
                ui.label("Select CPU cores:");

                if let Some(selection) = self.edit_group_selection.as_mut() {
                    ui.horizontal_wrapped(|ui| {
                        for (i, selected) in selection.iter_mut().enumerate() {
                            ui.checkbox(selected, format!("Core {}", i));
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("💾 Save").clicked() {
                            self.state.groups[index].cores = selection.iter().enumerate().filter_map(|(i, &v)| if v { Some(i) } else { None }).collect();
                            Self::save_state(&self.state);
                            self.edit_group_index = None;
                        }
                        if ui.button("❌ Delete Group").clicked() {
                            self.state.groups.remove(index);
                            Self::save_state(&self.state);
                            self.edit_group_index = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.edit_group_index = None;
                        }
                    });
                }
            });

            if !open {
                self.edit_group_index = None;
                self.edit_group_selection = None;
            }
        }
    }

    pub fn draw_log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log_window {
            return;
        }

        let mut open = true;
        Window::new("Execution Log").resizable(true).open(&mut open).show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                for log in &self.log_text {
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
                self.log_text.clear();
            }
        });

        self.show_log_window = open;
    }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme_index = (self.theme_index + 1) % 3;
        let visuals = match self.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
    }

    fn handle_dropped_file(&mut self, ctx: &egui::Context) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
        }
    }

    fn create_group(&mut self) {
        let name_str = self.new_group_name.trim();
        if name_str.is_empty() {
            return;
        }

        let cores: Vec<_> = self.core_selection.iter().enumerate()
            .filter_map(|(i, &v)| if v { Some(i) } else { None })
            .collect();
        if cores.is_empty() {
            return;
        }

        self.state.groups.push(CoreGroup {
            name: name_str.to_string(),
            cores,
            programs: vec![],
        });

        self.new_group_name.clear();
        self.core_selection.fill(false);
        self.show_group_window = false;
        Self::save_state(&self.state);
    }

    fn save_log(&mut self, message: String) {
        self.log_text.push(message);
    }

    fn remove_program_from_group(&mut self, group_index: usize, prog_path: &std::path::Path) {
        if let Some(group) = self.state.groups.get_mut(group_index) {
            group.programs.retain(|p| p != prog_path);
            Self::save_state(&self.state);
        }
    }

    fn run_program_with_affinity(&mut self, group_index: usize, prog_path: std::path::PathBuf) {
        if let Some(group) = self.state.groups.get(group_index) {
            let label = prog_path.file_name()
                .map_or_else(|| prog_path.display().to_string(), |n| n.to_string_lossy().to_string());
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let ts = format!("{:02}:{:02}:{:02}", (now.as_secs() % 86400) / 3600, (now.as_secs() % 3600) / 60, now.as_secs() % 60);

            let cores = group.cores.clone(); // Clone cores to avoid borrowing issues
            self.save_log(format!("[{}] Starting '{}', app: {}", ts, label, prog_path.display()));

            match crate::affinity::run_with_affinity(prog_path.clone(), &cores) {
                Ok(_) => self.save_log(format!("[{}] OK: started '{}'", ts, label)),
                Err(e) => self.save_log(format!("[{}] ERROR: {}", ts, e)),
            }
        }
    }

    fn load_state() -> AppState {
        let path = std::env::current_exe().map(|mut p| {
            p.set_file_name("state.json");
            p
        }).unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path).ok()
            .and_then(|data| serde_json::from_str::<AppState>(&data).ok())
            .unwrap_or_else(|| AppState { groups: vec![] })
    }

    fn save_state(state: &AppState) {
        if let Ok(json) = serde_json::to_string_pretty(state) {
            let _ = std::fs::write("state.json", json);
        }
    }
}