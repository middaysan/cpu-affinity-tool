use crate::app::models::cpu_schema::CpuSchema;
use crate::app::models::{
    effective_total_threads, AppStateStorage, AppStatus, AppToRun, CoreGroup, LogManager,
};
use crate::app::navigation::WindowRoute;
use crate::app::runtime::commands::{apps, groups, launch, preferences};
use crate::app::runtime::{RuntimeRegistry, UiState};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Facade combining persisted, transient UI, and runtime tracking state.
pub struct AppState {
    pub(crate) persistent_state: Arc<RwLock<AppStateStorage>>,
    pub(crate) ui: UiState,
    pub(crate) runtime: RuntimeRegistry,
    pub(crate) log_manager: LogManager,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            persistent_state: Arc::new(RwLock::new(AppStateStorage::load_state())),
            ui: UiState::new(effective_total_threads()),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
        }
    }

    pub fn get_groups(&self) -> Vec<CoreGroup> {
        match self.persistent_state.read() {
            Ok(state) => state.groups.clone(),
            Err(_) => Vec::new(),
        }
    }

    pub fn get_group_name(&self, index: usize) -> Option<String> {
        match self.persistent_state.read() {
            Ok(state) => state.groups.get(index).map(|group| group.name.clone()),
            Err(_) => None,
        }
    }

    pub fn get_group_is_hidden(&self, index: usize) -> Option<bool> {
        match self.persistent_state.read() {
            Ok(state) => state.groups.get(index).map(|group| group.is_hidden),
            Err(_) => None,
        }
    }

    pub fn set_group_is_hidden(&mut self, index: usize, is_hidden: bool) {
        groups::set_group_is_hidden(&self.persistent_state, index, is_hidden);
    }

    pub fn get_group_run_all_button(&self, index: usize) -> Option<bool> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(index)
            .map(|group| group.run_all_button)
    }

    pub fn get_group_programs(&self, index: usize) -> Option<Vec<AppToRun>> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(index)
            .map(|group| group.programs.clone())
    }

    pub fn get_group_program(&self, group_index: usize, program_index: usize) -> Option<AppToRun> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(group_index)
            .and_then(|group| group.programs.get(program_index).cloned())
    }

    pub fn get_group_cores(&self, index: usize) -> Option<Vec<usize>> {
        self.persistent_state
            .read()
            .unwrap()
            .groups
            .get(index)
            .map(|group| group.cores.clone())
    }

    pub fn get_cpu_schema(&self) -> CpuSchema {
        self.persistent_state.read().unwrap().cpu_schema.clone()
    }

    pub fn swap_groups(&mut self, index1: usize, index2: usize) -> bool {
        groups::swap_groups(&self.persistent_state, index1, index2)
    }

    pub fn add_apps_to_group(
        &mut self,
        group_index: usize,
        paths: Vec<PathBuf>,
    ) -> Result<(), String> {
        apps::add_apps_to_group(&self.persistent_state, group_index, paths)
    }

    pub fn update_program(
        &mut self,
        group_index: usize,
        program_index: usize,
        program: AppToRun,
    ) -> bool {
        apps::update_program(&self.persistent_state, group_index, program_index, program)
    }

    pub fn get_theme_index(&self) -> usize {
        self.persistent_state.read().unwrap().theme_index
    }

    pub fn update_group_properties(
        &mut self,
        index: usize,
        name: String,
        cores: Vec<usize>,
        run_all_button: bool,
    ) -> bool {
        groups::update_group_properties(&self.persistent_state, index, name, cores, run_all_button)
    }

    pub fn remove_group(&mut self, index: usize) -> bool {
        groups::remove_group(&self.persistent_state, index)
    }

    pub fn start_app_with_autorun(&mut self) {
        launch::start_app_with_autorun(
            &self.persistent_state,
            &self.runtime,
            &mut self.log_manager,
        );
    }

    pub fn reset_group_form(&mut self) {
        self.ui.reset_group_form();
    }

    pub fn toggle_theme(&mut self) {
        preferences::toggle_theme(&self.persistent_state);
    }

    pub fn toggle_process_monitoring(&mut self) {
        preferences::toggle_process_monitoring(&self.persistent_state);
    }

    pub fn is_process_monitoring_enabled(&self) -> bool {
        self.persistent_state
            .read()
            .unwrap()
            .process_monitoring_enabled
    }

    pub fn create_group(&mut self) {
        groups::create_group(&self.persistent_state, &mut self.ui, &mut self.log_manager);
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        self.ui.set_current_window(window);
    }

    pub fn remove_app_from_group(&mut self, group_index: usize, programm_index: usize) {
        apps::remove_app_from_group(
            &self.persistent_state,
            &mut self.log_manager,
            group_index,
            programm_index,
        );
    }

    pub fn start_editing_group(&mut self, group_index: usize) {
        groups::start_editing_group(
            &self.persistent_state,
            &mut self.ui,
            &mut self.log_manager,
            group_index,
        );
    }

    pub fn run_app_with_affinity_sync(
        &mut self,
        group_index: usize,
        prog_index: usize,
        app_to_run: AppToRun,
    ) {
        launch::run_app_with_affinity_sync(
            &self.persistent_state,
            &self.runtime,
            &mut self.log_manager,
            group_index,
            prog_index,
            app_to_run,
        );
    }

    pub fn get_app_status_sync(&mut self, app_key: &str) -> AppStatus {
        self.runtime.get_app_status_sync(app_key)
    }

    pub fn get_running_app_pids(&self, app_key: &str) -> Option<Vec<u32>> {
        self.runtime.get_running_app_pids(app_key)
    }

    pub fn get_tip(&mut self, current_time: f64) -> &str {
        self.ui.current_tip(current_time)
    }

    pub fn open_app_run_settings(&mut self, group_index: usize, program_index: usize) {
        apps::open_app_edit_session(&mut self.ui, group_index, program_index);
    }

    pub fn close_app_run_settings(&mut self) {
        apps::close_app_edit_session(&mut self.ui);
    }

    pub fn ensure_current_edit_loaded(&mut self, group_idx: usize, prog_idx: usize) -> bool {
        apps::ensure_current_edit_loaded(&self.persistent_state, &mut self.ui, group_idx, prog_idx)
    }
}
