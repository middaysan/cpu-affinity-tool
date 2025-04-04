mod app_state;
mod os_cmd;
use os_cmd::{OsCmd, OsCmdTrait, PriorityClass};

use app_state::AppToRun;
use eframe::egui;

mod views;
use views::{run_settings, central, group_editor, header, logs};

pub struct CpuAffinityApp {
    state: app_state::AppState,
    core_selection: Vec<bool>,
    new_group_name: String,
    dropped_file: Option<std::path::PathBuf>,
    show_group_window: bool,
    show_app_run_settings: bool,
    edit_app_clone: Option<AppToRun>,
    edit_app_to_run_settings: Option<(usize, usize)>,
    theme_index: usize,
    log_text: Vec<String>,
    show_log_window: bool,
    edit_group_index: Option<usize>,
    edit_group_selection: Option<Vec<bool>>,
}

impl Default for CpuAffinityApp {
    fn default() -> Self {
        let state = app_state::AppState::load_state();
        Self {
            state: state,
            core_selection: vec![false; num_cpus::get()],
            new_group_name: String::new(),
            dropped_file: None,
            show_group_window: false,
            show_app_run_settings: false,
            edit_app_to_run_settings: None,
            edit_app_clone: None,
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
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
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
        self.edit_group_index = None;
        self.edit_group_selection = None;
        self.show_group_window = false;
        self.new_group_name.clear();
        self.core_selection.fill(false);
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
        let name_str = self.new_group_name.trim();
        if name_str.is_empty() { 
            self.add_to_log("Group name cannot be empty".to_string());
            return; 
        }

        let cores: Vec<_> = self.core_selection.iter().enumerate()
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
        });

        self.reset_group_form();
        self.state.save_state();
    }

    pub fn add_to_log(&mut self, message: String) {
        self.log_text.push(message);
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
