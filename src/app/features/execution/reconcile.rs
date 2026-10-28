mod failures;

use crate::app::features::execution::MonitorEventSender;
use crate::app::features::execution::RunningAppStatusCache;
use crate::app::features::rules::RulesContext;
use crate::app::features::system_tweaks::tweaks::scheduling::fields_for_rule;
use crate::app::models::{AppRuntimeKey, AppStateStorage, AppStatus, RunningApps};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::events::ShellEvent;
use failures::{Failure, FailureLog};
use os_api::gaming::ProcessSettingsFields;
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
    fields: ProcessSettingsFields,
    launch_identity: Option<super::steam_pending::LaunchIdentity>,
}

#[derive(Clone, Copy)]
struct DesiredSettings {
    mask: usize,
    priority: PriorityClass,
    fields: ProcessSettingsFields,
}
impl DesiredSettings {
    fn same(self, other: Self) -> bool {
        self.fields == other.fields
            && (!self.fields.affinity || self.mask == other.mask)
            && (!self.fields.priority || self.priority == other.priority)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProcessSettingsIterationOutcome {
    changed: bool,
    notifications: Vec<String>,
    failures: Vec<Failure>,
}

trait ProcessSettingsOs {
    fn image(&mut self, _pid: u32, _token: u64) -> Option<std::path::PathBuf> {
        None
    }

    fn apply_process_fields_if_instance(
        &mut self,
        pid: u32,
        expected_instance_token: u64,
        mask: usize,
        priority: PriorityClass,
        fields: ProcessSettingsFields,
        apply_changes: bool,
    ) -> Result<ProcessSettingsApplyOutcome, String>;
}

struct RealProcessSettingsOs;

impl ProcessSettingsOs for RealProcessSettingsOs {
    fn image(&mut self, pid: u32, token: u64) -> Option<std::path::PathBuf> {
        OS::get_process_image_path_and_instance_token(pid)
            .ok()
            .filter(|(_, actual)| *actual == token)
            .map(|(path, _)| path)
    }

    fn apply_process_fields_if_instance(
        &mut self,
        pid: u32,
        expected_instance_token: u64,
        mask: usize,
        priority: PriorityClass,
        fields: ProcessSettingsFields,
        apply_changes: bool,
    ) -> Result<ProcessSettingsApplyOutcome, String> {
        OS::apply_process_fields_if_instance(
            pid,
            expected_instance_token,
            mask,
            priority,
            fields,
            apply_changes,
        )
    }
}

pub async fn run_process_settings_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    running_app_statuses: RunningAppStatusCache,
    app_state: Arc<RwLock<AppStateStorage>>,
    monitor_tx: MonitorEventSender,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut os = RealProcessSettingsOs;
    let mut failure_log = FailureLog::default();

    loop {
        interval.tick().await;
        crate::render_profile_scope!(Reconcile);

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
            (
                collect_program_settings(&state),
                state.process_monitoring_enabled,
            )
        };

        if let Ok(mut apps) = running_apps.try_write() {
            let outcome =
                reconcile_settings_with_os(&mut apps, &state_snapshot, monitoring_enabled, &mut os);
            let status_cache_changed =
                record_confirmed_match_statuses(&apps, &running_app_statuses);

            drop(apps);

            for message in failure_log.report(outcome.failures, std::time::Instant::now()) {
                monitor_tx.try_send(ShellEvent::MonitorError(format!("MONITOR: {message}")));
            }

            if !outcome.notifications.is_empty() {
                for message in outcome.notifications {
                    monitor_tx.try_send(ShellEvent::Monitor(format!("MONITOR: {}", message)));
                    #[cfg(debug_assertions)]
                    println!("MONITOR: {}", message);
                }
            }

            if outcome.changed || status_cache_changed {
                monitor_tx.try_send(ShellEvent::RuntimeStateChanged);
            }
        }
    }
}

fn record_confirmed_match_statuses(apps: &RunningApps, statuses: &RunningAppStatusCache) -> bool {
    let mut statuses = statuses.write().unwrap();
    let mut changed = false;
    for (app_key, app) in &apps.apps {
        if app.settings_matched && statuses.get(app_key) == Some(&AppStatus::SettingsMismatch) {
            statuses.insert(app_key.clone(), AppStatus::Running);
            changed = true;
        }
    }
    changed
}

fn collect_program_settings(
    state: &AppStateStorage,
) -> HashMap<AppRuntimeKey, ProgramRuntimeSettings> {
    let mut settings = HashMap::new();
    let rules = RulesContext::from_storage(state);
    for (group_index, group) in state.groups.iter().enumerate() {
        let group_id = rules
            .group_id_for_index(group_index)
            .expect("group identity");
        for (rule_index, app) in group.programs.iter().enumerate() {
            let rule_id = rules
                .rule_id_for_index(group_index, rule_index)
                .expect("rule identity");
            let fields = fields_for_rule(state, &group_id, &rule_id);
            let Ok(expected_mask) = (if fields.affinity {
                affinity_mask_from_cores(&group.cores)
            } else {
                Ok(0)
            }) else {
                continue;
            };
            settings.insert(
                rules.runtime_key(app, &rule_id),
                ProgramRuntimeSettings {
                    name: app.name.clone(),
                    group_id: group_id.clone(),
                    rule_id,
                    expected_mask,
                    expected_priority: app.priority,
                    fields,
                    launch_identity: super::steam_pending::identity_for_app(app),
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

#[cfg(test)]
fn process_settings_iteration_with_os<O: ProcessSettingsOs>(
    apps: &mut RunningApps,
    state: &AppStateStorage,
    monitoring_enabled: bool,
    os: &mut O,
) -> ProcessSettingsIterationOutcome {
    apps.control.configure(state);
    let expected = collect_program_settings(state);
    apps.mutate_membership(|apps| {
        for (key, app) in &mut apps.apps {
            if let Some(spec) = expected.get(key) {
                app.group_id = spec.group_id.clone();
                app.rule_id = spec.rule_id.clone();
            }
        }
    });
    let control = apps.control.clone();
    control.observe(
        apps,
        super::control::policy::Policy::read(state).map_or(u64::MAX, |policy| policy.generation()),
        |_, _| None,
    );
    reconcile_settings_with_os(
        apps,
        &collect_program_settings(state),
        monitoring_enabled,
        os,
    )
}

fn reconcile_settings_with_os<O: ProcessSettingsOs>(
    apps: &mut RunningApps,
    key_to_settings: &HashMap<AppRuntimeKey, ProgramRuntimeSettings>,
    monitoring_enabled: bool,
    os: &mut O,
) -> ProcessSettingsIterationOutcome {
    let scheduling = apps.scheduling.clone();
    let control = apps.control.clone();
    apps.mutate_membership(|apps| {
        let mut outcome = ProcessSettingsIterationOutcome::default();

        for (app_key, app) in apps.apps.iter_mut() {
            if let Some(settings) = key_to_settings.get(app_key) {
                app.group_id = settings.group_id.clone();
                app.rule_id = settings.rule_id.clone();

                let mut all_matched = !app.pids.is_empty();

                for &pid in &app.pids {
                    let Some(&instance_token) = app.pid_instance_tokens.get(&pid) else {
                        all_matched = false;
                        continue;
                    };

                    let initial = super::steam_pending::initial_apply(
                        &settings.rule_id,
                        settings.launch_identity.clone(),
                        pid,
                        instance_token,
                    );
                    let apply_changes = monitoring_enabled || initial;
                    let image = os.image(pid, instance_token);
                    let desired = DesiredSettings {
                        mask: settings.expected_mask,
                        priority: settings.expected_priority,
                        fields: settings.fields,
                    };
                    let mut effective = desired;
                    let request = super::control::WriteRequest {
                        rule: &settings.rule_id,
                        key: app_key,
                        pid,
                        token: instance_token,
                        image: image.as_deref(),
                        mask: desired.mask,
                        priority: desired.priority,
                        fields: desired.fields,
                        change: apply_changes,
                    };
                    let mut apply =
                        |goal: DesiredSettings, write: bool, rule: &RuleId, name: &str| {
                            let fields = scheduling
                                .select(pid, instance_token, goal.fields, write)
                                .map_err(|error| format!("{error:?}"))?;
                            let applied = os
                                .apply_process_fields_if_instance(
                                    pid,
                                    instance_token,
                                    goal.mask,
                                    goal.priority,
                                    fields,
                                    write,
                                )
                                .inspect_err(|error| {
                                    // Only a native correction attempt is an error. Observers and
                                    // unresolved ownership remain read-only, without warning spam.
                                    if write && (fields.affinity || fields.priority) {
                                        outcome.failures.push(Failure::new(
                                            rule,
                                            name,
                                            pid,
                                            instance_token,
                                            error,
                                        ));
                                    }
                                })?;
                            Ok((applied, fields, write))
                        };
                    let guarded = if initial {
                        control.initial(request, |owner| {
                            effective = DesiredSettings {
                                mask: owner.mask,
                                priority: owner.app.priority,
                                fields: owner.fields,
                            };
                            apply(effective, true, &owner.rule, &owner.app.name)
                        })
                    } else {
                        control
                            .write(request, |write| {
                                apply(desired, write, &settings.rule_id, &settings.name)
                            })
                            .map(|(value, _)| value)
                    };
                    let (applied, fields, can_write) = match guarded {
                        Ok(value) => value,
                        Err(_) => {
                            all_matched = false;
                            continue;
                        }
                    };
                    all_matched &= effective.same(desired);
                    match applied {
                        applied if applied.affinity_changed || applied.priority_changed => {
                            all_matched = false;
                            if can_write {
                                if applied.affinity_changed {
                                    outcome.notifications.push(format!(
                                        "Fixed affinity for {} (PID {}): {:X} -> {:X}",
                                        settings.name,
                                        pid,
                                        applied.previous_affinity,
                                        effective.mask
                                    ));
                                }
                                if applied.priority_changed {
                                    outcome.notifications.push(format!(
                                        "Fixed priority for {} (PID {}): {:?} -> {:?}",
                                        settings.name,
                                        pid,
                                        applied.previous_priority,
                                        effective.priority
                                    ));
                                }
                            }
                        }
                        _ => {
                            if initial && fields == effective.fields {
                                super::steam_pending::confirm_instance(
                                    &settings.rule_id,
                                    settings.launch_identity.clone(),
                                    pid,
                                    instance_token,
                                    if matches!(
                                        settings.launch_identity,
                                        Some(super::steam_pending::LaunchIdentity::Known(_))
                                    ) {
                                        !app.primary_instances.is_empty()
                                    } else {
                                        app.primary_instances.get(&pid) == Some(&instance_token)
                                    },
                                );
                            }
                        }
                    }
                }

                if app.settings_matched != all_matched {
                    app.settings_matched = all_matched;
                    outcome.changed = true;
                }
            }
        }

        outcome
    })
}

#[cfg(test)]
mod tests {
    use super::{
        affinity_mask_from_cores, process_settings_iteration_with_os,
        record_confirmed_match_statuses, ProcessSettingsOs,
    };
    use crate::app::models::{
        AppStateStorage, AppStatus, AppToRun, CoreGroup, CpuSchema, RunningApps,
    };
    use crate::app::shared::ids::{GroupId, RuleId};
    use os_api::{PriorityClass, ProcessSettingsApplyOutcome};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    struct FakeProcessSettingsOs {
        affinity: HashMap<u32, usize>,
        priority: HashMap<u32, PriorityClass>,
        tokens: HashMap<u32, u64>,
        affinity_sets: Vec<(u32, usize)>,
        priority_sets: Vec<(u32, PriorityClass)>,
        failures: HashMap<u32, String>,
    }

    impl FakeProcessSettingsOs {
        fn new(affinity: HashMap<u32, usize>, priority: HashMap<u32, PriorityClass>) -> Self {
            Self {
                affinity,
                priority,
                tokens: HashMap::new(),
                affinity_sets: Vec::new(),
                priority_sets: Vec::new(),
                failures: HashMap::new(),
            }
        }
    }

    impl ProcessSettingsOs for FakeProcessSettingsOs {
        fn apply_process_fields_if_instance(
            &mut self,
            pid: u32,
            expected_instance_token: u64,
            mask: usize,
            priority: PriorityClass,
            fields: os_api::gaming::ProcessSettingsFields,
            apply_changes: bool,
        ) -> Result<ProcessSettingsApplyOutcome, String> {
            if self.tokens.get(&pid).copied() != Some(expected_instance_token) {
                return Err("process instance no longer matches the tracked PID".to_string());
            }
            if let Some(error) = self.failures.get(&pid) {
                return Err(error.clone());
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
            let affinity_changed = fields.affinity && previous_affinity != mask;
            let priority_changed = fields.priority && previous_priority != priority;
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
        AppStateStorage::test_fixture(|state| {
            state.version = 5;
            state.groups = groups_with_programs();
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
        })
    }

    fn group_id(value: usize) -> GroupId {
        GroupId(format!("group-{value}"))
    }

    #[test]
    fn s112_off_deferred_launch_applies_effective_owner_then_stops_enforcing() {
        use crate::app::features::execution::{control::policy::Policy, steam_pending};
        let mut state = sample_state();
        state.groups[0].programs[0]
            .additional_processes
            .push("game.exe".into());
        state.groups[1].programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 912112,
            executable: None,
        };
        let mut before = state.clone();
        before.groups.pop();
        Policy::prepare_save(&before, &mut state).unwrap();
        let rules = crate::app::features::rules::RulesContext::from_storage(&state);
        let launched = rules.rule_id_for_index(1, 0).unwrap();
        assert!(steam_pending::mark_steam_launch(&launched, 912112));
        let mut apps = RunningApps::default();
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(42, 4)]),
            HashMap::from([(42, PriorityClass::Normal)]),
        );
        for (i, group) in state.groups.iter().enumerate() {
            let rule = rules.rule_id_for_index(i, 0).unwrap();
            let key = rules.runtime_key(&group.programs[0], &rule);
            apps.add_app(&key, 42, rules.group_id_for_index(i).unwrap(), rule);
            seed_tracked_instance(&mut apps, &key, 42, &mut os);
            if i == 1 {
                let app = apps.apps.get_mut(&key).unwrap();
                app.primary_instances
                    .insert(42, app.pid_instance_tokens[&42]);
            }
        }
        for _ in 0..3 {
            process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        }
        assert_eq!(
            os.affinity[&42], 1,
            "launch intent must apply the earlier owner's settings"
        );
        assert!(!steam_pending::steam_launch_pending(
            &launched,
            Some(912112)
        ));
        os.affinity.insert(42, 8);
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert_eq!(
            os.affinity[&42], 8,
            "Off must remain read-only after confirmed initial correction"
        );
        steam_pending::finish_steam_launch(&launched, 912112);
    }
    #[test]
    fn s110_legacy_conflicting_rules_do_not_rewrite_one_process_before_owner_choice() {
        let state = sample_state();
        let rules = crate::app::features::rules::RulesContext::from_storage(&state);
        let mut apps = RunningApps::default();
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(42, 8)]),
            HashMap::from([(42, PriorityClass::Normal)]),
        );
        for (index, group) in state.groups.iter().enumerate() {
            let key = group.programs[0].get_key();
            apps.add_app(
                &key,
                42,
                rules.group_id_for_index(index).unwrap(),
                rules.rule_id_for_index(index, 0).unwrap(),
            );
            seed_tracked_instance(&mut apps, &key, 42, &mut os);
        }
        process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert!(
            os.affinity_sets.is_empty(),
            "old conflicting rules require an explicit owner choice"
        );
        assert!(os.priority_sets.is_empty());
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
    fn s103_chosen_executable_keeps_initial_apply_with_monitor_off_and_waits_for_readback() {
        let mut state = sample_state();
        state.groups[0].programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 876543,
            executable: None,
        };
        let rules = crate::app::features::rules::RulesContext::from_storage(&state);
        state.rule_identities = Some(rules.to_persisted_identities());
        let rule = rules.rule_id_for_index(0, 0).unwrap();
        let group = rules.group_id_for_index(0).unwrap();
        let old_key = state.groups[0].programs[0].get_key();
        assert!(super::super::steam_pending::mark_steam_launch(
            &rule, 876543
        ));
        state.groups[0].programs[0].launch_target = crate::app::models::LaunchTarget::Steam {
            app_id: 876543,
            executable: Some(r"C:\Game\game.exe".into()),
        };
        let key = state.groups[0].programs[0].get_key();
        assert_ne!(old_key, key);
        let mut apps = RunningApps::default();
        apps.add_app(&key, 42, group, rule.clone());
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(42, 2)]),
            HashMap::from([(42, os_api::PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 42, &mut os);
        apps.apps
            .get_mut(&key)
            .unwrap()
            .primary_instances
            .insert(42, 10042);
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert_eq!(os.affinity_sets, vec![(42, 1)]);
        assert!(
            super::super::steam_pending::steam_launch_pending(&rule, Some(876543)),
            "Setter success is not readback"
        );
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert!(!super::super::steam_pending::steam_launch_pending(
            &rule,
            Some(876543)
        ));
        os.affinity.insert(42, 2);
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert_eq!(
            os.affinity_sets.len(),
            1,
            "Off must not continually enforce a completed instance"
        );
        super::super::steam_pending::finish_steam_launch(&rule, 876543);
    }
    #[test]
    fn s103_empty_roster_cannot_confirm_settings() {
        let state = sample_state();
        let key = state.groups[0].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 42, group_id(0), rule_id(0));
        apps.apps.get_mut(&key).unwrap().pids.clear();
        let mut os = FakeProcessSettingsOs::new(HashMap::new(), HashMap::new());
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert!(!apps.apps[&key].settings_matched);
    }

    #[test]
    fn handoff_monitor_reserves_module_fields_only_on_enabled_platform() {
        use crate::app::features::{
            rules::RulesContext,
            system_tweaks::{
                config::{Association, ModulePreference, Profile, TuningConfig, TuningEnvelope},
                platform::contract::{ActivationEligibility, ActivationMode, ProfileId},
                tweaks,
            },
        };
        let mut state = sample_state();
        let rules = RulesContext::from_storage(&state);
        state.rule_identities = Some(rules.to_persisted_identities());
        let profile = ProfileId::new(1).unwrap();
        let modules = tweaks::modules()
            .into_iter()
            .filter(|m| ["cpu_sets", "process_priority"].contains(&m.descriptor().id.as_str()))
            .map(|m| ModulePreference {
                feature: m.descriptor().id,
                enabled: true,
                mode: ActivationMode::ForegroundSession,
                config: m.editor().default_config(),
            })
            .collect();
        state.tuning = TuningEnvelope::from_config(&TuningConfig {
            profiles: vec![Profile {
                version: 1,
                id: profile,
                name: "Game".into(),
                modules,
            }],
            associations: vec![Association {
                version: 1,
                group: rules.group_id_for_index(1).unwrap(),
                rule: rules.rule_id_for_index(1, 0).unwrap(),
                profile,
                eligibility: ActivationEligibility::Foreground,
            }],
            next_profile: 2,
            ..Default::default()
        })
        .unwrap();
        let saved_tuning = serde_json::to_value(&state.tuning).unwrap();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 323, group_id(1), rule_id(0));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(323, 1)]),
            HashMap::from([(323, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 323, &mut os);
        process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        if crate::app::features::system_tweaks::MODULES_ENABLED {
            assert!(os.affinity_sets.is_empty());
            assert!(os.priority_sets.is_empty());
        } else {
            assert_eq!(os.affinity_sets, vec![(323, 0b110)]);
            assert_eq!(os.priority_sets, vec![(323, PriorityClass::High)]);
        }
        assert_eq!(serde_json::to_value(&state.tuning).unwrap(), saved_tuning);
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
        let before = apps.membership_generation().unwrap();

        let outcome =
            process_settings_iteration_with_os(&mut apps, &reordered_state, false, &mut os);

        assert!(!outcome.changed);
        let app = apps.apps.get(&key).unwrap();
        assert_eq!(app.group_id, group_id(0));
        assert_eq!(app.rule_id, rule_id(0));
        assert_eq!(apps.membership_generation(), Some(before + 1));
        process_settings_iteration_with_os(&mut apps, &reordered_state, false, &mut os);
        assert_eq!(apps.membership_generation(), Some(before + 1));
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
    fn s113_monitor_reports_denied_member_without_skipping_other_processes() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(89, 1), (90, 1)]),
            HashMap::from([(89, PriorityClass::Normal), (90, PriorityClass::Normal)]),
        );
        apps.add_app(&key, 89, group_id(1), rule_id(0));
        apps.apps.get_mut(&key).unwrap().pids.push(90);
        for pid in [89, 90] {
            seed_tracked_instance(&mut apps, &key, pid, &mut os);
        }
        os.failures.insert(
            89,
            "OpenProcess failed: Access is denied. (0x80070005)".into(),
        );
        let outcome = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert_eq!(os.affinity_sets, vec![(90, 6)]);
        assert!(!apps.apps[&key].settings_matched);
        assert_eq!(outcome.failures.len(), 1);
        let diagnostic = &outcome.failures[0].message;
        assert!(
            diagnostic.contains("Access is denied"),
            "native failure disappeared: {diagnostic}"
        );
        assert!(
            diagnostic.contains("PID 89") && diagnostic.contains(&state.groups[1].programs[0].name)
        );
        let off = process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert!(
            off.failures.is_empty(),
            "read-only inspection is not a failed correction"
        );
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
    fn test_confirmed_monitor_correction_clears_a_pessimistic_mismatch_hint() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 90, group_id(1), rule_id(0));
        let statuses = Arc::new(RwLock::new(HashMap::from([(
            key.clone(),
            AppStatus::SettingsMismatch,
        )])));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(90, 0b001)]),
            HashMap::from([(90, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 90, &mut os);

        process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert!(!record_confirmed_match_statuses(&apps, &statuses));
        assert_eq!(
            statuses.read().unwrap().get(&key),
            Some(&AppStatus::SettingsMismatch)
        );

        process_settings_iteration_with_os(&mut apps, &state, true, &mut os);
        assert!(record_confirmed_match_statuses(&apps, &statuses));
        assert_eq!(
            statuses.read().unwrap().get(&key),
            Some(&AppStatus::Running)
        );
    }

    #[test]
    fn test_confirmed_match_cache_transition_is_reported_without_a_process_state_change() {
        let state = sample_state();
        let key = state.groups[0].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 91, group_id(0), rule_id(0));
        let statuses = Arc::new(RwLock::new(HashMap::from([(
            key.clone(),
            AppStatus::SettingsMismatch,
        )])));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(91, 0b001)]),
            HashMap::from([(91, PriorityClass::Normal)]),
        );
        seed_tracked_instance(&mut apps, &key, 91, &mut os);

        let outcome = process_settings_iteration_with_os(&mut apps, &state, true, &mut os);

        assert!(!outcome.changed);
        assert!(record_confirmed_match_statuses(&apps, &statuses));
        assert_eq!(
            statuses.read().unwrap().get(&key),
            Some(&AppStatus::Running)
        );
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

    #[test]
    fn s107_known_app_launch_applies_real_members_once_with_monitor_off() {
        let (mut state, mut apps) = crate::app::features::execution::known_tracking_fixture();
        state.groups[0].cores = vec![0];
        let app = &state.groups[0].programs[0];
        let key = app.get_key();
        let rule = apps.apps[&key].rule_id.clone();
        let identity = super::super::steam_pending::identity_for_app(app).unwrap();
        assert!(matches!(
            super::super::steam_pending::begin_launch(&rule, identity.clone()),
            super::super::steam_pending::SteamLaunchBegin::Started(_)
        ));
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(1, 255), (2, 255)]),
            HashMap::from([(1, PriorityClass::Normal), (2, PriorityClass::Normal)]),
        );
        os.tokens = HashMap::from([(1, 1), (2, 2)]);
        for _ in 0..2 {
            process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        }
        assert_eq!(os.affinity[&1], 1);
        assert_eq!(os.affinity[&2], 1);
        assert!(!super::super::steam_pending::launch_pending(
            &rule,
            Some(identity.clone())
        ));
        os.affinity.insert(1, 7);
        process_settings_iteration_with_os(&mut apps, &state, false, &mut os);
        assert_eq!(
            os.affinity[&1], 7,
            "Off must not become continuous enforcement after initial readback"
        );
    }
}

#[cfg(test)]
pub(super) fn native_known_iteration(
    apps: &mut RunningApps,
    state: &AppStateStorage,
    enabled: bool,
) {
    apps.control.configure(state);
    apps.control.observe(
        apps,
        super::control::policy::Policy::read(state)
            .unwrap()
            .generation(),
        |pid, token| {
            OS::get_process_image_path_and_instance_token(pid)
                .ok()
                .filter(|(_, actual)| *actual == token)
                .map(|(path, _)| path)
        },
    );
    reconcile_settings_with_os(
        apps,
        &collect_program_settings(state),
        enabled,
        &mut RealProcessSettingsOs,
    );
}
