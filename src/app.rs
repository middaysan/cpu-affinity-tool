use eframe::egui;
use crate::models::{AppState, CoreGroup};

mod views;
use views::{header, central, group_editor, logs};

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
        Self {
            state: AppState::load_state(),
            core_selection: vec![false; num_cpus::get()],
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
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.get(0).and_then(|f| f.path.clone())) {
            self.dropped_file = Some(path);
        }

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

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme_index = (self.theme_index + 1) % 3;
        ctx.set_visuals(match self.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        });
    }

    fn create_group(&mut self) {
        let name_str = self.new_group_name.trim();
        if name_str.is_empty() { return; }

        let cores: Vec<_> = self.core_selection.iter().enumerate()
            .filter_map(|(i, &v)| v.then_some(i))
            .collect();
        if cores.is_empty() { return; }

        self.state.groups.push(CoreGroup {
            name: name_str.to_string(),
            cores,
            programs: vec![],
        });

        self.reset_group_form();
        self.state.save_state();
    }

    fn add_to_log(&mut self, message: String) {
        self.log_text.push(message);
    }

    fn remove_app_from_group(&mut self, group_index: usize, prog_path: &std::path::Path) {
        if let Some(group) = self.state.groups.get_mut(group_index) {
            group.programs.retain(|p| p != prog_path);
            self.state.save_state();
        }
    }

    fn run_app_with_affinity(&mut self, group_index: usize, prog_path: std::path::PathBuf) {
        let groups = self.state.groups.clone();
        let group = match groups.get(group_index) {
            Some(g) => g,
            None => return,
        };
        
        let label = prog_path.file_name()
            .map_or_else(|| prog_path.display().to_string(), |n| n.to_string_lossy().to_string());
        
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let ts = format!("{:02}:{:02}:{:02}", 
            (now.as_secs() % 86400) / 3600, 
            (now.as_secs() % 3600) / 60, 
            now.as_secs() % 60);
        
        self.add_to_log(format!("[{}] Starting '{}', app: {}", ts, label, prog_path.display()));

        match crate::affinity::run_with_affinity(prog_path.clone(), &group.cores) {
            Ok(_) => self.add_to_log(format!("[{}] OK: started '{}'", ts, label)),
            Err(e) => self.add_to_log(format!("[{}] ERROR: {}", ts, e)),
        }
    }
}
