use crate::app::adapters::discovery;
use crate::app::models::{AddAppsOutcome, AppStateStorage, AppToRun, CoreGroup};
use os_api::InstalledAppCatalogEntry;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditableGroup {
    pub name: String,
    pub selected_cores: Vec<usize>,
    pub run_all_enabled: bool,
}

pub(super) fn validate_group_values(
    name: &str,
    cpus: &[usize],
    total: usize,
) -> Result<(String, Vec<usize>), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Group name cannot be empty".into());
    }
    if name.len() > 256 || name.contains('\0') {
        return Err("Group name must contain at most 256 bytes and no NUL".into());
    }
    if cpus.is_empty() {
        return Err("At least one core must be selected".into());
    }
    if cpus.len() > 4096 || cpus.iter().any(|cpu| *cpu >= total) {
        return Err("Select existing logical CPUs".into());
    }
    let mut cpus = cpus.to_vec();
    cpus.sort_unstable();
    cpus.dedup();
    Ok((name.into(), cpus))
}

pub fn set_group_is_hidden(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    index: usize,
    is_hidden: bool,
) -> bool {
    let mut state = persistent_state.write().unwrap();
    if let Some(group) = state.groups.get_mut(index) {
        if group.is_hidden != is_hidden {
            group.is_hidden = is_hidden;
            return true;
        }
    }
    false
}

pub fn move_group_to_index(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    source_index: usize,
    target_index: usize,
) -> bool {
    let mut state = persistent_state.write().unwrap();
    if source_index >= state.groups.len()
        || target_index >= state.groups.len()
        || source_index == target_index
    {
        return false;
    }

    let group = state.groups.remove(source_index);
    state.groups.insert(target_index, group);
    true
}

pub fn create_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_name: &str,
    core_selection: &[bool],
    run_all_enabled: bool,
) -> Result<(), String> {
    let (group_name_trimmed, selected_cores) = validate_group_values(
        group_name,
        &selected_cores(core_selection),
        core_selection.len(),
    )?;

    let mut state = persistent_state.write().unwrap();
    state.groups.push(CoreGroup {
        name: group_name_trimmed.to_string(),
        cores: selected_cores,
        programs: vec![],
        is_hidden: false,
        run_all_button: run_all_enabled,
    });
    Ok(())
}

pub fn update_group_properties(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    index: usize,
    name: String,
    core_selection: &[bool],
    run_all_button: bool,
) -> Result<bool, String> {
    let (group_name_trimmed, selected_cores) =
        validate_group_values(&name, &selected_cores(core_selection), core_selection.len())?;

    let mut state = persistent_state.write().unwrap();
    if index < state.groups.len() {
        state.groups[index].name = group_name_trimmed.to_string();
        state.groups[index].cores = selected_cores;
        state.groups[index].run_all_button = run_all_button;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn load_group_for_edit(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
) -> Option<EditableGroup> {
    let state = persistent_state.read().unwrap();
    state.groups.get(group_index).map(|group| EditableGroup {
        name: group.name.clone(),
        selected_cores: group.cores.clone(),
        run_all_enabled: group.run_all_button,
    })
}

fn append_imports(
    group: &mut CoreGroup,
    apps: Vec<AppToRun>,
    mut first_error: Option<String>,
) -> AddAppsOutcome {
    let mut keys: std::collections::HashSet<_> =
        group.programs.iter().map(AppToRun::get_key).collect();
    let mut added_count = 0;
    for app in apps {
        if !keys.insert(app.get_key()) {
            first_error.get_or_insert_with(|| {
                format!(
                    "App '{}' is already in this group; skipped the duplicate",
                    app.name
                )
            });
            continue;
        }
        group.programs.push(app);
        added_count += 1;
    }
    AddAppsOutcome {
        added_count,
        first_error,
    }
}

pub fn add_apps_to_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    paths: Vec<std::path::PathBuf>,
) -> AddAppsOutcome {
    let discovered = discovery::apps_from_dropped_paths(paths);
    let mut state = persistent_state.write().unwrap();
    if let Some(group) = state.groups.get_mut(group_index) {
        append_imports(group, discovered.apps, discovered.first_error)
    } else {
        AddAppsOutcome {
            added_count: 0,
            first_error: Some(format!("Group with index {group_index} not found")),
        }
    }
}

pub fn add_installed_app_to_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    entry: InstalledAppCatalogEntry,
) -> AddAppsOutcome {
    match discovery::app_from_installed_entry(entry) {
        Ok(app) => {
            let mut state = persistent_state.write().unwrap();
            if let Some(group) = state.groups.get_mut(group_index) {
                append_imports(group, vec![app], None)
            } else {
                AddAppsOutcome {
                    added_count: 0,
                    first_error: Some(format!("Group with index {group_index} not found")),
                }
            }
        }
        Err(err) => AddAppsOutcome {
            added_count: 0,
            first_error: Some(err),
        },
    }
}

pub fn load_rule(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_idx: usize,
    rule_idx: usize,
) -> Option<AppToRun> {
    persistent_state
        .read()
        .unwrap()
        .groups
        .get(group_idx)
        .and_then(|group| group.programs.get(rule_idx))
        .cloned()
}

#[cfg(test)]
fn update_rule(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    program_index: usize,
    program: AppToRun,
) -> bool {
    let mut state = persistent_state.write().unwrap();
    if let Some(group) = state.groups.get_mut(group_index) {
        if program_index < group.programs.len() {
            group.programs[program_index] = program;
            return true;
        }
    }
    false
}

#[cfg(test)]
fn remove_rule_from_group(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    group_index: usize,
    program_index: usize,
) -> Option<String> {
    let mut state = persistent_state.write().unwrap();
    let group = state.groups.get_mut(group_index)?;
    if program_index < group.programs.len() {
        let path = group.programs[program_index].launch_target_label();
        group.programs.remove(program_index);
        return Some(path);
    }
    None
}

pub fn move_rule_between_groups_at(
    state: &mut AppStateStorage,
    source_group_index: usize,
    source_rule_index: usize,
    target_group_index: usize,
    target_rule_index: usize,
) -> Option<AppToRun> {
    if source_group_index >= state.groups.len()
        || target_group_index >= state.groups.len()
        || source_rule_index >= state.groups[source_group_index].programs.len()
        || target_rule_index > state.groups[target_group_index].programs.len()
    {
        return None;
    }

    if source_group_index == target_group_index {
        let programs = &mut state.groups[source_group_index].programs;
        let moved = programs.remove(source_rule_index);
        let insert_index = if target_rule_index > source_rule_index {
            target_rule_index - 1
        } else {
            target_rule_index
        };
        programs.insert(insert_index, moved.clone());
        return Some(moved);
    }

    let moved = state.groups[source_group_index]
        .programs
        .remove(source_rule_index);
    state.groups[target_group_index]
        .programs
        .insert(target_rule_index, moved.clone());
    Some(moved)
}

fn selected_cores(core_selection: &[bool]) -> Vec<usize> {
    core_selection
        .iter()
        .enumerate()
        .filter_map(|(index, &selected)| selected.then_some(index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::{CpuSchema, LaunchTarget};
    use os_api::PriorityClass;
    use std::path::PathBuf;

    fn sample_persistent_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage::test_fixture(|state| {
            state.version = 5;
            state.groups = vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![1, 3],
                programs: vec![AppToRun {
                    name: "Sample".to_string(),
                    launch_target: LaunchTarget::Path {
                        dropped_path: PathBuf::from(r"C:\Sample.lnk"),
                        bin_path: PathBuf::from(r"C:\Sample.exe"),
                    },
                    args: vec![],
                    additional_processes: vec![],
                    builtin_process_binding: None,
                    manage_descendants: false,
                    autorun: false,
                    priority: PriorityClass::Normal,
                }],
                is_hidden: false,
                run_all_button: true,
            }];
            state.cpu_schema = CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            };
            state.theme_index = 0;
            state.process_monitoring_enabled = false;
            state.windows_event_log_diagnostics_enabled = true;
            state.start_minimized = false;
            state.rule_identities = None;
            state.loaded_version = 5;
            state.pending_pre_v6_backup = false;
            state.tuning = Default::default();
        })))
    }

    #[test]
    fn s104_import_separates_steam_client_and_games_and_skips_only_exact_duplicates() {
        let client = AppToRun::new_path(
            r"C:\Steam.exe".into(),
            vec![],
            r"C:\Steam.exe".into(),
            os_api::PriorityClass::Normal,
            false,
        );
        let mut game = client.clone();
        game.name = "Game".into();
        game.launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 570,
            executable: None,
        };
        let mut group = CoreGroup {
            name: "Games".into(),
            cores: vec![0],
            programs: vec![client],
            is_hidden: false,
            run_all_button: false,
        };
        let outcome = append_imports(&mut group, vec![game.clone(), game.clone()], None);
        assert_eq!(outcome.added_count, 1);
        assert!(outcome.first_error.unwrap().contains("already"));
        assert_eq!(group.programs.len(), 2);
        let second = append_imports(&mut group, vec![game], None);
        assert_eq!(second.added_count, 0);
        assert_eq!(group.programs.len(), 2);
    }

    #[test]
    fn test_create_and_edit_group() {
        let persistent_state = sample_persistent_state();
        create_group(&persistent_state, "Work", &[true, false, true, false], true).unwrap();
        assert_eq!(persistent_state.read().unwrap().groups.len(), 2);

        let editable = load_group_for_edit(&persistent_state, 0).unwrap();
        assert_eq!(editable.name, "Games");
        assert_eq!(editable.selected_cores, vec![1, 3]);
        assert!(editable.run_all_enabled);

        assert!(update_group_properties(
            &persistent_state,
            0,
            "Edited".to_string(),
            &[false, true, false, true],
            false,
        )
        .unwrap());
        assert_eq!(persistent_state.read().unwrap().groups[0].name, "Edited");
    }

    #[test]
    fn test_create_group_validation_does_not_mutate_state() {
        let persistent_state = sample_persistent_state();

        assert_eq!(
            create_group(&persistent_state, "   ", &[true, false], true),
            Err("Group name cannot be empty".to_string())
        );
        assert_eq!(
            create_group(&persistent_state, "No Cores", &[false, false], true),
            Err("At least one core must be selected".to_string())
        );

        let state = persistent_state.read().unwrap();
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "Games");
    }

    #[test]
    fn test_update_group_validation_and_missing_index_do_not_mutate_state() {
        let persistent_state = sample_persistent_state();

        assert_eq!(
            update_group_properties(&persistent_state, 0, " ".to_string(), &[true], false),
            Err("Group name cannot be empty".to_string())
        );
        assert_eq!(
            update_group_properties(&persistent_state, 0, "Edited".to_string(), &[false], false),
            Err("At least one core must be selected".to_string())
        );
        assert_eq!(
            update_group_properties(&persistent_state, 99, "Missing".to_string(), &[true], false),
            Ok(false)
        );

        let state = persistent_state.read().unwrap();
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "Games");
        assert_eq!(state.groups[0].cores, vec![1, 3]);
        assert!(state.groups[0].run_all_button);
    }

    #[test]
    fn test_group_visibility_and_move_report_only_real_changes() {
        let persistent_state = sample_persistent_state();
        create_group(&persistent_state, "Work", &[true, false], false).unwrap();

        assert!(set_group_is_hidden(&persistent_state, 0, true));
        assert!(!set_group_is_hidden(&persistent_state, 0, true));
        assert!(!set_group_is_hidden(&persistent_state, 99, true));

        assert!(move_group_to_index(&persistent_state, 0, 1));
        assert!(!move_group_to_index(&persistent_state, 0, 99));

        let state = persistent_state.read().unwrap();
        assert_eq!(state.groups[0].name, "Work");
        assert_eq!(state.groups[1].name, "Games");
        assert!(state.groups[1].is_hidden);
    }

    #[test]
    fn test_move_rule_between_groups_rejects_invalid_indices_without_mutation() {
        let persistent_state = sample_persistent_state();
        create_group(&persistent_state, "Work", &[true, false], false).unwrap();
        let mut state = persistent_state.write().unwrap();
        let before = state
            .groups
            .iter()
            .map(|group| {
                (
                    group.name.clone(),
                    group.cores.clone(),
                    group.programs.len(),
                    group.is_hidden,
                    group.run_all_button,
                )
            })
            .collect::<Vec<_>>();

        assert!(move_rule_between_groups_at(&mut state, 9, 0, 1, 0).is_none());
        assert!(move_rule_between_groups_at(&mut state, 0, 9, 1, 0).is_none());
        assert!(move_rule_between_groups_at(&mut state, 0, 0, 9, 0).is_none());
        assert!(move_rule_between_groups_at(&mut state, 0, 0, 1, 9).is_none());

        let after = state
            .groups
            .iter()
            .map(|group| {
                (
                    group.name.clone(),
                    group.cores.clone(),
                    group.programs.len(),
                    group.is_hidden,
                    group.run_all_button,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(after, before);
    }

    #[test]
    fn test_update_and_remove_rule() {
        let persistent_state = sample_persistent_state();
        let updated = AppToRun {
            name: "Updated".to_string(),
            launch_target: LaunchTarget::Path {
                dropped_path: PathBuf::from(r"C:\Sample.lnk"),
                bin_path: PathBuf::from(r"C:\Updated.exe"),
            },
            args: vec!["--debug".to_string()],
            additional_processes: vec!["helper.exe".to_string()],
            builtin_process_binding: None,
            manage_descendants: false,
            autorun: true,
            priority: PriorityClass::High,
        };

        assert!(update_rule(&persistent_state, 0, 0, updated));
        let removed = remove_rule_from_group(&persistent_state, 0, 0);
        assert_eq!(removed.as_deref(), Some(r"C:\Updated.exe"));
        assert!(persistent_state.read().unwrap().groups[0]
            .programs
            .is_empty());
    }
}
