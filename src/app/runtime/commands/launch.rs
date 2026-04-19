use crate::app::models::{AppStateStorage, AppToRun, LogManager};
use crate::app::runtime::RuntimeRegistry;
use os_api::OS;
use std::sync::{Arc, RwLock};

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
    for (g_i, p_i, app_to_run) in collect_autorun_items(persistent_state) {
        run_app_with_affinity_sync(persistent_state, runtime, log_manager, g_i, p_i, app_to_run);
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
    let app_key = app_to_run.get_key();
    let group = {
        let state = persistent_state.read().unwrap();
        match state.groups.get(group_index) {
            Some(g) => g.clone(),
            None => {
                log_manager.add_entry(format!("Error: Group index {group_index} not found"));
                return;
            }
        }
    };

    if let Some(pids) = runtime.get_running_app_pids(&app_key) {
        let mask = group.cores.iter().fold(0usize, |acc, &i| acc | (1 << i));
        for &pid in &pids {
            let _ = OS::set_process_affinity_by_pid(pid, mask);
            let _ = OS::set_process_priority_by_pid(pid, app_to_run.priority);
        }

        let was_focused = pids.iter().any(|&pid| OS::focus_window_by_pid(pid));
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
    }

    let label = app_to_run
        .bin_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| app_to_run.bin_path.display().to_string());

    log_manager.add_entry(format!(
        "Starting '{}', app: {}",
        label,
        app_to_run.display()
    ));

    match OS::run(
        app_to_run.bin_path,
        app_to_run.args,
        &group.cores,
        app_to_run.priority,
    ) {
        Ok(pid) => {
            let is_new_app = !runtime.contains_app(&app_key);

            if is_new_app {
                let added = runtime.add_running_app(&app_key, pid, group_index, prog_index);
                if added {
                    log_manager.add_entry(format!("App started with PID: {pid}"));
                } else {
                    log_manager.add_entry(format!(
                        "App started with PID: {pid} but couldn't be tracked (lock busy)"
                    ));
                }
            } else {
                let _ = runtime.add_pid_to_existing_app(&app_key, pid);
                log_manager.add_entry(format!(
                    "New instance of existing app started with PID: {pid}"
                ));
            }
        }
        Err(e) => log_manager.add_entry(format!("ERROR: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::collect_autorun_items;
    use crate::app::models::{AppStateStorage, AppToRun, CoreGroup, CpuSchema};
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    fn sample_state() -> Arc<RwLock<AppStateStorage>> {
        Arc::new(RwLock::new(AppStateStorage {
            version: 4,
            groups: vec![
                CoreGroup {
                    name: "Games".to_string(),
                    cores: vec![0, 1],
                    programs: vec![
                        AppToRun::new(
                            PathBuf::from(r"C:\one.lnk"),
                            vec![],
                            PathBuf::from(r"C:\one.exe"),
                            os_api::PriorityClass::Normal,
                            false,
                        ),
                        AppToRun::new(
                            PathBuf::from(r"C:\two.lnk"),
                            vec![],
                            PathBuf::from(r"C:\two.exe"),
                            os_api::PriorityClass::Normal,
                            true,
                        ),
                    ],
                    is_hidden: false,
                    run_all_button: true,
                },
                CoreGroup {
                    name: "Work".to_string(),
                    cores: vec![2, 3],
                    programs: vec![AppToRun::new(
                        PathBuf::from(r"C:\three.lnk"),
                        vec![],
                        PathBuf::from(r"C:\three.exe"),
                        os_api::PriorityClass::High,
                        true,
                    )],
                    is_hidden: false,
                    run_all_button: false,
                },
            ],
            cpu_schema: CpuSchema {
                model: "Test CPU".to_string(),
                clusters: Vec::new(),
            },
            theme_index: 0,
            process_monitoring_enabled: false,
        }))
    }

    #[test]
    fn test_collect_autorun_items_preserves_indices() {
        let items = collect_autorun_items(&sample_state());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, 0);
        assert_eq!(items[0].1, 1);
        assert_eq!(items[0].2.bin_path, PathBuf::from(r"C:\two.exe"));
        assert_eq!(items[1].0, 1);
        assert_eq!(items[1].1, 0);
        assert_eq!(items[1].2.bin_path, PathBuf::from(r"C:\three.exe"));
    }
}
