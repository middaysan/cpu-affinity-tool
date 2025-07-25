/// The models module contains all the data structures and state management components
/// of the application. This includes the core application state, UI state, and structures
/// for representing and managing CPU core groups and applications.

/// Log management functionality
mod log_manager;
/// Central application state management
mod app_state;
/// Main application structure
mod app;
/// CPU core grouping functionality
mod core_group;
/// Persistent state storage
mod app_state_storage;
/// Application execution configuration
mod app_to_run;
/// Running application tracking
mod running_app;

// Public re-exports of key structures for use in other modules
pub use log_manager::LogManager;
pub use core_group::GroupFormState;
pub use app_state::AppState;
pub use app::App;
pub use app_to_run::AppToRun;
