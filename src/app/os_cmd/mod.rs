use std::path::PathBuf;

pub trait OsCmdTrait {
    fn run(file_path: PathBuf, args: Vec<String>, cores: &[usize]) -> Result<(), String>;
    fn parse_dropped_file(file_path: PathBuf) -> Option<(PathBuf, Vec<String>)>;
}

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::OsCmd;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::OsCmd;