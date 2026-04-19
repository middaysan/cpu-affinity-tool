use crate::app::models::{AppStateStorage, RunningApps};
use eframe::egui;
use os_api::OS;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

pub async fn run_process_settings_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    ctx: egui::Context,
    monitor_tx: std::sync::mpsc::Sender<String>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));

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

        let mut key_to_program_group = HashMap::new();
        for group in &groups {
            for program in &group.programs {
                key_to_program_group.insert(program.get_key(), (program, group));
            }
        }

        let mut changed = false;
        let mut notifications = Vec::new();

        if let Ok(mut apps) = running_apps.try_write() {
            for (app_key, app) in apps.apps.iter_mut() {
                if let Some(&(program, group)) = key_to_program_group.get(app_key) {
                    if let Some(new_group_idx) = groups.iter().position(|g| std::ptr::eq(g, group))
                    {
                        app.group_index = new_group_idx;
                        if let Some(new_prog_idx) =
                            group.programs.iter().position(|p| p.get_key() == *app_key)
                        {
                            app.prog_index = new_prog_idx;
                        }
                    }

                    let mut expected_mask = 0usize;
                    for &i in &group.cores {
                        if i < (std::mem::size_of::<usize>() * 8) {
                            expected_mask |= 1 << i;
                        }
                    }

                    let expected_priority = program.priority;
                    let mut all_matched = true;

                    for &pid in &app.pids {
                        if let Ok(current_mask) = OS::get_process_affinity(pid) {
                            if current_mask != expected_mask {
                                all_matched = false;
                                if monitoring_enabled
                                    && OS::set_process_affinity_by_pid(pid, expected_mask).is_ok()
                                {
                                    notifications.push(format!(
                                        "Fixed affinity for {} (PID {}): {:X} -> {:X}",
                                        program.name, pid, current_mask, expected_mask
                                    ));
                                }
                            }
                        }

                        if let Ok(current_priority) = OS::get_process_priority(pid) {
                            if current_priority != expected_priority {
                                all_matched = false;
                                if monitoring_enabled
                                    && OS::set_process_priority_by_pid(pid, expected_priority)
                                        .is_ok()
                                {
                                    notifications.push(format!(
                                        "Fixed priority for {} (PID {}): {:?} -> {:?}",
                                        program.name, pid, current_priority, expected_priority
                                    ));
                                }
                            }
                        }
                    }

                    if app.settings_matched != all_matched {
                        app.settings_matched = all_matched;
                        changed = true;
                    }
                }
            }
        }

        if !notifications.is_empty() {
            for msg in notifications {
                let _ = monitor_tx.send(format!("MONITOR: {}", msg));
                #[cfg(debug_assertions)]
                println!("MONITOR: {}", msg);
            }
        }

        if changed {
            ctx.request_repaint();
        }
    }
}
