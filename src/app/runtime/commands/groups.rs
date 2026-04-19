use crate::app::models::{AppStateStorage, CoreGroup, LogManager};
use crate::app::navigation::{GroupRoute, WindowRoute};
use crate::app::runtime::UiState;
use std::sync::{Arc, RwLock};

pub fn set_group_is_hidden(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    index: usize,
    is_hidden: bool,
) -> bool {
    {
        let mut state = persistent_state.write().unwrap();
        if let Some(group) = state.groups.get_mut(index) {
            if group.is_hidden != is_hidden {
                group.is_hidden = is_hidden;
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

pub fn swap_groups(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    index1: usize,
    index2: usize,
) -> bool {
    {
        let mut state = persistent_state.write().unwrap();
        if index1 < state.groups.len() && index2 < state.groups.len() {
            state.groups.swap(index1, index2);
            true
        } else {
            false
        }
    }
}

pub fn create_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    ui: &mut UiState,
    log_manager: &mut LogManager,
) -> bool {
    let group_name_trimmed = ui.group_form.group_name.trim();
    if group_name_trimmed.is_empty() {
        log_manager.add_entry("Group name cannot be empty".into());
        return false;
    }

    let selected_cores: Vec<usize> = ui
        .group_form
        .core_selection
        .iter()
        .enumerate()
        .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
        .collect();

    if selected_cores.is_empty() {
        log_manager.add_entry("At least one core must be selected".into());
        return false;
    }

    {
        let mut state = persistent_state.write().unwrap();
        state.groups.push(CoreGroup {
            name: group_name_trimmed.to_string(),
            cores: selected_cores,
            programs: vec![],
            is_hidden: false,
            run_all_button: ui.group_form.run_all_enabled,
        });
    }
    true
}

pub fn update_group_properties(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    index: usize,
    name: String,
    cores: Vec<usize>,
    run_all_button: bool,
) -> bool {
    {
        let mut state = persistent_state.write().unwrap();
        if index < state.groups.len() {
            state.groups[index].name = name;
            state.groups[index].cores = cores;
            state.groups[index].run_all_button = run_all_button;
            true
        } else {
            false
        }
    }
}

pub fn remove_group(persistent_state: &Arc<RwLock<AppStateStorage>>, index: usize) -> bool {
    {
        let mut state = persistent_state.write().unwrap();
        if index < state.groups.len() {
            state.groups.remove(index);
            true
        } else {
            false
        }
    }
}

pub fn start_editing_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    ui: &mut UiState,
    log_manager: &mut LogManager,
    group_index: usize,
) {
    let total_cores = ui.group_form.core_selection.len();
    {
        let state = persistent_state.read().unwrap();
        ui.group_form.core_selection = {
            let mut selection = vec![false; total_cores];
            if let Some(group) = state.groups.get(group_index) {
                for &core in &group.cores {
                    if core < total_cores {
                        selection[core] = true;
                    }
                }
            }
            selection
        };

        if let Some(group) = state.groups.get(group_index) {
            ui.group_form.group_name = group.name.clone();
            ui.group_form.run_all_enabled = group.run_all_button;
        } else {
            log_manager.add_entry(format!("Group with index {group_index} not found"));
        }
    }

    ui.group_form.editing_index = Some(group_index);
    ui.group_form.last_clicked_core = None;
    ui.current_window = WindowRoute::Groups(GroupRoute::Edit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::AppStateStorage;
    use crate::app::models::CpuSchema;

    fn sample_persistent_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 4,
            groups: vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![1, 3],
                programs: vec![],
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
    fn test_create_group_mutates_state() {
        let persistent_state = sample_persistent_state();
        let mut ui = UiState::new(4);
        let mut log_manager = LogManager::default();

        ui.group_form.group_name = "Work".to_string();
        ui.group_form.run_all_enabled = true;
        ui.group_form.core_selection[0] = true;
        ui.group_form.core_selection[2] = true;

        assert!(create_group(&persistent_state, &mut ui, &mut log_manager));

        let state = persistent_state.read().unwrap();
        let group = state.groups.last().unwrap();
        assert_eq!(group.name, "Work");
        assert_eq!(group.cores, vec![0, 2]);
        assert!(group.run_all_button);
        assert_eq!(ui.group_form.group_name, "Work");
        assert!(ui.group_form.core_selection[0]);
        assert!(ui.group_form.core_selection[2]);
    }

    #[test]
    fn test_start_editing_group_populates_form() {
        let persistent_state = sample_persistent_state();
        let mut ui = UiState::new(4);
        let mut log_manager = LogManager::default();

        start_editing_group(&persistent_state, &mut ui, &mut log_manager, 0);

        assert_eq!(ui.group_form.editing_index, Some(0));
        assert_eq!(ui.group_form.group_name, "Games");
        assert!(ui.group_form.run_all_enabled);
        assert_eq!(ui.group_form.core_selection, vec![false, true, false, true]);
        assert!(matches!(
            ui.current_window,
            WindowRoute::Groups(GroupRoute::Edit)
        ));
    }

    #[test]
    fn test_swap_hide_update_and_remove_group() {
        let persistent_state = sample_persistent_state();
        {
            let mut state = persistent_state.write().unwrap();
            state.groups.push(CoreGroup {
                name: "Work".to_string(),
                cores: vec![0],
                programs: vec![],
                is_hidden: false,
                run_all_button: false,
            });
        }

        assert!(swap_groups(&persistent_state, 0, 1));
        set_group_is_hidden(&persistent_state, 0, true);
        assert!(update_group_properties(
            &persistent_state,
            0,
            "Edited".to_string(),
            vec![2, 3],
            true
        ));
        assert!(remove_group(&persistent_state, 1));

        let state = persistent_state.read().unwrap();
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "Edited");
        assert_eq!(state.groups[0].cores, vec![2, 3]);
        assert!(state.groups[0].is_hidden);
        assert!(state.groups[0].run_all_button);
    }
}
