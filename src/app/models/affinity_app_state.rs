use crate::app::models::affinity_app_state_storage::AffinityAppStateStorage;
use crate::app::controllers;
use crate::app::models::app_to_run::{RunAppEditState, AppToRun};
use crate::app::models::core_group::{CoreGroup, GroupFormState};
use crate::app::models::LogManager;
use crate::app::os_cmd::OsCmd;

use std::path::PathBuf;
use num_cpus;
use eframe::egui;

pub struct AffinityAppState {
    pub current_window: controllers::WindowController,
    pub controller_changed: bool,
    pub persistent_state: AffinityAppStateStorage, // Holds persistent data like theme, groups, etc.
    pub group_form: GroupFormState,
    pub app_edit_state: RunAppEditState,
    pub dropped_files: Option<Vec<PathBuf>>,
    pub log_manager: LogManager,
}

impl AffinityAppState {
    pub fn new(ctx: &egui::Context) -> Self {
        let app = Self {
            persistent_state: AffinityAppStateStorage::load_state(),
            current_window: controllers::WindowController::Groups(controllers::Group::ListGroups),
            controller_changed: false,
            group_form: GroupFormState {
                editing_index: None,
                editing_selection: None,
                core_selection: vec![false; num_cpus::get()],
                group_name: String::new(),
                run_all_enabled: false,
                is_visible: false,
            },
            app_edit_state: RunAppEditState {
                current_edit: None,
                run_settings: None,
            },
            dropped_files: None,
            log_manager: LogManager { entries: vec![] },
        };

        // Установить тему из состояния
        let visuals = match app.persistent_state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);

        app
    }
}

impl AffinityAppState {
    /// Resets the group form state.
    pub fn reset_group_form(&mut self) {
        self.group_form.reset();
    }

    /// Toggles the UI theme between default, light, and dark modes and saves the state.
    pub fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.persistent_state.theme_index = (self.persistent_state.theme_index + 1) % 3;
        let visuals = match self.persistent_state.theme_index {
            0 => egui::Visuals::default(),
            1 => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        ctx.set_visuals(visuals);
        self.persistent_state.save_state();
    }

    /// Creates a new core group from the group form data.
    /// Validates that group name is non-empty and at least one core is selected.
    pub fn create_group(&mut self) {
        let group_name_trimmed = self.group_form.group_name.trim();
        if group_name_trimmed.is_empty() {
            self.log_manager.add_entry("Group name cannot be empty".into());
            return;
        }

        // Gather indices of selected cores.
        let selected_cores: Vec<usize> = self.group_form.core_selection.iter()
            .enumerate()
            .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
            .collect();

        if selected_cores.is_empty() {
            self.log_manager.add_entry("At least one core must be selected".into());
            return;
        }

        // Add new group to the persistent application state.
        self.persistent_state.groups.push(CoreGroup {
            name: group_name_trimmed.to_string(),
            cores: selected_cores,
            programs: vec![],
            run_all_button: self.group_form.run_all_enabled,
        });

        self.reset_group_form();
        self.persistent_state.save_state();
    }

    /// Sets a new window and marks the controller as changed.
    pub fn set_current_window(&mut self, window: controllers::WindowController) {
        self.current_window = window;
        self.controller_changed = true;
    }

    /// Remove an application from a specified group by binary path.
    pub fn remove_app_from_group(&mut self, group_index: usize, programm_index: usize) {
        if let Some(group) = self.persistent_state.groups.get_mut(group_index) {
            if programm_index < group.programs.len() {
                let app = &group.programs[programm_index];
                self.log_manager.add_entry(format!("Removing app: {}", app.bin_path.display()));
                group.programs.remove(programm_index);
            }
        }
    }

    /// Prepares the group form for editing an existing group.
    /// It fills the form with the group data and updates associated clusters.
    pub fn start_editing_group(&mut self, group_index: usize) {
        let total_cores = self.group_form.core_selection.len();
        // Update the core selection based on the selected group's cores.
        self.group_form.core_selection = {
            let mut selection = vec![false; total_cores];
            for &core in &self.persistent_state.groups[group_index].cores {
                if core < total_cores {
                    selection[core] = true;
                }
            }
            selection
        };

        self.group_form.group_name = self.persistent_state.groups[group_index].name.clone();
        self.group_form.editing_index = Some(group_index);
        self.group_form.run_all_enabled = self.persistent_state.groups[group_index].run_all_button;

        // Map the cores to their corresponding clusters.
        // This is a critical operation that ensures UI consistency.
        self.persistent_state.clusters = self.persistent_state.groups[group_index].cores.iter()
            .map(|&ci| self.persistent_state.clusters.get(ci).cloned().unwrap_or_default())
            .collect();

        self.set_current_window(controllers::WindowController::Groups(controllers::Group::EditGroup));
    }

    /// Runs an application with a specified CPU affinity based on the provided group.
    /// Logs the start of the app and any resulting errors.
    pub fn run_app_with_affinity(&mut self, group_index: usize, app_to_run: AppToRun) {
        let group = match self.persistent_state.groups.get(group_index) {
            Some(g) => g,
            None => return,
        };

        // Extract a human-readable label from the binary path.
        let label = app_to_run.bin_path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| app_to_run.bin_path.display().to_string());
        
        self.log_manager.add_entry(format!("Starting '{}', app: {}", label, app_to_run.display()));

        match OsCmd::run(app_to_run.bin_path, app_to_run.args, &group.cores, app_to_run.priority) {
            Ok(_) => self.log_manager.add_entry(format!("OK: started '{}'", label)),
            Err(e) => self.log_manager.add_entry(format!("ERROR: {}", e)),
        }
    }
}
