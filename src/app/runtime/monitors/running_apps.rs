use crate::app::models::{AppStateStorage, AppToRun, RunningApps};
use eframe::egui;
use os_api::OS;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone, Default)]
struct ProcessSnapshot {
    children_of: HashMap<u32, Vec<u32>>,
    names: HashMap<u32, String>,
}

#[derive(Debug, Clone)]
struct ConfiguredProgram {
    program: AppToRun,
    names: Vec<String>,
    group_index: usize,
    prog_index: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RunningAppsIterationOutcome {
    changed: bool,
    notifications: Vec<String>,
}

trait RunningAppsOs {
    fn snapshot_process_tree(&self) -> Result<ProcessSnapshot, String>;
    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String>;
    fn is_pid_live(&self, pid: u32) -> bool;
}

struct RealRunningAppsOs;

impl RunningAppsOs for RealRunningAppsOs {
    fn snapshot_process_tree(&self) -> Result<ProcessSnapshot, String> {
        OS::snapshot_process_tree().map(|tree| ProcessSnapshot {
            children_of: tree.children_of,
            names: tree.names,
        })
    }

    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
        OS::get_process_image_path(pid)
    }

    fn is_pid_live(&self, pid: u32) -> bool {
        OS::is_pid_live(pid)
    }
}

pub async fn run_running_app_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    ctx: egui::Context,
    monitor_tx: std::sync::mpsc::Sender<String>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let os = RealRunningAppsOs;

    loop {
        interval.tick().await;

        let configured_programs = {
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

            collect_configured_programs(&state)
        };

        let snapshot = match os.snapshot_process_tree() {
            Ok(snapshot) => snapshot,
            Err(_) => continue,
        };
        let name_to_pids = build_name_to_pids(&snapshot);

        if let Ok(mut apps) = running_apps.try_write() {
            let outcome = process_running_apps_iteration_with_os(
                &mut apps,
                configured_programs,
                &snapshot,
                &name_to_pids,
                &os,
            );

            for message in outcome.notifications {
                let _ = monitor_tx.send(message);
            }

            if outcome.changed {
                ctx.request_repaint();
            }
        }
    }
}

fn collect_configured_programs(state: &AppStateStorage) -> Vec<ConfiguredProgram> {
    let mut programs = Vec::new();

    for (group_index, group) in state.groups.iter().enumerate() {
        for (prog_index, program) in group.programs.iter().enumerate() {
            let mut names = Vec::new();

            if let Some(name) = program.name.split('.').next() {
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }

            if let Some(name) = program
                .bin_path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .and_then(|name| name.split('.').next())
            {
                if !name.is_empty() && !names.contains(&name.to_string()) {
                    names.push(name.to_string());
                }
            }

            if let Some(name) = program
                .dropped_path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .and_then(|name| name.split('.').next())
            {
                if !name.is_empty() && !names.contains(&name.to_string()) {
                    names.push(name.to_string());
                }
            }

            if !names.is_empty() {
                programs.push(ConfiguredProgram {
                    program: program.clone(),
                    names,
                    group_index,
                    prog_index,
                });
            }
        }
    }

    programs
}

fn build_name_to_pids(snapshot: &ProcessSnapshot) -> HashMap<String, Vec<u32>> {
    let mut name_to_pids: HashMap<String, Vec<u32>> = HashMap::new();

    for (&pid, full_name) in &snapshot.names {
        let name = full_name.split('.').next().unwrap_or("").to_string();
        name_to_pids
            .entry(name.to_lowercase())
            .or_default()
            .push(pid);
    }

    name_to_pids
}

fn find_all_descendants_with_snapshot(
    snapshot: &ProcessSnapshot,
    parent_pid: u32,
    descendants: &mut Vec<u32>,
) {
    let mut visited = HashSet::new();
    let mut stack = vec![parent_pid];

    while let Some(current_pid) = stack.pop() {
        if !visited.insert(current_pid) {
            continue;
        }

        if let Some(children) = snapshot.children_of.get(&current_pid) {
            for &child_pid in children {
                if !descendants.contains(&child_pid) {
                    descendants.push(child_pid);
                }
                stack.push(child_pid);
            }
        }
    }
}

fn process_running_apps_iteration_with_os<O: RunningAppsOs>(
    apps: &mut RunningApps,
    configured_programs: Vec<ConfiguredProgram>,
    snapshot: &ProcessSnapshot,
    name_to_pids: &HashMap<String, Vec<u32>>,
    os: &O,
) -> RunningAppsIterationOutcome {
    let mut processed_keys = HashSet::new();
    let mut outcome = RunningAppsIterationOutcome::default();

    for configured in configured_programs {
        let key = configured.program.get_key();
        processed_keys.insert(key.clone());
        let mut verified_pids = Vec::new();

        for name in &configured.names {
            let target_lower = name.to_lowercase();
            for (process_name, pids) in name_to_pids {
                if process_name.starts_with(&target_lower) {
                    for &pid in pids {
                        if verified_pids.contains(&pid) {
                            continue;
                        }

                        if *process_name == target_lower {
                            if let Ok(image_path) = os.get_process_image_path(pid) {
                                if key.starts_with(&image_path.display().to_string()) {
                                    verified_pids.push(pid);
                                }
                            }
                        } else {
                            verified_pids.push(pid);
                        }
                    }
                }
            }
        }

        for process_name in &configured.program.additional_processes {
            let search_name = process_name.split('.').next().unwrap_or("").to_lowercase();
            if !search_name.is_empty() {
                for (candidate_name, pids) in name_to_pids {
                    if candidate_name.starts_with(&search_name) {
                        for &pid in pids {
                            if !verified_pids.contains(&pid) {
                                verified_pids.push(pid);
                            }
                        }
                    }
                }
            }
        }

        if let Some(app) = apps.apps.get_mut(&key) {
            let old_pid_count = app.pids.len();

            if !app.pids.is_empty() {
                find_all_descendants_with_snapshot(snapshot, app.pids[0], &mut app.pids);
            }

            for pid in verified_pids {
                if !app.pids.contains(&pid) {
                    app.pids.push(pid);
                }
            }

            app.pids.retain(|&pid| os.is_pid_live(pid));

            if app.pids.len() != old_pid_count {
                outcome.changed = true;
            }

            if app.pids.is_empty() {
                outcome
                    .notifications
                    .push(format!("App stopped: {}", configured.program.name));
                apps.remove_app(&key);
                outcome.changed = true;
            }
        } else if !verified_pids.is_empty() {
            outcome.notifications.push(format!(
                "App detected: {} (PID {})",
                configured.program.name, verified_pids[0]
            ));
            apps.add_app(
                &key,
                verified_pids[0],
                configured.group_index,
                configured.prog_index,
            );
            outcome.changed = true;

            if let Some(app) = apps.apps.get_mut(&key) {
                for pid in verified_pids.into_iter().skip(1) {
                    if !app.pids.contains(&pid) {
                        app.pids.push(pid);
                    }
                }
            }
        }
    }

    let app_keys: Vec<String> = apps.apps.keys().cloned().collect();
    for key in app_keys {
        if !processed_keys.contains(&key) {
            if let Some(app) = apps.apps.get_mut(&key) {
                let old_pid_count = app.pids.len();

                if !app.pids.is_empty() {
                    find_all_descendants_with_snapshot(snapshot, app.pids[0], &mut app.pids);
                }

                app.pids.retain(|&pid| os.is_pid_live(pid));

                if app.pids.is_empty() {
                    apps.remove_app(&key);
                    outcome.changed = true;
                } else if app.pids.len() != old_pid_count {
                    outcome.changed = true;
                }
            }
        }
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        build_name_to_pids, collect_configured_programs, find_all_descendants_with_snapshot,
        process_running_apps_iteration_with_os, ProcessSnapshot, RunningAppsOs,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, RunningApps};
    use os_api::PriorityClass;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    struct FakeRunningAppsOs {
        snapshot: Result<ProcessSnapshot, String>,
        image_paths: HashMap<u32, PathBuf>,
        live_pids: HashSet<u32>,
    }

    impl Default for FakeRunningAppsOs {
        fn default() -> Self {
            Self {
                snapshot: Ok(ProcessSnapshot::default()),
                image_paths: HashMap::new(),
                live_pids: HashSet::new(),
            }
        }
    }

    impl RunningAppsOs for FakeRunningAppsOs {
        fn snapshot_process_tree(&self) -> Result<ProcessSnapshot, String> {
            self.snapshot.clone()
        }

        fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
            self.image_paths
                .get(&pid)
                .cloned()
                .ok_or_else(|| format!("missing image path for pid {pid}"))
        }

        fn is_pid_live(&self, pid: u32) -> bool {
            self.live_pids.contains(&pid)
        }
    }

    fn sample_program_state() -> AppStateStorage {
        let mut app = AppToRun::new(
            PathBuf::from(r"C:\game.lnk"),
            vec![],
            PathBuf::from(r"C:\game.exe"),
            PriorityClass::High,
            false,
        );
        app.additional_processes = vec!["helper.exe".to_string()];

        AppStateStorage {
            version: 4,
            groups: vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![0, 1],
                programs: vec![app],
                is_hidden: false,
                run_all_button: true,
            }],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
        }
    }

    fn run_iteration(
        apps: &mut RunningApps,
        configured: Vec<super::ConfiguredProgram>,
        os: &FakeRunningAppsOs,
    ) -> super::RunningAppsIterationOutcome {
        let snapshot = os.snapshot.clone().unwrap();
        let name_to_pids = build_name_to_pids(&snapshot);
        process_running_apps_iteration_with_os(apps, configured, &snapshot, &name_to_pids, os)
    }

    #[test]
    fn test_collect_configured_programs_preserves_indices_and_names() {
        let state = sample_program_state();
        let configured = collect_configured_programs(&state);

        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].group_index, 0);
        assert_eq!(configured[0].prog_index, 0);
        assert!(configured[0].names.contains(&"game".to_string()));
    }

    #[test]
    fn test_build_name_to_pids_lowercases_names() {
        let snapshot = ProcessSnapshot {
            children_of: HashMap::new(),
            names: HashMap::from([(10, "Game.exe".to_string()), (11, "HELPER.EXE".to_string())]),
        };

        let name_to_pids = build_name_to_pids(&snapshot);

        assert_eq!(name_to_pids.get("game"), Some(&vec![10]));
        assert_eq!(name_to_pids.get("helper"), Some(&vec![11]));
    }

    #[test]
    fn test_newly_detected_app_creates_tracking_entry_and_notification() {
        let state = sample_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "game.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\game.exe"))]),
            live_pids: HashSet::from([10]),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(outcome.notifications, vec!["App detected: game (PID 10)"]);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![10])
        );
    }

    #[test]
    fn test_exact_name_match_requires_matching_image_path() {
        let state = sample_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "game.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\other.exe"))]),
            live_pids: HashSet::from([10]),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.is_empty());
    }

    #[test]
    fn test_additional_processes_attach_extra_pids_to_same_app() {
        let state = sample_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (10, "game.exe".to_string()),
                    (11, "helper.exe".to_string()),
                ]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\game.exe"))]),
            live_pids: HashSet::from([10, 11]),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![10, 11])
        );
    }

    #[test]
    fn test_stale_tracked_app_is_removed_when_process_stops() {
        let state = sample_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 10, 0, 0);
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::new(),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::new(),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(outcome.notifications, vec!["App stopped: game"]);
        assert!(!apps.apps.contains_key(&key));
    }

    #[test]
    fn test_stale_tracked_app_is_removed_when_configuration_disappears() {
        let state = sample_program_state();
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 10, 0, 0);
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::new(),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::new(),
        };

        let outcome = run_iteration(&mut apps, Vec::new(), &os);

        assert!(outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(!apps.apps.contains_key(&key));
    }

    #[test]
    fn test_find_all_descendants_walks_known_child_to_grandchild() {
        let snapshot = ProcessSnapshot {
            children_of: HashMap::from([(10, vec![11]), (11, vec![12])]),
            names: HashMap::new(),
        };
        let mut descendants = vec![10, 11];

        find_all_descendants_with_snapshot(&snapshot, 10, &mut descendants);

        assert!(descendants.contains(&12));
    }
}
