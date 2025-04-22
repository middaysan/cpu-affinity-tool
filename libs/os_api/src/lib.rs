
mod process;
pub use process::PriorityClass;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

// Экспорт нужной реализации под общим интерфейсом
#[cfg(target_os = "windows")]
pub use windows::OS;
#[cfg(target_os = "linux")]
pub use linux::OS;