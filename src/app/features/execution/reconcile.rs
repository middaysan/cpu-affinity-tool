use crate::app::features::rules::RulesContext;
use crate::app::features::execution::MonitorEventSender;
use crate::app::models::{AppRuntimeKey, AppStateStorage, RunningApps};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::events::ShellEvent;
use os_api::{PriorityClass, ProcessSettingsApplyOutcome, OS};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone)]
struct ProgramRuntimeSettings {
    name: String,
    group_id: GroupId,
    rule_id: RuleId,
    expected_mask: usize,
    expected_priority: PriorityClass,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProcessSettingsIterationOutcome {
    changed: bool,
    notifications: Vec<String>,
}

trait ProcessSettingsOs {
    fn apply_process_settings_if_instance(
        &mut self,
        pid: u32,
        expected_instance_token: u64,
        mask: usize,
        priority: PriorityClass,
        apply_changes: bool,
    ) -> Result<ProcessSettingsApplyOutcome, String>;
}

struct RealProcessSettingsOs;

impl ProcessSettingsOs for RealProcessSettingsOs {
    fn apply_process_settings_if_instance(
        &mut self,
        pid: u32,
        expected_instance_token: u64,
        mask: usize,
        priority: PriorityClass,
        apply_changes: bool,
    ) -> Result<ProcessSettingsApplyOutcome, String> {
        OS::apply_process_settings_if_instance(
            pid,
            expected_instance_token,
            mask,
            priority,
            apply_changes,
        )
    }
}

pub async fn run_process_settings_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    monitor_tx: MonitorEventSender,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut os = RealProcessSettingsOs;

    loop {
        interval.tick().await;

        let (state_snapshot, monitoring_enabled) = {
            let state = match app_state.read() {
                Ok(guard) => guard,
                Err(_) => {
                    monitor_tx.try_send(ShellEvent::Warning(
                        "WARNING: persistent_state lock poisoned, skipping monitor iteration"
                            .to_string(),
                    ));
                    continue;
                }
            };
            (state.clone(), state.process_monitoring_enabled)
        };

        if let Ok(mut apps) = running_apps.try_write() {
            let outcome = process_settings_iteration_with_os(
                &mut apps,
                &state_snapshot,
                monitoring_enabled,
                &mut os,
            );

            if !outcome.notifications.is_empty() {
                for message in outcome.notifications {
                    monitor_tx.try_send(ShellEvent::Monitor(format!("MONITOR: {}", message)));
                    #[cfg(debug_assertions)]
                    println!("MONITOR: {}", message);
                }
            }

            if outcome.changed {
                monitor_tx.try_send(ShellEvent::RuntimeStateChanged);
            }
        }
    }
}

fn collect_program_settings(
    state: &AppStateStorage,
) -> HashMap<AppRuntimeKey, ProgramRuntimeSettings> {
    let mut settings = HashMap::new();
    let rules = RulesContext::from_storage(state);
    let snapshot = rules.snapshot(state);

    for group in snapshot.groups {
        let Ok(expected_mask) = affinity_mask_from_cores(&group.cores) else {
            continue;
        };

        for program in group.rules {
            settings.insert(
                program.app.get_key(),
                ProgramRuntimeSettings {
                    name: program.app.name.clone(),
                    group_id: group.id.clone(),
                    rule_id: program.id,
                    expected_mask,
                    expected_priority: program.app.priority,
                },
            );
        }
    }

    settings
}

fn affinity_mask_from_cores(cores: &[usize]) -> Result<usize, String> {
    let mut mask = 0usize;
    for &core_index in cores {
        let bit = 1usize
            .checked_shl(core_index as u32)
            .ok_or_else(|| format!("core index {core_index} out of range for affinity mask"))?;
        mask |= bit;
    }
    if mask == 0 {
        return Err("affinity mask is empty".to_string());
    }
    Ok(mask)
}

fn process_settings_iteration_with_os<O: ProcessSettingsOs>(
    apps: &mut RunningApps,
    state: &AppStateStorage,
    monitoring_enabled: bool,
    os: &mut O,
) -> ProcessSettingsIterationOutcome {
    let key_to_settings = collect_program_settings(state);
    let mut outcome = ProcessSettingsIterationOutcome::default();

    for (app_key, app) in apps.apps.iter_mut() {
        if let Some(settings) = key_to_settings.get(app_key) {
            app.group_id = settings.group_id.clone();
            app.rule_id = settings.rule_id.clone();

            let mut all_matched = true;

            for &pid in &app.pids {
                let Some(&instance_token) = app.pid_instance_tokens.get(&pid) else {
                    all_matched = false;
                    continue;
                };

                match os.apply_process_settings_if_instance(
                    pid,
                    instance_token,
                    settings.expected_mask,
                    settings.expected_priority,
                    monitoring_enabled,
                ) {
                    Ok(applied) if applied.affinity_changed || applied.priority_changed => {
                        all_matched = false;
                        if monitoring_enabled {
                            if applied.affinity_changed {
                                outcome.notifications.push(format!(
                                    "Fixed affinity for {} (PID {}): {:X} -> {:X}",
                                    settings.name,
                                    pid,
                                    applied.previous_affinity,
                                    settings.expected_mask
                                ));
                            }
                            if applied.priority_changed {
                                outcome.notifications.push(format!(
                                    "Fixed priority for {} (PID {}): {:?} -> {:?}",
                                    settings.name,
                                    pid,
                                    applied.previous_priority,
                                    settings.expected_priority
                                ));
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(_) => all_matched = false,
                }
            }

            if app.settings_matched != all_matched {
                app.settings_matched = all_matched;
                outcome.changed = true;
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::{affinity_mask_from_cores, process_settings_iteration_with_os, ProcessSettingsOs};
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, RunningApps};
    use crate::app::shared::ids::{GroupId, RuleId};
    use os_api::{PriorityClass, ProcessSettingsApplyOutcome};
    use std::collections::HashMap;
    use std::path::PathBuf;

    struct FakeProcessSettingsOs {
        affinity: HashMap<u32, usize>,
        priority: HashMap<u32, PriorityClass>,
        tokens: HashMap<u32, u64>,
        affinity_sets: Vec<(u32, usize)>,
        priority_sets: Vec<(u32, PriorityClass)>,
    }

    impl FakeProcessSettingsOs {
        fn new(affinity: HashMap<u32, usize>, priority: HashMap<u32, PriorityClass>) -> Self {
            Self {
                affinity,
                priority,
                tokens: HashMap::new(),
                affinity_sets: Vec::new(),
                priority_sets: Vec::new(),
            }
        }
    }

    impl ProcessSettingsOs for FakeProcessSettingsOs {
        fn apply_process_settings_if_instance(
            &mut self,
            pid: u32,
            expected_instance_token: u64,
            mask: usize,
            priority: PriorityClass,
            apply_changes: bool,
        ) -> Result<ProcessSettingsApplyOutcome, String> {
            if self.tokens.get(&pid).copied() != Some(expected_instance_token) {
                return Err("process instance no longer matches the tracked PID".to_string());
            }
            let previous_affinity = self
                .affinity
                .get(&pid)
                .copied()
                .ok_or_else(|| format!("missing affinity for pid {pid}"))?;
            let previous_priority = self
                .priority
                .get(&pid)
                .copied()
                .ok_or_else(|| format!("missing priority for pid {pid}"))?;
            let affinity_changed = previous_affinity != mask;
            let priority_changed = previous_priority != priority;
            if apply_changes && affinity_changed {
                self.affinity.insert(pid, mask);
                self.affinity_sets.push((pid, mask));
            }
            if apply_changes && priority_changed {
                self.priority.insert(pid, priority);
                self.priority_sets.push((pid, priority));
            }
            Ok(ProcessSettingsApplyOutcome {
                previous_affinity,
                previous_priority,
                affinity_changed,
                priority_changed,
            })
        }
    }

    fn groups_with_programs() -> Vec<CoreGroup> {
        vec![
            CoreGroup {
                name: "Media".to_string(),
                cores: vec![0],
                programs: vec![AppToRun::new_path(
                    PathBuf::from(r"C:\media.lnk"),
                    vec![],
                    PathBuf::from(r"C:\media.exe"),
                    PriorityClass::Normal,
                    false,
                )],
                is_hidden: false,
                run_all_button: true,
            },
            CoreGroup {
                name: "Games".to_string(),
                cores: vec![1, 2],
                programs: vec![AppToRun::new_path(
                    PathBuf::from(r"C:\game.lnk"),
                    vec![],
                    PathBuf::from(r"C:\game.exe"),
                    PriorityClass::High,
                    false,
                )],
                is_hidden: false,
                run_all_button: true,
            },
        ]
    }

    fn sample_state() -> AppStateStorage {
        AppStateStorage {
            version: 5,
            groups: groups_with_programs(),
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
            windows_event_log_diagnostics_enabled: true,
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }
    }

    fn group_id(value: usize) -> GroupId {
        GroupId(format!("group-{value}"))
    }

    fn rule_id(value: usize) -> RuleId {
        RuleId(format!("rule-{value}"))
    }

    fn seed_tracked_instance(
        apps: &mut RunningApps,
        key: &crate::app::models::AppRuntimeKey,
        pid: u32,
        os: &mut FakeProcessSettingsOs,
    ) {
        let token = u64::from(pid) + 10_000;
        apps.apps
            .get_mut(key)
            .unwrap()
            .pid_instance_tokens
            .insert(pid, token);
        os.tokens.insert(pid, token);
    }

    #[test]
    fn test_remap_group_and_rule_ids_by_app_key() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 77, group_id(1), rule_id(0));
        if let Some(app) = apps.apps.get_mut(&key) {
            app.group_id = group_id(9);
            app.rule_id = rule_id(9);
        }

        let mut reordered_state = state.clone();
        reordered_state.groups = vec![state.groups[1].clone()];
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(77, 0b110)]),
            HashMap::from([(77, PriorityClass::High)]),
        );
        seed_tracked_instance(&mut apps, &key, 77, &mut os);

        let outcome =
            process_settings_iteration_with_os(&mut apps, &reordered_state, false, &mut os);

        assert!(!outcome.changed);
        let app = apps.apps.get(&key).unwrap();
        assert_eq!(app.group_id, group_id(0));
        assert_eq!(app.rule_id, rule_id(0));
    }

    #[test]
    fn test_mismatch_without_monitoring_updates_status_without_correction() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 88, group_id(1), rule_id(0));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(88, 0b001)]),
            HashMap::from([(88, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 88, &mut os);

        let outcome = process_settings_iteration_with_os(&mut apps, &state, false, &mut os);

        assert!(outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(os.affinity_sets.is_empty());
        assert!(os.priority_sets.is_empty());
        assert!(!apps.apps.get(&key).unwrap().settings_matched);
    }

    #[test]
    fn test_mismatch_with_monitoring_triggers_corrections_and_notifications() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 89, group_id(1), rule_id(0));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(89, 0b001)]),
            HashMap::from([(89, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 89, &mut os);

        let outcome = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);

        assert!(outcome.changed);
        assert_eq!(os.affinity_sets, vec![(89, 0b110)]);
        assert_eq!(os.priority_sets, vec![(89, PriorityClass::High)]);
        assert_eq!(outcome.notifications.len(), 2);
        assert!(!apps.apps.get(&key).unwrap().settings_matched);
    }

    #[test]
    fn test_second_pass_returns_to_settings_matched_after_correction() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 90, group_id(1), rule_id(0));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(90, 0b001)]),
            HashMap::from([(90, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 90, &mut os);

        let first = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert!(first.changed);
        assert!(!apps.apps.get(&key).unwrap().settings_matched);

        let second = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert!(second.changed);
        assert!(second.notifications.is_empty());
        assert!(apps.apps.get(&key).unwrap().settings_matched);
    }

    #[test]
    fn test_matched_settings_produce_no_notifications() {
        let state = sample_state();
        let key = state.groups[0].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 91, group_id(0), rule_id(0));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(91, 0b001)]),
            HashMap::from([(91, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 91, &mut os);

        let outcome = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.get(&key).unwrap().settings_matched);
    }

    #[test]
    fn stale_pid_token_never_applies_settings() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 92, group_id(1), rule_id(0));
        apps.apps
            .get_mut(&key)
            .unwrap()
            .pid_instance_tokens
            .insert(92, 1);
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(92, 0b001)]),
            HashMap::from([(92, PriorityClass::Normal)]),
        );
        os.tokens.insert(92, 2);

        let outcome = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);

        assert!(outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(os.affinity_sets.is_empty());
        assert!(os.priority_sets.is_empty());
    }

    #[test]
    fn affinity_mask_rejects_empty_and_out_of_range_cores() {
        assert_eq!(
            affinity_mask_from_cores(&[]),
            Err("affinity mask is empty".to_string())
        );
        let error = affinity_mask_from_cores(&[usize::BITS as usize]).unwrap_err();
        assert!(error.contains("out of range"));
    }
}
