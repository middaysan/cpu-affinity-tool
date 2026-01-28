/// Main application structure
mod app;
/// Central application state management
mod app_state;
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
/// of the application. This includes the core application state, UI state, and structures
/// for representing and managing CPU core groups and applications.
/// Log management functionality
mod log_manager;
mod meta;
/// Running application tracking
mod running_app;

// Public re-exports of key structures for use in other modules
pub use app::App;
pub use app_state::AppState;
pub use app_state_storage::AppStateStorage;
pub use app_to_run::{AppToRun, RunAppEditState};
pub use core_group::GroupFormState;
pub use cpu_schema::{CoreInfo, CoreType, CpuCluster, CpuSchema};
pub use log_manager::LogManager;
pub use meta::APP_VERSION;
pub use running_app::{AppStatus, RunningApps};
