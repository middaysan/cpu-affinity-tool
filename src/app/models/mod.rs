/// Persistent state storage
mod app_state_storage;
/// Application execution configuration
mod app_to_run;
/// CPU core grouping functionality
mod core_group;
/// CPU schema presets for popular processors
pub mod cpu_presets;
/// CPU schema and core types
pub mod cpu_schema;
/// The models module contains all the data structures and state management components
/// of the application. This includes persisted domain state and structures
/// for representing and managing CPU core groups and applications.
/// Log management functionality
mod log_manager;
mod meta;
/// Running application tracking
mod running_app;

// Public re-exports of key structures for use in other modules
pub use app_state_storage::AppStateStorage;
pub use app_to_run::{AppRuntimeKey, AppToRun, LaunchTarget};
pub use core_group::{AddAppsOutcome, CoreGroup};
pub use cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
pub use log_manager::LogManager;
pub use meta::{effective_cpu_model, effective_total_threads, APP_VERSION};
pub use running_app::{AppStatus, RunningApps};
