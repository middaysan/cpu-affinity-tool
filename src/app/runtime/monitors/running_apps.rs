use crate::app::models::{AppStateStorage, RunningApps};
use eframe::egui;
use os_api::OS;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

pub async fn run_running_app_monitor(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    app_state: Arc<RwLock<AppStateStorage>>,
    ctx: egui::Context,
    monitor_tx: std::sync::mpsc::Sender<String>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

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
            let mut programs = Vec::new();
            for (g_idx, group) in state.groups.iter().enumerate() {
                for (p_idx, program) in group.programs.iter().enumerate() {
                    let mut names = Vec::new();

                    if let Some(n) = program.name.split('.').next() {
                        if !n.is_empty() {
                            names.push(n.to_string());
                        }
                    }

                    if let Some(n) = program
                        .bin_path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .and_then(|s| s.split('.').next())
                    {
                        if !n.is_empty() && !names.contains(&n.to_string()) {
                            names.push(n.to_string());
                        }
                    }

                    if let Some(n) = program
                        .dropped_path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .and_then(|s| s.split('.').next())
                    {
                        if !n.is_empty() && !names.contains(&n.to_string()) {
                            names.push(n.to_string());
                        }
                    }

                    if !names.is_empty() {
                        programs.push((program.clone(), names, g_idx, p_idx));
                    }
                }
            }
            programs
        };

        let tree = match OS::snapshot_process_tree() {
            Ok(t) => t,
            Err(_) => continue,
        };

        let mut name_to_pids: HashMap<String, Vec<u32>> = HashMap::new();
        for (&pid, full_name) in &tree.names {
            let name = full_name.split('.').next().unwrap_or("").to_string();
            name_to_pids
                .entry(name.to_lowercase())
                .or_default()
                .push(pid);
        }

        if let Ok(mut apps) = running_apps.try_write() {
            let mut processed_keys = HashSet::new();
            let mut changed = false;

            for (program, names, g_idx, p_idx) in configured_programs {
                let key = program.get_key();
                processed_keys.insert(key.clone());
                let mut verified_pids = Vec::new();

                for name in names {
                    let target_lower = name.to_lowercase();
                    for (p_name, pids) in &name_to_pids {
                        if p_name.starts_with(&target_lower) {
                            for &pid in pids {
                                if verified_pids.contains(&pid) {
                                    continue;
                                }

                                if *p_name == target_lower {
                                    if let Ok(image_path) = OS::get_process_image_path(pid) {
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

                for proc_name in &program.additional_processes {
                    let search_name = proc_name.split('.').next().unwrap_or("").to_lowercase();
                    if !search_name.is_empty() {
                        for (p_name, pids) in &name_to_pids {
                            if p_name.starts_with(&search_name) {
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
                        OS::find_all_descendants_with_tree(app.pids[0], &mut app.pids, &tree);
                    }

                    for pid in verified_pids {
                        if !app.pids.contains(&pid) {
                            app.pids.push(pid);
                        }
                    }

                    app.pids.retain(|&pid| OS::is_pid_live(pid));

                    if app.pids.len() != old_pid_count {
                        changed = true;
                    }

                    if app.pids.is_empty() {
                        let _ = monitor_tx.send(format!("App stopped: {}", program.name));
                        apps.remove_app(&key);
                        changed = true;
                    }
                } else if !verified_pids.is_empty() {
                    let _ = monitor_tx.send(format!(
                        "App detected: {} (PID {})",
                        program.name, verified_pids[0]
                    ));
                    apps.add_app(&key, verified_pids[0], g_idx, p_idx);
                    changed = true;

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
                            OS::find_all_descendants_with_tree(app.pids[0], &mut app.pids, &tree);
                        }
                        app.pids.retain(|&pid| OS::is_pid_live(pid));

                        if app.pids.is_empty() {
                            apps.remove_app(&key);
                            changed = true;
                        } else if app.pids.len() != old_pid_count {
                            changed = true;
                        }
                    }
                }
            }

            if changed {
                ctx.request_repaint();
            }
        }
    }
}
