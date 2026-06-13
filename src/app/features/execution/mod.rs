mod launch;
mod reconcile;
mod store;
mod tracking;

use crate::app::features::diagnostics::DiagnosticEvent;
use crate::app::models::{normalize_process_name, AppStateStorage, RunningApps};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

pub use launch::{run_app_with_affinity_sync, start_app_with_autorun, LaunchDispatchOutcome};
pub use reconcile::run_process_settings_monitor;
pub use store::RuntimeRegistry;
pub(crate) use store::{
    cleanup_orphaned_package_owners, ensure_package_owner_claim,
    resolve_installed_package_runtime_info_cached, InstalledPackageTrackingState,
};
pub use tracking::run_running_app_monitor;

pub(crate) fn is_excluded_installed_auto_process(process_name: &str) -> bool {
    matches!(
        normalize_process_name(process_name).as_str(),
        "backgroundtaskhost"
    )
}

pub fn spawn_monitors(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    persistent_state: Arc<RwLock<AppStateStorage>>,
) -> Receiver<DiagnosticEvent> {
    let (monitor_tx, monitor_rx) = std::sync::mpsc::channel();

    tokio::spawn(run_running_app_monitor(
        running_apps.clone(),
        installed_package_tracking,
        persistent_state.clone(),
        monitor_tx.clone(),
    ));
    tokio::spawn(run_process_settings_monitor(
        running_apps,
        persistent_state,
        monitor_tx,
    ));

    monitor_rx
}
