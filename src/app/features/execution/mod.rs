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
pub(crate) use monitor_events::{
    monitor_event_channel, MonitorDrainStatus, MonitorEventReceiver, MonitorEventSender,
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

pub fn spawn_monitors(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    persistent_state: Arc<RwLock<AppStateStorage>>,
) -> MonitorEventReceiver {
    let (monitor_tx, monitor_rx) = monitor_event_channel();
    let (legacy_monitor_tx, legacy_monitor_rx) = std::sync::mpsc::channel();
    let bridge_tx = monitor_tx.clone();

    // `reconcile` still uses the legacy std sender while its process-identity
    // hardening is being integrated. The bridge is the only temporary
    // compatibility point; it preserves a bounded, non-blocking GUI queue.
    std::thread::Builder::new()
        .name("monitor-event-bridge".to_string())
        .spawn(move || {
            while let Ok(event) = legacy_monitor_rx.recv() {
                bridge_tx.try_send(event);
            }
        })
        .expect("failed to start monitor event bridge");

    tokio::spawn(run_running_app_monitor(
        running_apps.clone(),
        installed_package_tracking,
        persistent_state.clone(),
        monitor_tx.clone(),
    ));
    tokio::spawn(run_process_settings_monitor(
        running_apps,
        persistent_state,
        legacy_monitor_tx,
    ));

    monitor_rx
}
