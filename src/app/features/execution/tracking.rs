use crate::app::features::execution::{
    cleanup_orphaned_package_owners, ensure_package_owner_claim,
    is_excluded_installed_auto_process, resolve_installed_package_runtime_info_cached,
    InstalledPackageTrackingState,
};
use crate::app::features::rules::RulesContext;
use crate::app::models::{
    normalize_process_name, AppRuntimeKey, AppStateStorage, AppToRun, LaunchTarget, RunningApps,
};
use crate::app::shared::ids::{GroupId, RuleId};
use crate::app::shell::events::ShellEvent;
use os_api::{InstalledPackageRuntimeInfo, OS};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Clone, Default)]
struct ProcessSnapshot {
    children_of: HashMap<u32, Vec<u32>>,
    names: HashMap<u32, String>,
}

#[derive(Debug, Clone)]
enum ConfiguredProgramMatcher {
    Path {
        primary_name: Option<String>,
        fallback_names: Vec<String>,
        bin_path: PathBuf,
    },
    Installed {
        aumid: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PackageLocalPidCandidates {
    same_aumid_pids: Vec<u32>,
    no_identity_pids: Vec<u32>,
}

#[derive(Debug, Clone)]
struct ConfiguredProgramSnapshot {
    key: AppRuntimeKey,
    display_name: String,
    additional_processes_normalized: Vec<String>,
    matcher: ConfiguredProgramMatcher,
    group_id: GroupId,
    rule_id: RuleId,
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
    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String>;
    fn resolve_installed_package_runtime_info(
        &self,
        aumid: &str,
    ) -> Result<InstalledPackageRuntimeInfo, String>;
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

pub async fn run_running_app_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    monitor_tx: std::sync::mpsc::Sender<ShellEvent>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let os = RealRunningAppsOs;

    loop {
        interval.tick().await;

        let configured_programs = {
            let state = match app_state.read() {
                Ok(guard) => guard,
                Err(_) => {
                    let _ = monitor_tx.send(ShellEvent::Warning(
                        "WARNING: persistent_state lock poisoned, skipping monitor iteration"
                            .to_string(),
                    ));
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
        let aumid_to_seed_pids = if configured_programs.iter().any(|configured| {
            matches!(
                configured.matcher,
                ConfiguredProgramMatcher::Installed { .. }
            )
        }) {
            build_aumid_to_seed_pids(&snapshot, &os)
        } else {
            HashMap::new()
        };

        if let Ok(mut apps) = running_apps.try_write() {
            let outcome = process_running_apps_iteration_with_os(
                &mut apps,
                configured_programs,
                &snapshot,
                &name_to_pids,
                &aumid_to_seed_pids,
                &installed_package_tracking,
                &os,
            );

            for message in outcome.notifications {
                let _ = monitor_tx.send(ShellEvent::Monitor(message));
            }

            if outcome.changed {
                let _ = monitor_tx.send(ShellEvent::RuntimeStateChanged);
            }
        }
    }
}

fn collect_configured_programs(state: &AppStateStorage) -> Vec<ConfiguredProgramSnapshot> {
    let mut programs = Vec::new();
    let rules = RulesContext::from_storage(state);
    let snapshot = rules.snapshot(state);

    for group in snapshot.groups {
        for program in group.rules {
            let matcher = match &program.app.launch_target {
                LaunchTarget::Path { bin_path, .. } => {
                    let tracked_names = collect_tracked_process_names(&program.app);
                    if tracked_names.is_empty() {
                        continue;
                    }
                    let primary_name = program.app.primary_process_name_normalized();
                    let fallback_names = tracked_names
                        .iter()
                        .filter(|name| Some(*name) != primary_name.as_ref())
                        .cloned()
                        .collect();

                    ConfiguredProgramMatcher::Path {
                        primary_name: primary_name
                            .filter(|primary| tracked_names.iter().any(|name| name == primary)),
                        fallback_names,
                        bin_path: bin_path.clone(),
                    }
                }
                LaunchTarget::Installed { aumid } => ConfiguredProgramMatcher::Installed {
                    aumid: aumid.to_lowercase(),
                },
            };

            programs.push(ConfiguredProgramSnapshot {
                key: program.app.get_key(),
                display_name: program.app.name.clone(),
                additional_processes_normalized: collect_tracked_process_names(&program.app),
                matcher,
                group_id: group.id.clone(),
                rule_id: program.id,
            });
        }
    }

    programs
}

fn collect_tracked_process_names(program: &AppToRun) -> Vec<String> {
    let mut names = Vec::new();

    for process_name in &program.additional_processes {
        push_process_name(&mut names, process_name);
    }

    names
}

fn push_process_name(names: &mut Vec<String>, candidate: &str) {
    let normalized = normalize_process_name(candidate);
    if !normalized.is_empty() && !names.contains(&normalized) {
        names.push(normalized);
    }
}

fn build_name_to_pids(snapshot: &ProcessSnapshot) -> HashMap<String, Vec<u32>> {
    let mut name_to_pids: HashMap<String, Vec<u32>> = HashMap::new();

    for (&pid, full_name) in &snapshot.names {
        let name = normalize_process_name(full_name);
        if !name.is_empty() {
            name_to_pids.entry(name).or_default().push(pid);
        }
    }

    name_to_pids
}

fn build_aumid_to_seed_pids<O: RunningAppsOs>(
    snapshot: &ProcessSnapshot,
    os: &O,
) -> HashMap<String, Vec<u32>> {
    let mut aumid_to_pids: HashMap<String, Vec<u32>> = HashMap::new();

    for &pid in snapshot.names.keys() {
        if let Ok(Some(aumid)) = os.get_process_app_user_model_id(pid) {
            aumid_to_pids
                .entry(aumid.to_lowercase())
                .or_default()
                .push(pid);
        }
    }

    aumid_to_pids
}

fn collect_path_verified_pids<O: RunningAppsOs>(
    matcher: &ConfiguredProgramMatcher,
    name_to_pids: &HashMap<String, Vec<u32>>,
    os: &O,
) -> Vec<u32> {
    let ConfiguredProgramMatcher::Path {
        primary_name,
        bin_path,
        ..
    } = matcher
    else {
        return Vec::new();
    };
    let Some(primary_name) = primary_name else {
        return Vec::new();
    };

    let mut verified_pids = Vec::new();

    if let Some(pids) = name_to_pids.get(primary_name) {
        for &pid in pids {
            if verified_pids.contains(&pid) {
                continue;
            }

            if let Ok(image_path) = os.get_process_image_path(pid) {
                if path_eq_case_insensitive(&image_path, bin_path) {
                    verified_pids.push(pid);
                }
            }
        }
    }

    verified_pids
}

fn path_eq_case_insensitive(left: &Path, right: &Path) -> bool {
    if left
        .to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
    {
        return true;
    }

    canonicalized_path_string(left)
        .zip(canonicalized_path_string(right))
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(&right))
}

fn canonicalized_path_string(path: &Path) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|canonical| canonical.to_string_lossy().replace('/', "\\"))
        .map(|text| {
            if let Some(stripped) = text.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{stripped}")
            } else if let Some(stripped) = text.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                text
            }
        })
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

fn collect_package_local_pid_candidates<O: RunningAppsOs>(
    tracked_pids: &[u32],
    snapshot: &ProcessSnapshot,
    install_root: &Path,
    expected_aumid: &str,
    os: &O,
) -> Result<PackageLocalPidCandidates, String> {
    let tracked_set: HashSet<u32> = tracked_pids.iter().copied().collect();
    let mut candidates = PackageLocalPidCandidates::default();

    for &pid in snapshot.names.keys() {
        if tracked_set.contains(&pid) {
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

fn extend_with_named_processes(
    tracked_pids: &mut Vec<u32>,
    additional_processes: &[String],
    name_to_pids: &HashMap<String, Vec<u32>>,
) {
    for pid in collect_named_process_pids(additional_processes, name_to_pids) {
        push_unique_pid(tracked_pids, pid);
    }
}

fn collect_named_process_pids(
    additional_processes: &[String],
    name_to_pids: &HashMap<String, Vec<u32>>,
) -> Vec<u32> {
    let mut tracked_pids = Vec::new();

    for process_name in additional_processes {
        if let Some(pids) = name_to_pids.get(process_name) {
            for &pid in pids {
                push_unique_pid(&mut tracked_pids, pid);
            }
        }
    }

    tracked_pids
}

fn push_unique_pid(tracked_pids: &mut Vec<u32>, pid: u32) {
    if !tracked_pids.contains(&pid) {
        tracked_pids.push(pid);
    }
}

fn is_auto_managed_installed_pid(snapshot: &ProcessSnapshot, pid: u32) -> bool {
    match snapshot.names.get(&pid) {
        Some(name) => !is_excluded_installed_auto_process(name),
        None => true,
    }
}

fn is_known_auto_managed_installed_pid(snapshot: &ProcessSnapshot, pid: u32) -> bool {
    snapshot
        .names
        .get(&pid)
        .is_some_and(|name| !is_excluded_installed_auto_process(name))
}

fn retain_auto_managed_installed_pids(
    snapshot: &ProcessSnapshot,
    tracked_pids: &mut Vec<u32>,
    explicit_pids: &HashSet<u32>,
) {
    tracked_pids.retain(|&pid| {
        explicit_pids.contains(&pid) || is_auto_managed_installed_pid(snapshot, pid)
    });
}

fn extend_with_descendants(snapshot: &ProcessSnapshot, tracked_pids: &mut Vec<u32>) {
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

fn retain_live_pids<O: RunningAppsOs>(tracked_pids: &mut Vec<u32>, os: &O) {
    tracked_pids.retain(|&pid| os.is_pid_live(pid));
}

fn process_running_apps_iteration_with_os<O: RunningAppsOs>(
    apps: &mut RunningApps,
    configured_programs: Vec<ConfiguredProgramSnapshot>,
    snapshot: &ProcessSnapshot,
    name_to_pids: &HashMap<String, Vec<u32>>,
    aumid_to_seed_pids: &HashMap<String, Vec<u32>>,
    installed_package_tracking: &Arc<RwLock<InstalledPackageTrackingState>>,
    os: &O,
) -> RunningAppsIterationOutcome {
    let mut processed_keys = HashSet::new();
    let mut outcome = RunningAppsIterationOutcome::default();

    for configured in configured_programs {
        let key = configured.key.clone();
        processed_keys.insert(key.clone());
        let was_tracked = apps.apps.contains_key(&key);

        let mut detected_pids = match &configured.matcher {
            ConfiguredProgramMatcher::Path { fallback_names, .. } => {
                let mut pids = collect_path_verified_pids(&configured.matcher, name_to_pids, os);
                extend_with_named_processes(&mut pids, fallback_names, name_to_pids);
                pids
            }
            ConfiguredProgramMatcher::Installed { aumid } => {
                aumid_to_seed_pids.get(aumid).cloned().unwrap_or_default()
            }
        };

        let installed_package_info = match &configured.matcher {
            ConfiguredProgramMatcher::Installed { aumid }
                if was_tracked || !detected_pids.is_empty() =>
            {
                resolve_installed_package_runtime_info_cached(
                    installed_package_tracking,
                    aumid,
                    |aumid| os.resolve_installed_package_runtime_info(aumid),
                )
                .ok()
            }
            _ => None,
        };
        let has_auto_managed_installed_evidence = match &configured.matcher {
            ConfiguredProgramMatcher::Installed { .. } => {
                detected_pids
                    .iter()
                    .any(|&pid| is_known_auto_managed_installed_pid(snapshot, pid))
                    || apps.apps.get(&key).is_some_and(|app| {
                        app.pids
                            .iter()
                            .any(|&pid| is_known_auto_managed_installed_pid(snapshot, pid))
                    })
            }
            ConfiguredProgramMatcher::Path { .. } => false,
        };
        let owns_package = installed_package_info
            .as_ref()
            .filter(|_| has_auto_managed_installed_evidence)
            .map(|info| {
                let mut tracking = installed_package_tracking.write().unwrap();
                ensure_package_owner_claim(&mut tracking, apps, &info.package_family_name, &key)
            })
            .unwrap_or(false);

        if let Some(app) = apps.apps.get_mut(&key) {
            app.group_id = configured.group_id.clone();
            app.rule_id = configured.rule_id.clone();
            let old_pids = app.pids.clone();

            match &configured.matcher {
                ConfiguredProgramMatcher::Path { fallback_names, .. } => {
                    extend_with_descendants(snapshot, &mut app.pids);
                    for pid in detected_pids.drain(..) {
                        push_unique_pid(&mut app.pids, pid);
                    }
                    extend_with_named_processes(&mut app.pids, fallback_names, name_to_pids);
                    extend_with_descendants(snapshot, &mut app.pids);
                }
                ConfiguredProgramMatcher::Installed { aumid } => {
                    let explicit_pids = collect_named_process_pids(
                        &configured.additional_processes_normalized,
                        name_to_pids,
                    );
                    let explicit_pid_set: HashSet<u32> = explicit_pids.iter().copied().collect();
                    let mut managed_pids = app.pids.clone();

                    extend_with_descendants(snapshot, &mut managed_pids);
                    for pid in detected_pids.drain(..) {
                        push_unique_pid(&mut managed_pids, pid);
                    }

                    if let Some(package_info) = installed_package_info.as_ref() {
                        if let Ok(package_candidates) = collect_package_local_pid_candidates(
                            &managed_pids,
                            snapshot,
                            &package_info.install_root,
                            aumid,
                            os,
                        ) {
                            for pid in package_candidates.same_aumid_pids {
                                push_unique_pid(&mut managed_pids, pid);
                            }
                            if owns_package {
                                for pid in package_candidates.no_identity_pids {
                                    push_unique_pid(&mut managed_pids, pid);
                                }
                            }
                        }
                    }

                    retain_auto_managed_installed_pids(
                        snapshot,
                        &mut managed_pids,
                        &explicit_pid_set,
                    );
                    for pid in explicit_pids {
                        push_unique_pid(&mut managed_pids, pid);
                    }
                    extend_with_descendants(snapshot, &mut managed_pids);
                    retain_auto_managed_installed_pids(
                        snapshot,
                        &mut managed_pids,
                        &explicit_pid_set,
                    );
                    app.pids = managed_pids;
                }
            }

            retain_live_pids(&mut app.pids, os);

            if app.pids != old_pids {
                outcome.changed = true;
            }

            if app.pids.is_empty() {
                outcome
                    .notifications
                    .push(format!("App stopped: {}", configured.display_name));
                apps.remove_app(&key);
                outcome.changed = true;
            }

            continue;
        }

        let should_track = match &configured.matcher {
            ConfiguredProgramMatcher::Path { .. } => !detected_pids.is_empty(),
            ConfiguredProgramMatcher::Installed { .. } => !detected_pids.is_empty(),
        };

        if !should_track {
            continue;
        }

        match &configured.matcher {
            ConfiguredProgramMatcher::Path { .. } => {
                extend_with_descendants(snapshot, &mut detected_pids);
            }
            ConfiguredProgramMatcher::Installed { aumid } => {
                let explicit_pids = collect_named_process_pids(
                    &configured.additional_processes_normalized,
                    name_to_pids,
                );
                let explicit_pid_set: HashSet<u32> = explicit_pids.iter().copied().collect();

                extend_with_descendants(snapshot, &mut detected_pids);
                if let Some(package_info) = installed_package_info.as_ref() {
                    if let Ok(package_candidates) = collect_package_local_pid_candidates(
                        &detected_pids,
                        snapshot,
                        &package_info.install_root,
                        aumid,
                        os,
                    ) {
                        for pid in package_candidates.same_aumid_pids {
                            push_unique_pid(&mut detected_pids, pid);
                        }
                        if owns_package {
                            for pid in package_candidates.no_identity_pids {
                                push_unique_pid(&mut detected_pids, pid);
                            }
                        }
                    }
                }

                retain_auto_managed_installed_pids(snapshot, &mut detected_pids, &explicit_pid_set);
                for pid in explicit_pids {
                    push_unique_pid(&mut detected_pids, pid);
                }
                extend_with_descendants(snapshot, &mut detected_pids);
                retain_auto_managed_installed_pids(snapshot, &mut detected_pids, &explicit_pid_set);
            }
        }
        retain_live_pids(&mut detected_pids, os);

        if detected_pids.is_empty() {
            continue;
        }

        outcome.notifications.push(format!(
            "App detected: {} (PID {})",
            configured.display_name, detected_pids[0]
        ));
        apps.add_app(
            &key,
            detected_pids[0],
            configured.group_id,
            configured.rule_id,
        );
        outcome.changed = true;

        if let Some(app) = apps.apps.get_mut(&key) {
            for pid in detected_pids.into_iter().skip(1) {
                if !app.pids.contains(&pid) {
                    app.pids.push(pid);
                }
            }
        }
    }

    let app_keys: Vec<AppRuntimeKey> = apps.apps.keys().cloned().collect();
    for key in app_keys {
        if !processed_keys.contains(&key) {
            if let Some(app) = apps.apps.get_mut(&key) {
                let old_pid_count = app.pids.len();

                extend_with_descendants(snapshot, &mut app.pids);
                retain_live_pids(&mut app.pids, os);

                if app.pids.is_empty() {
                    apps.remove_app(&key);
                    outcome.changed = true;
                } else if app.pids.len() != old_pid_count {
                    outcome.changed = true;
                }
            }
        }
    }

    cleanup_orphaned_package_owners(&mut installed_package_tracking.write().unwrap(), apps);

    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        build_aumid_to_seed_pids, build_name_to_pids, collect_configured_programs,
        extend_with_descendants, process_running_apps_iteration_with_os, ConfiguredProgramMatcher,
        ProcessSnapshot, RunningAppsOs,
    };
    use crate::app::features::execution::InstalledPackageTrackingState;
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, RunningApps};
    use crate::app::shared::ids::{GroupId, RuleId};
    use os_api::{InstalledPackageRuntimeInfo, PriorityClass};
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    struct FakeRunningAppsOs {
        snapshot: Result<ProcessSnapshot, String>,
        image_paths: HashMap<u32, PathBuf>,
        live_pids: HashSet<u32>,
        aumids: HashMap<u32, String>,
        aumid_lookup_count: Cell<usize>,
        installed_package_infos: HashMap<String, Result<InstalledPackageRuntimeInfo, String>>,
        metadata_lookup_count: Cell<usize>,
    }

    impl Default for FakeRunningAppsOs {
        fn default() -> Self {
            Self {
                snapshot: Ok(ProcessSnapshot::default()),
                image_paths: HashMap::new(),
                live_pids: HashSet::new(),
                aumids: HashMap::new(),
                aumid_lookup_count: Cell::new(0),
                installed_package_infos: HashMap::new(),
                metadata_lookup_count: Cell::new(0),
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

        fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String> {
            self.aumid_lookup_count
                .set(self.aumid_lookup_count.get() + 1);
            Ok(self.aumids.get(&pid).cloned())
        }

        fn resolve_installed_package_runtime_info(
            &self,
            aumid: &str,
        ) -> Result<InstalledPackageRuntimeInfo, String> {
            self.metadata_lookup_count
                .set(self.metadata_lookup_count.get() + 1);
            self.installed_package_infos
                .get(aumid)
                .cloned()
                .unwrap_or_else(|| Err(format!("missing package metadata for {aumid}")))
        }
    }

    fn sample_path_program_state() -> AppStateStorage {
        let mut app = AppToRun::new_path(
            PathBuf::from(r"C:\game.lnk"),
            vec![],
            PathBuf::from(r"C:\game.exe"),
            PriorityClass::High,
            false,
        );
        app.additional_processes.push("helper.exe".to_string());

        AppStateStorage {
            version: 5,
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
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }
    }

    fn sample_installed_program_state() -> AppStateStorage {
        let mut app = AppToRun::new_installed(
            "Spotify".to_string(),
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            PriorityClass::Normal,
            false,
        );
        app.additional_processes = vec!["spotifyhelper.exe".to_string()];

        AppStateStorage {
            version: 5,
            groups: vec![CoreGroup {
                name: "Media".to_string(),
                cores: vec![2, 3],
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
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }
    }

    fn sample_shared_package_installed_program_state() -> AppStateStorage {
        AppStateStorage {
            version: 5,
            groups: vec![CoreGroup {
                name: "Media".to_string(),
                cores: vec![2, 3],
                programs: vec![
                    AppToRun::new_installed(
                        "Spotify".to_string(),
                        "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                        PriorityClass::Normal,
                        false,
                    ),
                    AppToRun::new_installed(
                        "Spotify Launcher".to_string(),
                        "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!SpotifyLauncher".to_string(),
                        PriorityClass::Normal,
                        false,
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
            rule_identities: None,
            loaded_version: 5,
            pending_pre_v6_backup: false,
        }
    }

    fn run_iteration(
        apps: &mut RunningApps,
        configured: Vec<super::ConfiguredProgramSnapshot>,
        os: &FakeRunningAppsOs,
    ) -> super::RunningAppsIterationOutcome {
        let installed_package_tracking =
            Arc::new(RwLock::new(InstalledPackageTrackingState::default()));
        let snapshot = os.snapshot.clone().unwrap();
        let name_to_pids = build_name_to_pids(&snapshot);
        let aumid_to_seed_pids = if configured
            .iter()
            .any(|program| matches!(program.matcher, ConfiguredProgramMatcher::Installed { .. }))
        {
            build_aumid_to_seed_pids(&snapshot, os)
        } else {
            HashMap::new()
        };
        process_running_apps_iteration_with_os(
            apps,
            configured,
            &snapshot,
            &name_to_pids,
            &aumid_to_seed_pids,
            &installed_package_tracking,
            os,
        )
    }

    fn group_id(value: usize) -> GroupId {
        GroupId(format!("group-{value}"))
    }

    fn rule_id(value: usize) -> RuleId {
        RuleId(format!("rule-{value}"))
    }

    #[test]
    fn test_collect_configured_programs_preserves_ids_and_names() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);

        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].group_id, group_id(0));
        assert_eq!(configured[0].rule_id, rule_id(0));
        assert_eq!(configured[0].display_name, "game");
        assert_eq!(
            configured[0].additional_processes_normalized,
            vec!["game".to_string(), "helper".to_string()]
        );

        let super::ConfiguredProgramMatcher::Path {
            primary_name,
            fallback_names,
            ..
        } = &configured[0].matcher
        else {
            panic!("expected path matcher");
        };
        assert_eq!(primary_name.as_deref(), Some("game"));
        assert_eq!(fallback_names, &vec!["helper".to_string()]);
    }

    #[test]
    fn test_path_program_with_no_visible_tracked_names_has_no_hidden_fallbacks() {
        let mut state = sample_path_program_state();
        state.groups[0].programs[0].name = "Friendly Name".to_string();
        state.groups[0].programs[0].additional_processes.clear();

        let configured = collect_configured_programs(&state);

        assert!(configured.is_empty());
    }

    #[test]
    fn test_build_name_to_pids_lowercases_names() {
        let snapshot = ProcessSnapshot {
            children_of: HashMap::new(),
            names: HashMap::from([
                (10, "Game.exe".to_string()),
                (11, "HELPER.EXE".to_string()),
                (12, "foo.bar.exe".to_string()),
            ]),
        };

        let name_to_pids = build_name_to_pids(&snapshot);

        assert_eq!(name_to_pids.get("game"), Some(&vec![10]));
        assert_eq!(name_to_pids.get("helper"), Some(&vec![11]));
        assert_eq!(name_to_pids.get("foo.bar"), Some(&vec![12]));
    }

    #[test]
    fn test_collect_configured_programs_normalizes_installed_matcher_inputs() {
        let state = sample_installed_program_state();
        let configured = collect_configured_programs(&state);

        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].display_name, "Spotify");
        assert_eq!(
            configured[0].additional_processes_normalized,
            vec!["spotifyhelper".to_string()]
        );

        let super::ConfiguredProgramMatcher::Installed { aumid } = &configured[0].matcher else {
            panic!("expected installed matcher");
        };
        assert_eq!(aumid, "spotifyab.spotifymusic_zpdnekdrzrea0!spotify");
    }

    #[test]
    fn test_newly_detected_path_app_creates_tracking_entry_and_notification() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "game.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\game.exe"))]),
            live_pids: HashSet::from([10]),
            ..Default::default()
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
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "game.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\other.exe"))]),
            live_pids: HashSet::from([10]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.is_empty());
    }

    #[test]
    fn test_primary_process_name_does_not_prefix_match_helper_processes() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "gamehelper.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\gamehelper.exe"))]),
            live_pids: HashSet::from([10]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(apps.apps.is_empty());
    }

    #[test]
    fn test_additional_processes_attach_extra_pids_to_same_path_app() {
        let state = sample_path_program_state();
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
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![10, 11])
        );
        assert_eq!(os.aumid_lookup_count.get(), 0);
    }

    #[test]
    fn test_explicit_fallback_process_name_can_cold_detect_path_app() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(11, "helper.exe".to_string())]),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::from([11]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![11])
        );
    }

    #[test]
    fn test_explicit_fallback_process_names_match_exactly() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(11, "helper64.exe".to_string())]),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::from([11]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(apps.apps.is_empty());
    }

    #[test]
    fn test_installed_seed_discovery_creates_tracking_entry() {
        let state = sample_installed_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(20, "spotify.exe".to_string())]),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::from([20]),
            aumids: HashMap::from([(
                20,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(
            outcome.notifications,
            vec!["App detected: Spotify (PID 20)"]
        );
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![20])
        );
    }

    #[test]
    fn test_installed_background_host_seed_does_not_create_tracking_entry() {
        let state = sample_installed_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(99, "backgroundTaskHost.exe".to_string())]),
            }),
            live_pids: HashSet::from([99]),
            aumids: HashMap::from([(
                99,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.is_empty());
    }

    #[test]
    fn test_installed_target_attaches_additional_processes_after_seed() {
        let state = sample_installed_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (20, "spotify.exe".to_string()),
                    (21, "spotifyhelper.exe".to_string()),
                ]),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::from([20, 21]),
            aumids: HashMap::from([(
                20,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![20, 21])
        );
    }

    #[test]
    fn test_installed_background_host_does_not_keep_app_running_after_main_exit() {
        let mut state = sample_installed_program_state();
        state.groups[0].programs[0].additional_processes.clear();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 20, group_id(0), rule_id(0));
        apps.apps.get_mut(&key).unwrap().pids.push(99);
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(99, "backgroundTaskHost.exe".to_string())]),
            }),
            live_pids: HashSet::from([99]),
            aumids: HashMap::from([(
                99,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(outcome.notifications, vec!["App stopped: Spotify"]);
        assert!(!apps.apps.contains_key(&key));
    }

    #[test]
    fn test_installed_explicit_background_host_name_overrides_auto_filter() {
        let mut state = sample_installed_program_state();
        state.groups[0].programs[0].additional_processes =
            vec!["backgroundTaskHost.exe".to_string()];
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (20, "spotify.exe".to_string()),
                    (99, "backgroundTaskHost.exe".to_string()),
                ]),
            }),
            live_pids: HashSet::from([20, 99]),
            aumids: HashMap::from([
                (
                    20,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    99,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
            ]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        let key = state.groups[0].programs[0].get_key();
        let tracked: HashSet<u32> = apps.apps[&key].pids.iter().copied().collect();
        assert!(outcome.changed);
        assert_eq!(tracked, HashSet::from([20, 99]));
    }

    #[test]
    fn test_tracked_installed_target_attaches_package_local_pid_after_seed() {
        let mut state = sample_installed_program_state();
        state.groups[0].programs[0].additional_processes.clear();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 20, group_id(0), rule_id(0));
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (20, "spotify.exe".to_string()),
                    (22, "spotifyhelper.exe".to_string()),
                ]),
            }),
            image_paths: HashMap::from([
                (
                    20,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Spotify.exe"),
                ),
                (
                    22,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyHelper.exe"),
                ),
            ]),
            live_pids: HashSet::from([20, 22]),
            aumids: HashMap::from([(
                20,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            installed_package_infos: HashMap::from([(
                "spotifyab.spotifymusic_zpdnekdrzrea0!spotify".to_string(),
                Ok(InstalledPackageRuntimeInfo {
                    aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                    package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
                    install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
                }),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&key).map(|app| app.pids.clone()),
            Some(vec![20, 22])
        );
    }

    #[test]
    fn test_installed_target_does_not_blindly_cold_discover_helper_only_processes() {
        let state = sample_installed_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(21, "spotifyhelper.exe".to_string())]),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::from([21]),
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
            metadata_lookup_count: Cell::new(0),
            installed_package_infos: HashMap::from([(
                "spotifyab.spotifymusic_zpdnekdrzrea0!spotify".to_string(),
                Ok(InstalledPackageRuntimeInfo {
                    aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                    package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
                    install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
                }),
            )]),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.is_empty());
        assert_eq!(os.metadata_lookup_count.get(), 0);
    }

    #[test]
    fn test_path_only_iteration_skips_aumid_lookup_sweep() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([(10, "game.exe".to_string())]),
            }),
            image_paths: HashMap::from([(10, PathBuf::from(r"C:\game.exe"))]),
            live_pids: HashSet::from([10]),
            aumids: HashMap::from([(
                10,
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
            )]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(os.aumid_lookup_count.get(), 0);
        assert_eq!(os.metadata_lookup_count.get(), 0);
    }

    #[test]
    fn test_shared_package_no_identity_pids_attach_only_to_first_active_target() {
        let state = sample_shared_package_installed_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let first_key = state.groups[0].programs[0].get_key();
        let second_key = state.groups[0].programs[1].get_key();
        apps.add_app(&first_key, 20, group_id(0), rule_id(0));
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::from([
                    (20, "spotify.exe".to_string()),
                    (30, "spotifylauncher.exe".to_string()),
                    (31, "spotifyhelper.exe".to_string()),
                ]),
            }),
            image_paths: HashMap::from([
                (
                    20,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\Spotify.exe"),
                ),
                (
                    30,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyLauncher.exe"),
                ),
                (
                    31,
                    PathBuf::from(r"C:\Program Files\WindowsApps\Spotify\SpotifyHelper.exe"),
                ),
            ]),
            live_pids: HashSet::from([20, 30, 31]),
            aumids: HashMap::from([
                (
                    20,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".to_string(),
                ),
                (
                    30,
                    "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!SpotifyLauncher".to_string(),
                ),
            ]),
            installed_package_infos: HashMap::from([
                (
                    "spotifyab.spotifymusic_zpdnekdrzrea0!spotify".to_string(),
                    Ok(InstalledPackageRuntimeInfo {
                        aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify".into(),
                        package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
                        install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
                    }),
                ),
                (
                    "spotifyab.spotifymusic_zpdnekdrzrea0!spotifylauncher".to_string(),
                    Ok(InstalledPackageRuntimeInfo {
                        aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!SpotifyLauncher".into(),
                        package_family_name: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0".into(),
                        install_root: PathBuf::from(r"C:\Program Files\WindowsApps\Spotify"),
                    }),
                ),
            ]),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(
            apps.apps.get(&first_key).map(|app| app.pids.clone()),
            Some(vec![20, 31])
        );
        assert_eq!(
            apps.apps.get(&second_key).map(|app| app.pids.clone()),
            Some(vec![30])
        );
    }

    #[test]
    fn test_stale_tracked_app_is_removed_when_process_stops() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 10, group_id(0), rule_id(0));
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::new(),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::new(),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(outcome.notifications, vec!["App stopped: game"]);
        assert!(!apps.apps.contains_key(&key));
    }

    #[test]
    fn test_stale_tracked_app_is_removed_when_configuration_disappears() {
        let state = sample_path_program_state();
        let mut apps = RunningApps::default();
        let key = state.groups[0].programs[0].get_key();
        apps.add_app(&key, 10, group_id(0), rule_id(0));
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::new(),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::new(),
            ..Default::default()
        };

        let outcome = run_iteration(&mut apps, Vec::new(), &os);

        assert!(outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(!apps.apps.contains_key(&key));
    }

    #[test]
    fn test_extend_with_descendants_walks_known_child_to_grandchild() {
        let snapshot = ProcessSnapshot {
            children_of: HashMap::from([(10, vec![11]), (11, vec![12])]),
            names: HashMap::new(),
        };
        let mut tracked_pids = vec![10];

        extend_with_descendants(&snapshot, &mut tracked_pids);

        assert_eq!(tracked_pids, vec![10, 11, 12]);
    }
}
