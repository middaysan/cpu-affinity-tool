use crate::app::models::{AppRuntimeKey, AppStateStorage, AppToRun, LaunchTarget, RunningApps};
use eframe::egui;
use os_api::OS;
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
        names: Vec<String>,
        bin_path: PathBuf,
    },
    Installed {
        aumid: String,
    },
}

#[derive(Debug, Clone)]
struct ConfiguredProgramSnapshot {
    key: AppRuntimeKey,
    display_name: String,
    additional_processes_normalized: Vec<String>,
    matcher: ConfiguredProgramMatcher,
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
    fn get_process_app_user_model_id(&self, pid: u32) -> Result<Option<String>, String>;
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

fn collect_configured_programs(state: &AppStateStorage) -> Vec<ConfiguredProgramSnapshot> {
    let mut programs = Vec::new();

    for (group_index, group) in state.groups.iter().enumerate() {
        for (prog_index, program) in group.programs.iter().enumerate() {
            let matcher = match &program.launch_target {
                LaunchTarget::Path { bin_path, .. } => {
                    let names = collect_path_candidate_names(program);
                    if names.is_empty() {
                        continue;
                    }

                    ConfiguredProgramMatcher::Path {
                        names,
                        bin_path: bin_path.clone(),
                    }
                }
                LaunchTarget::Installed { aumid } => ConfiguredProgramMatcher::Installed {
                    aumid: aumid.to_lowercase(),
                },
            };

            programs.push(ConfiguredProgramSnapshot {
                key: program.get_key(),
                display_name: program.name.clone(),
                additional_processes_normalized: collect_additional_process_names(program),
                matcher,
                group_index,
                prog_index,
            });
        }
    }

    programs
}

fn collect_path_candidate_names(program: &AppToRun) -> Vec<String> {
    let mut names = Vec::new();

    push_stem_name(&mut names, Some(program.name.as_str()));
    push_path_stem_name(&mut names, program.bin_path());
    push_path_stem_name(&mut names, program.dropped_path());

    names
}

fn collect_additional_process_names(program: &AppToRun) -> Vec<String> {
    let mut names = Vec::new();

    for process_name in &program.additional_processes {
        push_stem_name(&mut names, Some(process_name));
    }

    names
}

fn push_stem_name(names: &mut Vec<String>, candidate: Option<&str>) {
    let Some(candidate) = candidate else {
        return;
    };

    let normalized = normalize_process_name(candidate);
    if !normalized.is_empty() && !names.contains(&normalized) {
        names.push(normalized);
    }
}

fn push_path_stem_name(names: &mut Vec<String>, path: Option<&Path>) {
    let stem = path
        .and_then(|path| path.file_name())
        .and_then(|file_name| file_name.to_str())
        .and_then(|name| name.split('.').next());
    push_stem_name(names, stem);
}

fn build_name_to_pids(snapshot: &ProcessSnapshot) -> HashMap<String, Vec<u32>> {
    let mut name_to_pids: HashMap<String, Vec<u32>> = HashMap::new();

    for (&pid, full_name) in &snapshot.names {
        let name = full_name.split('.').next().unwrap_or("").to_lowercase();
        if !name.is_empty() {
            name_to_pids.entry(name).or_default().push(pid);
        }
    }

    name_to_pids
}

fn normalize_process_name(candidate: &str) -> String {
    candidate
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
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
    let ConfiguredProgramMatcher::Path { names, bin_path } = matcher else {
        return Vec::new();
    };

    let mut verified_pids = Vec::new();

    for name in names {
        let target_lower = name.to_lowercase();

        for (process_name, pids) in name_to_pids {
            if !process_name.starts_with(&target_lower) {
                continue;
            }

            for &pid in pids {
                if verified_pids.contains(&pid) {
                    continue;
                }

                if process_name == &target_lower {
                    if let Ok(image_path) = os.get_process_image_path(pid) {
                        if path_eq_case_insensitive(&image_path, bin_path) {
                            verified_pids.push(pid);
                        }
                    }
                } else {
                    verified_pids.push(pid);
                }
            }
        }
    }

    verified_pids
}

fn path_eq_case_insensitive(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn extend_with_named_processes(
    tracked_pids: &mut Vec<u32>,
    additional_processes: &[String],
    name_to_pids: &HashMap<String, Vec<u32>>,
) {
    for process_name in additional_processes {
        if process_name.is_empty() {
            continue;
        }

        for (candidate_name, pids) in name_to_pids {
            if candidate_name.starts_with(process_name) {
                for &pid in pids {
                    if !tracked_pids.contains(&pid) {
                        tracked_pids.push(pid);
                    }
                }
            }
        }
    }
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
    os: &O,
) -> RunningAppsIterationOutcome {
    let mut processed_keys = HashSet::new();
    let mut outcome = RunningAppsIterationOutcome::default();

    for configured in configured_programs {
        let key = configured.key.clone();
        processed_keys.insert(key.clone());

        let mut detected_pids = match &configured.matcher {
            ConfiguredProgramMatcher::Path { .. } => {
                let mut pids = collect_path_verified_pids(&configured.matcher, name_to_pids, os);
                extend_with_named_processes(
                    &mut pids,
                    &configured.additional_processes_normalized,
                    name_to_pids,
                );
                pids
            }
            ConfiguredProgramMatcher::Installed { aumid } => {
                aumid_to_seed_pids.get(aumid).cloned().unwrap_or_default()
            }
        };

        if let Some(app) = apps.apps.get_mut(&key) {
            let old_pid_count = app.pids.len();

            extend_with_descendants(snapshot, &mut app.pids);
            for pid in detected_pids.drain(..) {
                if !app.pids.contains(&pid) {
                    app.pids.push(pid);
                }
            }

            if matches!(
                configured.matcher,
                ConfiguredProgramMatcher::Installed { .. }
            ) || !configured.additional_processes_normalized.is_empty()
            {
                extend_with_named_processes(
                    &mut app.pids,
                    &configured.additional_processes_normalized,
                    name_to_pids,
                );
            }

            extend_with_descendants(snapshot, &mut app.pids);
            retain_live_pids(&mut app.pids, os);

            if app.pids.len() != old_pid_count {
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

        extend_with_descendants(snapshot, &mut detected_pids);
        extend_with_named_processes(
            &mut detected_pids,
            &configured.additional_processes_normalized,
            name_to_pids,
        );
        extend_with_descendants(snapshot, &mut detected_pids);
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
            configured.group_index,
            configured.prog_index,
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

    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        build_aumid_to_seed_pids, build_name_to_pids, collect_configured_programs,
        extend_with_descendants, process_running_apps_iteration_with_os, ConfiguredProgramMatcher,
        ProcessSnapshot, RunningAppsOs,
    };
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema, RunningApps};
    use os_api::PriorityClass;
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    struct FakeRunningAppsOs {
        snapshot: Result<ProcessSnapshot, String>,
        image_paths: HashMap<u32, PathBuf>,
        live_pids: HashSet<u32>,
        aumids: HashMap<u32, String>,
        aumid_lookup_count: Cell<usize>,
    }

    impl Default for FakeRunningAppsOs {
        fn default() -> Self {
            Self {
                snapshot: Ok(ProcessSnapshot::default()),
                image_paths: HashMap::new(),
                live_pids: HashSet::new(),
                aumids: HashMap::new(),
                aumid_lookup_count: Cell::new(0),
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
    }

    fn sample_path_program_state() -> AppStateStorage {
        let mut app = AppToRun::new_path(
            PathBuf::from(r"C:\game.lnk"),
            vec![],
            PathBuf::from(r"C:\game.exe"),
            PriorityClass::High,
            false,
        );
        app.additional_processes = vec!["helper.exe".to_string()];

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
        }
    }

    fn run_iteration(
        apps: &mut RunningApps,
        configured: Vec<super::ConfiguredProgramSnapshot>,
        os: &FakeRunningAppsOs,
    ) -> super::RunningAppsIterationOutcome {
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
            os,
        )
    }

    #[test]
    fn test_collect_configured_programs_preserves_indices_and_names() {
        let state = sample_path_program_state();
        let configured = collect_configured_programs(&state);

        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].group_index, 0);
        assert_eq!(configured[0].prog_index, 0);
        assert_eq!(configured[0].display_name, "game");
        assert_eq!(
            configured[0].additional_processes_normalized,
            vec!["helper".to_string()]
        );

        let super::ConfiguredProgramMatcher::Path { names, .. } = &configured[0].matcher else {
            panic!("expected path matcher");
        };
        assert!(names.contains(&"game".to_string()));
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
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
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
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
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
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
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
            aumid_lookup_count: Cell::new(0),
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
            aumid_lookup_count: Cell::new(0),
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
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(!outcome.changed);
        assert!(outcome.notifications.is_empty());
        assert!(apps.apps.is_empty());
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
            aumid_lookup_count: Cell::new(0),
        };

        let outcome = run_iteration(&mut apps, configured, &os);

        assert!(outcome.changed);
        assert_eq!(os.aumid_lookup_count.get(), 0);
    }

    #[test]
    fn test_stale_tracked_app_is_removed_when_process_stops() {
        let state = sample_path_program_state();
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
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
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
        apps.add_app(&key, 10, 0, 0);
        let os = FakeRunningAppsOs {
            snapshot: Ok(ProcessSnapshot {
                children_of: HashMap::new(),
                names: HashMap::new(),
            }),
            image_paths: HashMap::new(),
            live_pids: HashSet::new(),
            aumids: HashMap::new(),
            aumid_lookup_count: Cell::new(0),
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
