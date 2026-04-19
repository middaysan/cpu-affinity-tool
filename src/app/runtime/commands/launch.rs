use crate::app::models::{AppStateStorage, AppToRun, LogManager};
use crate::app::runtime::RuntimeRegistry;
use os_api::{PriorityClass, OS};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

trait LaunchOs {
    fn set_process_affinity_by_pid(&self, pid: u32, mask: usize) -> Result<(), String>;
    fn set_process_priority_by_pid(&self, pid: u32, priority: PriorityClass) -> Result<(), String>;
    fn focus_window_by_pid(&self, pid: u32) -> bool;
    fn run(
        &self,
        bin_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String>;
}

struct RealLaunchOs;

impl LaunchOs for RealLaunchOs {
    fn set_process_affinity_by_pid(&self, pid: u32, mask: usize) -> Result<(), String> {
        OS::set_process_affinity_by_pid(pid, mask)
    }

    fn set_process_priority_by_pid(&self, pid: u32, priority: PriorityClass) -> Result<(), String> {
        OS::set_process_priority_by_pid(pid, priority)
    }

    fn focus_window_by_pid(&self, pid: u32) -> bool {
        OS::focus_window_by_pid(pid)
    }

    fn run(
        &self,
        bin_path: PathBuf,
        args: Vec<String>,
        cores: &[usize],
        priority: PriorityClass,
    ) -> Result<u32, String> {
        OS::run(bin_path, args, cores, priority)
    }
}

pub(crate) fn collect_autorun_items(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
) -> Vec<(usize, usize, AppToRun)> {
    let state = persistent_state.read().unwrap();
    state
        .groups
        .iter()
        .enumerate()
        .flat_map(|(g_i, group)| {
            group
                .programs
                .iter()
                .enumerate()
                .filter_map(move |(p_i, app)| {
                    if app.autorun {
                        Some((g_i, p_i, app.clone()))
                    } else {
                        None
                    }
                })
        })
        .collect()
}

pub fn start_app_with_autorun(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
) {
    let os = RealLaunchOs;

    for (g_i, p_i, app_to_run) in collect_autorun_items(persistent_state) {
        run_app_with_affinity_sync_with_os(
            persistent_state,
            runtime,
            log_manager,
            g_i,
            p_i,
            app_to_run,
            &os,
        );
    }
}

pub fn run_app_with_affinity_sync(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_index: usize,
    prog_index: usize,
    app_to_run: AppToRun,
) {
    let os = RealLaunchOs;

    run_app_with_affinity_sync_with_os(
        persistent_state,
        runtime,
        log_manager,
        group_index,
        prog_index,
        app_to_run,
        &os,
    );
}

fn run_app_with_affinity_sync_with_os<O: LaunchOs>(
    persistent_state: &Arc<RwLock<AppStateStorage>>,
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_index: usize,
    prog_index: usize,
    app_to_run: AppToRun,
    os: &O,
) {
    let group_cores = {
        let state = persistent_state.read().unwrap();
        match state.groups.get(group_index) {
            Some(group) => group.cores.clone(),
            None => {
                log_manager.add_important_sticky_once(format!(
                    "Error: Group index {group_index} not found"
                ));
                return;
            }
        }
    };

    run_launch_decision(
        runtime,
        log_manager,
        group_index,
        prog_index,
        app_to_run,
        group_cores,
        os,
    );
}

fn run_launch_decision<O: LaunchOs>(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    group_index: usize,
    prog_index: usize,
    app_to_run: AppToRun,
    group_cores: Vec<usize>,
    os: &O,
) {
    let app_key = app_to_run.get_key();

    if let Some(pids) = runtime.get_running_app_pids(&app_key) {
        let mask = group_cores.iter().fold(0usize, |acc, &i| acc | (1 << i));
        for &pid in &pids {
            let _ = os.set_process_affinity_by_pid(pid, mask);
            let _ = os.set_process_priority_by_pid(pid, app_to_run.priority);
        }

        let was_focused = pids.iter().any(|&pid| os.focus_window_by_pid(pid));
        if was_focused {
            log_manager.add_entry(format!(
                "App already running: {}, settings reapplied and window focused",
                app_to_run.display()
            ));
            return;
        }

        log_manager.add_entry(format!(
            "App already running: {}, settings reapplied but no window found to focus",
            app_to_run.display()
        ));
        return;
    }

    let label = app_to_run
        .bin_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| app_to_run.bin_path.display().to_string());
    let display = app_to_run.display();
    let priority = app_to_run.priority;

    log_manager.add_entry(format!("Starting '{}', app: {}", label, display));

    match os.run(app_to_run.bin_path, app_to_run.args, &group_cores, priority) {
        Ok(pid) => record_started_pid(runtime, log_manager, &app_key, pid, group_index, prog_index),
        Err(e) => log_manager.add_important_sticky_once(format!("ERROR: {e}")),
    }
}

fn record_started_pid(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    app_key: &str,
    pid: u32,
    group_index: usize,
    prog_index: usize,
) {
    let is_new_app = !runtime.contains_app(app_key);

    if is_new_app {
        let added = runtime.add_running_app(app_key, pid, group_index, prog_index);
        if added {
            log_manager.add_entry(format!("App started with PID: {pid}"));
        } else {
            log_manager.add_entry(format!(
                "App started with PID: {pid} but couldn't be tracked (lock busy)"
            ));
        }
    } else {
        let _ = runtime.add_pid_to_existing_app(app_key, pid);
        log_manager.add_entry(format!(
            "New instance of existing app started with PID: {pid}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_autorun_items, record_started_pid, run_app_with_affinity_sync_with_os,
        run_launch_decision, LaunchOs,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, LogManager};
    use crate::app::runtime::RuntimeRegistry;
    use os_api::PriorityClass;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    struct FakeLaunchOs {
        affinity_calls: RefCell<Vec<(u32, usize)>>,
        priority_calls: RefCell<Vec<(u32, PriorityClass)>>,
        focus_results: HashMap<u32, bool>,
        run_calls: RefCell<Vec<(PathBuf, Vec<String>, Vec<usize>, PriorityClass)>>,
        run_result: RefCell<Result<u32, String>>,
    }

    impl Default for FakeLaunchOs {
        fn default() -> Self {
            Self {
                affinity_calls: RefCell::new(Vec::new()),
                priority_calls: RefCell::new(Vec::new()),
                focus_results: HashMap::new(),
                run_calls: RefCell::new(Vec::new()),
                run_result: RefCell::new(Ok(0)),
            }
        }
    }

    impl LaunchOs for FakeLaunchOs {
        fn set_process_affinity_by_pid(&self, pid: u32, mask: usize) -> Result<(), String> {
            self.affinity_calls.borrow_mut().push((pid, mask));
            Ok(())
        }

        fn set_process_priority_by_pid(
            &self,
            pid: u32,
            priority: PriorityClass,
        ) -> Result<(), String> {
            self.priority_calls.borrow_mut().push((pid, priority));
            Ok(())
        }

        fn focus_window_by_pid(&self, pid: u32) -> bool {
            self.focus_results.get(&pid).copied().unwrap_or(false)
        }

        fn run(
            &self,
            bin_path: PathBuf,
            args: Vec<String>,
            cores: &[usize],
            priority: PriorityClass,
        ) -> Result<u32, String> {
            self.run_calls
                .borrow_mut()
                .push((bin_path, args, cores.to_vec(), priority));
            self.run_result.borrow().clone()
        }
    }

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 4,
            groups: vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![0, 1],
                programs: vec![
                    AppToRun::new(
                        PathBuf::from(r"C:\one.lnk"),
                        vec![],
                        PathBuf::from(r"C:\one.exe"),
                        PriorityClass::Normal,
                        false,
                    ),
                    AppToRun::new(
                        PathBuf::from(r"C:\two.lnk"),
                        vec!["--autorun".to_string()],
                        PathBuf::from(r"C:\two.exe"),
                        PriorityClass::High,
                        true,
                    ),
                ],
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

    fn sample_app() -> AppToRun {
        AppToRun::new(
            PathBuf::from(r"C:\game.lnk"),
            vec!["--fullscreen".to_string()],
            PathBuf::from(r"C:\game.exe"),
            PriorityClass::High,
            false,
        )
    }

    #[test]
    fn test_collect_autorun_items_preserves_indices() {
        let items = collect_autorun_items(&sample_state());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1, 1);
        assert_eq!(items[0].2.bin_path, PathBuf::from(r"C:\two.exe"));
        assert_eq!(items[0].2.args, vec!["--autorun".to_string()]);
    }

    #[test]
    fn test_already_running_with_focus_reapplies_settings_without_launch() {
        let runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app(&app_key, 41, 3, 4));
        assert!(runtime.add_pid_to_existing_app(&app_key, 42));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            focus_results: HashMap::from([(41, false), (42, true)]),
            run_result: RefCell::new(Ok(999)),
            ..Default::default()
        };

        run_launch_decision(&runtime, &mut log_manager, 3, 4, app, vec![0, 2], &os);

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(41, 5), (42, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(41, PriorityClass::High), (42, PriorityClass::High)]
        );
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("window focused")));
    }

    #[test]
    fn test_already_running_without_focus_does_not_launch_duplicate() {
        let runtime = RuntimeRegistry::new();
        let app = sample_app();
        let app_key = app.get_key();
        assert!(runtime.add_running_app(&app_key, 77, 0, 0));
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(555)),
            ..Default::default()
        };

        run_launch_decision(&runtime, &mut log_manager, 0, 0, app, vec![1, 3], &os);

        assert!(os.run_calls.borrow().is_empty());
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("no window found to focus")));
    }

    #[test]
    fn test_missing_group_logs_critical_and_stops() {
        let state = sample_state();
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs::default();

        run_app_with_affinity_sync_with_os(
            &state,
            &runtime,
            &mut log_manager,
            99,
            0,
            sample_app(),
            &os,
        );

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            log_manager
                .entries
                .iter()
                .filter(|entry| entry.message.contains("Group index 99 not found"))
                .count(),
            2
        );
    }

    #[test]
    fn test_fresh_launch_success_tracks_pid_in_runtime_registry() {
        let state = sample_state();
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Ok(4242)),
            ..Default::default()
        };
        let app = sample_app();
        let app_key = app.get_key();

        run_app_with_affinity_sync_with_os(&state, &runtime, &mut log_manager, 0, 0, app, &os);

        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4242]));
        assert!(log_manager
            .entries
            .iter()
            .any(|entry| entry.message == "App started with PID: 4242"));
    }

    #[test]
    fn test_record_started_pid_appends_to_existing_runtime_entry_without_duplicates() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let key = sample_app().get_key();
        assert!(runtime.add_running_app(&key, 41, 1, 2));

        record_started_pid(&runtime, &mut log_manager, &key, 5150, 1, 2);
        record_started_pid(&runtime, &mut log_manager, &key, 5150, 1, 2);

        let pids = runtime.get_running_app_pids(&key).unwrap();
        assert!(pids.contains(&41));
        assert!(pids.contains(&5150));
        assert_eq!(pids.iter().filter(|&&pid| pid == 5150).count(), 1);
        assert!(log_manager.entries.iter().any(|entry| entry
            .message
            .contains("New instance of existing app started")));
    }

    #[test]
    fn test_launch_failure_logs_important_and_sticky_error() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let os = FakeLaunchOs {
            run_result: RefCell::new(Err("boom".to_string())),
            ..Default::default()
        };

        run_launch_decision(&runtime, &mut log_manager, 0, 0, sample_app(), vec![0], &os);

        assert_eq!(
            log_manager
                .entries
                .iter()
                .filter(|entry| entry.message == "ERROR: boom")
                .count(),
            2
        );
    }
}
