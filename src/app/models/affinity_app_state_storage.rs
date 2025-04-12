use serde::{Deserialize, Serialize};
use crate::app::models::core_group::CoreGroup;


#[derive(Serialize, Deserialize)]
pub struct AffinityAppStateStorage {
    pub groups: Vec<CoreGroup>,
    pub clusters: Vec<Vec<usize>>,
    pub theme_index: usize,
}

impl AffinityAppStateStorage {
    pub fn load_state() -> AffinityAppStateStorage {
        let path = std::env::current_exe().map(|mut p| {
            p.set_file_name("state.json");
            p
        }).unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path).ok()
            .and_then(|data| serde_json::from_str::<AffinityAppStateStorage>(&data).ok())
            .unwrap_or_else(|| {
                let default_state = AffinityAppStateStorage { groups: Vec::new(), clusters: Vec::new(), theme_index: 0 };
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&default_state).unwrap_or_default());
                default_state
            })
    }

    pub fn save_state(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self) {
            let _ = std::fs::write("state.json", json);
        }
    }
}