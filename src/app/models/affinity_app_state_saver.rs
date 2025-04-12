use serde::{Deserialize, Serialize};
use crate::app::models::core_group::CoreGroup;


#[derive(Serialize, Deserialize)]
pub struct AffinityAppStateSaver {
    pub groups: Vec<CoreGroup>,
    pub clusters: Vec<Vec<usize>>,
    pub theme_index: usize,
}

impl AffinityAppStateSaver {
    pub fn load_state() -> AffinityAppStateSaver {
        let path = std::env::current_exe().map(|mut p| {
            p.set_file_name("state.json");
            p
        }).unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path).ok()
            .and_then(|data| serde_json::from_str::<AffinityAppStateSaver>(&data).ok())
            .unwrap_or_else(|| AffinityAppStateSaver { groups: Vec::new(), clusters: Vec::new(), theme_index: 0 })
    }

    pub fn save_state(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self) {
            let _ = std::fs::write("state.json", json);
        }
    }
}