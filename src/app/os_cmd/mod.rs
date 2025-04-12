use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone,PartialEq, Eq, Copy)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    Normal,
    AboveNormal,
    High,
    Realtime,
}

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows::OsCmd;

#[cfg(not(target_os = "windows"))]
mod linux;
#[cfg(not(target_os = "windows"))]
pub use linux::OsCmd;
