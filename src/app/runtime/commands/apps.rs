use crate::app::models::{AppStateStorage, AppToRun, LogManager};
use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::UiState;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

pub fn add_apps_to_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let result = {
        let mut state = persistent_state.write().unwrap();
        if let Some(group) = state.groups.get_mut(group_index) {
            group.add_app_to_group(paths)
        } else {
            Err(format!("Group with index {group_index} not found"))
        }
    };
    if result.is_ok() {
        persistent_state.read().unwrap().save_state();
    }
    result
}

pub fn update_program(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    program_index: usize,
    program: AppToRun,
) -> bool {
    let updated = {
        let mut state = persistent_state.write().unwrap();
        if let Some(group) = state.groups.get_mut(group_index) {
            if program_index < group.programs.len() {
                group.programs[program_index] = program;
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if updated {
        persistent_state.read().unwrap().save_state();
    }
    updated
}

pub fn remove_app_from_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    log_manager: &mut LogManager,
    group_index: usize,
    program_index: usize,
) {
    let removed_path = {
        let mut state = persistent_state.write().unwrap();
        if let Some(group) = state.groups.get_mut(group_index) {
            if program_index < group.programs.len() {
                let path = group.programs[program_index].bin_path.display().to_string();
                group.programs.remove(program_index);
                Some(path)
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(path) = removed_path {
        persistent_state.read().unwrap().save_state();
        log_manager.add_entry(format!("Removing app: {}", path));
    }
}

pub fn open_app_edit_session(ui: &mut UiState, group_index: usize, program_index: usize) {
    ui.app_edit_state.current_edit = None;
    ui.app_edit_state.run_settings = Some((group_index, program_index));
    ui.current_window = WindowRoute::AppRunSettings;
}

pub fn close_app_edit_session(ui: &mut UiState) {
    ui.app_edit_state.current_edit = None;
    ui.app_edit_state.run_settings = None;
    ui.current_window = WindowRoute::Groups(GroupRoute::List);
}

pub fn ensure_current_edit_loaded(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    ui: &mut UiState,
    group_idx: usize,
    prog_idx: usize,
) -> bool {
    if ui.app_edit_state.current_edit.is_none() {
        let program = persistent_state
            .read()
            .unwrap()
            .groups
            .get(group_idx)
            .and_then(|group| group.programs.get(prog_idx))
            .cloned();

        if let Some(original) = program {
            ui.app_edit_state.current_edit = Some(original);
        } else {
            close_app_edit_session(ui);
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::{AppStateStorage, CoreGroup, CpuSchema};
    use os_api::PriorityClass;

    fn sample_persistent_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
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
        }))
    }

    #[test]
    fn test_update_and_remove_program() {
        let persistent_state = sample_persistent_state();
        let mut log_manager = LogManager::default();
        let updated = AppToRun {
            name: "Updated".to_string(),
            dropped_path: PathBuf::from(r"C:\Sample.lnk"),
            args: vec!["--debug".to_string()],
            bin_path: PathBuf::from(r"C:\Updated.exe"),
            additional_processes: vec!["helper.exe".to_string()],
            autorun: true,
            priority: PriorityClass::High,
        };

        assert!(update_program(&persistent_state, 0, 0, updated.clone()));
        remove_app_from_group(&persistent_state, &mut log_manager, 0, 0);

        let state = persistent_state.read().unwrap();
        assert!(state.groups[0].programs.is_empty());
        assert_eq!(log_manager.entries.len(), 1);
    }

    #[test]
    fn test_open_ensure_and_close_app_edit_session() {
        let persistent_state = sample_persistent_state();
        let mut ui = UiState::new(4);

        open_app_edit_session(&mut ui, 0, 0);
        assert!(matches!(ui.current_window, WindowRoute::AppRunSettings));
        assert!(ensure_current_edit_loaded(&persistent_state, &mut ui, 0, 0));
        assert_eq!(
            ui.app_edit_state
                .current_edit
                .as_ref()
                .map(|app| app.name.as_str()),
            Some("Sample")
        );

        close_app_edit_session(&mut ui);
        assert!(ui.app_edit_state.current_edit.is_none());
        assert!(matches!(
            ui.current_window,
            WindowRoute::Groups(GroupRoute::List)
        ));
    }
}
