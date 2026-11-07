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

#[cfg(test)]
mod tests {
    use super::super::{GroupEdit, RulesContext};
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
        let mut state = persistent_state.write().unwrap();
        state.groups[0].is_hidden = true;
        let mut rules = RulesContext::from_storage(&state);
        let old_ids = rules.to_persisted_identities();
        let old_programs = state.groups[0].programs.clone();
        GroupEdit {
            group: None,
            name: " Work ".into(),
            cpus: vec![2, 0, 2],
            run_all: true,
            hidden: None,
        }
        .apply(&mut state, &mut rules, 4)
        .unwrap();
        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.groups[1].cores, vec![0, 2]);
        assert_eq!(state.groups[1].name, "Work");
        assert!(!state.groups[1].is_hidden);
        let ids = rules.to_persisted_identities();
        assert_eq!(ids.groups[0], old_ids.groups[0]);
        assert_ne!(ids.groups[0].id, ids.groups[1].id);
        GroupEdit {
            group: Some(ids.groups[0].id.clone()),
            name: " Edited ".into(),
            cpus: vec![3, 1, 3],
            run_all: false,
            hidden: None,
        }
        .apply(&mut state, &mut rules, 4)
        .unwrap();
        assert_eq!(
            rules.to_persisted_identities(),
            ids,
            "An edit cannot allocate new identities"
        );
        assert_eq!(state.groups[0].programs, old_programs);
        assert!(state.groups[0].is_hidden, "GUI edits preserve visibility");
        drop(state);
        let editable = load_group_for_edit(&persistent_state, 0).unwrap();
        assert_eq!(editable.name, "Edited");
        assert_eq!(editable.selected_cores, vec![1, 3]);
        assert!(!editable.run_all_enabled);
        GroupEdit {
            group: Some(ids.groups[0].id.clone()),
            name: "Edited".into(),
            cpus: vec![1, 3],
            run_all: false,
            hidden: Some(false),
        }
        .apply(&mut persistent_state.write().unwrap(), &mut rules, 4)
        .unwrap();
        assert!(
            !persistent_state.read().unwrap().groups[0].is_hidden,
            "Explicit agent visibility edits remain supported"
        );
    }

    #[test]
    fn test_group_validation_does_not_mutate_state_or_ids() {
        let persistent_state = sample_persistent_state();
        let mut state = persistent_state.write().unwrap();
        let mut rules = RulesContext::from_storage(&state);
        let before = serde_json::to_value(&*state).unwrap();
        let ids = rules.to_persisted_identities();
        let target = ids.groups[0].id.clone();
        for group in [None, Some(target)] {
            for (name, cpus, error) in [
                ("   ", vec![0], "Group name cannot be empty"),
                ("No Cores", vec![], "At least one core must be selected"),
                ("Invalid CPU", vec![4], "Select existing logical CPUs"),
            ] {
                assert_eq!(
                    GroupEdit {
                        group: group.clone(),
                        name: name.into(),
                        cpus,
                        run_all: true,
                        hidden: Some(true)
                    }
                    .apply(&mut state, &mut rules, 4)
                    .unwrap_err(),
                    error
                );
            }
        }
        assert!(GroupEdit {
            group: Some(crate::app::shared::ids::GroupId("missing".into())),
            name: "Valid".into(),
            cpus: vec![0],
            run_all: true,
            hidden: None
        }
        .apply(&mut state, &mut rules, 4)
        .is_err());
        assert_eq!(serde_json::to_value(&*state).unwrap(), before);
        assert_eq!(rules.to_persisted_identities(), ids);
    }

    #[test]
    fn test_group_visibility_and_move_report_only_real_changes() {
        let persistent_state = sample_persistent_state();
        persistent_state.write().unwrap().groups.push(CoreGroup {
            name: "Work".into(),
            cores: vec![0],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        });

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
        persistent_state.write().unwrap().groups.push(CoreGroup {
            name: "Work".into(),
            cores: vec![0],
            programs: vec![],
            is_hidden: false,
            run_all_button: false,
        });
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
