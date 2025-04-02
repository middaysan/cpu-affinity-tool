use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreGroup {
    pub name: String,
    pub cores: Vec<usize>,
    pub programs: Vec<PathBuf>,
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