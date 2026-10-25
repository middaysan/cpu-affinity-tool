pub mod background;
mod catalog;
pub mod core_parking;
pub mod cpu_load;
pub mod gaming;
pub mod gaming_cpu_power;
pub mod gaming_diagnostics;
pub mod pc_checks;
pub mod platform;
mod process;
#[cfg(all(target_os = "windows", feature = "render-profiling"))]
pub mod render_profile;
mod shortcut;
pub mod tuning;
pub mod windows_tweaks;
pub use catalog::{
    InstalledAppCatalogEntry, InstalledAppCatalogSource, InstalledAppCatalogTarget,
    InstalledPackageRuntimeInfo,
};
pub use process::{PriorityClass, ProcessSettingsApplyOutcome};
pub use shortcut::ShortcutSpec;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::agent;

// Export the necessary implementation under a common interface
#[cfg(target_os = "linux")]
pub use linux::OS;
#[cfg(target_os = "windows")]
pub use windows::{
    LocalIpcClientError, LocalIpcEndpoint, LocalIpcGuard, LocalIpcRequest, LocalIpcServer,
    LocalIpcWake, OS, WindowActivation, WindowFileDrop, WindowFileDropGuard,
    WindowsApplicationFailure,
};
