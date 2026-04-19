use crate::app::models::{AppRuntimeKey, AppStateStorage, CoreGroup, RunningApps};
use eframe::egui;
use os_api::{PriorityClass, OS};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone)]
struct ProgramRuntimeSettings {
    name: String,
    group_index: usize,
    prog_index: usize,
    expected_mask: usize,
    expected_priority: PriorityClass,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProcessSettingsIterationOutcome {
    changed: bool,
    notifications: Vec<String>,
}

trait ProcessSettingsOs {
    fn get_process_affinity(&mut self, pid: u32) -> Result<usize, String>;
    fn get_process_priority(&mut self, pid: u32) -> Result<PriorityClass, String>;
    fn set_process_affinity_by_pid(&mut self, pid: u32, mask: usize) -> Result<(), String>;
    fn set_process_priority_by_pid(
        &mut self,
        pid: u32,
        priority: PriorityClass,
    ) -> Result<(), String>;
}

struct RealProcessSettingsOs;

impl ProcessSettingsOs for RealProcessSettingsOs {
    fn get_process_affinity(&mut self, pid: u32) -> Result<usize, String> {
        OS::get_process_affinity(pid)
    }

    fn get_process_priority(&mut self, pid: u32) -> Result<PriorityClass, String> {
        OS::get_process_priority(pid)
    }

    fn set_process_affinity_by_pid(&mut self, pid: u32, mask: usize) -> Result<(), String> {
        OS::set_process_affinity_by_pid(pid, mask)
    }

    fn set_process_priority_by_pid(
        &mut self,
        pid: u32,
        priority: PriorityClass,
    ) -> Result<(), String> {
        OS::set_process_priority_by_pid(pid, priority)
    }
}

pub async fn run_process_settings_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    ctx: egui::Context,
    monitor_tx: std::sync::mpsc::Sender<String>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
    let mut os = RealProcessSettingsOs;

    loop {
        interval.tick().await;

        let (groups, monitoring_enabled) = {
            let state = match app_state.read() {
                Ok(guard) => guard,
                Err(_) => {
                    let _ = monitor_tx.send(
                        "WARNING: persistent_state lock poisoned, skipping monitor iteration"
                            .to_string(),
                    );
                    continue;
                }
            };
            (state.groups.clone(), state.process_monitoring_enabled)
        };

        if let Ok(mut apps) = running_apps.try_write() {
            let outcome =
                process_settings_iteration_with_os(&mut apps, &groups, monitoring_enabled, &mut os);

            if !outcome.notifications.is_empty() {
                for message in outcome.notifications {
                    let _ = monitor_tx.send(format!("MONITOR: {}", message));
                    #[cfg(debug_assertions)]
                    println!("MONITOR: {}", message);
                }
            }

            if outcome.changed {
                ctx.request_repaint();
            }
        }
    }
}

fn collect_program_settings(
    groups: &[CoreGroup],
) -> HashMap<AppRuntimeKey, ProgramRuntimeSettings> {
    let mut settings = HashMap::new();

    for (group_index, group) in groups.iter().enumerate() {
        let mut expected_mask = 0usize;
        for &core_index in &group.cores {
            if core_index < (std::mem::size_of::<usize>() * 8) {
                expected_mask |= 1 << core_index;
            }
        }

        for (prog_index, program) in group.programs.iter().enumerate() {
            settings.insert(
                program.get_key(),
                ProgramRuntimeSettings {
                    name: program.name.clone(),
                    group_index,
                    prog_index,
                    expected_mask,
                    expected_priority: program.priority,
                },
            );
        }
    }

    settings
}

fn process_settings_iteration_with_os<O: ProcessSettingsOs>(
    apps: &mut RunningApps,
    groups: &[CoreGroup],
    monitoring_enabled: bool,
    os: &mut O,
) -> ProcessSettingsIterationOutcome {
    let key_to_settings = collect_program_settings(groups);
    let mut outcome = ProcessSettingsIterationOutcome::default();

    for (app_key, app) in apps.apps.iter_mut() {
        if let Some(settings) = key_to_settings.get(app_key) {
            app.group_index = settings.group_index;
            app.prog_index = settings.prog_index;

            let mut all_matched = true;

            for &pid in &app.pids {
                if let Ok(current_mask) = os.get_process_affinity(pid) {
                    if current_mask != settings.expected_mask {
                        all_matched = false;
                        if monitoring_enabled
                            && os
                                .set_process_affinity_by_pid(pid, settings.expected_mask)
                                .is_ok()
                        {
                            outcome.notifications.push(format!(
                                "Fixed affinity for {} (PID {}): {:X} -> {:X}",
                                settings.name, pid, current_mask, settings.expected_mask
                            ));
                        }
                    }
                }

                if let Ok(current_priority) = os.get_process_priority(pid) {
                    if current_priority != settings.expected_priority {
                        all_matched = false;
                        if monitoring_enabled
                            && os
                                .set_process_priority_by_pid(pid, settings.expected_priority)
                                .is_ok()
                        {
                            outcome.notifications.push(format!(
                                "Fixed priority for {} (PID {}): {:?} -> {:?}",
                                settings.name, pid, current_priority, settings.expected_priority
                            ));
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
}

#[cfg(test)]
mod tests {
    use super::{process_settings_iteration_with_os, ProcessSettingsOs};
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, RunningApps};
    use os_api::PriorityClass;
    use std::collections::HashMap;
    use std::path::PathBuf;

    struct FakeProcessSettingsOs {
        affinity: HashMap<u32, usize>,
        priority: HashMap<u32, PriorityClass>,
        affinity_sets: Vec<(u32, usize)>,
        priority_sets: Vec<(u32, PriorityClass)>,
    }

    impl FakeProcessSettingsOs {
        fn new(affinity: HashMap<u32, usize>, priority: HashMap<u32, PriorityClass>) -> Self {
            Self {
                affinity,
                priority,
                affinity_sets: Vec::new(),
                priority_sets: Vec::new(),
            }
        }
    }

    impl ProcessSettingsOs for FakeProcessSettingsOs {
        fn get_process_affinity(&mut self, pid: u32) -> Result<usize, String> {
            self.affinity
                .get(&pid)
                .copied()
                .ok_or_else(|| format!("missing affinity for pid {pid}"))
        }

        fn get_process_priority(&mut self, pid: u32) -> Result<PriorityClass, String> {
            self.priority
                .get(&pid)
                .copied()
                .ok_or_else(|| format!("missing priority for pid {pid}"))
        }

        fn set_process_affinity_by_pid(&mut self, pid: u32, mask: usize) -> Result<(), String> {
            self.affinity.insert(pid, mask);
            self.affinity_sets.push((pid, mask));
            Ok(())
        }

        fn set_process_priority_by_pid(
            &mut self,
            pid: u32,
            priority: PriorityClass,
        ) -> Result<(), String> {
            self.priority.insert(pid, priority);
            self.priority_sets.push((pid, priority));
            Ok(())
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
        }
    }

    #[test]
    fn test_remap_group_and_program_indices_by_app_key() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 77, 1, 0);
        if let Some(app) = apps.apps.get_mut(&key) {
            app.group_index = 9;
            app.prog_index = 9;
        }

        let reordered_groups = vec![state.groups[1].clone()];
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(77, 0b110)]),
            HashMap::from([(77, PriorityClass::High)]),
        );

        let outcome =
            process_settings_iteration_with_os(&mut apps, &reordered_groups, false, &mut os);

        assert!(!outcome.changed);
        let app = apps.apps.get(&key).unwrap();
        assert_eq!(app.group_index, 0);
        assert_eq!(app.prog_index, 0);
    }

    #[test]
    fn test_mismatch_without_monitoring_updates_status_without_correction() {
        let state = sample_state();
        let key = state.groups[1].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 88, 1, 0);
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(88, 0b001)]),
            HashMap::from([(88, PriorityClass::Normal)]),
        );

        let outcome = process_settings_iteration_with_os(&mut apps, &state.groups, false, &mut os);

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
        apps.add_app(&key, 89, 1, 0);
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(89, 0b001)]),
            HashMap::from([(89, PriorityClass::Normal)]),
        );

        let outcome = process_settings_iteration_with_os(&mut apps, &state.groups, true, &mut os);

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
        apps.add_app(&key, 90, 1, 0);
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(90, 0b001)]),
            HashMap::from([(90, PriorityClass::Normal)]),
        );

        let first = process_settings_iteration_with_os(&mut apps, &state.groups, true, &mut os);
        assert!(first.changed);
        assert!(!apps.apps.get(&key).unwrap().settings_matched);

        let second = process_settings_iteration_with_os(&mut apps, &state.groups, true, &mut os);
        assert!(second.changed);
        assert!(second.notifications.is_empty());
        assert!(apps.apps.get(&key).unwrap().settings_matched);
    }

    #[test]
    fn test_matched_settings_produce_no_notifications() {
        let state = sample_state();
        let key = state.groups[0].programs[0].get_key();
        let mut apps = RunningApps::default();
        apps.add_app(&key, 91, 0, 0);
        let mut os = FakeProcessSettingsOs::new(
            HashMap::from([(91, 0b001)]),
            HashMap::from([(91, PriorityClass::Normal)]),
        );

        let outcome = process_settings_iteration_with_os(&mut apps, &state.groups, true, &mut os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.get(&key).unwrap().settings_matched);
    }
}
