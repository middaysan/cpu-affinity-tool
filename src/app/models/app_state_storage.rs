mod migrations;
mod schema_refresh;
mod state_path;
mod storage_io;

#[cfg(test)]
mod tests;

use crate::app::models::core_group::CoreGroup;
use crate::app::models::cpu_schema::CpuSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Current version of the application state schema.
pub const CURRENT_APP_STATE_VERSION: u32 = 4;

/// Storage for persistent application state that can be serialized to and deserialized from JSON.
/// This structure is responsible for saving and loading the application state between sessions.
#[derive(Serialize, Deserialize, Clone)]
pub struct AppStateStorage {
    /// Version of the application state schema
    /// Used for migrations between different versions
    pub version: u32,
    /// List of core groups defined by the user
    pub groups: Vec<CoreGroup>,
    /// CPU schema configuration
    pub cpu_schema: CpuSchema,
    /// Index of the currently selected UI theme (0: default, 1: light, 2: dark)
    pub theme_index: usize,
    /// Flag indicating whether process monitoring is enabled
    #[serde(default)]
    pub process_monitoring_enabled: bool,
}

impl AppStateStorage {
    /// Loads the application state from the default JSON file.
    pub fn load_state() -> AppStateStorage {
        let path = state_path::get_state_path();
        Self::load_from_path(&path)
    }

    fn load_from_path(path: &Path) -> AppStateStorage {
        storage_io::read_state_file(path)
            .and_then(|data| migrations::load_from_data(&data, path))
            .unwrap_or_else(|| {
                storage_io::backup_state_file(path);

                let default_state = schema_refresh::build_default_state();
                let _ = default_state.save_to_path(path);

                default_state
            })
    }

    fn save_to_path(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        storage_io::save_to_path(self, path)
    }

    #[cfg_attr(test, allow(dead_code))]
    pub fn try_save_state(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = state_path::get_state_path();
        self.save_to_path(&path)
    }
}
