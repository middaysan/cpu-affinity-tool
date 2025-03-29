use eframe::egui::{CentralPanel, Checkbox, Context, ScrollArea, TextEdit, TopBottomPanel, Window, Button};
use eframe::{run_native, App, NativeOptions};
use std::path::PathBuf;
use std::process::Command;
use std::os::windows::io::AsRawHandle;
use parselnk::Lnk;
use windows::Win32::System::Threading::SetProcessAffinityMask;
use windows::Win32::Foundation::HANDLE;
use shlex;

#[derive(Debug, Clone)]
struct CoreGroup {
    name: String,
    cores: Vec<usize>,
    programs: Vec<PathBuf>,
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
        Self {
            num_cores,
            core_selection: vec![false; num_cores],
            groups: Vec::new(),
            new_group_name: String::new(),
            dropped_file: None,
            show_group_window: false,
        }
    }
}

impl App for CpuAffinityApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
        }

        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.heading("CPU Affinity Group Manager");
        });

        CentralPanel::default().show(ctx, |ui| {
            if ui.button("Создать группу ядер").clicked() {
                self.show_group_window = true;
            }

            ui.separator();
            ui.label("Группы и программы:");

            let mut dropped_assigned = false;

            ScrollArea::vertical().show(ui, |ui| {
                for group in &mut self.groups {
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
                            dropped_assigned = true;
                        }
                    }

                    ui.separator();
                }
            });

            if let Some(dropped) = &self.dropped_file {
                if dropped_assigned {
                    self.dropped_file = None;
                } else {
                    ui.separator();
                    ui.label(format!("Ожидает добавления: {}", dropped.display()));
                    for group in &mut self.groups {
                        if ui.button(format!("Добавить в группу {}", group.name)).clicked() {
                            group.programs.push(dropped.clone());
                            self.dropped_file = None;
                            break;
                        }
                    }
                }
            }
        });

        let mut close_window = false;

        if self.show_group_window {
            Window::new("Создание группы ядер")
                .show(ctx, |ui| {
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
                            if !self.new_group_name.trim().is_empty() {
                                let cores = self
                                    .core_selection
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                                    .collect::<Vec<_>>();

                                if !cores.is_empty() {
                                    self.groups.push(CoreGroup {
                                        name: self.new_group_name.trim().to_string(),
                                        cores,
                                        programs: Vec::new(),
                                    });
                                    self.new_group_name.clear();
                                    self.core_selection = vec![false; self.num_cores];
                                    close_window = true;
                                }
                            }
                        }

                        if ui.button("Отмена").clicked() {
                            self.new_group_name.clear();
                            self.core_selection = vec![false; self.num_cores];
                            close_window = true;
                        }
                    });
                });
        }

        if close_window {
            self.show_group_window = false;
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
        Ok(child) => {
            unsafe {
                let handle = HANDLE(child.as_raw_handle() as isize);
                if let Err(e) = SetProcessAffinityMask(handle, affinity_mask) {
                    eprintln!("Не удалось установить маску affinity: {:?}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("Ошибка запуска: {:?}", e);
        }
    }
}

fn resolve_lnk_target_with_args(lnk_path: &PathBuf) -> Option<(PathBuf, Vec<String>)> {
    match Lnk::try_from(lnk_path.as_path()) {
        Ok(link) => {
            let path = link.link_info.local_base_path.clone().map(PathBuf::from)?;
            let args = link.string_data.command_line_arguments.unwrap_or_default();
            let split_args = shlex::split(&args).unwrap_or_else(|| vec![args]);
            Some((path, split_args))
        }
        Err(_) => None,
    }
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