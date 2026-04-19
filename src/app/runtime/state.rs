use crate::app::models::cpu_schema::CpuSchema;
use crate::app::models::{
    effective_total_threads, AddAppsOutcome, AppStateStorage, AppStatus, AppToRun, LogManager,
};
use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::commands::{apps, groups, launch, preferences};
use crate::app::runtime::{RuntimeRegistry, UiState};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CentralProgramSnapshot {
    pub group_index: usize,
    pub program_index: usize,
    pub name: String,
    pub bin_path_display: String,
    pub app_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CentralGroupSnapshot {
    pub group_index: usize,
    pub name: String,
    pub cores: Vec<usize>,
    pub is_hidden: bool,
    pub run_all_button: bool,
    pub programs: Vec<CentralProgramSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CentralPanelSnapshot {
    pub groups: Vec<CentralGroupSnapshot>,
}

/// Facade combining persisted, transient UI, and runtime tracking state.
pub struct AppState {
    pub(crate) persistent_state: Arc<RwLock<AppStateStorage>>,
    pub(crate) ui: UiState,
    pub(crate) runtime: RuntimeRegistry,
    pub(crate) log_manager: LogManager,
    #[cfg(test)]
    save_count: usize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            persistent_state: Arc::new(RwLock::new(AppStateStorage::load_state())),
            ui: UiState::new(effective_total_threads()),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
            #[cfg(test)]
            save_count: 0,
        }
    }

    #[cfg(test)]
    fn save_count(&self) -> usize {
        self.save_count
    }

    fn persist_state(&mut self) -> bool {
        #[cfg(test)]
        {
            self.save_count += 1;
            true
        }

        #[cfg(not(test))]
        {
            let save_result = match self.persistent_state.read() {
                Ok(state) => state.try_save_state(),
                Err(_) => {
                    self.log_manager.add_sticky_once(
                        "WARNING: persistent_state lock poisoned during save".into(),
                    );
                    return false;
                }
            };

            if let Err(err) = save_result {
                self.log_manager
                    .add_important_sticky_once(format!("ERROR: Failed to save state: {err}"));
                return false;
            }

            true
        }
    }

    pub fn build_central_panel_snapshot(&self) -> CentralPanelSnapshot {
        match self.persistent_state.read() {
            Ok(state) => CentralPanelSnapshot {
                groups: state
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(group_index, group)| CentralGroupSnapshot {
                        group_index,
                        name: group.name.clone(),
                        cores: group.cores.clone(),
                        is_hidden: group.is_hidden,
                        run_all_button: group.run_all_button,
                        programs: group
                            .programs
                            .iter()
                            .enumerate()
                            .map(|(program_index, program)| CentralProgramSnapshot {
                                group_index,
                                program_index,
                                name: program.name.clone(),
                                bin_path_display: program.bin_path.to_string_lossy().to_string(),
                                app_key: program.get_key(),
                            })
                            .collect(),
                    })
                    .collect(),
            },
            Err(_) => CentralPanelSnapshot::default(),
        }
    }

    pub fn get_group_name(&self, index: usize) -> Option<String> {
        match self.persistent_state.read() {
            Ok(state) => state.groups.get(index).map(|group| group.name.clone()),
            Err(_) => None,
        }
    }

    pub fn set_group_is_hidden(&mut self, index: usize, is_hidden: bool) {
        if groups::set_group_is_hidden(&self.persistent_state, index, is_hidden) {
            let _ = self.persist_state();
        }
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

    pub fn get_cpu_schema(&self) -> CpuSchema {
        self.persistent_state.read().unwrap().cpu_schema.clone()
    }

    pub fn swap_groups(&mut self, index1: usize, index2: usize) -> bool {
        let swapped = groups::swap_groups(&self.persistent_state, index1, index2);
        if swapped {
            let _ = self.persist_state();
        }
        swapped
    }

    pub fn add_selected_files_to_group(&mut self, group_index: usize, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        let attempted_count = paths.len();
        let group_name = self.get_group_name(group_index).unwrap_or_default();
        self.log_manager.add_entry(format!(
            "Adding executables to group: {group_name}, paths: {paths:?}"
        ));

        let outcome = apps::add_apps_to_group(&self.persistent_state, group_index, paths);
        self.handle_add_apps_outcome(&group_name, attempted_count, outcome);
    }

    pub fn consume_dropped_files_into_group(&mut self, group_index: usize) -> bool {
        let Some(files) = self.ui.dropped_files.take() else {
            return false;
        };

        if files.is_empty() {
            return false;
        }

        let files_count = files.len();
        let group_name = self.get_group_name(group_index).unwrap_or_default();

        let outcome = apps::add_apps_to_group(&self.persistent_state, group_index, files);
        self.handle_add_apps_outcome(&group_name, files_count, outcome);

        true
    }

    fn handle_add_apps_outcome(
        &mut self,
        group_name: &str,
        attempted_count: usize,
        outcome: AddAppsOutcome,
    ) {
        if outcome.added_count > 0 {
            let _ = self.persist_state();

            if outcome.added_count == attempted_count {
                self.log_manager
                    .add_entry(format!("Added executables to group: {group_name}"));
            } else {
                self.log_manager.add_entry(format!(
                    "Added {} executables to group: {}",
                    outcome.added_count, group_name
                ));
            }
        }

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding executables: {err}"));
        }
    }

    pub fn get_theme_index(&self) -> usize {
        self.persistent_state.read().unwrap().theme_index
    }

    pub fn start_app_with_autorun(&mut self) {
        launch::start_app_with_autorun(
            &self.persistent_state,
            &self.runtime,
            &mut self.log_manager,
        );
    }

    pub fn toggle_theme(&mut self) {
        preferences::toggle_theme(&self.persistent_state);
        let _ = self.persist_state();
    }

    pub fn toggle_process_monitoring(&mut self) {
        preferences::toggle_process_monitoring(&self.persistent_state);
        let _ = self.persist_state();
    }

    pub fn is_process_monitoring_enabled(&self) -> bool {
        self.persistent_state
            .read()
            .unwrap()
            .process_monitoring_enabled
    }

    pub fn commit_group_form_session(&mut self) {
        let should_save = if let Some(index) = self.ui.group_form.editing_index {
            let selected_cores: Vec<usize> = self
                .ui
                .group_form
                .core_selection
                .iter()
                .enumerate()
                .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                .collect();

            groups::update_group_properties(
                &self.persistent_state,
                index,
                self.ui.group_form.group_name.clone(),
                selected_cores,
                self.ui.group_form.run_all_enabled,
            )
        } else {
            groups::create_group(&self.persistent_state, &mut self.ui, &mut self.log_manager)
        };

        if should_save {
            let _ = self.persist_state();
        }

        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn delete_current_group_form_target(&mut self) {
        if let Some(index) = self.ui.group_form.editing_index {
            if groups::remove_group(&self.persistent_state, index) {
                let _ = self.persist_state();
            }
        }

        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn cancel_group_form_session(&mut self) {
        self.ui.reset_group_form();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn set_current_window(&mut self, window: WindowRoute) {
        self.ui.set_current_window(window);
    }

    pub fn start_editing_group(&mut self, group_index: usize) {
        groups::start_editing_group(
            &self.persistent_state,
            &mut self.ui,
            &mut self.log_manager,
            group_index,
        );
    }

    fn run_app_with_affinity_sync(
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

    pub fn run_group_program(&mut self, group_index: usize, program_index: usize) {
        if let Some(app_to_run) = self.get_group_program(group_index, program_index) {
            self.run_app_with_affinity_sync(group_index, program_index, app_to_run);
        }
    }

    pub fn run_group(&mut self, group_index: usize) {
        let Some(programs) = self.get_group_programs(group_index) else {
            return;
        };

        if programs.is_empty() {
            let group_name = self.get_group_name(group_index).unwrap_or_default();
            self.log_manager
                .add_entry(format!("No executables to run in group: {group_name}"));
            return;
        }

        for (program_index, program) in programs.into_iter().enumerate() {
            self.run_app_with_affinity_sync(group_index, program_index, program);
        }
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

    pub fn commit_current_app_edit_session(&mut self) {
        if let (Some((group_idx, prog_idx)), Some(updated_app)) = (
            self.ui.app_edit_state.run_settings,
            self.ui.app_edit_state.current_edit.clone(),
        ) {
            if apps::update_program(&self.persistent_state, group_idx, prog_idx, updated_app) {
                let _ = self.persist_state();
            }
        }

        apps::close_app_edit_session(&mut self.ui);
    }

    pub fn delete_current_app_edit_target(&mut self) {
        if let Some((group_idx, prog_idx)) = self.ui.app_edit_state.run_settings {
            if let Some(path) =
                apps::remove_app_from_group(&self.persistent_state, group_idx, prog_idx)
            {
                let _ = self.persist_state();
                self.log_manager
                    .add_entry(format!("Removing app: {}", path));
            }
        }

        apps::close_app_edit_session(&mut self.ui);
    }

    pub fn clear_logs(&mut self) {
        self.log_manager.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, LogManager};
    use crate::app::navigation::{GroupRoute, WindowRoute};
    use crate::app::runtime::{RuntimeRegistry, UiState};
    use os_api::PriorityClass;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    fn sample_state() -> AppState {
        AppState {
            persistent_state: Arc::new(RwLock::new(AppStateStorage {
                version: 4,
                groups: vec![CoreGroup {
                    name: "Games".to_string(),
                    cores: vec![0, 1],
                    programs: vec![AppToRun {
                        name: "Sample".to_string(),
                        dropped_path: PathBuf::from(r"C:\Sample.lnk"),
                        args: vec![],
                        bin_path: PathBuf::from(r"C:\Sample.exe"),
                        additional_processes: vec![],
                        autorun: false,
                        priority: PriorityClass::Normal,
                    }],
                    is_hidden: false,
                    run_all_button: true,
                }],
                cpu_schema: CpuSchema {
                    model: "Test CPU".to_string(),
                    clusters: Vec::new(),
                },
                theme_index: 0,
                process_monitoring_enabled: false,
            })),
            ui: UiState::new(4),
            runtime: RuntimeRegistry::new(),
            log_manager: LogManager::default(),
            save_count: 0,
        }
    }

    #[test]
    fn test_commit_group_form_session_preserves_invalid_create_closeout() {
        let mut app = sample_state();
        app.ui.current_window = WindowRoute::Groups(GroupRoute::Create);

        app.commit_group_form_session();

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.group_form.group_name.is_empty());
        assert!(app
            .ui
            .group_form
            .core_selection
            .iter()
            .all(|selected| !selected));
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
        assert_eq!(app.save_count(), 0);
        assert_eq!(app.log_manager.entries.len(), 1);
        assert_eq!(
            app.log_manager.entries[0].message,
            "Group name cannot be empty"
        );
    }

    #[test]
    fn test_commit_current_app_edit_session_updates_and_closes() {
        let mut app = sample_state();
        app.ui.current_window = WindowRoute::AppRunSettings;
        app.ui.app_edit_state.run_settings = Some((0, 0));
        app.ui.app_edit_state.current_edit = Some(AppToRun {
            name: "Updated".to_string(),
            dropped_path: PathBuf::from(r"C:\Sample.lnk"),
            args: vec!["--debug".to_string()],
            bin_path: PathBuf::from(r"C:\Updated.exe"),
            additional_processes: vec!["helper.exe".to_string()],
            autorun: true,
            priority: PriorityClass::High,
        });

        app.commit_current_app_edit_session();

        let state = app.persistent_state.read().unwrap();
        let updated = &state.groups[0].programs[0];
        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.bin_path, PathBuf::from(r"C:\Updated.exe"));
        drop(state);
        assert_eq!(app.save_count(), 1);

        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.app_edit_state.current_edit.is_none());
        assert!(app.ui.app_edit_state.run_settings.is_none());
    }

    #[test]
    fn test_toggle_theme_and_monitoring_save_once() {
        let mut app = sample_state();

        app.toggle_theme();
        assert_eq!(app.get_theme_index(), 1);
        assert_eq!(app.save_count(), 1);

        app.toggle_process_monitoring();
        assert!(app.is_process_monitoring_enabled());
        assert_eq!(app.save_count(), 2);
    }

    #[test]
    fn test_successful_group_create_and_delete_save_once_each() {
        let mut app = sample_state();
        app.ui.group_form.group_name = "Work".to_string();
        app.ui.group_form.core_selection[2] = true;

        app.commit_group_form_session();
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 2);
        assert_eq!(app.save_count(), 1);

        app.ui.group_form.editing_index = Some(1);
        app.delete_current_group_form_target();
        assert_eq!(app.persistent_state.read().unwrap().groups.len(), 1);
        assert_eq!(app.save_count(), 2);
    }

    #[test]
    fn test_delete_current_app_edit_target_saves_once() {
        let mut app = sample_state();
        app.ui.app_edit_state.run_settings = Some((0, 0));

        app.delete_current_app_edit_target();

        assert!(app.persistent_state.read().unwrap().groups[0]
            .programs
            .is_empty());
        assert_eq!(app.save_count(), 1);
    }

    #[test]
    fn test_noop_delete_current_app_edit_target_does_not_save() {
        let mut app = sample_state();

        app.delete_current_app_edit_target();

        assert_eq!(app.save_count(), 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_add_selected_files_partial_success_saves_once() {
        let mut app = sample_state();

        app.add_selected_files_to_group(0, vec![r"C:\valid.exe".into(), r"C:\broken".into()]);

        let state = app.persistent_state.read().unwrap();
        assert_eq!(state.groups[0].programs.len(), 2);
        assert_eq!(
            state.groups[0].programs[1].bin_path,
            PathBuf::from(r"C:\valid.exe")
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_add_selected_files_all_invalid_does_not_save() {
        let mut app = sample_state();

        app.add_selected_files_to_group(0, vec![r"C:\broken".into()]);

        assert_eq!(
            app.persistent_state.read().unwrap().groups[0]
                .programs
                .len(),
            1
        );
        assert_eq!(app.save_count(), 0);
    }

    #[test]
    fn test_central_snapshot_preserves_indices() {
        let app = sample_state();
        app.persistent_state
            .write()
            .unwrap()
            .groups
            .push(CoreGroup {
                name: "Work".to_string(),
                cores: vec![2, 3],
                programs: vec![],
                is_hidden: true,
                run_all_button: false,
            });

        let snapshot = app.build_central_panel_snapshot();

        assert_eq!(snapshot.groups.len(), 2);
        assert_eq!(snapshot.groups[0].group_index, 0);
        assert_eq!(snapshot.groups[0].programs[0].group_index, 0);
        assert_eq!(snapshot.groups[0].programs[0].program_index, 0);
        assert_eq!(snapshot.groups[1].group_index, 1);
        assert!(snapshot.groups[1].is_hidden);
    }
}
