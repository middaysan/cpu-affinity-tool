use crate::app::models::cpu_schema::CpuSchema;
use crate::app::models::{
    effective_total_threads, AddAppsOutcome, AppRuntimeKey, AppStateStorage, AppStatus, AppToRun,
    LogManager, StateStorageMode,
};
use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::commands::{apps, groups, launch, preferences};
use crate::app::runtime::{RuntimeRegistry, UiState};
use os_api::{InstalledAppCatalogEntry, InstalledAppCatalogTarget};
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CentralProgramSnapshot {
    pub group_index: usize,
    pub program_index: usize,
    pub name: String,
    pub launch_target_detail: String,
    pub app_key: AppRuntimeKey,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledAppPickerRowSnapshot {
    pub entry_index: usize,
    pub name: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InstalledAppPickerSnapshot {
    pub query: String,
    pub is_refreshing: bool,
    pub last_error: Option<String>,
    pub rows: Vec<InstalledAppPickerRowSnapshot>,
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
                                launch_target_detail: program.launch_target_detail(),
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
            "Adding app targets to group: {group_name}, paths: {paths:?}"
        ));

        let outcome = apps::add_apps_to_group(&self.persistent_state, group_index, paths);
        self.handle_add_apps_outcome(&group_name, attempted_count, outcome);
    }

    pub fn add_installed_app_to_group(
        &mut self,
        group_index: usize,
        entry: InstalledAppCatalogEntry,
    ) {
        let group_name = self.get_group_name(group_index).unwrap_or_default();
        let app_name = entry.name.clone();
        let outcome = apps::add_installed_app_to_group(&self.persistent_state, group_index, entry);

        if outcome.added_count > 0 {
            let _ = self.persist_state();
            self.log_manager.add_entry(format!(
                "Added installed app '{app_name}' to group: {group_name}"
            ));
        }

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding installed app '{app_name}': {err}"));
        }
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
                    .add_entry(format!("Added app targets to group: {group_name}"));
            } else {
                self.log_manager.add_entry(format!(
                    "Added {} app targets to group: {}",
                    outcome.added_count, group_name
                ));
            }
        }

        if let Some(err) = outcome.first_error {
            self.log_manager
                .add_entry(format!("Error adding app targets: {err}"));
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
        let leaving_installed_app_picker =
            matches!(self.ui.current_window, WindowRoute::InstalledAppPicker)
                && !matches!(window, WindowRoute::InstalledAppPicker);

        if leaving_installed_app_picker {
            self.reset_installed_app_picker_session();
        }

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
                .add_entry(format!("No app targets to run in group: {group_name}"));
            return;
        }

        for (program_index, program) in programs.into_iter().enumerate() {
            self.run_app_with_affinity_sync(group_index, program_index, program);
        }
    }

    pub fn get_app_status_sync(&mut self, app_key: &AppRuntimeKey) -> AppStatus {
        self.runtime.get_app_status_sync(app_key)
    }

    pub fn get_running_app_pids(&self, app_key: &AppRuntimeKey) -> Option<Vec<u32>> {
        self.runtime.get_running_app_pids(app_key)
    }

    pub fn get_tip(&mut self, current_time: f64) -> &str {
        self.ui.current_tip(current_time)
    }

    pub fn open_installed_app_picker(&mut self, group_index: usize) {
        let picker = &mut self.ui.installed_app_picker;
        picker.target_group_index = Some(group_index);
        picker.query.clear();
        picker.last_error = None;
        picker.needs_focus = true;
        picker.selected_entry_index = picker.entries.first().map(|_| 0);
        self.normalize_installed_app_picker_selection();
        self.ui.set_current_window(WindowRoute::InstalledAppPicker);
        self.request_installed_app_picker_refresh();
    }

    pub fn close_installed_app_picker(&mut self) {
        self.reset_installed_app_picker_session();
        self.ui
            .set_current_window(WindowRoute::Groups(GroupRoute::List));
    }

    pub fn build_installed_app_picker_snapshot(&self) -> InstalledAppPickerSnapshot {
        let picker = &self.ui.installed_app_picker;
        let rows = self
            .filtered_installed_app_entry_indices()
            .into_iter()
            .map(|entry_index| {
                let entry = &picker.entries[entry_index];
                let detail = match &entry.target {
                    InstalledAppCatalogTarget::Aumid(aumid) => format!("Installed app ({aumid})"),
                    InstalledAppCatalogTarget::Path(path) => path.display().to_string(),
                };

                InstalledAppPickerRowSnapshot {
                    entry_index,
                    name: entry.name.clone(),
                    detail,
                    selected: picker.selected_entry_index == Some(entry_index),
                }
            })
            .collect();

        InstalledAppPickerSnapshot {
            query: picker.query.clone(),
            is_refreshing: picker.is_refreshing,
            last_error: picker.last_error.clone(),
            rows,
        }
    }

    pub fn set_installed_app_picker_query(&mut self, query: String) {
        self.ui.installed_app_picker.query = query;
        self.normalize_installed_app_picker_selection();
    }

    pub fn select_installed_app_picker_entry(&mut self, entry_index: usize) {
        self.ui.installed_app_picker.selected_entry_index = Some(entry_index);
    }

    pub fn select_next_installed_app_picker_entry(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        if filtered.is_empty() {
            self.ui.installed_app_picker.selected_entry_index = None;
            return;
        }

        let picker = &mut self.ui.installed_app_picker;
        let next_position = picker
            .selected_entry_index
            .and_then(|selected| filtered.iter().position(|&idx| idx == selected))
            .map(|pos| (pos + 1).min(filtered.len() - 1))
            .unwrap_or(0);
        picker.selected_entry_index = Some(filtered[next_position]);
    }

    pub fn select_previous_installed_app_picker_entry(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        if filtered.is_empty() {
            self.ui.installed_app_picker.selected_entry_index = None;
            return;
        }

        let picker = &mut self.ui.installed_app_picker;
        let prev_position = picker
            .selected_entry_index
            .and_then(|selected| filtered.iter().position(|&idx| idx == selected))
            .map(|pos| pos.saturating_sub(1))
            .unwrap_or(0);
        picker.selected_entry_index = Some(filtered[prev_position]);
    }

    pub fn request_installed_app_picker_refresh(&mut self) {
        if self.ui.installed_app_picker.is_refreshing {
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.ui.installed_app_picker.is_refreshing = true;
        self.ui.installed_app_picker.last_error = None;
        self.ui.installed_app_picker.refresh_rx = Some(rx);

        std::thread::spawn(move || {
            let _ = tx.send(os_api::OS::list_supported_start_apps());
        });
    }

    pub fn poll_installed_app_picker_refresh(&mut self) {
        let Some(rx) = self.ui.installed_app_picker.refresh_rx.take() else {
            return;
        };

        match rx.try_recv() {
            Ok(result) => {
                self.ui.installed_app_picker.is_refreshing = false;

                let previous_selection = self
                    .ui
                    .installed_app_picker
                    .selected_entry_index
                    .and_then(|idx| self.ui.installed_app_picker.entries.get(idx).cloned());

                match result {
                    Ok(entries) => {
                        self.ui.installed_app_picker.entries = entries;
                        self.ui.installed_app_picker.last_error = None;
                        self.ui.installed_app_picker.selected_entry_index = previous_selection
                            .and_then(|selected| {
                                self.ui
                                    .installed_app_picker
                                    .entries
                                    .iter()
                                    .position(|entry| entry == &selected)
                            });
                    }
                    Err(err) => {
                        self.log_manager
                            .add_entry(format!("Installed app refresh failed: {err}"));
                        self.ui.installed_app_picker.last_error = Some(err);
                    }
                }

                self.normalize_installed_app_picker_selection();
            }
            Err(TryRecvError::Empty) => {
                self.ui.installed_app_picker.refresh_rx = Some(rx);
            }
            Err(TryRecvError::Disconnected) => {
                self.ui.installed_app_picker.is_refreshing = false;
                self.log_manager
                    .add_entry("Installed app refresh channel disconnected".into());
                self.ui.installed_app_picker.last_error =
                    Some("Installed app refresh channel disconnected".into());
            }
        }
    }

    pub fn take_installed_app_picker_focus_request(&mut self) -> bool {
        if self.ui.installed_app_picker.needs_focus {
            self.ui.installed_app_picker.needs_focus = false;
            true
        } else {
            false
        }
    }

    pub fn confirm_selected_installed_app(&mut self) -> bool {
        let Some(group_index) = self.ui.installed_app_picker.target_group_index else {
            return false;
        };

        let Some(entry_index) = self
            .ui
            .installed_app_picker
            .selected_entry_index
            .or_else(|| self.filtered_installed_app_entry_indices().first().copied())
        else {
            return false;
        };

        let Some(entry) = self
            .ui
            .installed_app_picker
            .entries
            .get(entry_index)
            .cloned()
        else {
            return false;
        };

        self.add_installed_app_to_group(group_index, entry);
        self.close_installed_app_picker();
        true
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

    pub fn active_data_dir(&self) -> PathBuf {
        AppStateStorage::active_data_dir()
    }

    pub fn active_storage_mode(&self) -> StateStorageMode {
        AppStateStorage::active_storage_mode()
    }

    pub fn open_active_data_dir(&mut self) {
        let data_dir = self.active_data_dir();
        if let Err(err) = os_api::OS::open_directory(&data_dir) {
            self.log_manager.add_important_sticky_once(format!(
                "ERROR: Failed to open data folder '{}': {err}",
                data_dir.display()
            ));
        }
    }

    fn filtered_installed_app_entry_indices(&self) -> Vec<usize> {
        let query = self.ui.installed_app_picker.query.trim().to_lowercase();

        self.ui
            .installed_app_picker
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if query.is_empty() || entry.name.to_lowercase().contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }

    fn normalize_installed_app_picker_selection(&mut self) {
        let filtered = self.filtered_installed_app_entry_indices();
        let picker = &mut self.ui.installed_app_picker;

        if filtered.is_empty() {
            picker.selected_entry_index = None;
            return;
        }

        if picker
            .selected_entry_index
            .is_some_and(|index| filtered.contains(&index))
        {
            return;
        }

        picker.selected_entry_index = Some(filtered[0]);
    }

    fn reset_installed_app_picker_session(&mut self) {
        let picker = &mut self.ui.installed_app_picker;
        picker.target_group_index = None;
        picker.query.clear();
        picker.selected_entry_index = None;
        picker.is_refreshing = false;
        picker.last_error = None;
        picker.needs_focus = false;
        picker.refresh_rx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::app::models::{
        AppStateStorage, AppToRun, CoreGroup, CpuSchema, LaunchTarget, LogManager,
    };
    use crate::app::navigation::{GroupRoute, WindowRoute};
    use crate::app::runtime::{RuntimeRegistry, UiState};
    use os_api::PriorityClass;
    use os_api::{InstalledAppCatalogEntry, InstalledAppCatalogTarget};
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    fn sample_state() -> AppState {
        AppState {
            persistent_state: Arc::new(RwLock::new(AppStateStorage {
                version: 5,
                groups: vec![CoreGroup {
                    name: "Games".to_string(),
                    cores: vec![0, 1],
                    programs: vec![AppToRun::new_path(
                        PathBuf::from(r"C:\Sample.lnk"),
                        vec![],
                        PathBuf::from(r"C:\Sample.exe"),
                        PriorityClass::Normal,
                        false,
                    )],
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
        let mut updated = AppToRun::new_path(
            PathBuf::from(r"C:\Sample.lnk"),
            vec!["--debug".to_string()],
            PathBuf::from(r"C:\Updated.exe"),
            PriorityClass::High,
            true,
        );
        updated.name = "Updated".to_string();
        updated.additional_processes = vec!["helper.exe".to_string()];
        app.ui.app_edit_state.current_edit = Some(updated);

        app.commit_current_app_edit_session();

        let state = app.persistent_state.read().unwrap();
        let updated = &state.groups[0].programs[0];
        assert_eq!(updated.name, "Updated");
        assert_eq!(
            updated.bin_path(),
            Some(PathBuf::from(r"C:\Updated.exe").as_path())
        );
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
            state.groups[0].programs[1].bin_path(),
            Some(PathBuf::from(r"C:\valid.exe").as_path())
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

    #[test]
    fn test_installed_app_picker_open_query_navigation_and_close() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![
            InstalledAppCatalogEntry {
                name: "Spotify".into(),
                target: InstalledAppCatalogTarget::Aumid(
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                ),
            },
            InstalledAppCatalogEntry {
                name: "Steam".into(),
                target: InstalledAppCatalogTarget::Aumid("ValveCorporation.Steam!Steam".into()),
            },
        ];

        app.open_installed_app_picker(0);
        assert_eq!(app.ui.installed_app_picker.target_group_index, Some(0));
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::InstalledAppPicker
        ));
        assert!(app.take_installed_app_picker_focus_request());
        assert!(!app.take_installed_app_picker_focus_request());

        app.set_installed_app_picker_query("steam".into());
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));

        app.select_previous_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));

        app.set_installed_app_picker_query(String::new());
        app.select_next_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(1));
        app.select_previous_installed_app_picker_entry();
        assert_eq!(app.ui.installed_app_picker.selected_entry_index, Some(0));

        app.close_installed_app_picker();
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.installed_app_picker.target_group_index.is_none());
        assert!(app.ui.installed_app_picker.query.is_empty());
    }

    #[test]
    fn test_confirm_selected_installed_app_adds_entry_and_saves_once() {
        let mut app = sample_state();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry {
            name: "Spotify".into(),
            target: InstalledAppCatalogTarget::Aumid(
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            ),
        }];

        app.open_installed_app_picker(0);
        assert!(app.confirm_selected_installed_app());

        let state = app.persistent_state.read().unwrap();
        let added = &state.groups[0].programs[1];
        assert!(matches!(
            added.launch_target,
            LaunchTarget::Installed { .. }
        ));
        assert_eq!(
            added.installed_aumid(),
            Some("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify")
        );
        drop(state);
        assert_eq!(app.save_count(), 1);
        assert!(matches!(
            app.ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
        assert!(app.ui.installed_app_picker.target_group_index.is_none());
    }

    #[test]
    fn test_leaving_picker_route_clears_session_but_keeps_cached_entries() {
        let mut app = sample_state();
        let (_tx, rx) = std::sync::mpsc::channel();
        app.ui.installed_app_picker.entries = vec![InstalledAppCatalogEntry {
            name: "Spotify".into(),
            target: InstalledAppCatalogTarget::Aumid(
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            ),
        }];
        app.open_installed_app_picker(0);
        app.ui.installed_app_picker.query = "spot".into();
        app.ui.installed_app_picker.last_error = Some("boom".into());
        app.ui.installed_app_picker.is_refreshing = true;
        app.ui.installed_app_picker.refresh_rx = Some(rx);

        app.set_current_window(WindowRoute::Logs);

        assert!(matches!(app.ui.current_window, WindowRoute::Logs));
        assert!(app.ui.installed_app_picker.target_group_index.is_none());
        assert!(app.ui.installed_app_picker.query.is_empty());
        assert!(app.ui.installed_app_picker.last_error.is_none());
        assert!(!app.ui.installed_app_picker.is_refreshing);
        assert!(app.ui.installed_app_picker.refresh_rx.is_none());
        assert_eq!(app.ui.installed_app_picker.entries.len(), 1);
    }
}
