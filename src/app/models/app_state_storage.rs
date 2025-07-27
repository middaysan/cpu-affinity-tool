use crate::app::models::core_group::CoreGroup;
use serde::{Deserialize, Serialize};

/// Current version of the application state schema
pub const CURRENT_APP_STATE_VERSION: u32 = 2;

/// Storage for persistent application state that can be serialized to and deserialized from JSON.
/// This structure is responsible for saving and loading the application state between sessions.
#[derive(Serialize, Deserialize, Clone)]
pub struct AppStateStorage {
    /// Version of the application state schema
    /// Used for migrations between different versions
    pub version: u32,
    /// List of core groups defined by the user
    pub groups: Vec<CoreGroup>,
    /// CPU clusters configuration (groups of cores that belong to the same physical CPU)
    pub clusters: Vec<Vec<usize>>,
    /// Index of the currently selected UI theme (0: default, 1: light, 2: dark)
    pub theme_index: usize,
    /// Flag indicating whether process monitoring is enabled
    #[serde(default)]
    pub process_monitoring_enabled: bool,
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
        let path = std::env::current_exe()
            .map(|mut p| {
                p.set_file_name("state.json");
                p
            })
            .unwrap_or_else(|_| "state.json".into());

        std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| {
                // Try to parse as the current version
                let parsed_result = serde_json::from_str::<AppStateStorage>(&data);

                if let Ok(mut state) = parsed_result {
                    // Check if we need to migrate from an older version
                    if state.version < CURRENT_APP_STATE_VERSION {
                        // Currently we're just updating the version number
                        // In the future, more complex migrations can be added here
                        state.version = CURRENT_APP_STATE_VERSION;

                        // Save the migrated state back to disk
                        if let Ok(json) = serde_json::to_string_pretty(&state) {
                            let _ = std::fs::write(&path, json);
                        }
                    }
                    Some(state)
                } else {
                    // Try to parse as a legacy version (without version field)
                    #[derive(Deserialize)]
                    struct LegacyAppStateStorage {
                        pub groups: Vec<CoreGroup>,
                        pub clusters: Vec<Vec<usize>>,
                        pub theme_index: usize,
                    }

                    let legacy_result = serde_json::from_str::<LegacyAppStateStorage>(&data);

                    if let Ok(legacy_state) = legacy_result {
                        // Migrate from legacy to current version
                        let migrated_state = AppStateStorage {
                            version: CURRENT_APP_STATE_VERSION,
                            groups: legacy_state.groups,
                            clusters: legacy_state.clusters,
                            theme_index: legacy_state.theme_index,
                            process_monitoring_enabled: false, // Default to disabled for migrated states
                        };

                        // Save the migrated state back to disk
                        if let Ok(json) = serde_json::to_string_pretty(&migrated_state) {
                            let _ = std::fs::write(&path, json);
                        }

                        Some(migrated_state)
                    } else {
                        None
                    }
                }
            })
            .unwrap_or_else(|| {
                // Create a new default state with the current version
                let default_state = AppStateStorage {
                    version: CURRENT_APP_STATE_VERSION,
                    groups: Vec::new(),
                    clusters: Vec::new(),
                    theme_index: 0,
                    process_monitoring_enabled: false, // Default to disabled
                };

                // Save the default state to disk
                let _ = std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&default_state).unwrap_or_default(),
                );

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
