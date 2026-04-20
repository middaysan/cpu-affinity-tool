mod process_settings;
mod running_apps;

use crate::app::models::{AppStateStorage, RunningApps};
use crate::app::runtime::runtime_registry::InstalledPackageTrackingState;
use eframe::egui;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;

pub use process_settings::run_process_settings_monitor;
pub use running_apps::run_running_app_monitor;

pub fn spawn_monitors(
    running_apps: Arc<TokioRwLock<RunningApps>>,
    installed_package_tracking: Arc<RwLock<InstalledPackageTrackingState>>,
    persistent_state: Arc<RwLock<AppStateStorage>>,
    ctx: egui::Context,
) -> Receiver<String> {
    let (monitor_tx, monitor_rx) = std::sync::mpsc::channel();

    tokio::spawn(run_running_app_monitor(
        running_apps.clone(),
        installed_package_tracking,
        persistent_state.clone(),
        ctx.clone(),
        monitor_tx.clone(),
    ));
    tokio::spawn(run_process_settings_monitor(
        running_apps,
        persistent_state,
        ctx,
        monitor_tx,
    ));

    monitor_rx
}
