use eframe::egui::{CentralPanel, Checkbox, Context, ScrollArea, TextEdit, TopBottomPanel, Window, Button};
use eframe::{run_native, App, NativeOptions};
use std::path::PathBuf;
use std::process::Command;
use std::os::windows::io::AsRawHandle;
use parselnk::Lnk;
use windows::Win32::System::Threading::SetProcessAffinityMask;
use windows::Win32::Foundation::HANDLE;

#[derive(Debug, Clone)]
struct CoreGroup {
    name: String,
    cores: Vec<usize>,
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
            ui.label("Created groups:");

            ScrollArea::vertical().show(ui, |ui| {
                for group in &self.groups {
                    ui.horizontal(|ui| {
                        if ui.button("Run Here").clicked() {
                            println!("Running with affinity for group: {}", group.name);
                            if let Some(file) = &self.dropped_file {
                                run_with_affinity(file.clone(), &group.cores);
                            }
                        }
                        ui.label(format!("{}: cores {:?}", group.name, group.cores));
                    });
                }
            });

            if let Some(path) = &self.dropped_file {
                ui.separator();
                ui.label(format!("Dropped file: {}", path.display()));
            }
        });

        let mut close_window = false;

        if self.show_group_window {
            Window::new("Создание группы ядер")
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Group name:");
                        ui.add(TextEdit::singleline(&mut self.new_group_name));
                    });

                    ui.label("Select CPU cores:");
                    ui.horizontal_wrapped(|ui| {
                        for i in 0..self.num_cores {
                            ui.add(Checkbox::new(&mut self.core_selection[i], format!("Core {}", i)));
                        }
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Create").clicked() {
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

    let resolved = if file_path.extension().and_then(|e| e.to_str()) == Some("lnk") {
        resolve_lnk_target(&file_path).unwrap_or(file_path.clone())
    } else {
        file_path.clone()
    };

    print!("Running file: {}", resolved.display());

    if let Ok(child) = Command::new(&resolved).spawn() {
        unsafe {
            let handle = HANDLE(child.as_raw_handle() as isize);
            if let Err(e) = SetProcessAffinityMask(handle, affinity_mask) {
                eprintln!("Failed to set process affinity mask: {:?}", e);
            }
        }
    }
}

fn resolve_lnk_target(lnk_path: &PathBuf) -> Option<PathBuf> {
    match Lnk::try_from(lnk_path.as_path()) {
        Ok(link) => link.link_info.local_base_path.clone().map(PathBuf::from),
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