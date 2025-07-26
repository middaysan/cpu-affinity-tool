mod process;
pub use process::PriorityClass;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

// Export the necessary implementation under a common interface
#[cfg(target_os = "linux")]
pub use linux::OS;
#[cfg(target_os = "windows")]
pub use windows::OS;
