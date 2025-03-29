#![windows_subsystem = "windows"]

use eframe::egui::{CentralPanel, Checkbox, Context, ScrollArea, TextEdit, TopBottomPanel, Window};
use eframe::{run_native, App, NativeOptions};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::os::windows::io::AsRawHandle;
use parselnk::Lnk;
use windows::Win32::System::Threading::SetProcessAffinityMask;
use windows::Win32::Foundation::HANDLE;
use shlex;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoreGroup {
    name: String,
    cores: Vec<usize>,
    programs: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct AppState {
    groups: Vec<CoreGroup>,
}

struct CpuAffinityApp {
    num_cores: usize,
    core_selection: Vec<bool>,
    groups: Vec<CoreGroup>,
    new_group_name: String,
    dropped_file: Option<PathBuf>,
    show_group_window: bool,
}

impl Default for CpuAffinityApp {
    fn default() -> Self {
        let num_cores = num_cpus::get();
        let groups = Self::load_state();
        Self {
            num_cores,
            core_selection: vec![false; num_cores],
            groups,
            new_group_name: String::new(),
            dropped_file: None,
            show_group_window: false,
        }
    }
}

impl App for CpuAffinityApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_file(ctx);
        self.render_top_panel(ctx);
        self.render_main_panel(ctx);
        self.render_group_window(ctx);
    }
}

impl CpuAffinityApp {
    fn handle_dropped_file(&mut self, ctx: &Context) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
        }
    }

    fn render_top_panel(&self, ctx: &Context) {
        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("CPU Affinity Group Manager");
        });
    }

    fn render_main_panel(&mut self, ctx: &Context) {
        CentralPanel::default().show(ctx, |ui| {
            if ui.button("Создать группу ядер").clicked() {
                self.show_group_window = true;
            }

            ui.separator();
            ui.label("Группы и программы:");

            let mut dropped_assigned = false;

            ScrollArea::vertical().show(ui, |ui| {
                let mut updated_groups = Vec::new();

                for group in &mut self.groups {
                    let mut group_updated = false;

                    let response = ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}: ядра {:?}", group.name, group.cores));
                        });

                        for prog in &group.programs {
                            ui.horizontal(|ui| {
                                if ui.button("▶").clicked() {
                                    run_with_affinity(prog.clone(), &group.cores);
                                }
                                ui.label(prog.display().to_string());
                            });
                        }
                    });

                    if let Some(dropped) = &self.dropped_file {
                        if response.response.rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                            group.programs.push(dropped.clone());
                            self.dropped_file = None;
                            group_updated = true;
                            dropped_assigned = true;
                        }
                    }

                    if group_updated {
                        updated_groups.push(group.clone());
                    }

                    ui.separator();
                }

                if !updated_groups.is_empty() {
                    self.save_state();
                }
            });

            if let Some(dropped) = &self.dropped_file {
                if !dropped_assigned {
                    ui.separator();
                    ui.label(format!("Ожидает добавления: {}", dropped.display()));
                    for group in &mut self.groups {
                        if ui.button(format!("Добавить в группу {}", group.name)).clicked() {
                            group.programs.push(dropped.clone());
                            self.dropped_file = None;
                            self.save_state();
                            break;
                        }
                    }
                }
            }
        });
    }

    fn render_group_window(&mut self, ctx: &Context) {
        if self.show_group_window {
            let mut close_window = false;

            Window::new("Создание группы ядер").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Имя группы:");
                    ui.add(TextEdit::singleline(&mut self.new_group_name));
                });

                ui.label("Выбор ядер CPU:");
                ui.horizontal_wrapped(|ui| {
                    for i in 0..self.num_cores {
                        ui.add(Checkbox::new(&mut self.core_selection[i], format!("Core {}", i)));
                    }
                });

                ui.horizontal(|ui| {
                    if ui.button("Создать").clicked() {
                        if let Some(group) = self.create_core_group() {
                            self.groups.push(group);
                            self.save_state();
                            close_window = true;
                        }
                    }

                    if ui.button("Отмена").clicked() {
                        self.reset_group_form();
                        close_window = true;
                    }
                });
            });

            if close_window {
                self.show_group_window = false;
            }
        }
    }

    fn create_core_group(&mut self) -> Option<CoreGroup> {
        if self.new_group_name.trim().is_empty() {
            return None;
        }

        let selected_cores = self
            .core_selection
            .iter()
            .enumerate()
            .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
            .collect::<Vec<_>>();

        if selected_cores.is_empty() {
            return None;
        }

        let group = CoreGroup {
            name: self.new_group_name.trim().to_string(),
            cores: selected_cores,
            programs: Vec::new(),
        };

        self.reset_group_form();
        Some(group)
    }

    fn reset_group_form(&mut self) {
        self.new_group_name.clear();
        self.core_selection = vec![false; self.num_cores];
    }

    fn state_file_path() -> PathBuf {
        std::env::current_exe()
            .map(|mut path| {
                path.set_file_name("state.json");
                path
            })
            .unwrap_or_else(|_| PathBuf::from("state.json"))
    }

    fn load_state() -> Vec<CoreGroup> {
        let path = Self::state_file_path();
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(state) = serde_json::from_str::<AppState>(&data) {
                return state.groups;
            }
        }
        Vec::new()
    }

    fn save_state(&self) {
        let path = Self::state_file_path();
        let state = AppState {
            groups: self.groups.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = fs::write(path, json);
        }
    }
}

fn run_with_affinity(file_path: PathBuf, cores: &[usize]) {
    let affinity_mask: usize = cores.iter().map(|&i| 1 << i).sum();

    let (resolved, args) = if file_path.extension().and_then(|e| e.to_str()) == Some("lnk") {
        resolve_lnk_target_with_args(&file_path).unwrap_or((file_path.clone(), vec![]))
    } else {
        (file_path.clone(), vec![])
    };

    println!("Запуск: {}", resolved.display());

    let mut cmd = Command::new(&resolved);
    if !args.is_empty() {
        cmd.args(args);
    }

    match cmd.spawn() {
        Ok(child) => unsafe {
            let handle = HANDLE(child.as_raw_handle() as isize);
            if let Err(e) = SetProcessAffinityMask(handle, affinity_mask) {
                eprintln!("Не удалось установить маску affinity: {:?}", e);
            }
        },
        Err(e) => {
            eprintln!("Ошибка запуска: {:?}", e);
        }
    }
}

fn resolve_lnk_target_with_args(lnk_path: &PathBuf) -> Option<(PathBuf, Vec<String>)> {
    Lnk::try_from(lnk_path.as_path()).ok().and_then(|link| {
        let path = link.link_info.local_base_path.clone().map(PathBuf::from)?;
        let args = link.string_data.command_line_arguments.unwrap_or_default();
        let split_args = shlex::split(&args).unwrap_or_else(|| vec![args]);
        Some((path, split_args))
    })
}

fn main() {
    let options = NativeOptions::default();
    run_native(
        "CPU Affinity Tool",
        options,
        Box::new(|_cc| Box::new(CpuAffinityApp::default())),
    )
    .unwrap();
}