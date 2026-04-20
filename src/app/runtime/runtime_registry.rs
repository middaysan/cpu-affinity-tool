use crate::app::models::{AppRuntimeKey, AppStatus, RunningApps};
use os_api::InstalledPackageRuntimeInfo;
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

#[derive(Debug, Default)]
pub(crate) struct InstalledPackageTrackingState {
    metadata_by_aumid: HashMap<String, Result<InstalledPackageRuntimeInfo, String>>,
    package_owner_by_family: HashMap<String, AppRuntimeKey>,
}

fn normalize_aumid_key(aumid: &str) -> String {
    aumid.to_lowercase()
}

fn normalize_package_family_key(package_family_name: &str) -> String {
    package_family_name.to_lowercase()
}

/// Runtime-only process tracking state.
pub struct RuntimeRegistry {
    pub(crate) running_apps: Arc<TokioRwLock<RunningApps>>,
    pub(crate) installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    pub(crate) running_apps_statuses: HashMap<AppRuntimeKey, AppStatus>,
    pub(crate) monitor_rx: Option<Receiver<String>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            running_apps: Arc::new(TokioRwLock::new(RunningApps::default())),
            installed_package_tracking: Arc::new(RwLock::new(
                InstalledPackageTrackingState::default(),
            )),
            running_apps_statuses: HashMap::new(),
            monitor_rx: None,
        }
    }

    pub(crate) fn resolve_installed_package_runtime_info_with<F>(
        &self,
        aumid: &str,
        resolver: F,
    ) -> Result<InstalledPackageRuntimeInfo, String>
    where
        F: FnOnce(&str) -> Result<InstalledPackageRuntimeInfo, String>,
    {
        resolve_installed_package_runtime_info_cached(
            &self.installed_package_tracking,
            aumid,
            resolver,
        )
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

pub(crate) fn resolve_installed_package_runtime_info_cached<F>(
    installed_package_tracking: &Arc<RwLock<InstalledPackageTrackingState>>,
    aumid: &str,
    resolver: F,
) -> Result<InstalledPackageRuntimeInfo, String>
where
    F: FnOnce(&str) -> Result<InstalledPackageRuntimeInfo, String>,
{
    let normalized_aumid = normalize_aumid_key(aumid);

    if let Some(cached) = installed_package_tracking
        .read()
        .unwrap()
        .metadata_by_aumid
        .get(&normalized_aumid)
        .cloned()
    {
        return cached;
    }

    let resolved = resolver(aumid);
    installed_package_tracking
        .write()
        .unwrap()
        .metadata_by_aumid
        .insert(normalized_aumid, resolved.clone());
    resolved
}

pub(crate) fn ensure_package_owner_claim(
    tracking: &mut InstalledPackageTrackingState,
    running_apps: &RunningApps,
    package_family_name: &str,
    app_key: &AppRuntimeKey,
) -> bool {
    let normalized_package_family = normalize_package_family_key(package_family_name);
    if normalized_package_family.is_empty() {
        return false;
    }

    match tracking
        .package_owner_by_family
        .get(&normalized_package_family)
        .cloned()
    {
        Some(owner) if owner == *app_key => true,
        Some(owner) if running_apps.apps.contains_key(&owner) => false,
        _ => {
            tracking
                .package_owner_by_family
                .insert(normalized_package_family, app_key.clone());
            true
        }
    }
}

pub(crate) fn cleanup_orphaned_package_owners(
    tracking: &mut InstalledPackageTrackingState,
    running_apps: &RunningApps,
) {
    tracking
        .package_owner_by_family
        .retain(|_, owner| running_apps.apps.contains_key(owner));
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_orphaned_package_owners, ensure_package_owner_claim,
        resolve_installed_package_runtime_info_cached, InstalledPackageTrackingState,
    };
    use crate::app::models::{AppToRun, RunningApps};
    use os_api::{InstalledPackageRuntimeInfo, PriorityClass};
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};

    fn installed_app(name: &str, aumid: &str, priority: PriorityClass) -> AppToRun {
        AppToRun::new_installed(name.to_string(), aumid.to_string(), priority, false)
    }

    #[test]
    fn test_resolve_installed_package_runtime_info_cached_reuses_cached_result() {
        let tracking = Arc::new(RwLock::new(InstalledPackageTrackingState::default()));
        let calls = Cell::new(0usize);

        let first = resolve_installed_package_runtime_info_cached(&tracking, "Pkg!App", |aumid| {
            calls.set(calls.get() + 1);
            Ok(InstalledPackageRuntimeInfo {
                aumid: aumid.to_string(),
                package_family_name: "Pkg".into(),
                install_root: PathBuf::from(r"C:\WindowsApps\Pkg"),
            })
        })
        .unwrap();
        let second =
            resolve_installed_package_runtime_info_cached(&tracking, "pkg!app", |_aumid| {
                calls.set(calls.get() + 1);
                Err("should not be called".into())
            })
            .unwrap();

        assert_eq!(calls.get(), 1);
        assert_eq!(first, second);
    }

    #[test]
    fn test_resolve_installed_package_runtime_info_cached_caches_failure() {
        let tracking = Arc::new(RwLock::new(InstalledPackageTrackingState::default()));
        let calls = Cell::new(0usize);

        let first = resolve_installed_package_runtime_info_cached(&tracking, "Pkg!App", |_aumid| {
            calls.set(calls.get() + 1);
            Err("boom".into())
        });
        let second =
            resolve_installed_package_runtime_info_cached(&tracking, "Pkg!App", |_aumid| {
                calls.set(calls.get() + 1);
                Ok(InstalledPackageRuntimeInfo {
                    aumid: "Pkg!App".into(),
                    package_family_name: "Pkg".into(),
                    install_root: PathBuf::from(r"C:\WindowsApps\Pkg"),
                })
            });

        assert_eq!(calls.get(), 1);
        assert_eq!(first, Err("boom".into()));
        assert_eq!(second, Err("boom".into()));
    }

    #[test]
    fn test_first_active_target_wins_package_owner_claim() {
        let mut tracking = InstalledPackageTrackingState::default();
        let mut running_apps = RunningApps::default();
        let first = installed_app("Spotify", "Pkg!AppA", PriorityClass::Normal).get_key();
        let second = installed_app("Spotify Launcher", "Pkg!AppB", PriorityClass::Normal).get_key();
        running_apps.add_app(&first, 10, 0, 0);
        running_apps.add_app(&second, 20, 0, 1);

        assert!(ensure_package_owner_claim(
            &mut tracking,
            &running_apps,
            "Pkg",
            &first
        ));
        assert!(!ensure_package_owner_claim(
            &mut tracking,
            &running_apps,
            "Pkg",
            &second
        ));
    }

    #[test]
    fn test_cleanup_orphaned_package_owners_releases_stale_claims() {
        let mut tracking = InstalledPackageTrackingState::default();
        let mut running_apps = RunningApps::default();
        let first = installed_app("Spotify", "Pkg!AppA", PriorityClass::Normal).get_key();
        let second = installed_app("Spotify Launcher", "Pkg!AppB", PriorityClass::Normal).get_key();

        running_apps.add_app(&first, 10, 0, 0);
        assert!(ensure_package_owner_claim(
            &mut tracking,
            &running_apps,
            "Pkg",
            &first
        ));

        running_apps.remove_app(&first);
        cleanup_orphaned_package_owners(&mut tracking, &running_apps);

        running_apps.add_app(&second, 20, 0, 1);
        assert!(ensure_package_owner_claim(
            &mut tracking,
            &running_apps,
            "Pkg",
            &second
        ));
    }
}
