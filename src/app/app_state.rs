use super::os_cmd::{OsCmd, OsCmdTrait};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppToRun {
    pub name: String,
    pub dropped_path: PathBuf,
    pub args: Vec<String>,
    pub bin_path: PathBuf,
}

impl AppToRun {
    pub fn new(dropped_path: PathBuf, args: Vec<String>, bin_path: PathBuf) -> Self {
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
            bin_path 
        }
    }

    pub fn display(&self) -> String {
        format!("{} {}(src: {})", self.bin_path.display(), self.args.join(" "), self.dropped_path.display())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    pub name: String,
    pub cores: Vec<usize>,
    pub programs: Vec<AppToRun>,
}

impl CoreGroup {
    pub fn add_app_to_group(&mut self, dropped_paths: Vec<std::path::PathBuf>) {
        if dropped_paths.is_empty() {
            return;
        }

        for path in dropped_paths {
            let parsed_app_file = OsCmd::parse_dropped_file(path.clone());

            if let Some((target, args)) = parsed_app_file {
                let app_to_run = AppToRun::new(
                    path, 
                    args, 
                    target
                );

                self.programs.push(app_to_run);
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AppState {
    pub groups: Vec<CoreGroup>,
}

impl AppState {
    pub fn load_state() -> AppState {
        let path = std::env::current_exe().map(|mut p| {
            p.set_file_name("state.json");
            p
        }).unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path).ok()
            .and_then(|data| serde_json::from_str::<AppState>(&data).ok())
            .unwrap_or_else(|| AppState { groups: vec![] })
    }

    pub fn save_state(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self) {
            let _ = std::fs::write("state.json", json);
        }
    }
}