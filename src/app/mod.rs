mod app_state;
mod os_cmd;
use std::path::PathBuf;

use os_cmd::{OsCmd, OsCmdTrait, PriorityClass};

use app_state::AppToRun;
use eframe::egui;

mod views;
use views::{run_settings, central, group_editor, header, logs};

pub struct Logs {
    pub show: bool,
    pub log_text: Vec<String>,
}

pub struct Groups {
    edit_index: Option<usize>,
    edit_selection: Option<Vec<bool>>,
    core_selection: Vec<bool>,
    new_name: String,
    enable_run_all_button: bool,
}

pub struct CpuAffinityApp {
    state: app_state::AppState,
    groups: Groups,
    dropped_files: Option<Vec<PathBuf>>,
    show_group_window: bool,
    show_app_run_settings: bool,
    edit_app_clone: Option<AppToRun>,
    edit_app_to_run_settings: Option<(usize, usize)>,
    theme_index: usize,
    logs: Logs,
}

impl Default for CpuAffinityApp {
    fn default() -> Self {
        let state = app_state::AppState::load_state();
        Self {
            state: state,
            groups: Groups {
                edit_index: None,
                edit_selection: None,
                core_selection: vec![false; num_cpus::get()],
                new_name: String::new(),
                enable_run_all_button: false,
            },
            show_group_window: false,
            dropped_files: None,
            show_app_run_settings: false,
            edit_app_to_run_settings: None,
            edit_app_clone: None,
            logs: Logs {
                show: false,
                log_text: vec![],
            },
            theme_index: 0,
        }
    }
}

impl eframe::App for CpuAffinityApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for dropped files
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let files: Vec<PathBuf> = ctx.input(|i| 
            i.raw.dropped_files.iter()
            .filter_map(|f| f.path.clone())
            .collect());
            
            if !files.is_empty() {
            self.dropped_files = Some(files);
            }
        }

        run_settings::draw_app_run_settings(self, ctx);
        header::draw_top_panel(self, ctx);
        central::draw_central_panel(self, ctx);
        group_editor::group_window(self, ctx);
        logs::draw_logs_window(self, ctx);
    }
}

impl CpuAffinityApp {
    pub fn reset_group_form(&mut self) {
        self.groups.edit_index = None;
        self.groups.edit_selection = None;
        self.groups.enable_run_all_button = false;
        self.show_group_window = false;
        self.groups.new_name.clear();
        self.groups.core_selection.fill(false);
    }

    pub fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme_index = (self.theme_index + 1) % 3;
        ctx.set_visuals(match self.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        });
    }

    pub fn create_group(&mut self) {
        let name_str = self.groups.new_name.trim();
        if name_str.is_empty() { 
            self.add_to_log("Group name cannot be empty".to_string());
            return; 
        }

        let cores: Vec<_> = self.groups.core_selection.iter().enumerate()
            .filter_map(|(i, &v)| v.then_some(i))
            .collect();

        if cores.is_empty() { 
            self.add_to_log("At least one core must be selected".to_string());
            return; 
        }

        self.state.groups.push(app_state::CoreGroup {
            name: name_str.to_string(),
            cores,
            programs: vec![],
            run_all_button: self.groups.enable_run_all_button,
        });

        self.reset_group_form();
        self.state.save_state();
    }

    pub fn add_to_log(&mut self, message: String) {
        self.logs.log_text.push(message);
    }

    pub fn remove_app_from_group(&mut self, group_index: usize, prog_path: &std::path::Path) {
        if let Some(group) = self.state.groups.get_mut(group_index) {
            group.programs.retain(|p| p.bin_path != prog_path);
            self.edit_app_clone = None;
            self.state.save_state();
        }
    }

    pub fn run_app_with_affinity(&mut self, group_index: usize, app_to_run: AppToRun) {
        let groups = self.state.groups.clone();
        let group = match groups.get(group_index) {
            Some(g) => g,
            None => return,
        };

        let label = app_to_run.bin_path.file_name()
            .map_or_else(|| app_to_run.bin_path.display().to_string(), |n| n.to_string_lossy().to_string());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let ts = format!("{:02}:{:02}:{:02}", 
            (now.as_secs() % 86400) / 3600, 
            (now.as_secs() % 3600) / 60, 
            now.as_secs() % 60);

        self.add_to_log(format!("[{}] Starting '{}', app: {}", ts, label, app_to_run.display()));

        match OsCmd::run(app_to_run.bin_path, app_to_run.args, &group.cores, app_to_run.priority) {
            Ok(_) => self.add_to_log(format!("[{}] OK: started '{}'", ts, label)),
            Err(e) => self.add_to_log(format!("[{}] ERROR: {}", ts, e)),
        }
    }
}
