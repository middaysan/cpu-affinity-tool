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
/// The models module contains all the data structures and state management components
/// of the application. This includes the core application state, UI state, and structures
/// for representing and managing CPU core groups and applications.
/// Log management functionality
mod log_manager;
/// Running application tracking
mod running_app;

// Public re-exports of key structures for use in other modules
pub use app::App;
pub use app_state::AppState;
pub use app_to_run::AppToRun;
pub use core_group::GroupFormState;
pub use log_manager::LogManager;
