//! Explicit desktop acceptance with bounded, test-owned client executables only.
use super::*;
use crate::app::{
    features::{
        execution::{self, AppRowAction, AppRowActionRequest, RuntimeRegistry},
        rules::{known_apps, RulesContext},
    },
    models::{AppToRun, CoreGroup, LogManager},
};
use os_api::PriorityClass;
use std::{
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    process::Command,
};

struct OwnedFixtures(PathBuf);
impl Drop for OwnedFixtures {
    fn drop(&mut self) {
        for (pid, name) in os_api::OS::get_all_process_names() {
            if ![
                "steam.exe",
                "steamwebhelper.exe",
                "discord.exe",
                "update.exe",
            ]
            .contains(&name.to_ascii_lowercase().as_str())
            {
                continue;
            }
            let Ok((image, token)) = os_api::OS::get_process_image_path_and_instance_token(pid)
            else {
                continue;
            };
            if image
                .canonicalize()
                .is_ok_and(|image| image.starts_with(&self.0))
            {
                if let Ok(process) =
                    os_api::agent::ExactProcess::open(pid, token, false, true, true)
                {
                    let _ = process.terminate();
                }
            }
        }
    }
}
struct Collector(CpuLoadRuntime);
impl Deref for Collector {
    type Target = CpuLoadRuntime;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for Collector {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for Collector {
    fn drop(&mut self) {
        self.shutdown();
        let until = Instant::now() + Duration::from_secs(8);
        while !self.is_stopped() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(self.is_stopped(), "owned CPU collector did not stop");
    }
}

fn owned_image_pids(path: &Path) -> Vec<(u32, u64)> {
    let expected = path.canonicalize().unwrap();
    os_api::OS::get_all_process_names()
        .into_iter()
        .filter(|(_, current)| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(current))
        })
        .filter_map(|(pid, _)| {
            let (image, token) = os_api::OS::get_process_image_path_and_instance_token(pid).ok()?;
            image
                .canonicalize()
                .ok()?
                .eq(&expected)
                .then_some((pid, token))
        })
        .collect()
}

fn focus_owned(
    runtime: &mut RuntimeRegistry,
    source: &Arc<RwLock<AppStateStorage>>,
    index: usize,
    log: &mut LogManager,
) -> bool {
    let app = source.read().unwrap().groups[0].programs[index].clone();
    let key = app.get_key();
    assert_eq!(
        execution::run_app_row_action(
            source,
            runtime,
            log,
            AppRowActionRequest {
                group_index: 0,
                program_index: index,
                app,
                action: AppRowAction::Focus
            }
        ),
        execution::LaunchDispatchOutcome::Accepted
    );
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        if let Ok((_, pid)) = os_api::OS::foreground_process() {
            if runtime
                .running_apps_handle()
                .blocking_read()
                .apps
                .get(&key)
                .is_some_and(|app| app.pid_instance_tokens.contains_key(&pid))
            {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
#[ignore = "explicit elevated desktop acceptance; only fixtures prepared by scripts/test-windows-known-apps.ps1; about 25 seconds"]
fn native_known_app_launch_focus_affinity_cpu_ram() {
    use std::os::windows::process::CommandExt;
    let launch_root = PathBuf::from(
        std::env::var_os("CAT_KNOWN_FIXTURE_DIR").expect("prepare owned fixtures first"),
    );
    let root = launch_root.canonicalize().unwrap();
    let target = std::env::current_dir()
        .unwrap()
        .join("target")
        .canonicalize()
        .unwrap();
    assert!(root.starts_with(target));
    assert_eq!(
        std::fs::read_to_string(root.join("fixture.marker")).unwrap(),
        "cpu-affinity-tool-known-app-test-v1"
    );
    let _cleanup = OwnedFixtures(root.clone());
    let steam = launch_root.join("Steam/steam.exe");
    let discord = launch_root.join("Discord/Discord.exe");
    let mut programs = vec![
        AppToRun::new_path(
            steam.clone(),
            vec!["--owned-fixture".into()],
            steam,
            PriorityClass::BelowNormal,
            false,
        ),
        AppToRun::new_path(
            root.join("Discord.lnk"),
            vec!["--processStart".into(), "Discord.exe".into()],
            discord.clone(),
            PriorityClass::BelowNormal,
            false,
        ),
    ];
    for app in &mut programs {
        known_apps::prepare_new_gui_import(app);
        assert!(
            app.builtin_process_binding.is_some(),
            "native image product/layout qualification failed"
        );
    }
    assert_eq!(
        programs[0].additional_processes,
        ["steam.exe", "steamwebhelper.exe"]
    );
    assert_eq!(programs[1].additional_processes, ["Discord.exe"]);
    let mut duplicate = programs[0].clone();
    duplicate.args = vec!["-silent".into()];
    programs.push(duplicate);
    let count = os_api::cpu_load::logical_cpu_count();
    assert!(count > 0);
    let core = (count - 1).min(19);
    let mut state = AppStateStorage::test_fixture(|state| {
        state.groups = vec![CoreGroup {
            name: "Owned clients".into(),
            cores: vec![core],
            programs,
            is_hidden: false,
            run_all_button: false,
        }];
        state.rule_identities = None;
        state.cpu_load_history_seconds = 30;
        state.process_monitoring_enabled = false;
    });
    let ids = RulesContext::from_storage(&state).to_persisted_identities();
    let group = ids.groups[0].id.clone();
    let rules = ids.groups[0].rule_ids.clone();
    state.mark_ready_for_current_schema_save(ids);
    let path = root.join("state.json");
    state.try_save_to_path(&path).unwrap();
    let loaded = AppStateStorage::load_from_path(&path);
    assert_eq!(
        serde_json::to_value(&loaded.groups).unwrap(),
        serde_json::to_value(&state.groups).unwrap()
    );
    let source = Arc::new(RwLock::new(loaded));
    let mut runtime = RuntimeRegistry::new();
    let mut log = LogManager::default();
    let foreign_path = root.join("Foreign/bin/cef/cef.win64/steamwebhelper.exe");
    let mut foreign = Command::new(&foreign_path)
        .arg("--fixture-helper")
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let foreign_pid = foreign.id();
    let foreign_mask = os_api::OS::get_process_affinity(foreign_pid).unwrap();
    for index in 0..2 {
        let app = source.read().unwrap().groups[0].programs[index].clone();
        assert_eq!(
            execution::run_app_with_affinity_sync(&source, &runtime, &mut log, 0, index, app),
            execution::LaunchDispatchOutcome::Accepted
        );
    }
    let until = Instant::now() + Duration::from_secs(12);
    let mut updater = None;
    while Instant::now() < until {
        execution::native_known_refresh(&runtime, &source.read().unwrap()).unwrap();
        updater = owned_image_pids(&discord).first().copied().or(updater);
        let running = runtime.running_apps_handle();
        let apps = running.blocking_read();
        if apps.apps.len() == 3
            && apps
                .apps
                .values()
                .all(|app| app.pids.len() == 2 && app.settings_matched)
        {
            break;
        }
        drop(apps);
        std::thread::sleep(Duration::from_millis(150));
    }
    {
        let running = runtime.running_apps_handle();
        let apps = running.blocking_read();
        if apps.apps.len() != 3 {
            for (pid, name) in os_api::OS::get_all_process_names() {
                let Ok((image, token)) = os_api::OS::get_process_image_path_and_instance_token(pid)
                else {
                    continue;
                };
                if image
                    .canonicalize()
                    .is_ok_and(|image| image.starts_with(&root))
                {
                    let verified =
                        os_api::agent::ExactProcess::open(pid, token, false, false, true)
                            .map(|_| true);
                    for app in &source.read().unwrap().groups[0].programs[..2] {
                        let scope = known_apps::project(app).unwrap();
                        let role = os_api::app_identity::canonical_scope(&scope.root, &image)
                            .and_then(|(root, image)| scope.witness_role_under_root(&root, &image));
                        eprintln!("owned candidate {pid}/{name}: role={role:?}, verified={verified:?}, image={image:?}, root={:?}",scope.root);
                    }
                }
            }
        }
        assert_eq!(apps.apps.len(), 3);
        for app in apps.apps.values() {
            assert_eq!(app.pids.len(), 2);
            assert!(app.settings_matched);
            for (&pid, &token) in &app.pid_instance_tokens {
                assert_ne!(pid, foreign_pid);
                let process = os_api::agent::ExactProcess::open(pid, token, false, false, true)
                    .unwrap()
                    .state()
                    .unwrap();
                assert_eq!(process.affinity, 1usize << core);
                assert_eq!(process.priority, PriorityClass::BelowNormal);
            }
        }
    }
    assert_eq!(
        os_api::OS::get_process_affinity(foreign_pid).unwrap(),
        foreign_mask,
        "other installation's helper was changed"
    );
    let (updater_pid, updater_token) = updater.expect("ordinary launcher was not observed");
    let updater = os_api::agent::ExactProcess::open(updater_pid, updater_token, false, false, true)
        .unwrap()
        .state()
        .unwrap();
    assert!(!updater.elevated);
    assert_eq!(updater.priority, PriorityClass::Normal);
    if count > 1 {
        assert_ne!(
            updater.affinity,
            1usize << core,
            "launcher inherited the app's core restriction"
        );
    }
    assert!(runtime
        .running_apps_handle()
        .blocking_read()
        .apps
        .values()
        .all(|app| !app.pids.contains(&updater_pid)));
    // Keep the previous standard shortcut route covered alongside the current stub.
    let legacy = launch_root.join("Discord/Update.exe");
    os_api::app_identity::launch_ordinary(
        &legacy,
        &["--processStart".into(), "Discord.exe".into()],
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    let (legacy_pid, legacy_token) = loop {
        if let Some(instance) = owned_image_pids(&legacy).first() {
            break *instance;
        }
        assert!(Instant::now() < until, "legacy launcher was not observed");
        std::thread::sleep(Duration::from_millis(25));
    };
    let legacy_state =
        os_api::agent::ExactProcess::open(legacy_pid, legacy_token, false, false, true)
            .unwrap()
            .state()
            .unwrap();
    assert!(!legacy_state.elevated);
    assert_eq!(legacy_state.priority, PriorityClass::Normal);
    if count > 1 {
        assert_ne!(legacy_state.affinity, 1usize << core);
    }
    let mut collector = Collector(CpuLoadRuntime::new(
        runtime.running_apps_handle(),
        true,
        source.clone(),
    ));
    collector.set_history_seconds(30);
    collector.configure(true, false);
    let targets: Vec<_> = rules
        .iter()
        .map(|rule| RuleTarget {
            group: group.clone(),
            rule: rule.clone(),
        })
        .collect();
    let until = Instant::now() + Duration::from_secs(12);
    let snapshot = loop {
        execution::native_known_refresh(&runtime, &source.read().unwrap()).unwrap();
        let snapshot = collector.snapshot();
        if targets.iter().all(|target| {
            snapshot
                .processes
                .get(target)
                .copied()
                .flatten()
                .is_some_and(|value| value > 0.0)
                && snapshot
                    .process_memory
                    .get(target)
                    .is_some_and(|usage| usage.measured == 2 && usage.bytes.is_some())
        }) && snapshot
            .tracked_cpus
            .get(core)
            .copied()
            .flatten()
            .is_some_and(|value| value > 0.0)
        {
            break snapshot;
        }
        assert!(
            Instant::now() < until,
            "native CPU/RAM did not become available: {:?} / {:?}",
            snapshot.error,
            snapshot.detail_error
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(
        snapshot.instances, 4,
        "overlapping rules duplicated the native target set"
    );
    assert!(runtime
        .running_apps_handle()
        .blocking_read()
        .apps
        .values()
        .all(|app| !app.pids.contains(&legacy_pid) && !app.pids.contains(&updater_pid)));
    let steam_ram = snapshot.process_memory[&targets[0]].bytes.unwrap();
    let discord_ram = snapshot.process_memory[&targets[1]].bytes.unwrap();
    assert!(steam_ram > 8 * 1024 * 1024 && discord_ram > 8 * 1024 * 1024);
    assert_eq!(snapshot.process_memory[&targets[2]].bytes, Some(steam_ram));
    assert_eq!(
        snapshot.group_memory.as_ref().unwrap()[&group].bytes,
        Some(steam_ram + discord_ram)
    );
    assert_eq!(
        snapshot.processes[&targets[0]],
        snapshot.processes[&targets[2]]
    );
    let expected = snapshot.process_cpus[&targets[0]][core].unwrap()
        + snapshot.process_cpus[&targets[1]][core].unwrap();
    assert!((snapshot.tracked_cpus[core].unwrap() - expected).abs() < 0.1);
    let mut focused = vec![];
    for index in 0..2 {
        assert!(
            focus_owned(&mut runtime, &source, index, &mut log),
            "native foreground activation refused"
        );
        let (window, _) = os_api::OS::foreground_process().unwrap();
        os_api::OS::hide_window(windows::Win32::Foundation::HWND(window as *mut _));
        assert!(
            focus_owned(&mut runtime, &source, index, &mut log),
            "hidden client activation refused"
        );
        focused.push(true);
    }
    collector.configure(false, false);
    let until = Instant::now() + Duration::from_secs(5);
    while !collector.snapshot().cpus.is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(collector.snapshot().cpus.is_empty());
    assert!(collector.snapshot().process_memory.is_empty());
    let result = serde_json::json!({"status":"pass","schema":13,"apps":2,"overlapping_rules":3,"unique_instances":snapshot.instances,"core":core,"app_cpu_percent":[snapshot.processes[&targets[0]],snapshot.processes[&targets[1]]],"tracked_core_percent":snapshot.tracked_cpus[core],"ram_bytes":[steam_ram,discord_ram],"group_ram_bytes":steam_ram+discord_ram,"focus_and_hidden_activation":focused,"ordinary_unscheduled_updater":true,"foreign_helper_unchanged":true,"cpu_off_clears_values":true});
    if let Some(path) = std::env::var_os("CAT_KNOWN_RESULT_PATH") {
        let path = PathBuf::from(path);
        assert_eq!(path.parent().unwrap().canonicalize().unwrap(), root);
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    println!("{result}");
    let _ = foreign.kill();
    let _ = foreign.wait();
}
