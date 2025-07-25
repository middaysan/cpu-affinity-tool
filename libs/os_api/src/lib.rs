
mod process;
pub use process::PriorityClass;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

// Export the necessary implementation under a common interface
#[cfg(target_os = "windows")]
pub use windows::OS;
#[cfg(target_os = "linux")]
pub use linux::OS;