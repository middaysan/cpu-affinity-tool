use crate::app::models::{AppRuntimeKey, AppStateStorage, AppToRun, LaunchTarget, LogManager};
use crate::app::runtime::runtime_registry::{
    ensure_package_owner_claim, InstalledPackageTrackingState,
};
use crate::app::runtime::RuntimeRegistry;
use os_api::{InstalledPackageRuntimeInfo, PriorityClass, OS};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone, Default)]
struct LaunchProcessSnapshot {
    children_of: HashMap<u32, Vec<u32>>,
    names: HashMap<u32, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PostLaunchCorrectionOutcome {
    tracked_pids: Vec<u32>,
    no_identity_package_pids: Vec<u32>,
    new_pids_added: usize,
    saw_identity_seed: bool,
}

struct PostLaunchCorrectionRequest {
    running_apps: Arc<TokioRwLock<crate::app::models::RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    app_key: AppRuntimeKey,
    initial_pid: u32,
    group_index: usize,
    prog_index: usize,
    group_cores: Vec<usize>,
    priority: PriorityClass,
    expected_aumid: String,
    installed_package_info: Option<InstalledPackageRuntimeInfo>,
    prelaunch_package_pids: HashSet<u32>,
}

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
    fn activate_application(&self, aumid: &str) -> Result<u32, String>;
    fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String>;
    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String>;
    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String>;
    fn resolve_installed_package_runtime_info(
        &self,
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String>;
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

    fn activate_application(&self, aumid: &str) -> Result<u32, String> {
        OS::activate_application(aumid)
    }

    fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String> {
        OS::snapshot_process_tree().map(|tree| LaunchProcessSnapshot {
            children_of: tree.children_of,
            names: tree.names,
        })
    }

    fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
        OS::get_process_image_path(pid)
    }

    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String> {
        OS::get_process_app_user_model_id(pid)
    }

    fn resolve_installed_package_runtime_info(
        &self,
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String> {
        OS::resolve_installed_package_runtime_info(aumid)
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
    let mask = group_cores.iter().fold(0usize, |acc, &i| acc | (1 << i));

    if let Some(pids) = runtime.get_running_app_pids(&app_key) {
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

    let label = match &app_to_run.launch_target {
        LaunchTarget::Path { bin_path, .. } => bin_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| bin_path.display().to_string()),
        LaunchTarget::Installed { .. } => app_to_run.name.clone(),
    };
    let display = app_to_run.display();
    let priority = app_to_run.priority;
    let (installed_package_info, prelaunch_package_pids) =
        if let LaunchTarget::Installed { aumid } = &app_to_run.launch_target {
            match runtime
                .resolve_installed_package_runtime_info_with(aumid, |aumid| {
                    os.resolve_installed_package_runtime_info(aumid)
                })
                .ok()
                .and_then(|info| {
                    collect_package_local_pids_from_live_snapshot(os, &info.install_root)
                        .ok()
                        .map(|pids| (info, pids))
                }) {
                Some((info, pids)) => (Some(info), pids),
                None => (None, HashSet::new()),
            }
        } else {
            (None, HashSet::new())
        };

    log_manager.add_entry(format!("Starting '{}', app: {}", label, display));

    let launch_result = match &app_to_run.launch_target {
        LaunchTarget::Path { bin_path, .. } => os.run(
            bin_path.clone(),
            app_to_run.args.clone(),
            &group_cores,
            priority,
        ),
        LaunchTarget::Installed { aumid } => os.activate_application(aumid),
    };

    match launch_result {
        Ok(pid) => {
            if matches!(app_to_run.launch_target, LaunchTarget::Installed { .. }) {
                let _ = os.set_process_affinity_by_pid(pid, mask);
                let _ = os.set_process_priority_by_pid(pid, priority);
            }

            record_started_pid(runtime, log_manager, &app_key, pid, group_index, prog_index);

            if let LaunchTarget::Installed { aumid } = &app_to_run.launch_target {
                spawn_post_launch_correction(PostLaunchCorrectionRequest {
                    running_apps: runtime.running_apps.clone(),
                    installed_package_tracking: runtime.installed_package_tracking.clone(),
                    app_key,
                    initial_pid: pid,
                    group_index,
                    prog_index,
                    group_cores,
                    priority,
                    expected_aumid: aumid.clone(),
                    installed_package_info,
                    prelaunch_package_pids,
                });
            }
        }
        Err(e) => log_manager.add_important_sticky_once(format!("ERROR: {e}")),
    }
}

fn record_started_pid(
    runtime: &RuntimeRegistry,
    log_manager: &mut LogManager,
    app_key: &AppRuntimeKey,
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

fn spawn_post_launch_correction(request: PostLaunchCorrectionRequest) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    handle.spawn(async move {
        let os = RealLaunchOs;
        let mut tracked_pids = vec![request.initial_pid];
        let mut stable_polls = 0usize;
        let mut saw_identity_seed = false;

        for delay_ms in [0u64, 100, 250, 500, 1000, 2000, 3500] {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            let outcome = post_launch_correction_poll_with_os(
                &os,
                &request.expected_aumid,
                &mut tracked_pids,
                &request.group_cores,
                request.priority,
                request.installed_package_info.as_ref(),
                &request.prelaunch_package_pids,
            );

            if let Ok(outcome) = outcome {
                if outcome.saw_identity_seed {
                    saw_identity_seed = true;
                }

                let mut attached_no_identity_pids = Vec::new();
                let mut newly_attached_package_pids = 0usize;
                if let Ok(mut apps) = request.running_apps.try_write() {
                    if !apps.apps.contains_key(&request.app_key) {
                        apps.add_app(
                            &request.app_key,
                            request.initial_pid,
                            request.group_index,
                            request.prog_index,
                        );
                    }

                    for &pid in &outcome.tracked_pids {
                        if let Some(app) = apps.apps.get_mut(&request.app_key) {
                            if !app.pids.contains(&pid) {
                                app.pids.push(pid);
                            }
                        }
                    }

                    if let Some(package_info) = &request.installed_package_info {
                        let mut package_tracking =
                            request.installed_package_tracking.write().unwrap();
                        let owns_package = ensure_package_owner_claim(
                            &mut package_tracking,
                            &apps,
                            &package_info.package_family_name,
                            &request.app_key,
                        );

                        if owns_package {
                            for &pid in &outcome.no_identity_package_pids {
                                if let Some(app) = apps.apps.get_mut(&request.app_key) {
                                    if !app.pids.contains(&pid) {
                                        app.pids.push(pid);
                                        attached_no_identity_pids.push(pid);
                                        let mask = request
                                            .group_cores
                                            .iter()
                                            .fold(0usize, |acc, &i| acc | (1 << i));
                                        let _ = os.set_process_affinity_by_pid(pid, mask);
                                        let _ =
                                            os.set_process_priority_by_pid(pid, request.priority);
                                        newly_attached_package_pids += 1;
                                    }
                                }
                            }
                        }
                    }
                }

                tracked_pids = outcome.tracked_pids;
                tracked_pids.extend(attached_no_identity_pids);

                if outcome.new_pids_added + newly_attached_package_pids == 0 {
                    stable_polls += 1;
                } else {
                    stable_polls = 0;
                }

                if saw_identity_seed && stable_polls >= 2 {
                    break;
                }
            }
        }
    });
}

fn post_launch_correction_poll_with_os<O: LaunchOs>(
    os: &O,
    expected_aumid: &str,
    tracked_pids: &mut Vec<u32>,
    group_cores: &[usize],
    priority: PriorityClass,
    installed_package_info: Option<&InstalledPackageRuntimeInfo>,
    prelaunch_package_pids: &HashSet<u32>,
) -> Result<PostLaunchCorrectionOutcome, String> {
    let snapshot = os.snapshot_process_tree()?;
    let before: HashSet<u32> = tracked_pids.iter().copied().collect();

    extend_with_descendants(&snapshot, tracked_pids);
    let mut no_identity_package_pids = Vec::new();

    if let Some(package_info) = installed_package_info {
        let package_candidates = collect_package_local_pid_candidates_from_snapshot(
            os,
            &snapshot,
            &package_info.install_root,
            expected_aumid,
            prelaunch_package_pids,
            &before,
        )?;

        for pid in package_candidates.same_aumid_pids {
            if !tracked_pids.contains(&pid) {
                tracked_pids.push(pid);
            }
        }
        no_identity_package_pids = package_candidates.no_identity_pids;
    }

    let mask = group_cores.iter().fold(0usize, |acc, &i| acc | (1 << i));
    let mut saw_identity_seed = false;

    for &pid in tracked_pids.iter() {
        let _ = os.set_process_affinity_by_pid(pid, mask);
        let _ = os.set_process_priority_by_pid(pid, priority);

        if !saw_identity_seed
            && os
                .get_process_app_user_model_id(pid)?
                .is_some_and(|aumid| aumid.eq_ignore_ascii_case(expected_aumid))
        {
            saw_identity_seed = true;
        }
    }

    let new_pids_added = tracked_pids
        .iter()
        .filter(|pid| !before.contains(pid))
        .count();

    Ok(PostLaunchCorrectionOutcome {
        tracked_pids: tracked_pids.clone(),
        no_identity_package_pids,
        new_pids_added,
        saw_identity_seed,
    })
}

fn extend_with_descendants(snapshot: &LaunchProcessSnapshot, tracked_pids: &mut Vec<u32>) {
    let mut visited: HashSet<u32> = tracked_pids.iter().copied().collect();
    let mut stack = tracked_pids.clone();

    while let Some(parent_pid) = stack.pop() {
        if let Some(children) = snapshot.children_of.get(&parent_pid) {
            for &child_pid in children {
                if visited.insert(child_pid) {
                    tracked_pids.push(child_pid);
                    stack.push(child_pid);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PackageLocalPidCandidates {
    same_aumid_pids: Vec<u32>,
    no_identity_pids: Vec<u32>,
}

fn collect_package_local_pids_from_live_snapshot<O: LaunchOs>(
    os: &O,
    install_root: &Path,
) -> Result<HashSet<u32>, String> {
    let snapshot = os.snapshot_process_tree()?;
    let mut pids = HashSet::new();

    for &pid in snapshot.names.keys() {
        if let Ok(image_path) = os.get_process_image_path(pid) {
            if path_is_under_root_case_insensitive(&image_path, install_root) {
                pids.insert(pid);
            }
        }
    }

    Ok(pids)
}

fn collect_package_local_pid_candidates_from_snapshot<O: LaunchOs>(
    os: &O,
    snapshot: &LaunchProcessSnapshot,
    install_root: &Path,
    expected_aumid: &str,
    prelaunch_package_pids: &HashSet<u32>,
    tracked_before: &HashSet<u32>,
) -> Result<PackageLocalPidCandidates, String> {
    let mut candidates = PackageLocalPidCandidates::default();

    for &pid in snapshot.names.keys() {
        if tracked_before.contains(&pid) || prelaunch_package_pids.contains(&pid) {
            continue;
        }

        let Ok(image_path) = os.get_process_image_path(pid) else {
            continue;
        };
        if !path_is_under_root_case_insensitive(&image_path, install_root) {
            continue;
        }

        match os.get_process_app_user_model_id(pid)? {
            Some(aumid) if !aumid.trim().is_empty() => {
                if aumid.eq_ignore_ascii_case(expected_aumid) {
                    candidates.same_aumid_pids.push(pid);
                }
            }
            _ => candidates.no_identity_pids.push(pid),
        }
    }

    Ok(candidates)
}

fn normalize_path_for_prefix(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn path_is_under_root_case_insensitive(path: &Path, install_root: &Path) -> bool {
    let path_normalized = normalize_path_for_prefix(path);
    let root_normalized = normalize_path_for_prefix(install_root);

    path_normalized == root_normalized || path_normalized.starts_with(&(root_normalized + "\\"))
}

#[cfg(test)]
mod tests {
    use super::{
        collect_autorun_items, post_launch_correction_poll_with_os, record_started_pid,
        run_app_with_affinity_sync_with_os, run_launch_decision, LaunchOs, LaunchProcessSnapshot,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, LogManager};
    use crate::app::runtime::RuntimeRegistry;
    use os_api::{InstalledPackageRuntimeInfo, PriorityClass};
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
        activate_calls: RefCell<Vec<String>>,
        activate_result: RefCell<Result<u32, String>>,
        snapshot_result: RefCell<Result<LaunchProcessSnapshot, String>>,
        image_paths: HashMap<u32, PathBuf>,
        process_aumids: HashMap<u32, String>,
        installed_package_info: RefCell<Result<InstalledPackageRuntimeInfo, String>>,
    }

    impl Default for FakeLaunchOs {
        fn default() -> Self {
            Self {
                affinity_calls: RefCell::new(Vec::new()),
                priority_calls: RefCell::new(Vec::new()),
                focus_results: HashMap::new(),
                run_calls: RefCell::new(Vec::new()),
                run_result: RefCell::new(Ok(0)),
                activate_calls: RefCell::new(Vec::new()),
                activate_result: RefCell::new(Ok(0)),
                snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot::default())),
                image_paths: HashMap::new(),
                process_aumids: HashMap::new(),
                installed_package_info: RefCell::new(Err("metadata unavailable".to_string())),
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

        fn activate_application(&self, aumid: &str) -> Result<u32, String> {
            self.activate_calls.borrow_mut().push(aumid.to_string());
            self.activate_result.borrow().clone()
        }

        fn snapshot_process_tree(&self) -> Result<LaunchProcessSnapshot, String> {
            self.snapshot_result.borrow().clone()
        }

        fn get_process_image_path(&self, pid: u32) -> Result<PathBuf, String> {
            self.image_paths
                .get(&pid)
                .cloned()
                .ok_or_else(|| format!("missing image path for pid {pid}"))
        }

        fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String> {
            Ok(self.process_aumids.get(&pid).cloned())
        }

        fn resolve_installed_package_runtime_info(
            &self,
            _aumid: &str,
        ) -> Result<InstalledPackageRuntimeInfo, String> {
            self.installed_package_info.borrow().clone()
        }
    }

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 5,
            groups: vec![CoreGroup {
                name: "Games".to_string(),
                cores: vec![0, 1],
                programs: vec![
                    AppToRun::new_path(
                        PathBuf::from(r"C:\one.lnk"),
                        vec![],
                        PathBuf::from(r"C:\one.exe"),
                        PriorityClass::Normal,
                        false,
                    ),
                    AppToRun::new_path(
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
        AppToRun::new_path(
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
        assert_eq!(
            items[0].2.bin_path(),
            Some(PathBuf::from(r"C:\two.exe").as_path())
        );
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

    #[test]
    fn test_installed_launch_uses_activation_and_immediate_apply() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );
        let app_key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            ..Default::default()
        };

        run_launch_decision(&runtime, &mut log_manager, 0, 0, app, vec![0, 2], &os);

        assert!(os.run_calls.borrow().is_empty());
        assert_eq!(
            os.activate_calls.borrow().as_slice(),
            &["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(4321, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(4321, PriorityClass::High)]
        );
        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4321]));
    }

    #[test]
    fn test_post_launch_correction_poll_attaches_descendants_and_reapplies_settings() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::from([(50, vec![51]), (51, vec![52])]),
                names: HashMap::from([
                    (50, "Spotify.exe".to_string()),
                    (51, "SpotifyHelper.exe".to_string()),
                    (52, "SpotifyHelper.exe".to_string()),
                ]),
            })),
            process_aumids: HashMap::from([(
                50,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };
        let mut tracked_pids = vec![50];

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut tracked_pids,
            &[1, 3],
            PriorityClass::AboveNormal,
            None,
            &std::collections::HashSet::new(),
        )
        .unwrap();

        assert_eq!(outcome.tracked_pids, vec![50, 51, 52]);
        assert!(outcome.no_identity_package_pids.is_empty());
        assert_eq!(outcome.new_pids_added, 2);
        assert!(outcome.saw_identity_seed);
        assert_eq!(
            os.affinity_calls.borrow().as_slice(),
            &[(50, 10), (51, 10), (52, 10)]
        );
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[
                (50, PriorityClass::AboveNormal),
                (51, PriorityClass::AboveNormal),
                (52, PriorityClass::AboveNormal),
            ]
        );
    }

    #[test]
    fn test_post_launch_correction_poll_attaches_only_new_package_local_pids() {
        let os = FakeLaunchOs {
            snapshot_result: RefCell::new(Ok(LaunchProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (40, "Spotify.exe".to_string()),
                    (41, "SpotifyLauncher.exe".to_string()),
                    (42, "SpotifyHelper.exe".to_string()),
                    (43, "SpotifyHelper.exe".to_string()),
                    (44, "SpotifyHelper.exe".to_string()),
                ]),
            })),
            image_paths: HashMap::from([
                (
                    40,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Spotify.exe"),
                ),
                (
                    41,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyLauncher.exe"),
                ),
                (
                    42,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyHelper.exe"),
                ),
                (43, PathBuf::from(r"C:\Other\SpotifyHelper.exe")),
                (
                    44,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Widget.exe"),
                ),
            ]),
            process_aumids: HashMap::from([
                (
                    40,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    42,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    44,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Widget".to_string(),
                ),
            ]),
            ..Default::default()
        };
        let mut tracked_pids = vec![40];
        let package_info = InstalledPackageRuntimeInfo {
            aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
            install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
        };
        let prelaunch = std::collections::HashSet::from([41]);

        let outcome = post_launch_correction_poll_with_os(
            &os,
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &mut tracked_pids,
            &[0, 1],
            PriorityClass::High,
            Some(&package_info),
            &prelaunch,
        )
        .unwrap();

        assert_eq!(outcome.tracked_pids, vec![40, 42]);
        assert_eq!(outcome.no_identity_package_pids, Vec::<u32>::new());
        assert_eq!(outcome.new_pids_added, 1);
        assert!(outcome.saw_identity_seed);
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(40, 3), (42, 3)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(40, PriorityClass::High), (42, PriorityClass::High)]
        );
    }

    #[test]
    fn test_installed_launch_with_metadata_resolve_failure_keeps_soft_fallback() {
        let runtime = RuntimeRegistry::new();
        let mut log_manager = LogManager::default();
        let app = AppToRun::new_installed(
            "Spotify".into(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
            PriorityClass::High,
            false,
        );
        let app_key = app.get_key();
        let os = FakeLaunchOs {
            activate_result: RefCell::new(Ok(4321)),
            installed_package_info: RefCell::new(Err("metadata unavailable".into())),
            ..Default::default()
        };

        run_launch_decision(&runtime, &mut log_manager, 0, 0, app, vec![0, 2], &os);

        assert_eq!(runtime.get_running_app_pids(&app_key), Some(vec![4321]));
        assert_eq!(
            os.activate_calls.borrow().as_slice(),
            &["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string()]
        );
        assert_eq!(os.affinity_calls.borrow().as_slice(), &[(4321, 5)]);
        assert_eq!(
            os.priority_calls.borrow().as_slice(),
            &[(4321, PriorityClass::High)]
        );
        assert!(!log_manager
            .entries
            .iter()
            .any(|entry| entry.message.contains("metadata unavailable")));
    }
}
