use crate::app::models::{AppRuntimeKey, AppStatus, RunningApps};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;

/// Runtime-only process tracking state.
pub struct RuntimeRegistry {
    pub(crate) running_apps: Arc<TokioRwLock<RunningApps>>,
    pub(crate) running_apps_statuses: HashMap<AppRuntimeKey, AppStatus>,
    pub(crate) monitor_rx: Option<Receiver<String>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            running_apps: Arc::new(TokioRwLock::new(RunningApps::default())),
            running_apps_statuses: HashMap::new(),
            monitor_rx: None,
        }
    }

    pub fn add_running_app(
        &self,
        app_key: &AppRuntimeKey,
        pid: u32,
        group_index: usize,
        prog_index: usize,
    ) -> bool {
        match self.running_apps.try_write() {
            Ok(mut apps) => {
                apps.add_app(app_key, pid, group_index, prog_index);
                true
            }
            Err(_) => false,
        }
    }

    pub fn contains_app(&self, app_key: &AppRuntimeKey) -> bool {
        self.running_apps
            .try_read()
            .map(|apps| apps.apps.contains_key(app_key))
            .unwrap_or(false)
    }

    pub fn add_pid_to_existing_app(&self, app_key: &AppRuntimeKey, pid: u32) -> bool {
        match self.running_apps.try_write() {
            Ok(mut apps) => {
                if let Some(app) = apps.apps.get_mut(app_key) {
                    if !app.pids.contains(&pid) {
                        app.pids.push(pid);
                    }
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    pub fn get_app_status_sync(&mut self, app_key: &AppRuntimeKey) -> AppStatus {
        if let Ok(apps) = self.running_apps.try_read() {
            let status = if let Some(app) = apps.apps.get(app_key) {
                if app.settings_matched {
                    AppStatus::Running
                } else {
                    AppStatus::SettingsMismatch
                }
            } else {
                AppStatus::NotRunning
            };
            self.running_apps_statuses.insert(app_key.clone(), status);
            status
        } else {
            self.running_apps_statuses
                .get(app_key)
                .copied()
                .unwrap_or(AppStatus::NotRunning)
        }
    }

    pub fn get_running_app_pids(&self, app_key: &AppRuntimeKey) -> Option<Vec<u32>> {
        if let Ok(apps) = self.running_apps.try_read() {
            apps.apps.get(app_key).map(|app| app.pids.clone())
        } else {
            None
        }
    }
}
