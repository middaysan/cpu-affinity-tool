mod launch;
mod monitor_events;
mod reconcile;
mod store;
mod tracking;

use crate::app::models::{normalize_process_name, AppStateStorage, RunningApps};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

pub(crate) use launch::{run_app_row_action, AppRowActionRequest};
pub use launch::{
    run_app_with_affinity_sync, start_app_with_autorun, AppRowAction, LaunchDispatchOutcome,
};
#[cfg(test)]
pub(crate) use monitor_events::monitor_event_channel;
pub(crate) use monitor_events::{
    monitor_event_channel_with_wake, MonitorDrainStatus, MonitorEventReceiver, MonitorEventSender,
    MonitorWake,
};
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

pub(crate) fn spawn_monitors_with_wake(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    persistent_state: Arc<RwLock<AppStateStorage>>,
    wake: Option<MonitorWake>,
) -> MonitorEventReceiver {
    let (monitor_tx, monitor_rx) = monitor_event_channel_with_wake(wake);

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
