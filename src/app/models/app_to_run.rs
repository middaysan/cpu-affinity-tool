use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use os_api::PriorityClass;

pub struct RunAppEditState {
    pub current_edit: Option<AppToRun>,
    pub run_settings: Option<(usize, usize)>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppToRun {
    pub name: String,
    pub dropped_path: PathBuf,
    pub args: Vec<String>,
    pub bin_path: PathBuf,
    pub priority: PriorityClass,
}

impl AppToRun {
    pub fn new(dropped_path: PathBuf, args: Vec<String>, bin_path: PathBuf, priority: PriorityClass) -> Self {
        let name = dropped_path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string()
            .rsplit('.')
            .next_back().unwrap().to_string();

        Self { 
            name,
            dropped_path, 
            args, 
            bin_path,
            priority,
        }
    }

    pub fn display(&self) -> String {
        format!("{} {}(src: {}) P({:?})", self.bin_path.display(), self.args.join(" "), self.dropped_path.display(), self.priority)
    }
}