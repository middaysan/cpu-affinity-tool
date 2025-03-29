use eframe::egui::{self, CentralPanel, Checkbox, Context, RichText, ScrollArea, TextEdit, TopBottomPanel, Window, TextureHandle, ColorImage};
use crate::models::{AppState, CoreGroup};
use crate::affinity::run_with_affinity;
use std::path::PathBuf;

use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::core::PCWSTR;
use std::path::Path;


pub struct CpuAffinityApp {
    num_cores: usize,
    core_selection: Vec<bool>,
    groups: Vec<CoreGroup>,
    new_group_name: String,
    dropped_file: Option<PathBuf>,
    show_group_window: bool,
    log_text: Vec<String>,
    show_log_window: bool,
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
            log_text: Vec::new(),
            show_log_window: false,
        }
    }
}

impl eframe::App for CpuAffinityApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_file(ctx);
        self.render_main_panel(ctx);
        self.render_group_window(ctx);
        self.render_bottom_panel(ctx);
        self.render_log_window(ctx);
    }
}

impl CpuAffinityApp {
    fn handle_dropped_file(&mut self, ctx: &Context) {
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
        }
    }

    fn render_main_panel(&mut self, ctx: &Context) {
        CentralPanel::default().show(ctx, |ui| {
            if ui.button("Create core group").clicked() {
                self.show_group_window = true;
            }

            let mut dropped_assigned = false;

            ScrollArea::vertical().show(ui, |ui| {
                dropped_assigned = self.render_groups(ui, ctx);
            });

            if let Some(dropped) = self.dropped_file.clone() {
                if !dropped_assigned {
                    self.render_dropped_file(ui, &dropped);
                }
            }
        });
    }

    fn render_groups(&mut self, ui: &mut eframe::egui::Ui, ctx: &Context) -> bool {
        let mut dropped_assigned = false;
        let mut updated_groups = Vec::new();
        let mut groups_to_remove = Vec::new();
        let mut response : egui::InnerResponse<()>;

        for (idx, group) in self.groups.iter_mut().enumerate() {
            ui.separator();
            let mut group_updated = false;

            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&group.name).underline());
                    ui.label(RichText::new(format!("cores: {:?}", group.cores)));

                    if ui.button( "X").clicked() {
                        groups_to_remove.push(idx);
                        group_updated = true;
                    }
                });
            });

            if group.programs.is_empty() {
                response = ui.group(|ui| {
                    if ui.button("Open exe file…").clicked() {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("All Files", &["exe", "lnk"])
                            .pick_files()
                        {
                            // Store the actual path that was selected, not the resolved target
                            paths.iter().for_each(|path| {
                                group.programs.push(path.clone());
                            });
                            group_updated = true;
                        }
                    }
                    ui.label("Open or drag files here to add them to the group.");
                });
            } else {
                response = ui.group(|ui| {
                    for prog in group.programs.clone() {
                        ui.horizontal(|ui| {
                            if ui.button("▶").clicked() {
                                let now = std::time::SystemTime::now();
                                let datetime = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                let current_time = format!("{:02}:{:02}:{:02}", 
                                    (datetime.as_secs() / 3600) % 24,
                                    (datetime.as_secs() / 60) % 60, 
                                    datetime.as_secs() % 60);
                                run_with_affinity(prog.clone(), &group.cores).unwrap_or_else(|e| {
                                    self.log_text.push(format!("[{}] {}", current_time, e));
                                });
                            }

                            if let Some(file_name) = prog.file_name() {
                                ui.label(file_name.to_string_lossy());
                            } else {
                                ui.label(prog.display().to_string());
                            }
    
                            if ui.button("Delete").clicked() {
                                let updated_programs: Vec<_> = group.programs.iter().filter(|p| *p != &prog).cloned().collect();
                                group.programs = updated_programs;
                                group_updated = true;
                            }
                        });
                    }
                });
            }

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
        }

        // Remove groups that were marked for deletion
        if !groups_to_remove.is_empty() {
            // Remove in reverse order to maintain correct indices
            groups_to_remove.sort_unstable_by(|a, b| b.cmp(a));
            for idx in groups_to_remove {
                self.groups.remove(idx);
            }
            self.save_state();
        }

        if !updated_groups.is_empty() {
            self.save_state();
        }

        dropped_assigned
    }

    fn render_dropped_file(&mut self, ui: &mut eframe::egui::Ui, dropped: &PathBuf) {
        ui.separator();
        ui.label(format!("Waiting to be added: {}", dropped.display()));
        for group in &mut self.groups {
            if ui.button(format!("Add to group {}", group.name)).clicked() {
                group.programs.push(dropped.clone());
                self.dropped_file = None;
                self.save_state();
                break;
            }
        }
    }

    fn render_group_window(&mut self, ctx: &Context) {
        if self.show_group_window {
            let mut close_window = false;

            Window::new("Create core group").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Group name:");
                    ui.add(TextEdit::singleline(&mut self.new_group_name));
                });

                ui.label("Select cores of CPU:");
                ui.horizontal_wrapped(|ui| {
                    for i in 0..self.num_cores {
                        ui.add(Checkbox::new(&mut self.core_selection[i], format!("Core {}", i)));
                    }
                });

                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() {
                        if let Some(group) = self.create_core_group() {
                            self.groups.push(group);
                            self.save_state();
                            close_window = true;
                        }
                    }

                    if ui.button("Cancel").clicked() {
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
        if let Ok(data) = std::fs::read_to_string(path) {
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
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("Error saving state: {:?}", e);
            }
        } else {
            eprintln!("Error serializing state to JSON");
        }
    }

    fn render_bottom_panel(&mut self, ctx: &Context) {
        if self.log_text.len() != 0 {
            TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
                let panel_height = 40.0; // Fixed height for the log panel
                ui.set_max_height(panel_height);
    
                if ui.button("Full logs").clicked() {
                    self.show_log_window = true;
                }
    
                ui.label("Last:");
    
                ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    if let Some(log) = self.log_text.last() {
                        
                        ui.label(log);
                    }
                    });
                });
            });
        }
    }

    fn render_log_window(&mut self, ctx: &Context) {
        if self.show_log_window {
            let mut show_log = self.show_log_window;
            Window::new("Log")
                .resizable(true)
                .min_size(egui::vec2(300.0, 100.0))
                .collapsible(true)
                .open(&mut show_log)
                .show(ctx, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        for log in &self.log_text {
                            ui.label(log);
                        }
                    });
                });
            
            // Update the state if the window was closed
            if !show_log {
                self.show_log_window = false;
            }
        }
    }

    fn load_icon_texture(&mut self, path: &Path, ctx: &Context) -> Option<TextureHandle> {
        let hicon = self.extract_icon(path)?;
    
        unsafe {
            let mut icon_info = ICONINFO::default();
            if GetIconInfo(hicon, &mut icon_info).is_ok() && !icon_info.hbmColor.is_invalid() {
                let mut bmp_info = BITMAP::default();
                GetObjectW(HGDIOBJ(icon_info.hbmColor.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bmp_info as *mut _ as *mut std::ffi::c_void));
    
                let width = bmp_info.bmWidth as usize;
                let height = bmp_info.bmHeight as usize;
    
                let mut pixels = vec![0u8; width * height * 4];
    
                let hdc = GetDC(None);
                let hdc_mem = CreateCompatibleDC(Some(hdc));
    
                let mut bmi = BITMAPINFO::default();
                bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bmi.bmiHeader.biWidth = width as i32;
                bmi.bmiHeader.biHeight = -(height as i32); // top-down bitmap
                bmi.bmiHeader.biPlanes = 1;
                bmi.bmiHeader.biBitCount = 32;
                bmi.bmiHeader.biCompression = BI_RGB.0;
    
                GetDIBits(
                    hdc_mem,
                    icon_info.hbmColor,
                    0,
                    height as u32,
                    Some(pixels.as_mut_ptr() as _),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
    
                ReleaseDC(None, hdc);
    
                // Конвертируем в egui::ColorImage
                let color_image = ColorImage::from_rgba_unmultiplied([width, height], &pixels);
                let texture = ctx.load_texture("exe_icon", color_image, Default::default());
                return Some(texture);
            }
            None
        }
    }

    fn extract_icon(&mut self, path: &Path) -> Option<HICON> {
        use std::os::windows::ffi::OsStrExt;
    
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut large_icons = [HICON::default(); 1];
        let mut small_icons = [HICON::default(); 1];
    
        unsafe {
            let count = ExtractIconExW(
                PCWSTR(wide_path.as_ptr()),
                0,
                Some(large_icons.as_mut_ptr()),
                Some(small_icons.as_mut_ptr()),
                1,
            );
            if count > 0 && !large_icons[0].is_invalid() {
                Some(large_icons[0])
            } else {
                None
            }
        }
    }
}
