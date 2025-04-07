use std::path::PathBuf;
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

pub trait OsCmdTrait {
    /// If the passed file is a shortcut or another type, returns a tuple with the target path and arguments.
    fn parse_dropped_file(file_path: PathBuf) -> Result<(PathBuf, Vec<String>), String>;

    /// Launches a process at the specified path with arguments and sets the affinity for the specified cores.
    /// Returns Ok(()) on success or Err with an error description.
    fn run(file_path: PathBuf, args: Vec<String>, cores: &[usize], priority: PriorityClass) -> Result<(), String>;
}

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "windows")]
pub use windows::OsCmd;

#[cfg(not(target_os = "windows"))]
mod linux;
#[cfg(not(target_os = "windows"))]
pub use linux::OsCmd;
