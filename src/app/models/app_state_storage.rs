use serde::{Deserialize, Serialize};
use crate::app::models::core_group::CoreGroup;

/// Storage for persistent application state that can be serialized to and deserialized from JSON.
/// This structure is responsible for saving and loading the application state between sessions.
#[derive(Serialize, Deserialize)]
pub struct AppStateStorage {
    /// List of core groups defined by the user
    pub groups: Vec<CoreGroup>,
    /// CPU clusters configuration (groups of cores that belong to the same physical CPU)
    pub clusters: Vec<Vec<usize>>,
    /// Index of the currently selected UI theme (0: default, 1: light, 2: dark)
    pub theme_index: usize,
}

impl AppStateStorage {
    /// Loads the application state from a JSON file.
    /// 
    /// Attempts to read the state from a file named "state.json" located in the same directory
    /// as the executable. If the file doesn't exist or can't be parsed, it creates a default state
    /// with empty groups and clusters, and theme_index set to 0.
    /// 
    /// # Returns
    /// 
    /// An `AppStateStorage` instance is either loaded from the file or created with default values.
    pub fn load_state() -> AppStateStorage {
        let path = std::env::current_exe().map(|mut p| {
            p.set_file_name("state.json");
            p
        }).unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path).ok()
            .and_then(|data| serde_json::from_str::<AppStateStorage>(&data).ok())
            .unwrap_or_else(|| {
                let default_state = AppStateStorage { groups: Vec::new(), clusters: Vec::new(), theme_index: 0 };
                let _ = std::fs::write(&path, serde_json::to_string_pretty(&default_state).unwrap_or_default());
                default_state
            })
    }

    /// Saves the current application state to a JSON file.
    /// 
    /// Serializes the current state to JSON and writes it to a file named "state.json"
    /// in the current directory. If serialization or writing fails, the error is silently ignored.
    pub fn save_state(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self) {
            let _ = std::fs::write("state.json", json);
        }
    }
}